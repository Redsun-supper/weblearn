# 硬性边界与工具链（完整说明）

> 原先在 `CLAUDE.md`「硬性边界」一节，2026-09 拆分。`CLAUDE.md` 里保留了精简版红线，
> **本文件是完整说明**：碰到 `备份/`、`pages.zip`、头像素材，或要构建 / 跑测试时读它。

## 1. 备份目录只读

`备份/`（含 `备份1/`、`备份2/`）是存档副本，**永不修改、永不删除、永不作为建设对象**。
读取 / 列出项目文件时一律跳过 `备份` 目录。

## 2. `pages.zip`

二进制备份包，不作为文本读取、不修改、不展开，除非用户明确要求检查。

## 3. `console.log('FAIL'`

一个**空文件**（0 字节），系误用重定向产生的残留：**不是合法代码，不要把它当代码，也不要试图「修复」它**；
除非用户确认，不要删除，也不要修改。

## 4. `go.sum`

后端已有 `go.sum`（GORM + glebarez/sqlite 等依赖已通过 `go mod tidy` 固化）；
新增依赖时用 `go mod tidy` 同步即可。

## 5. `image/avatar.png`（头像素材）

导航栏左上角的头像素材，**可以读、可以看**（助手已具备读图能力，`read_image` 直接读这个文件即可），
但**不要擅自替换**——主题 / 素材更换一律先问用户。

- **本体是真 PNG**（`89 50 4E 47` 文件头，颜色类型 6），**1330 × 1146**，约 **2.56 MB**
  （此前文档记的「WebP、199 KB」是读图工具生成的**归一化副本**格式，不是源文件，别照那个改文件扩展名）。
- 内容：五人合影（特朗普 / 马斯克 / 中间戴墨镜穿灰夹克的男性 / 黄仁勋 / 库克，一起竖大拇指）。
- 页面上的呈现：`index.html` 的 `.rounded-square`（`left: 30px`，垂直居中）内，**显示尺寸仅 64 × 64 px**，
  圆角 12px、3px 白边、`object-fit: cover`（源图左右各裁掉约 7%，五个人的脸都在框内）。`main.css` 的 `.avatar` 只负责填满容器。
- ⚠️ **为什么它偏重**：64×64 的显示尺寸在用 1330×1146 的原图。`dev-server.js` 已经给图片发
  `public, max-age=86400`（不再每次重下），但线上仍是 2.6MB 的首屏负担。优化方向是生成 128×128 缩略图
  给导航与个人中心用（原图保留），**涉及视觉素材，动手前必须先问用户** —— 详见根目录 `TODO.md` 第 2 条。

## 6. 本机工具链

- **Go**：便携版 `C:\Users\22629\go-portable\go\bin\go.exe`（go1.27.1，已
  `go env -w GOPROXY=https://goproxy.cn,direct GOSUMDB=off`，直接 `go build` 即可）。
- **Rust**：`cargo 1.97`，`wasm32-unknown-unknown` target 已装。
- **wasm-bindgen CLI**：`C:\Users\22629\.local\bin\wasm-bindgen-0.2.128-*\wasm-bindgen.exe`
  （**须与 Cargo.toml 的 wasm-bindgen 版本一致 0.2.128**）。
- ✅ **原生（宿主）Rust 也能编译链接**：host 目标为 `x86_64-pc-windows-gnu`，mingw gcc/ar 已在 PATH 上，
  所以 `backend-rust/` 用 `rusqlite` 的 `bundled` 特性（现场编译 sqlite3.c）可以正常 `cargo test` / `cargo build --release`。
  链接时的 `corrupt .drectve at end of def file` 是 mingw 的**无害告警**。
- ✅ **宿主 `cargo test` 现在可以运行**（本文档此前记录的「缺 mingw `as`/MSVC SDK 无法链接」**已不再成立**）。
  ⚠️ **两套测试的数字别混用**（2026-09 复测）：`backend-rust/` = **90 项**（41 单元 + 49 集成），
  `modules/english/engine/` = **118 项**；此前文档里那个「118」是**引擎**的，不是账号服务的。
- 完整验证路径：`pwsh scripts/verify.ps1`（一键跑 Go 构建/vet/测试 + 上面两套测试 + wasm32 目标检查），
  或手工：`cargo test` → `cargo check --target wasm32-unknown-unknown` → `cargo build --target wasm32-unknown-unknown --release`
  → `wasm-bindgen` 生成 `pkg/` → 浏览器端到端。
- **数据库备份**：`pwsh scripts/backup.ps1`（→ `backend-go/cmd/backup/main.go`）对 `guangxue.db` 与 `backend-rust/auth.db` 执行 SQLite
  `VACUUM INTO` 快照，产物落在 `backups/<时间戳>/`（已 gitignore），`-Keep N` 只保留最近 N 份（只删输出目录下形如 `20260930-225615` 的子目录）。
  ⚠️ **别用「复制 `.db` 文件」当备份**：实测磁盘上 `auth.db` 只有 4 KB、安全快照是 124 KB —— WAL 里那部分直接拷贝会丢（约 97%）。
- ⚠️ 但 `JsValue` 在非 wasm32 目标上未实现（调用即 `panic: function not implemented on non-wasm32 targets`，
  无法 unwinding 会直接 abort）：**纯计算层不要碰 `JsValue`**，把它留在 wasm 导出方法的边界上。

## 7. `word_reviews` 行是懒创建

单词由 `POST /api/words` 写入 `words` 表；首次提交复习时才创建对应 `word_reviews` 行。
`/api/reviews/new` = 无复习行的词。

## 相关文档

- 复习引擎构建细节 → [`review-engine.md`](review-engine.md)
- 接口清单 → [`admin-api.md`](admin-api.md)
