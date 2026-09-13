# 广学 · 后台管理（通用骨架）

独立于学生站的入口页，用于**管理平台内容**（词条等共享数据），不涉及用户个人数据。

| 项 | 说明 |
|----|------|
| 入口 | `/admin/`（本地：http://127.0.0.1:8899/admin/） |
| 入口文件 | `admin/index.html` |
| 与学生站的关系 | 完全独立：不走 `main.js` 的导航，也不使用学科页的 localStorage 缓存 |

```
admin/
├── index.html   入口页（布局骨架 + 鉴权遮罩预留位）
├── admin.css    后台样式，含通用组件（表格/表单/按钮/徽章/空状态/提示条）
├── admin.js     框架：布局、侧栏导航、hash 路由、学科后台加载、通用工具、鉴权预留
└── README.md    本文件
```

## ⚠️ 当前状态：没有登录，请勿部署到公网

任何能访问 `/admin/` 的人都能修改词条。**接入登录之前不要把这个目录部署到线上。**

- 前端预留位：`admin.js` 的 `checkAuth()` + `index.html` 的 `#adminAuthGate`
- 后端也要做：现在 Go 后端**没有鉴权中间件**，光靠前端拦截挡不住直接调接口的人
  （例如 `curl -X PUT /api/words/1`）。登录实现时两端要一起做。

页面上也常驻一条黄色提示条提醒这件事，接入登录后可移除。

## 新增一个学科后台

**第一步**：在 `modules/<学科>/admin/` 下写模块，导出：

```js
// modules/english/admin/english-admin.js
export var meta = { id: 'english', name: '英语', description: '词条管理' }; // 可选，用于侧栏

export function mount(container, ctx) {
    // 把界面渲染进 container
    container.appendChild(ctx.el('div', { class: 'admin-card', text: '英语后台' }));
}

export function unmount() {
    // 可选：切换学科时清理定时器/监听
}
```

**第二步**：在 `admin/admin.js` 的 `SUBJECT_ADMINS` 注册表里登记一行：

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

- [ ] **登录鉴权**：前端 `checkAuth()` 接会话校验 + 后端加鉴权中间件（两端一起做才有意义）
- [ ] 各学科后台模块（当前注册表为空，骨架会显示引导页）
