// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package database

import (
	"errors"
	"fmt"
	"path/filepath"
	"strings"
	"sync"

	"github.com/glebarez/sqlite"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"
)

// 账号库（账号服务 backend-rust 的 auth.db）的**只读**访问。
//
// 为什么必须只读：auth.db 的写入方只有账号服务一个进程，Go 侧只是「管理看板要读几个数字」。
// 一旦以读写方式打开，就可能与账号服务的写事务撞在一起（SQLITE_BUSY 会让**用户的登录 / 注册**
// 失败），也存在改坏这个库的风险 —— 而它没有词库那样的重建余地。
// 只读打开 + 只发 SELECT 把这条风险从「靠自觉」变成「驱动层面不允许」。
//
// ⚠️ 只读打开不是万无一失的，失败**一定会发生**：
//   - 路径配错 / 库被搬走 → 打不开；
//   - 账号服务停了、库还是 WAL 模式而 -shm / -wal 伴随文件被清掉 → SQLite 只读连接同样打不开；
//   - 库正在写 → 短暂 SQLITE_BUSY（靠 busy_timeout 等 5 秒）。
//
// 所以这一层的契约是：**任何失败都只返回值错误**，由 handler 降级成「账号侧数字全 0 + 一句原因」。
// 绝不允许 panic、也绝不允许把 Go 服务拖得起不来 —— 看板的数字不值得拿整个后端去赌。
// 判定与文案分别由 IsAuthDBUnavailable / AuthDBUnavailableReason 提供。

// authDBReadOnlyParams 只读 DSN 的查询参数（三段各有用途，缺一不可）：
//   - mode=ro：真正的只读。实测**没有它时打开一个不存在的路径会把空库创建出来**
//     （驱动默认 READWRITE|CREATE），对账号库来说那是灾难；
//   - _pragma=busy_timeout(5000)：账号服务正在写时先等 5 秒再报错，与 Rust 侧
//     `busy_timeout(5000)`（backend-rust/src/db.rs:67）同口径。实测该参数生效：
//     回读 `PRAGMA busy_timeout` 得到配置值。
const authDBReadOnlyParams = "?mode=ro&_pragma=busy_timeout(5000)"

// AuthDB 账号库的只读句柄：惰性打开，成功的结果缓存下来。
//
// 失败**不**缓存：账号服务重启、库被放回来之后，下一次请求就能自愈，
// 不必等 Go 服务重启（线上看板最常见的就是「账号服务没起来 → 看板一片 0」）。
type AuthDB struct {
	path string

	mu sync.Mutex
	db *gorm.DB
}

// NewAuthDB 建一个只读句柄；此时**不**碰文件（路径为空也算合法，调用时会报「未配置」）
func NewAuthDB(path string) *AuthDB {
	return &AuthDB{path: path}
}

// Path 返回配置的账号库路径（handler 用它拼错误文案）。
// nil 接收者也安全：看板宁可报「不可用」，也不该因为一个没初始化的句柄 panic。
func (a *AuthDB) Path() string {
	if a == nil {
		return ""
	}
	return a.path
}

// DB 返回只读连接；打不开时返回 *AuthDBUnavailableError。
func (a *AuthDB) DB() (*gorm.DB, error) {
	if a == nil {
		return nil, &AuthDBUnavailableError{Err: errors.New("账号库句柄未初始化")}
	}

	a.mu.Lock()
	defer a.mu.Unlock()

	if a.db != nil {
		return a.db, nil
	}
	if strings.TrimSpace(a.path) == "" {
		return nil, &AuthDBUnavailableError{Path: a.path, Err: errors.New("AUTH_DB_PATH 未配置")}
	}
	db, err := openReadOnly(a.path)
	if err != nil {
		return nil, &AuthDBUnavailableError{Path: a.path, Err: err}
	}
	a.db = db
	return db, nil
}

