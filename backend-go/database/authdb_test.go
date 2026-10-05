// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
package database

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/glebarez/sqlite"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"
)

// 本文件盯住「账号库只读访问」的几条硬约束（都是线上真会出现的状态）：
//   - mode=ro 真的生效：写操作必须失败，打不开的路径不能被顺手创建出来；
//   - 任何失败都只返回值错误（判定 + 文案给 handler 用），不 panic；
//   - 失败不缓存：账号服务回来后下一次请求要能自愈。

// newWritableDB 造一个可写的临时 SQLite 库（模拟账号服务那边建的库）。
func newWritableDB(t *testing.T, path string) *gorm.DB {
	t.Helper()
	db, err := gorm.Open(sqlite.Open(path), &gorm.Config{
		Logger: logger.Default.LogMode(logger.Silent),
	})
	if err != nil {
		t.Fatalf("建可写测试库失败: %v", err)
	}
	if err := db.Exec("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL)").Error; err != nil {
		t.Fatalf("建表失败: %v", err)
	}
	if err := db.Exec("INSERT INTO users (id, email) VALUES (1, 'a@example.com'), (2, 'b@example.com')").Error; err != nil {
		t.Fatalf("插数据失败: %v", err)
	}
	return db
}

// TestOpenReadOnlyRejectsWrites 只读打开后必须**写不动**：
// 这是「Go 侧不许碰账号库」这条约束唯一真正的执行者（注释约束不了代码）。
func TestOpenReadOnlyRejectsWrites(t *testing.T) {
	path := filepath.Join(t.TempDir(), "auth.db")
	writer := newWritableDB(t, path)
	if sqlDB, err := writer.DB(); err == nil {
		_ = sqlDB.Close()
	}

	db, err := openReadOnly(path)
	if err != nil {
		t.Fatalf("只读打开已存在的库失败: %v", err)
	}
	defer func() {
		if sqlDB, err := db.DB(); err == nil {
			_ = sqlDB.Close()
		}
	}()

	// 读得到
	var n int64
	if err := db.Raw("SELECT COUNT(*) FROM users").Scan(&n).Error; err != nil {
		t.Fatalf("只读查询失败: %v", err)
	}
	if n != 2 {
		t.Fatalf("users 期望 2 行，实际 %d", n)
	}

	// 写不动
	for _, stmt := range []string{
		"INSERT INTO users (id, email) VALUES (3, 'c@example.com')",
		"DELETE FROM users",
		"CREATE TABLE probe (x INTEGER)",
	} {
		if err := db.Exec(stmt).Error; err == nil {
			t.Fatalf("只读连接上 %q 居然成功了——mode=ro 没生效", stmt)
		} else if !strings.Contains(err.Error(), "readonly") {
			t.Fatalf("%q 期望报 readonly 错误，实际: %v", stmt, err)
		}
	}

	// 顺带确认 busy_timeout 是按 _pragma 设置进来的（账号服务在写时我们等 5 秒再报错）
	var busyTimeout int
	if err := db.Raw("PRAGMA busy_timeout").Scan(&busyTimeout).Error; err != nil {
		t.Fatalf("读 PRAGMA busy_timeout 失败: %v", err)
	}
	if busyTimeout != 5000 {
		t.Fatalf("busy_timeout 期望 5000（与 Rust 侧同口径），实际 %d", busyTimeout)
	}
}

// TestOpenReadOnlyMissingFile 打不开的路径必须返回错误，且**绝不能把它创建出来**。
// 少了 mode=ro 时驱动会以 READWRITE|CREATE 打开，顺手建一个 0 字节空库——
// 对账号库来说那是灾难（空库看起来「读得到」，但任何表都不存在）。
func TestOpenReadOnlyMissingFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "missing.db")

	if _, err := openReadOnly(path); err == nil {
		t.Fatal("打开不存在的库应当失败")
	}
	if _, err := os.Stat(path); err == nil {
		t.Fatal("只读打开把不存在的库创建出来了")
	}
}

