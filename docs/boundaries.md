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
- ✅ **当前状态（2026-10-02，用户要求「头像暂时设置成无」）**：页面不再引用本文件，两个 `<img>` 都指向
  `image/avatar-blank.png`（1×1 全透明、过滤字节 0、RGBA=0,0,0,0，**68 B**）——`index.html` 的
  `#navAvatar` 与 `account/index.html` 的 `.acc-hero-avatar`。**原图未删未改**
  （`E:\porject\4\image\avatar.png`，2,685,282 B，mtime 仍是 2026-08-29 21:15:34）。
  恢复方式：把这两处 `src` 改回 `image/avatar.png` / `../image/avatar.png` 即可，**不要**改按钮结构或
  `main.js` 的过场 —— 扩散遮罩读的是 `#navAvatar` 的 `getBoundingClientRect()`（`main.js:298`），与图片素材无关。
  副作用（正面）：首屏字节数从 3,103,040 B 降到约 **418 KB**（原图占其中 87%）。
  踩过的坑：第一版占位图是凭记忆贴的 base64，解出来是**蓝紫色**（filter=1 Sub + `G=0 B=255 A=127`），
  无头浏览器截图里那块蓝方块就是它 —— 生成/替换图片素材后**必须把 IDAT 解回来验像素**，不能只看文件头。

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
  ⚠️ **三套测试的数字别混用**（2026-10-02 复测）：`backend-rust/` = **90 项**（41 单元 + 49 集成），
  `modules/english/engine/` = **120 项**（118 + 单一循环池新增的置顶卡重置与 365 天封顶两条），
  `backend-go/` = **61 项**（handlers 43 + middleware 10 + routes 7 + database 1；
  阶段 1 补关键路径基线，阶段 2 补鉴权 / CSRF / 路由回归，P0-1 补进度按人隔离与迁移回归）；
  此前文档里那个「118」是**引擎**的，不是账号服务的。
- 完整验证路径：`pwsh scripts/verify.ps1`（一键跑 Go 构建/vet/测试 + 上面两套测试 + wasm32 目标检查），
  或手工：`cargo test` → `cargo check --target wasm32-unknown-unknown` → `cargo build --target wasm32-unknown-unknown --release`
  → `wasm-bindgen` 生成 `pkg/` → 浏览器端到端。
  ⚠️ **`pkg/` 曾经长期落后于源码**（2026-09-13 的产物跑在 09-18 的源码上），
  改完 `engine/src/` 一定要重建，产物大小可作参考：单一循环池后 `guangxue_wasm_bg.wasm` = **289,468 B**（此前 287,705 B）。
  ⚠️ **端到端必须走 `/index.html`**（SPA 外壳）：直接开 `/modules/english/english.html` 只有 HTML 片段、
  没有 `main.js`，按钮点了没反应，相对路径 `href="modules/english/english.css"` 也会被解析成
  `/modules/english/modules/english/english.css`（404）——这个 404 是**假象**，不是真缺文件。
- **数据库备份**：`pwsh scripts/backup.ps1`（→ `backend-go/cmd/backup/main.go`）对 `guangxue.db` 与 `backend-rust/auth.db` 执行 SQLite
  `VACUUM INTO` 快照，产物落在 `backups/<时间戳>/`（已 gitignore），`-Keep N` 只保留最近 N 份（只删输出目录下形如 `20260930-225615` 的子目录）。
  ⚠️ **别用「复制 `.db` 文件」当备份**：实测磁盘上 `auth.db` 只有 4 KB、安全快照是 124 KB —— WAL 里那部分直接拷贝会丢（约 97%）。
- **跨服务鉴权联调**：`pwsh scripts/verify-auth.ps1` 用**临时库**起两个真服务（默认 18081 账号服务 / 18080 Go，不碰真实库、不影响开发端口），
  端到端验证「账号服务发 Cookie → Go 本地验签」：匿名 `/api/reviews/stats` → 401 `unauthenticated`、账号服务发的 `gx_access` → 200、
  普通用户写词条 → 403 `forbidden`、外站 Origin 写请求 → 403（CSRF 闸门），以及 **P0-1 的进度隔离**
  （管理员与普通用户复习同一个词后，各自统计里都只有自己那 1 条）。实测 18 项全 PASS、约 6 秒。
  ⚠️ **别删这个脚本**：两侧的单元测试各自 mock 自己的密钥，密钥/算法/容差对不上时它们全绿，只有这里能发现。
- **单一循环池验收**：`pwsh scripts/verify-pool.ps1`（28 项，约 8 秒）走**真实 HTTP + 真实开发库**，
  逐条对照 [`review-pool-plan.md`](review-pool-plan.md) 第 7 节的 8 条验收口径。
  ⚠️ 它会**清空开发库的复习进度**（脚本开头自动跑 `cmd/discard-progress -yes`，会先快照到 `backups/`），
  所以它能反复运行，但别在用户正在用的时候跑。与 `verify-auth.ps1`（临时库、只读）分工不同。
