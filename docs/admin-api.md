# 管理后台与接口

> 原先在 `CLAUDE.md`，2026-09 拆分。改 `admin/`、`modules/english/admin/`、
> `/api/words*`、`/api/reviews/*` 前读一遍。

## 通用后台（`admin/`）

- **入口**：`admin/index.html`（本地 `/admin/`）。与学生站**完全独立**：不走 `main.js`、不使用学科页的 localStorage 缓存。
- **约定**：学科后台放 `modules/<学科>/admin/`，导出 `mount(container, ctx)`（可选 `unmount()`），再到 `admin/admin.js` 的 `SUBJECT_ADMINS` 登记一行；框架用动态 `import()` 按需加载。通用能力通过 `ctx` 注入：`api` / `toast` / `confirm` / `el` / `escapeHtml` / `setTitle`。
- **登录门禁已接入界面层**：`admin.js` 的 `checkAuth()` 已改为**异步**（调 `GET /api/auth/me`），`role=admin` 才渲染后台，否则只显示 `#adminAuthGate` 里的登录表单；顶栏 `#adminUser` 显示当前账号 + 退出登录。⚠️ 改这块要注意：`DOMContentLoaded` 里必须等 `checkAuth().then(...)` 再 `renderNav/route`。
- ✅ **服务端鉴权已补上（阶段 2）**：`/api/words` 的写接口（`POST` / `PUT /:id` / `DELETE /:id`）现在要**管理员**，`/api/reviews/*` 整组要**登录**，写请求还要过 CSRF 闸门（`Origin` 白名单）。做法是用与账号服务共享的 `AUTH_JWT_SECRET` 对 `gx_access` Cookie 做本地 HS256 验签（`backend-go/middleware/auth.go`，不查库、不回调）。前端的 401 处理：`admin.js` 的 `apiFetch` 提示后跳 `../account/?next=...`，`english.js` 提示后跳 `account/?next=...`。（历史口径：这两条曾长期是「任何人都能 `curl` 改数据，补中间件前别上公网」；现在服务端拦得住，但**会话撤销与多用户化仍未做**，公网部署照旧要走 [`roadmap.md`](roadmap.md) 的阶段 3。）

## 英语后台（`modules/english/admin/english-admin.js`）

词条列表（搜索 / 词书 / 单元 / 分页）、增删改查、**多释义编辑**（一个词性一块，每块可带自己的例句与译文；全空的行提交前会被丢掉，后端 `normalizeSenses` 再清一遍）、批量导入（粘贴 → `engine` 的 `parse_word_list` 解析 → 预览 → 前端每 200 条分批 POST）。导入字段按位置对应 **单词 / 音标 / 释义 / 例句 / 例句翻译**，多出的忽略；导入只填单条释义，多释义在编辑页补。

## 词条接口

`GET/POST /api/words`、`GET/PUT/DELETE /api/words/:id`、`GET /api/word-options`。
**权限**：读接口（`GET /api/words`、`GET /api/words/:id`、`GET /api/word-options`）公开；写接口（`POST /api/words`、`PUT /api/words/:id`、`DELETE /api/words/:id`）**要管理员**（`role=admin`），不带 Cookie 是 401 `unauthenticated`、普通用户是 403 `forbidden`。

- `PUT` 是全量更新；改名撞车返回 409。
- `DELETE` **会连带删除该词的 `word_reviews` 与 `review_logs`**（日志留着会让 stats 虚高）。⚠️ 词条是共享的、进度是私有的，所以这一下删掉的是**所有人**对该词的进度（P0-1 决策：不做软删除）。改错别字用 `PUT`，别删了重建。
- `POST /api/words` 查重**大小写不敏感**。
- `word-options` 刻意不在 `/api/words/options`，避免与 `/api/words/:id` 通配路由冲突。
- 词条带 `example_translation`（词条级例句翻译）与 `senses`（多释义数组）两个字段；两者都可空。`/api/reviews/due` 的 `dueCard` 里 `senses` 用 `models.WordSenses` 直接扫列，GORM 认 `sql.Scanner`，不需要额外 join。

## 复习调度接口

`/api/reviews/due`（只给已到期的，仍在）、`/api/reviews/new`（未学词候选）、**`/api/reviews/queue`**（整库按 `due_at` 升序，含未到期，分页带 `total`）、**`/api/reviews/probes`**（`due_at` **倒序**，即「到期最远」的抽查候选）、`/api/reviews/submit`、`/api/reviews/stats`。

- `submit` 请求体多一个 `is_probe`（默认 false），后端记进 `review_logs.is_probe`；`stability_before` 取库里真实旧值，**不要**改成引擎的输入状态，否则抽查会被统计成「今日新学」。
- 调度口径与前端消费方式见 [`review-engine.md`](review-engine.md)。

## 相关文档

- Go 后端其它接口与环境变量 → [`backend-auth.md`](backend-auth.md)
- 账号门禁背后的登录系统 → [`backend-auth.md`](backend-auth.md)