// Select 在只读连接上执行一条 SELECT 并把结果扫进 dest。
//
// 只提供这一个入口是刻意的：账号库**不允许**出现第二个执行 SQL 的地方，
// 也就不会有人无意间在这里写 INSERT / UPDATE。任何失败都归一成
// *AuthDBUnavailableError，调用方按「账号侧不可用」降级即可。
func (a *AuthDB) Select(dest interface{}, query string, args ...interface{}) error {
	db, err := a.DB()
	if err != nil {
		return err
	}
	if err := db.Raw(query, args...).Scan(dest).Error; err != nil {
		// 查询失败说明这条连接已经不可信（库被换掉 / 伴随文件消失）：
		// 丢掉它，下一次请求重新打开，而不是一直拿着坏连接报错。
		a.Invalidate()
		return &AuthDBUnavailableError{Path: a.path, Err: err}
	}
	return nil
}

// Invalidate 丢弃缓存的连接（查询失败后调用；也用于「账号库被换了一份」的场景）
func (a *AuthDB) Invalidate() {
	if a == nil {
		return
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.db == nil {
		return
	}
	if sqlDB, err := a.db.DB(); err == nil {
		_ = sqlDB.Close()
	}
	a.db = nil
}

// Close 关掉缓存的连接（进程退出 / 测试收尾用）
func (a *AuthDB) Close() error {
	if a == nil {
		return nil
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.db == nil {
		return nil
	}
	sqlDB, err := a.db.DB()
	a.db = nil
	if err != nil {
		return err
	}
	return sqlDB.Close()
}

// openReadOnly 以只读方式打开一个 SQLite 库。
//
// `file:` 前缀是 URL 形式查询参数的前提；Windows 路径要先把 `\` 换成 `/`，
// 否则反斜杠会被当成 URI 里的转义字符（实测 DSN 里必须是正斜杠）。
// 连接池压到 1：看板是低频只读查询，串行反而少踩 SQLite 的锁。
func openReadOnly(path string) (*gorm.DB, error) {
	dsn := "file:" + filepath.ToSlash(path) + authDBReadOnlyParams
	db, err := gorm.Open(sqlite.Open(dsn), &gorm.Config{
		// 账号库的查询不打业务日志：它是别人的库，报错文案由 handler 降级后给出，
		// 这里再刷一遍 GORM 的 SQL 日志只会淹没真正的告警。
		Logger: logger.Default.LogMode(logger.Silent),
	})
	if err != nil {
		return nil, fmt.Errorf("只读打开 %s 失败: %w", path, err)
	}
	if sqlDB, err := db.DB(); err == nil {
		sqlDB.SetMaxOpenConns(1)
		sqlDB.SetMaxIdleConns(1)
	}
	return db, nil
}

// AuthDBUnavailableError 「账号库读不到」。
// 带上路径与底层原因，好让 handler 给出「读的是哪个库、该去配什么」的可读文案。
type AuthDBUnavailableError struct {
	Path string
	Err  error
}

func (e *AuthDBUnavailableError) Error() string {
	return fmt.Sprintf("账号库不可用（%s）: %v", e.Path, e.Err)
}

// Unwrap 让 errors.Is / errors.As 能穿透到底层错误（例如 os.ErrNotExist）
func (e *AuthDBUnavailableError) Unwrap() error { return e.Err }

// IsAuthDBUnavailable 判定一个错误是不是「账号库读不到」——
// handler 用它与「业务库查询失败」（那要回 500）区分开：账号库读不到只降级，不回 500。
func IsAuthDBUnavailable(err error) bool {
	var target *AuthDBUnavailableError
	return errors.As(err, &target)
}

// AuthDBUnavailableReason 把「账号库不可用」写成给前端看的一句中文（填进 data.auth_db.error）。
//
// 这段文案会直接显示在管理看板上，所以要说清三件事：读的是哪个库、影响是什么、多半是什么原因。
// 光写「账号库不可用」会让管理员无从下手 —— 线上看板一片 0 的时候，第一句要能告诉他去查什么。
func AuthDBUnavailableReason(path string, err error) string {
	var unavailable *AuthDBUnavailableError
	if errors.As(err, &unavailable) {
		if unavailable.Path != "" {
			path = unavailable.Path
		}
		err = unavailable.Err
	}
	shown := strings.TrimSpace(path)
	if shown == "" {
		shown = "（未配置）"
	}
	reason := "未知原因"
	if err != nil {
		reason = err.Error()
	}
	return "账号库读不到（AUTH_DB_PATH=" + shown + "，账号服务可能没在运行）：" +
		"账号侧数字一律显示 0，业务侧数字（复习量 / 词条）不受影响。原因：" + reason
}
