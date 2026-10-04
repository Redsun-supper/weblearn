# 广学 · 待处理问题

> 这里记的是**已知但暂缓**的问题：要么等用户拍板，要么当前不值得为它加复杂度。
> 每条都写清「现象 / 复现 / 候选方案」，这样下次接手不用重新推一遍。
> 处理完一条就把对应小节删掉，别让它变成考古现场。

---

## 1. 沉浸模式下「返回」没有反向动画（暂缓，等用户决定）

**现象**：从个人中心点「返回主页面」时，如果落地页（`/`）的导航栏正处于**沉浸模式**
（英语复习页默认开启），头像没有落脚点，于是 `?from=account` 的反向倒放被跳过，
表现成「组件淡出 → 主界面直接出现」，少了「圆圈缩回头像」那一段。

**为什么是跳过而不是硬来**：沉浸模式下 `.rectangle { display: none }`，头像跟着一起隐藏，
动画结束时它无处安放；如果硬留一个浮在左上角，等用户切回导航栏就会看到**两个头像**。

**复现**：`localStorage.setItem('reviewImmersive', '1')`（或者干脆不设，默认就是开），
打开 `http://127.0.0.1:8899/account/`，点左上角「← 返回主页面」。

**判据代码**：`main.js` 的 `returnHasLandingSpot()` —— 它读 `reviewImmersive === '0'`
（与英语模块同一个键，同步可读，不会和它的异步初始化抢时序）。

**候选方案**（下次挑一个）：

- **(a) 本次临时显示导航栏**（沉浸偏好保留，只这一次例外）—— 最容易做，代价是返回时导航栏会露出来；
- **(b) 头像落地后单独浮在左上角**，等导航栏出现时再收回去 —— 需要额外处理「两个头像」的冲突；
- **(c) 保持现状**（跳过反向动画，直接撤遮罩）。

---

## 2. 头像 2.6MB，而显示尺寸只有 64×64（已临时置空，缩略图方案仍待定）

**⚠️ 2026-10-02 现状变化（用户要求「头像暂时设置成无」）**：导航栏与个人中心两处 `<img>` 都已改为
`image/avatar-blank.png`（1×1 全透明，68 B），**首屏字节数已从 3,103,040 B 降到约 418 KB**。
原图**未删未改**，恢复只需把 `index.html:32` 与 `account/index.html:52` 的 `src` 改回 `image/avatar.png`。
下面的「待定」因此不再是"省流量"，而是"**要不要恢复成一张真正的头像、以及恢复成什么尺寸**"。

**原现状**：`image/avatar.png` 是真 PNG（`89 50 4E 47` 文件头），1330×1146，约 **2.56MB**；
导航栏只显示 64×64、个人中心身份卡 72×72。

**已经做的**：`dev-server.js` 现在给图片/字体/wasm 发 `public, max-age=86400`
（以前全站 `no-store`，每次导航都要重下 2.6MB）。实测再进个人中心时头像传输 **0B**（命中缓存）。

**待定**：生成一张 128×128 缩略图（例如 `image/avatar-thumb.png`，约 5–15KB）给导航与个人中心用，
**原图保留不动**。省掉 99% 流量，线上（Nginx 那侧）收益比本地明显。

⚠️ **涉及视觉素材，动手前必须先问用户**（见 [`docs/boundaries.md`](docs/boundaries.md) 第 5 节与 `CLAUDE.md` 红线第 4 条）。

---

## 3. 登录后的个人中心画面 —— 已按「后台那种布局」改过一版

原先用户说「若已登录先不管未来有其他的新的登录后画面，现在这个是零时的」，
所以那一版（居中白卡 + 身份卡）只保证了两件事：检测登录态的**最短显示时长**
（`account.js` 的 `MIN_CHECK_MS = 600`）与面板之间的交叉淡出。

**2026-10 用户给了新造型：「将个人中心设计成类似后台的那种设计布局」**，已实现 ——
布局与 `admin/` 同一套（侧栏 + 顶栏 + 内容区 + hash 路由），底色保留整页深色渐变。
细节见 [`docs/backend-auth.md`](docs/backend-auth.md) 的「个人中心」一节。

