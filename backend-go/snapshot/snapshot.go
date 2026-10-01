// Package snapshot 提供 SQLite 的安全快照（`VACUUM INTO`）与快照目录的整理。
//
// 为什么「复制 .db 文件」不算备份：两个库都跑在 WAL 模式下，最近的写入可能还留在
// -wal 文件里，主库文件本身可能是旧的、甚至几乎是空的（backend-rust/auth.db 只有
// 4 KB，而旁边的 auth.db-wal 有 3.7 MB）。`VACUUM INTO` 在一次读事务里把整个库
// （含 WAL 中已提交的内容）压实写成一个新文件，而且**不修改源库**——服务在跑也能安全执行。
//
// 调用方：`cmd/backup`（命令行备份工具）与 `cmd/migrate`（迁移前自动留一份底）。
// 本包会直接往 stdout 打印进度——两个调用方都是命令行工具，没有必要再把输出回传一层。
package snapshot

import (
	"database/sql"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"

	"github.com/glebarez/sqlite" // 纯 Go SQLite 驱动，同时向 database/sql 注册了 "sqlite"
)

// StampPattern 是快照目录的命名格式；Prune 只认这个格式的子目录，别的一律不碰。
var StampPattern = regexp.MustCompile(`^\d{8}-\d{6}$`)

// SafeRoot 校验快照输出根目录并返回绝对路径。
// 安全闸沿用 scripts/clean-build-cache.ps1 的约定：永不把东西写进名字带「备份」的目录。
func SafeRoot(root string) (string, error) {
	abs, err := filepath.Abs(root)
	if err != nil {
		return "", err
	}
	if strings.Contains(abs, "备份") {
		return "", fmt.Errorf("拒绝：输出目录不能落在只读的「备份」目录里 -> %s", abs)
	}
	return abs, nil
}

// NewDir 在 root 下新建一个 <时间戳>/ 目录并返回其绝对路径。
func NewDir(root string) (string, error) {
	dir := filepath.Join(root, time.Now().Format("20060102-150405"))
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return "", err
	}
	return filepath.Abs(dir)
}

// Snapshot 把一个库压实写入 outDir；源文件不存在时跳过（不算失败）。
func Snapshot(src, outDir string) error {
	if _, err := os.Stat(src); err != nil {
		if errors.Is(err, os.ErrNotExist) {
			fmt.Printf("- 跳过 %s（文件不存在）\n", src)
			return nil
		}
		return err
	}

	// 源库可能开着 WAL：-wal/-shm 与主库同目录，VACUUM INTO 会把已提交内容一并读出来。
	db, err := sql.Open(sqlite.DriverName, src)
	if err != nil {
		return err
	}
	defer db.Close()
	db.SetMaxOpenConns(1)

	name := strings.TrimSuffix(filepath.Base(src), filepath.Ext(src))
	dst, err := filepath.Abs(filepath.Join(outDir, name+".db"))
	if err != nil {
		return err
	}
	if _, err := os.Stat(dst); err == nil {
		return fmt.Errorf("目标已存在，不覆盖：%s", dst)
	}

	// VACUUM INTO 的目标只能写成 SQL 字面量：单引号翻倍即可；Windows 路径里的
	// 反斜杠在 SQLite 字符串中是普通字符，不需要转义。
	stmt := "VACUUM INTO '" + strings.ReplaceAll(dst, "'", "''") + "'"
	if _, err := db.Exec(stmt); err != nil {
		return fmt.Errorf("VACUUM INTO 失败：%w", err)
	}

	// 备份要能自己证明是好的：对副本跑 integrity_check，并数一下表数量。
	tables, err := Check(dst)
	if err != nil {
		return fmt.Errorf("副本校验失败：%w", err)
	}
	info, err := os.Stat(dst)
	if err != nil {
		return err
	}
	fmt.Printf("✓ %-34s → %s（%.0f KB，%d 张表，integrity_check ok）\n",
		filepath.ToSlash(src), filepath.Base(dst), float64(info.Size())/1024, tables)
	return nil
}

// Check 对副本做完整性检查，返回表数量。
func Check(path string) (int, error) {
	db, err := sql.Open(sqlite.DriverName, path)
	if err != nil {
		return 0, err
	}
	defer db.Close()

	var verdict string
	if err := db.QueryRow("PRAGMA integrity_check").Scan(&verdict); err != nil {
		return 0, err
	}
	if verdict != "ok" {
		return 0, fmt.Errorf("integrity_check 返回 %q", verdict)
	}

	var tables int
	if err := db.QueryRow(
		"SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
	).Scan(&tables); err != nil {
		return 0, err
	}
	return tables, nil
}

// Prune 只保留最近 keep 份快照。
// 严格限定「输出根目录下、名字符合 <时间戳> 格式」的子目录，其他文件与目录一律不动，
// 免得手滑把别的东西删掉。
func Prune(root string, keep int) {
	entries, err := os.ReadDir(root)
	if err != nil {
		return
	}
	var stamps []string
	for _, e := range entries {
		if e.IsDir() && StampPattern.MatchString(e.Name()) {
			stamps = append(stamps, e.Name())
		}
	}
	if len(stamps) <= keep {
		return
	}
	// 时间戳格式本身可排序，升序即「由旧到新」
	sortStrings(stamps)
	for _, name := range stamps[:len(stamps)-keep] {
		p := filepath.Join(root, name)
		if err := os.RemoveAll(p); err != nil {
			fmt.Printf("- 清理旧快照 %s 失败：%v\n", name, err)
			continue
		}
		fmt.Printf("- 已清理旧快照 %s\n", name)
	}
}

// sortStrings 是最小插入排序：快照数量是两位数级别，性能无关紧要，
// 不值得为几行代码多引一个 sort 包。
func sortStrings(s []string) {
	for i := 1; i < len(s); i++ {
		for j := i; j > 0 && s[j] < s[j-1]; j-- {
			s[j], s[j-1] = s[j-1], s[j]
		}
	}
}
