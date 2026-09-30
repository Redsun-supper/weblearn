package main

// 数据库安全快照工具（走 SQLite 的 VACUUM INTO）
//
// 背景：本项目的两个库都跑在 WAL 模式下，最近的写入可能还留在 -wal 文件里，
// 主库文件本身可能是旧的、甚至几乎是空的（backend-rust/auth.db 只有 4 KB，
// 而旁边的 auth.db-wal 有 3.7 MB）。所以**「复制 .db 文件」不是备份**，会丢数据。
//
// 这里的做法是 SQLite 官方推荐的 `VACUUM INTO`：它在一次读事务里把整个库
// （含 WAL 中已提交的内容）压实写成一个新文件，而且**不修改源库**——
// 服务正在跑的时候也能安全执行。
//
// 用法（在 backend-go/ 目录下）：
//
//	go run ./cmd/backup                     # 快照两个库到 <仓库>/backups/<时间戳>/
//	go run ./cmd/backup -out D:\gx-backup   # 指定输出根目录
//	go run ./cmd/backup -db ../x.db         # 只备份指定库（-db 可重复）
//	go run ./cmd/backup -keep 10            # 只保留最近 10 份
//
// 退出码：0 = 全部成功；1 = 有任一库失败（方便脚本判断）。
//
// 更常用的入口是包装脚本：`pwsh scripts/backup.ps1`（负责找 Go 与拼路径）。

import (
	"database/sql"
	"errors"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"

	"github.com/glebarez/sqlite" // 纯 Go SQLite 驱动，同时向 database/sql 注册了 "sqlite"
)

// 快照目录的命名格式，prune 只认这个格式的子目录（别的一律不碰）
var stampPattern = regexp.MustCompile(`^\d{8}-\d{6}$`)

func main() {
	var (
		outRoot = flag.String("out", filepath.Join("..", "backups"), "快照输出根目录；每次运行会在其下新建 <时间戳>/ 子目录")
		keep    = flag.Int("keep", 0, "只保留最近 N 份快照（0 = 不清理）")
	)
	var dbs multiFlag
	flag.Var(&dbs, "db", "要备份的库文件路径，可重复指定；默认 guangxue.db 与 ../backend-rust/auth.db")
	flag.Parse()

	if len(dbs) == 0 {
		dbs = multiFlag{"guangxue.db", filepath.Join("..", "backend-rust", "auth.db")}
	}

	// 安全闸：与 scripts/clean-build-cache.ps1 同一套约定——永不碰「备份」目录
	if abs, _ := filepath.Abs(*outRoot); strings.Contains(abs, "备份") {
		fmt.Fprintf(os.Stderr, "拒绝：输出目录不能落在只读的「备份」目录里 -> %s\n", abs)
		os.Exit(1)
	}

	target := filepath.Join(*outRoot, time.Now().Format("20060102-150405"))
	if err := os.MkdirAll(target, 0o755); err != nil {
		fmt.Fprintf(os.Stderr, "创建快照目录失败：%v\n", err)
		os.Exit(1)
	}

	failed := 0
	for _, src := range dbs {
		if err := snapshot(src, target); err != nil {
			fmt.Printf("✗ %s：%v\n", src, err)
			failed++
		}
	}

	if *keep > 0 {
		prune(*outRoot, *keep)
	}

	if failed > 0 {
		fmt.Printf("\n有 %d 个库备份失败\n", failed)
		os.Exit(1)
	}
	abs, _ := filepath.Abs(target)
	fmt.Printf("\n快照目录：%s\n", abs)
}

// snapshot 把一个库压实写入 outDir；源文件不存在时跳过（不算失败）。
func snapshot(src, outDir string) error {
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
	tables, err := check(dst)
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

// check 对副本做完整性检查，返回表数量。
func check(path string) (int, error) {
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

// prune 只保留最近 keep 份快照。
// 严格限定「输出根目录下、名字符合 <时间戳> 格式」的子目录，其他文件与目录一律不动，
// 免得手滑把别的东西删掉。
func prune(root string, keep int) {
	entries, err := os.ReadDir(root)
	if err != nil {
		return
	}
	var stamps []string
	for _, e := range entries {
		if e.IsDir() && stampPattern.MatchString(e.Name()) {
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

// sortStrings 是最小插入排序：只为避免为几行代码多引一个 sort 包依赖的观感问题，
// 快照数量是两位数级别，性能无关紧要。
func sortStrings(s []string) {
	for i := 1; i < len(s); i++ {
		for j := i; j > 0 && s[j] < s[j-1]; j-- {
			s[j], s[j-1] = s[j-1], s[j]
		}
	}
}

// multiFlag 让 -db 可以重复出现。
type multiFlag []string

func (m *multiFlag) String() string { return strings.Join(*m, ", ") }

func (m *multiFlag) Set(v string) error {
	*m = append(*m, v)
	return nil
}
