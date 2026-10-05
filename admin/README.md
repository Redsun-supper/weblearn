# 广学 · 后台管理（通用骨架）

独立于学生站的入口页，用于**管理平台内容**（词条等共享数据），不涉及用户个人数据。

| 项 | 说明 |
|----|------|
| 入口 | `/admin/`（本地：http://127.0.0.1:8899/admin/） |
| 入口文件 | `admin/index.html` |
| 与学生站的关系 | 完全独立：不走 `main.js` 的导航，也不使用学科页的 localStorage 缓存 |

```
admin/
├── index.html   入口页（登录门禁 + 布局骨架）
├── admin.css    后台样式，含通用组件（表格/表单/按钮/徽章/空状态/提示条/门禁表单）
├── admin.js     框架：登录门禁、布局、侧栏导航、hash 路由、学科后台加载、通用工具
├── panels/      ✅ 管理面板模块（看板/邀请码/用户/审计）+ 共用样式 panels.css
│                壳无关：个人中心 `account/` 引它们，以后本页也能挂同一份
└── README.md    本文件
```

## 🔐 登录门禁与服务端鉴权（两道都已就位）

打开 `/admin/` 时 `admin.js` 会先问账号服务「我是谁」：

| 情况 | 页面行为 |
|------|---------|
| `GET /api/auth/me` 返回 200 且 `role=admin`（或 `super_admin`） | 隐藏门禁、渲染后台，顶栏显示邮箱 + 「退出登录」 |
| 200 但不是管理员 | 停留在门禁，提示「当前账号 xxx 不是管理员（role=user）」 |
| 401（未登录） | 停留在门禁，显示邮箱 + 密码的登录表单 |
| 账号服务没起来 | 门禁里提示「无法连接账号服务，请确认 backend-rust 已启动」 |

账号服务是独立的 Rust 服务（[`../backend-rust/README.md`](../backend-rust/README.md)），
登录态是 httpOnly Cookie，同源下前端直接用，不需要额外处理令牌。

✅ **服务端鉴权已在阶段 2 补齐**：Go 侧用与账号服务共享的 `AUTH_JWT_SECRET` 对 `gx_access` Cookie
做本地 HS256 验签（`backend-go/middleware/auth.go`，不查库、不回调账号服务）——
`/api/words` 的写接口要**管理员**、`/api/reviews/*` 要**登录**，写请求还要过 CSRF 闸门（`Origin` 白名单）。
所以「不经过页面直接 `curl -X PUT /api/words/1`」现在是 401 / 403。
前端对 401 的处理是提示后跳 `../account/?next=...`，403 只提示「没有权限」。

⚠️ **本页的定位（P1，2026-10 起）**：只做**内容管理**（词条增删改查与批量导入）。
数据看板 / 邀请码 / 用户管理 / 审计日志这四个**管理面板已经搬到个人中心**
（`/account/#/dashboard`、`#/invites`、`#/users`、`#/audit`，按角色显示）：
模块本体在 [`panels/`](panels/)（**壳无关**：导出 `meta` + `mount(container, ctx)`，个人中心与后台都能挂同一份），
样式共用 `panels/panels.css`（`pn-` 前缀，**零动效**）。本页顶栏那颗「管理后台」链接就是从个人中心指过来的。

**邀请码面板的四处「防手滑」**（2026-10-05 加，行为都在 `panels/invites.js` 里）：

- **记住上次填的那套值**：数量 / 每张次数 / 有效期 / 等级 / 自定义码存在**本机浏览器**（`localStorage` 键
  `guangxue.invites.create`），下次打开表单直接带上；旁边「恢复默认」清掉它。
  ⚠️ 备注**不自动填**，只把上次的内容显示成占位提示（每批备注都不一样，自动填最容易发错批次）。
