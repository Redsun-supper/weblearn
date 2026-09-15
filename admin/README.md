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
└── README.md    本文件
```

## 🔐 登录门禁：界面已接入，服务端鉴权仍未补

打开 `/admin/` 时 `admin.js` 会先问账号服务「我是谁」：

| 情况 | 页面行为 |
|------|---------|
| `GET /api/auth/me` 返回 200 且 `role=admin` | 隐藏门禁、渲染后台，顶栏显示邮箱 + 「退出登录」 |
| 200 但不是管理员 | 停留在门禁，提示「当前账号 xxx 不是管理员（role=user）」 |
| 401（未登录） | 停留在门禁，显示邮箱 + 密码的登录表单 |
| 账号服务没起来 | 门禁里提示「无法连接账号服务，请确认 backend-rust 已启动」 |

账号服务是独立的 Rust 服务（[`../backend-rust/README.md`](../backend-rust/README.md)），
登录态是 httpOnly Cookie，同源下前端直接用，不需要额外处理令牌。

⚠️ **这只是界面层的门禁**：Go 侧的 `/api/words` 等接口目前**还没有鉴权中间件**，
不经过页面直接调接口（`curl -X PUT /api/words/1`）仍然能改数据。
在补齐服务端鉴权之前，**仍然不要把 `/admin/` 部署到公网**。页面上的提示条写的也是这件事。

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
| `ctx.toast(msg, kind)` | 右下角提示，`kind` 取 `'success'` / `'error'` |
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
