# 前端要点（导航 / 缓存 / 头像入口）

> 原先在 `CLAUDE.md`，2026-09 拆分。改 `index.html`、`main.js`、`main.css` 前读一遍。
> 头像过场属于「页面切换动画」，动手前先看 `CLAUDE.md` 工作流程第 5 条（**先与用户商量**）。

- 导航栏 `rectangle` 内含头像 + 9 个导航项，字段 `data-page="<学科页路径>"`。
- `main.js` 用 **localStorage 缓存**（键前缀 `pageCache_`，30 天过期、自动清理；`CACHE_VERSION` 在结构或路径变更时整体失效），点击导航用 `fetch` 加载并缓存，默认展示英语。
- **学科页路径**：已有独立模块的学科写成 `modules/<学科>/<学科>.html`（当前仅英语）；其余 8 门仍是 `pages/<学科>.html` 占位（`<p>敬请期待</p>`）。
- **学科模块约定**：放在 `modules/<学科>/` 下并导出初始化函数，`main.js` 的 `initSubjectModule()` 按需动态 `import()`；**学科逻辑不得回流到 `main.js`**。
- 本地起站点用仓库根目录的 `dev-server.js`（Node 内置模块实现，静态文件 + `/api` 同源代理，等价线上 Nginx 形态）；不能直接双击 `index.html`（`file://` 下 `/api` 与 WASM 模块都会失败）。
- **头像 = 个人中心入口**（`index.html` 的 `#navAvatar`，样式在 `main.css` 的 `.rounded-square`）：入口位置已从右上角的文字链接改成左上角头像（**暂时方案**；原来的 `#navAccount` 文字入口与导航上的「退出」按钮都已撤掉，退出改在个人中心里做）。`main.js` 的 `renderAvatarState()` 问一次 `GET /api/auth/me`：已登录给头像挂 `is-signed` 点亮右下角绿点并把昵称写进 `title`，未登录只把文案改成「登录 / 注册 · 个人中心」；账号服务没起来时静默降级，头像照样能点。⚠️ 它是 `.rectangle` 的子元素，所以**沉浸模式下随导航栏一起隐藏**——复习页默认就是沉浸，那时要先点顶栏的「显示导航栏」才能看到头像。`.nav-items` 的 `right` 已从 190px 改回 30px，与头像的左侧留白对称。
- **点头像的过场**（`main.js` 的 `playAvatarZoom()` + `main.css` 的 `.avatar-zoom`）：以头像中心为圆心，用 `clip-path: circle()` 把一个圆放大到盖住四角（半径按窗口与头像位置实时算），然后跳 `account/?from=avatar`。⚠️ 两个坑：① 必须分两帧写**行内** `clip-path`，且第一帧要**临时把 transition 关掉**再强制重排，否则过渡起点会落在样式表的兜底值（圆心在屏幕正中），圆就从屏幕中心长出来了（实测踩过）；② 遮罩的渐变背景必须与 `account/account.css` 里 `body` 的背景**逐字一致**，跳到个人中心才看不出接缝。系统开启「减少动态效果」或不支持 `clip-path`（`CSS.supports` 探测）时直接跳转，不播过场。
- 个人中心（`account/`）的相关细节见 [`backend-auth.md`](backend-auth.md)；通用后台见 [`admin-api.md`](admin-api.md)。

## 相关文档

- 项目组成与端口 → [`overview.md`](overview.md)
- 英语页界面与动画 → [`english-ui.md`](english-ui.md)
- 硬性边界（含头像素材约束） → [`boundaries.md`](boundaries.md)