这一段仍算「按用户给的造型做的第一版」：配色、导航项的顺序与命名、要不要给设备列表加
「在那台设备上退出」之类的操作，都还没有被逐项确认过，用户随时可能再改。

---

## 4. 转场时长的「人工对齐」还没有兜底

同一个时长写在多处，靠注释互相提醒。**2026-09 又多了英语页那段过场**，现在一共这些：

| 时长 | 参与方 |
|------|--------|
| 460ms（头像过场） | `main.js` 的 `ZOOM_MS`/`goWhenSettled`、`main.css` 的 `.avatar-zoom`/`.is-morphing`、`account.css` 的 `accFall` |
| 460ms（头像落进侧栏） | `main.js` 的 `MORPH` 那段（与上一条同一次 `is-morphing` 过渡，共用 `ZOOM_MS`）、`main.css` 的 `.rounded-square.is-flying`、`account.css` 的 `.acc-sidebar-avatar`（**几何**由 `morphTarget()` 从 CSS 里读出来，不再写死数字） |
| 460ms（起始页过场） | `english.js` 的 `START_TOTAL_MS`、`english.css` 的 `startOut`/`startIn`、`main.css` 的 `.rectangle` + `body.is-immersive .rectangle` + `.content-container` |
| 160ms（起始页退场） | `english.js` 的 `START_OUT_MS`、`english.css` 的 `startOut` |
| 160 / 240ms（计划小字切换） | `english.js` 的 `PLAN_SWAP_OUT_MS`、`english.css` 的 `planOut` / `planIn` |
| 150 / 240ms | `english.js` 的 `TRANSITION_MS`、`english.css` 的 `studyOut`/`studyIn` |
| 60ms（个人中心检测屏淡出） | `account.js` 的 `TIMING.screenFade`、`account.css` 的 `.acc-check` 的 `transition` |

改一处忘另一处就会出现「过渡没跑完就换页」「两层动画错位」这类微妙问题（已经踩过一次）。

**候选**：用 CSS 自定义属性（`:root { --morph: 460ms }`）统一，JS 侧读 `getComputedStyle`
取值 —— 代价是 JS 要读一次样式，且 IE 系不支持（本项目已放弃 IE）。
暂时按「注释互指 + 验收脚本」兜着，不动。

**现状补充**：英语页那段过场的时长已经**三处交叉引用**（`english.js` / `english.css` / `main.css`
各自的注释里都写了「必须与另两处对齐」），[`docs/english-ui.md`](docs/english-ui.md) 与 `modules/english/README.md` 也各记了一遍。
也就是说这条的维护成本还在涨 —— 真要做统一，现在是个合适的时机。

---

## 5. 英语页：切走再切回来会回到起始页（已定方案，暂缓实现）

从英语页切到别的学科、再切回来时会回到**起始页**（要重新点一次「开始复习单词」），
而不是接着刚才那张卡。用户已拍板：这版先这样，「同一次会话内记住」留到以后。

**完整记录在 [`modules/english/FUTURE.md`](modules/english/FUTURE.md)** —— 里面写了为什么
这件事不是「记住一个布尔值」那么简单（DOM 每次重建 + `unmount()` 会 `free()` 掉 WASM 会话
+ 引擎没有「恢复到第 N 张」的接口），以及三个候选方案。要动它之前先把那份读完。

---

## 6. 单一循环池上线后遗留的三件小事（2026-10-02 记）

重构本身已完成并验收（28 项全通过，见 [`docs/review-pool-plan.md`](docs/review-pool-plan.md) 第 8 节），
下面三条是**当时刻意没做**的，都记着原因，别当成遗漏：

**(a) 顶栏 id 与文案对不上号**：`english.html` 的三个元素 id 仍是 `statTodayNew` / `statTodayReview` / `statRest`，
但文案已改成「今日已复习 / 池内到期 / 池内总数」，数据源也换成 `today_reviewed` / `pool_due` / `pool_size`。
改 id 属于结构改动（牵动 `english.css` 与 `english.js` 的选择器），收益为零，所以只改了文案与数据源。

