package main

// 数据库安全快照工具（走 SQLite 的 `VACUUM INTO`，实现见 backend-go/snapshot 包）。
//
// 背景：本项目的两个库都跑在 WAL 模式下，最近的写入可能还留在 -wal 文件里，
// 主库文件本身可能是旧的、甚至几乎是空的（backend-rust/auth.db 只有 4 KB，
// 而旁边的 auth.db-wal 有 3.7 MB）。所以**「复制 .db 文件」不是备份**，会丢数据。
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
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"backend-go/snapshot"
)

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

	absRoot, err := snapshot.SafeRoot(*outRoot)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}

	target, err := snapshot.NewDir(absRoot)
	if err != nil {
		fmt.Fprintf(os.Stderr, "创建快照目录失败：%v\n", err)
		os.Exit(1)
	}

	failed := 0
	for _, src := range dbs {
		if err := snapshot.Snapshot(src, target); err != nil {
			fmt.Printf("✗ %s：%v\n", src, err)
			failed++
		}
	}

	if *keep > 0 {
		snapshot.Prune(absRoot, *keep)
	}

	if failed > 0 {
		fmt.Printf("\n有 %d 个库备份失败\n", failed)
		os.Exit(1)
	}
	fmt.Printf("\n快照目录：%s\n", target)
}

// multiFlag 让 -db 可以重复出现。
type multiFlag []string

func (m *multiFlag) String() string { return strings.Join(*m, ", ") }

func (m *multiFlag) Set(v string) error {
	*m = append(*m, v)
	return nil
}
