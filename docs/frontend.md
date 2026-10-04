# 前端要点（导航 / 缓存 / 头像入口）

> 原先在 `CLAUDE.md`，2026-09 拆分。改 `index.html`、`main.js`、`main.css` 前读一遍。
> 头像过场属于「页面切换动画」，动手前先看 `CLAUDE.md` 工作流程第 5 条（**先与用户商量**）。

- 导航栏 `rectangle` 内含头像 + 9 个导航项，字段 `data-page="<学科页路径>"`。
- `main.js` 用 **localStorage 缓存**（键前缀 `pageCache_`，30 天过期、自动清理；`CACHE_VERSION` 在结构或路径变更时整体失效），点击导航用 `fetch` 加载并缓存，默认展示英语。
- **学科页路径**：已有独立模块的学科写成 `modules/<学科>/<学科>.html`（当前仅英语）；其余 8 门仍是 `pages/<学科>.html` 占位（`<p>敬请期待</p>`）。
- **学科模块约定**：放在 `modules/<学科>/` 下并导出初始化函数，`main.js` 的 `initSubjectModule()` 按需动态 `import()`；**学科逻辑不得回流到 `main.js`**。
- 本地起站点用仓库根目录的 `dev-server.js`（Node 内置模块实现，静态文件 + `/api` 同源代理，等价线上 Nginx 形态）；不能直接双击 `index.html`（`file://` 下 `/api` 与 WASM 模块都会失败）。
- **头像 = 个人中心入口**（`index.html` 的 `#navAvatar`，样式在 `main.css` 的 `.rounded-square`）：入口位置已从右上角的文字链接改成左上角头像（**暂时方案**；原来的 `#navAccount` 文字入口与导航上的「退出」按钮都已撤掉，退出改在个人中心里做）。`main.js` 的 `renderAvatarState()` 问一次 `GET /api/auth/me`：已登录给头像挂 `is-signed` 点亮右下角绿点并把昵称写进 `title`，未登录只把文案改成「登录 / 注册 · 个人中心」；账号服务没起来时静默降级，头像照样能点。⚠️ 它是 `.rectangle` 的子元素，所以**沉浸模式下随导航栏一起隐藏**——复习页默认就是沉浸，那时要先点顶栏的「显示导航栏」才能看到头像。`.nav-items` 的 `right` 已从 190px 改回 30px，与头像的左侧留白对称。
- **点头像的过场**（`main.js` 的 `playAvatarZoom()` + `main.css` 的 `.avatar-zoom`）：以头像中心为圆心，用 `clip-path: circle()` 把一个圆放大到盖住四角（半径按窗口与头像位置实时算），**同时**那颗头像自己缩小并向右下落到个人中心侧栏左上角，然后跳 `account/?from=avatar`。⚠️ 三个坑：① 必须分两帧写**行内** `clip-path`，且第一帧要**临时把 transition 关掉**再强制重排，否则过渡起点会落在样式表的兜底值（圆心在屏幕正中），圆就从屏幕中心长出来了（实测踩过）；② 遮罩的渐变背景必须与 `account/account.css` 里 `body` 的背景**逐字一致**，跳到个人中心才看不出接缝；③ 形变的**落点几何**由 `morphTarget()` 从 `account.css` 里**量出来**（临时往 body 插一个 `.acc-sidebar` + `.acc-sidebar-avatar` 探针元素，拿完当帧删掉），再按「圆角按起点那颗的比例换算」（起点 64×64 用 9px → 落点 40×40 取 9 × 40/64 = 5.6px，与 CSS 里的 6px 基本吻合；**尺寸原样用 CSS 的 40×40，不缩**）与「横向中心对齐、纵向保持 CSS 的 top」折算一次 —— 所以改 `account.css` 的侧栏内边距或头像尺寸**不用改 JS**，窄屏（侧栏横过来、头像缩到 36×36）也自动跟上。⚠️ 曾经把「半径比」（落点圆角 / 起点圆角 = 6/9）当成尺寸缩放系数，落点被算成 26.67×26.67（2026-10 实测过一次）；按半径比例换算**只用于圆角**。另外纵向也**不做中心对齐**：落点要盖住换页那一刻侧栏头像**真正**所在的位置（侧栏内边距 20px），居中会差 12px（起点头像中心 y = 50，侧栏那颗中心 y = 40）。这个「预读 `account/account.css`」由 `index.html` 末尾一小段动态插 `<link>` 完成（不阻塞首屏，失败就按兜底值走）。系统开启「减少动态效果」或不支持 `clip-path`（`CSS.supports` 探测）时直接跳转，不播过场。
- **形变的是「圆角矩形 → 圆角矩形」**：导航栏那颗是 64×64、`border-radius: 9px`（+ 3px 白边 + 白底），落点是侧栏那颗 40×40、圆角 6px —— 圆角**占外框的比例**两边一致（9/64 = 6/40），所以飞过去的一路只在位置/尺寸/圆角半径上过渡，看不出「圆角自己化了一下」。⚠️ `.rounded-square.is-flying`（过场期间的临时外观）里**不要写** `border-radius`，写了会在过场一开始就把圆角顶掉；圆角由 `playAvatarZoom` 用 `morph.radius` 写成行内值来过渡。早先那版会把它拉成一颗「返回主页面」胶囊（形变中变成矩形），2026-10 随个人中心改版一起去掉了。
- 个人中心（`account/`）的相关细节见 [`backend-auth.md`](backend-auth.md)；通用后台见 [`admin-api.md`](admin-api.md)。

## 相关文档

- 项目组成与端口 → [`overview.md`](overview.md)
- 英语页界面与动画 → [`english-ui.md`](english-ui.md)
- 硬性边界（含头像素材约束） → [`boundaries.md`](boundaries.md)
