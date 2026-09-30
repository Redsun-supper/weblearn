# 管理后台与接口

> 原先在 `CLAUDE.md`，2026-09 拆分。改 `admin/`、`modules/english/admin/`、
> `/api/words*`、`/api/reviews/*` 前读一遍。

## 通用后台（`admin/`）

- **入口**：`admin/index.html`（本地 `/admin/`）。与学生站**完全独立**：不走 `main.js`、不使用学科页的 localStorage 缓存。
- **约定**：学科后台放 `modules/<学科>/admin/`，导出 `mount(container, ctx)`（可选 `unmount()`），再到 `admin/admin.js` 的 `SUBJECT_ADMINS` 登记一行；框架用动态 `import()` 按需加载。通用能力通过 `ctx` 注入：`api` / `toast` / `confirm` / `el` / `escapeHtml` / `setTitle`。
- **登录门禁已接入界面层**：`admin.js` 的 `checkAuth()` 已改为**异步**（调 `GET /api/auth/me`），`role=admin` 才渲染后台，否则只显示 `#adminAuthGate` 里的登录表单；顶栏 `#adminUser` 显示当前账号 + 退出登录。⚠️ 改这块要注意：`DOMContentLoaded` 里必须等 `checkAuth().then(...)` 再 `renderNav/route`。
- ⚠️ **服务端鉴权仍未补**：Go 侧 `/api/words` 写接口任何人都能直接调（`curl -X PUT /api/words/1` 就能改数据），**在补中间件之前不要把 `/admin/` 或站点部署到公网**（页面上常驻提示条写的就是这件事）。下一期用同一 `AUTH_JWT_SECRET` 验签 + 查 `auth.db` 会话。

## 英语后台（`modules/english/admin/english-admin.js`）

词条列表（搜索 / 词书 / 单元 / 分页）、增删改查、**多释义编辑**（一个词性一块，每块可带自己的例句与译文；全空的行提交前会被丢掉，后端 `normalizeSenses` 再清一遍）、批量导入（粘贴 → `engine` 的 `parse_word_list` 解析 → 预览 → 前端每 200 条分批 POST）。导入字段按位置对应 **单词 / 音标 / 释义 / 例句 / 例句翻译**，多出的忽略；导入只填单条释义，多释义在编辑页补。

## 词条接口

`GET/POST /api/words`、`GET/PUT/DELETE /api/words/:id`、`GET /api/word-options`。

- `PUT` 是全量更新；改名撞车返回 409。
- `DELETE` **会连带删除该词的 `word_reviews` 与 `review_logs`**（日志留着会让 stats 虚高）。改错别字用 `PUT`，别删了重建。
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