**(b) 旧引擎字段还留在响应里**：`/api/reviews/stats` 仍返回 `total_words` / `new_words` / `due_cards`
（兼容字段，新前端不用）；`ReviewStats` 与 `dueCard` 里的 `stability` / `difficulty` 仍从库里读出来
（`QueueReviews` 已不再序列化它们，但 `SubmitReview` 要用 `stability_before` 记账）。
等确认没有别的调用方后可以把兼容字段删干净。

**(c) `review_logs` 里的旧数据没清**：清进度只删了 `word_reviews` 与 `review_logs` 两张表
（见 `cmd/discard-progress`），但**历史日志的 `is_reset` 全是 0**，所以「今日新学」在历史日期上
仍按旧口径统计。用户决策 E19/E20：日志全留、不加回退开关。真要清理只能用快照回滚。

---

## 7. `/api/reviews/stats` 的 `today_new` 口径依赖日志的 `is_reset` 列

`today_new` 数的是「今天 `review_logs` 里 `stability_before = 0` **且** `is_reset = false` 的条数」。
`is_reset` 是 2026-10-02 随单一循环池加的列（`models.ReviewLog.IsReset`），**旧日志该列全为 0**。
所以：如果一个词的进度行被清掉、日志还在，今天再学它会被算成「新学」一次——这是预期行为
（进度确实没了，对新用户来说它就是新词），但排查统计异常时要先想到这一点。

---

## 8. `.wasm` 的缓存要配版本号，否则更新引擎在用户那里「不生效」

**这是真实会伤到线上用户的一条**（本地开发也已经踩过一次，排查了很久）：

`dev-server.js:161` 与 [`README.md`](README.md) 的 Nginx 配置都按扩展名分流缓存策略，
`.wasm` 落在「非代码类」那一档 → **`public, max-age=86400`，强缓存一天**。
于是重建 `modules/english/engine/pkg/` 之后，浏览器跑的仍是旧引擎，**不报任何错**：
我修好的置顶排序在页面上始终没反应，一度以为是算法没生效。

**当前兜底**：`modules/english/english.js` 顶部的 `ENGINE_VERSION` 会拼进 `.js` 与 `_bg.wasm` 两个 URL
（`loadEngine()`），改引擎重建后手动 +1 即可绕开缓存。规矩已写进 [`docs/boundaries.md`](docs/boundaries.md) 第 8 节。

**更好的做法（待办）**：把版本号接进构建（例如读 `pkg/` 的哈希或构建时间写进一个 `engine-version.js`），
让「改了引擎忘了 +1」不再可能；Nginx 侧也考虑给 `.wasm` 发 `no-cache` + ETag（wasm 有 300KB，
每次条件请求换 304 很划算，强缓存一天的收益并不值得「更新不生效」这个风险）。

---

## 9. 多人共用出口 IP 会被登录限流一起锁住 15 分钟

`backend-rust/src/config.rs:150`：`login_ip: RateRule::new(20, Duration::from_secs(15 * 60))`
—— **单 IP 15 分钟 20 次登录**。教室里一个班共用一个校园网出口时，第 21 个人开始就会拿到 429，
而且之后 15 分钟内**同 IP 的所有人**都登不进来（不是他自己失败，是全班一起失败）。

验收脚本反复跑也会撞上它（记账在 Rust 进程内存里，重启账号服务即清零，
见 [`docs/boundaries.md`](docs/boundaries.md) 第 10 节）。

**候选方案**（等用户拍板，涉及安全取舍）：

- **(a) 提高阈值**（例如 60/15min）：最省事，但弱化了暴力破解防护；
- **(b) 按「邮箱 + IP」而不是只按 IP 计数**：同 IP 不同账号互不影响，攻击者仍受邮箱维度限制；
- **(c) 只对**失败**的登录计数**（成功即清零）：正常用户几乎不受影响，撞库仍被挡住 —— 推荐；
- **(d) 放行内网/校园网网段**：不通用，且香港服务器看不到校园网拓扑，不建议。

真正放人之前必须先把这条改掉，否则第一批用户会集体撞墙。