- **P0-1 迁移**：`go run ./cmd/migrate`（默认 dry-run，只报告不改）→ `go run ./cmd/migrate -apply`（**先自动快照**再改）。
  它删掉旧的 `UNIQUE(word_id)` 索引、清掉 `user_id = 0` 的历史行、补上 `(user_id, word_id)` 唯一索引；
  服务启动时会自检，旧结构会**拒绝启动**并提示这条命令。为什么必须显式做：**GORM 的 AutoMigrate 只补新索引、不删旧索引**
  （见 [`launch-plan.md`](launch-plan.md) 的 P0-1）。
- ⚠️ 但 `JsValue` 在非 wasm32 目标上未实现（调用即 `panic: function not implemented on non-wasm32 targets`，
  无法 unwinding 会直接 abort）：**纯计算层不要碰 `JsValue`**，把它留在 wasm 导出方法的边界上。

## 7. `word_reviews` 行是懒创建，且**按人一行**

单词由 `POST /api/words` 写入 `words` 表；某个用户首次提交复习时才为**他**创建 `word_reviews` 行。
`/api/reviews/new` = 当前用户**没有复习行**的词（别人学过不算）。
唯一键是 `(user_id, word_id)`；`user_id` 只来自 `gx_access` 令牌的 `sub`，**请求体里没有这个字段**
（前端无法替别人写进度）。P0-1 之前那些 `user_id = 0` 的历史行不对任何用户可见，迁移时会被清掉。

## 8. 前端静态资源的缓存：`.wasm` 是**强缓存一天**的

`dev-server.js:161` 按扩展名分流：`CODE_EXT`（html/js/mjs/css/json/txt/map）发 `no-cache` + ETag，**其它一律 `public, max-age=86400`** —— `.wasm` 落在后者，也就是**一天强缓存**。

后果不是「慢一点」，而是**改了引擎在浏览器里完全看不出效果**，且不报任何错：

- 重建 `modules/english/engine/pkg/` 之后打开页面，跑的还是旧 wasm；
- 我为此排查了很久：置顶排序明明已修好，页面上的「今日置顶」始终是 2/5，直到发现浏览器加载的仍是上一版二进制。

**规矩：每次改 `engine/src/` 并重建 `pkg/` 之后，必须把 `modules/english/english.js` 顶部的 `ENGINE_VERSION` 加一。**
`loadEngine()` 会把这个版本号同时拼到 `.js` 与 `_bg.wasm` 两个 URL 上（两个文件是分开缓存的，只给一个加版本号没用）。

同一个函数里还有第二个坑：wasm 的 URL 必须用 `import.meta.url` 拼**绝对**地址。
写成相对路径 `'./engine/pkg/…'` 会被解析到**文档**地址上，而英语页是 SPA 注入的
（文档地址是站点根 `/index.html`），实测 404 并卡在「复习功能加载失败」。

生产环境（Nginx）对应做法见 [`README.md`](../README.md) 的站点配置段：`.wasm` 同样不要发长强缓存。

## 9. 服务端的字段名要与引擎的 serde 键名逐字对齐

单一循环池把「今日置顶」的标记从服务端传给引擎：Go 侧 `dueCard.Daily` 的 JSON 键是 **`daily`**，
而 Rust 侧 `ApiCard` 的字段叫 `is_daily`。少了 `#[serde(rename = "daily")]` 时，
`#[serde(default)]` 会把它**静默**读成 `false`：不报错、不告警，5 张置顶卡被当成普通卡埋进池子中段，
「今日置顶 N/5」永远涨不上去。

- 锁这个坑的测试是 `modules/english/engine/src/session.rs` 的 `api_card_reads_server_daily_key`
  ——它用**真实键名**写 JSON。Rust 侧其它测试夹具都是按结构体字段名手写 JSON 的，
  所以只有这条能发现键名漂移（真的漏过一次）。
- 同理，凡是「服务端发一个标记、引擎据此分类」的字段，都要有**用真实响应键名**的解析测试；
  只测内部逻辑的单元测试对这类错误是盲的。

## 10. 反复跑验收会被登录限流挡住

`backend-rust/src/config.rs:150` 的 `login_ip` 是 **20 次 / 15 分钟 / 单 IP**。验收脚本每跑一轮要登录 2~4 次，
连跑几轮就整片 429（浏览器用例表现为「登录管理员账号 HTTP 429」，后面全线失败）。

计数器在 Rust 进程内存里，**重启账号服务即可清零**（不需要等 15 分钟）：

```powershell
$c = Get-NetTCPConnection -LocalPort 8081 -State Listen; Stop-Process -Id $c.OwningProcess -Force
Set-Location backend-rust; & '.\target\release\guangxue-auth.exe'
```

顺带：这也是同宿舍 / 同办公室共用出口 IP 会被一起锁 15 分钟的原因（见 `TODO.md`）。

## 相关文档

- 复习引擎构建细节 → [`review-engine.md`](review-engine.md)
- 接口清单 → [`admin-api.md`](admin-api.md)