// TestAuthDBUnavailableAndRecovers 失败要能判定、能给文案，并且**不缓存失败**：
// 账号服务起来之后（库被放回来），下一次请求必须自己好起来，不用重启 Go 服务。
func TestAuthDBUnavailableAndRecovers(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "auth.db")
	authDB := NewAuthDB(path)
	t.Cleanup(func() { _ = authDB.Close() })

	// ① 库还不存在
	var n int64
	err := authDB.Select(&n, "SELECT COUNT(*) FROM users")
	if err == nil {
		t.Fatal("库不存在时查询应当失败")
	}
	if !IsAuthDBUnavailable(err) {
		t.Fatalf("期望判定为「账号库不可用」，实际错误: %v", err)
	}
	reason := AuthDBUnavailableReason(authDB.Path(), err)
	for _, want := range []string{"AUTH_DB_PATH", path, "账号侧数字一律显示 0"} {
		if !strings.Contains(reason, want) {
			t.Fatalf("错误文案里应当出现 %q，实际：%s", want, reason)
		}
	}

	// ② 账号服务「起来了」：同一个句柄下一次查询就该成功（失败没有被缓存）
	writer := newWritableDB(t, path)
	if sqlDB, dbErr := writer.DB(); dbErr == nil {
		_ = sqlDB.Close()
	}
	if err := authDB.Select(&n, "SELECT COUNT(*) FROM users"); err != nil {
		t.Fatalf("库出现后查询应当成功，实际: %v", err)
	}
	if n != 2 {
		t.Fatalf("users 期望 2 行，实际 %d", n)
	}

	// ③ 查询失败之后要丢掉缓存的连接（下一次重新打开），并且仍然只报「不可用」
	if err := authDB.Select(&n, "SELECT COUNT(*) FROM 不存在的表"); err == nil {
		t.Fatal("查询不存在的表应当失败")
	} else if !IsAuthDBUnavailable(err) {
		t.Fatalf("查询失败也要归一成「账号库不可用」，实际: %v", err)
	}
	if err := authDB.Select(&n, "SELECT COUNT(*) FROM users"); err != nil {
		t.Fatalf("失败后重开连接应当恢复，实际: %v", err)
	}
	if n != 2 {
		t.Fatalf("恢复后 users 期望 2 行，实际 %d", n)
	}
}

// TestAuthDBEmptyPath 路径没配（直接构造 Config 的场景）时报「未配置」而不是去打开空路径名
func TestAuthDBEmptyPath(t *testing.T) {
	authDB := NewAuthDB("")
	var n int64
	err := authDB.Select(&n, "SELECT COUNT(*) FROM users")
	if err == nil || !IsAuthDBUnavailable(err) {
		t.Fatalf("空路径应当判定为不可用，实际: %v", err)
	}
	reason := AuthDBUnavailableReason(authDB.Path(), err)
	for _, want := range []string{"AUTH_DB_PATH=（未配置）", "账号服务可能没在运行"} {
		if !strings.Contains(reason, want) {
			t.Fatalf("文案里应当出现 %q，实际：%s", want, reason)
		}
	}
}

// TestAuthDBNilHandle 没初始化句柄（装配出错）时也只报「不可用」，不 panic：
// 看板少一半数字是可以接受的，把整个 Go 进程带崩不行。
func TestAuthDBNilHandle(t *testing.T) {
	var handle *AuthDB
	var n int64
	err := handle.Select(&n, "SELECT COUNT(*) FROM users")
	if err == nil || !IsAuthDBUnavailable(err) {
		t.Fatalf("nil 句柄应当判定为不可用，实际: %v", err)
	}
	if handle.Path() != "" {
		t.Fatalf("nil 句柄的 Path 应当是空串，实际 %q", handle.Path())
	}
	handle.Invalidate()
	if err := handle.Close(); err != nil {
		t.Fatalf("nil 句柄 Close 不该报错，实际: %v", err)
	}
	if reason := AuthDBUnavailableReason(handle.Path(), err); !strings.Contains(reason, "账号服务可能没在运行") {
		t.Fatalf("文案不对：%s", reason)
	}
}

// TestIsAuthDBUnavailableFalseForOthers 业务库的错误不能被误判成「账号库读不到」——
// 前者要回 500，后者只降级，判错会让看板把真正的故障吞掉。
func TestIsAuthDBUnavailableFalseForOthers(t *testing.T) {
	if IsAuthDBUnavailable(nil) {
		t.Fatal("nil 不该被判成账号库不可用")
	}
	if IsAuthDBUnavailable(gorm.ErrRecordNotFound) {
		t.Fatal("普通错误不该被判成账号库不可用")
	}
}