- **自己指定邀请码（`custom_code`）**：表单里那个「自定义码」框留空 = 照旧随机生成一批；
  填了 = 就用你给的那串（**正好 16 位、A-Z 与 0-9**，边打边转大写并抹掉手写的 `-`，那一次只出 1 张）。
  有效期旁边新增**「永不过期」勾选框**（勾上天数框禁用并按 `expires_in_days=0` 提交），
  于是「不过期但只能用一次」就是勾上它 + 每张次数填 1。
  ⚠️ 想定的那串**已经被用过**时，后端先回 409，面板把那张码的状态 / 已用次数 / 谁用过摆出来问一句；
  点「继续」才带 `allow_existing` 重发 —— **沿用它这张**（不新建第二张、也不覆盖），只改额度与有效期，
  兑换记录、已注册的账号、停用状态都不动。
- **发邮件前先拦一下**：勾选里混着「已用完 / 已过期 / 已停用」的码时，不再默默丢掉，
  而是列出是哪几张、为什么发不出去，确认后才把剩下那些与邮箱配对发送。
- **重新启用已用过的码**（`POST /api/auth/admin/invites/{id}/reset`，超管）：
  把 `used_count` 清零让它还能被兑换。**主动降安全**的动作 → 确认框里写明等级、之前谁用过、
  兑换记录不会删、操作会进审计（`invite_reset`）；停用/整批停用的确认框里也会点明
  「停用只拦住还没用掉的次数，已经注册出来的账号不会被回收，要收回权限得去「用户」面板封禁」。

## 新增一个学科后台

**第一步**：在 `modules/<学科>/admin/` 下写模块，导出 `mount` 与（可选的）`unmount`：

```js
// modules/english/admin/english-admin.js
export function mount(container, ctx) {
    // 把界面渲染进 container
    container.appendChild(ctx.el('div', { class: 'admin-card', text: '英语后台' }));
}

export function unmount() {
    // 可选：切换学科时清理定时器/监听
}
```

**第二步**：在 `admin/admin.js` 的 `SUBJECT_ADMINS` 注册表里登记一行（学科名称与描述写在注册表里，模块本身不需要再导出一份）：

```js
var SUBJECT_ADMINS = [
    { id: 'english', name: '英语', description: '词条管理', module: '../modules/english/admin/english-admin.js' }
];
```

> `module` 路径相对 `admin/admin.js` 解析，所以学科后台是 `'../modules/<学科>/admin/...'`。
> 用动态 `import()` 按需加载：**只有点进该学科才会加载它的后台代码**。

## ctx 提供的通用能力

学科后台不要各自重复实现请求、提示、DOM 构建：

| 能力 | 说明 |
|------|------|
| `ctx.api(path, {method, body})` | 取数/提交。自动 JSON、自动抛错（含后端 `code !== 200` 的业务错误） |
| `ctx.toast(msg, kind)` | 右下角提示，`kind` 取 `'success'` / `'error'`（`'ok'` 是 `'success'` 的别名，两个壳都会归一化 —— 面板作者顺手写 `'ok'` 也有绿底） |
| `ctx.confirm(msg)` | 确认框，返回 `Promise<boolean>` |
| `ctx.el(tag, attrs, children)` | 构建 DOM；默认写 `textContent`，避免把数据当 HTML 解析 |
| `ctx.escapeHtml(str)` | 转义（拼接字符串时才需要） |
| `ctx.setTitle(str)` | 改顶部标题 |

另外 `admin.css` 提供了可直接复用的组件类：`.admin-card`、`.admin-table`、`.admin-field`、
`.admin-grid-2`、`.admin-btn`（`-primary` / `-danger` / `-sm`）、`.admin-badge`、
`.admin-empty`、`.admin-pager`。

## 本地测试

```powershell
# 仓库根目录：起站点（静态文件 + /api 同源代理）
node dev-server.js

# 浏览器打开
#   http://127.0.0.1:8899/admin/
```

右上角会显示后端连通状态（调用 `/api/health`）。

## 后续要做的事

- [x] **登录门禁**：`checkAuth()` 已接 `GET /api/auth/me`（异步，渲染等校验结果）
- [ ] **服务端鉴权**：给 Go 侧 `/api/words` 写接口加中间件（用同一 `AUTH_JWT_SECRET` 验签 + 查 `auth.db` 会话）
- [ ] 各学科后台模块（当前注册表为空，骨架会显示引导页）
