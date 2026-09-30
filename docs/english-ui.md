# 英语页界面与动画

> 原先在 `CLAUDE.md`，2026-09 拆分。改英语页 DOM / CSS / 动画前读一遍。
> ⚠️ 这里所有「换屏 / 换状态」的动画都受 `CLAUDE.md` 工作流程第 5 条约束：**先与用户商量、由用户拍板**，不得顺手改。
> 调度与引擎逻辑见 [`review-engine.md`](review-engine.md)；跨文件时长清单见 [`../TODO.md`](../TODO.md) 第 4 条。

- **起始页（2026-09 新增）**：进英语页看到的第一屏是 `#studyStart`（中间一颗「开始复习单词」+ 一行淡色提示），`#reviewApp` 默认 `display:none`（**写在标签上**，不能等 JS 藏：`main.js` 是先注入片段再动态 import，首帧会闪出空复习界面）。**复习会话推迟到点击之后才建**（`beginReview`），原因有二：① 一挂载就建卡会自动朗读，而浏览器拦截「无用户交互」的语音，首次朗读被丢弃后 `unlockSpeech` 会在用户第一次点击时补读一遍 —— 听感就是「进来响一次、随便点一下又响一次」；② 路过英语页的人不必下载 WASM、不发那四个接口请求。
- **起始页 → 复习界面的过场**（460ms，与「点头像进个人中心」那段齐平）：点击后按钮先进「准备中」（`setPreparing`，文案 + 变淡 + disabled），**等第一张卡渲染好才换幕** —— 先换屏就会看到「空单词 + 正在加载」那一拍。时序：`0ms` 套用沉浸偏好（导航栏开始上滑、内容区开始上移 120px）+ 起始页淡出上移 → `160ms` 换幕（藏起始页 / 放复习界面，淡入下浮）→ `460ms` 收尾（摘动画类、恢复按钮、补上被压住的首卡朗读）。**节拍在 `english.js` 的 `START_*`，动作在 `english.css` 的 `startOut` / `startIn`，导航栏与内容区那两条过渡在 `main.css`** —— 三处时长必须一起改。首卡的自动朗读由 `state.holdAutoSpeak` 压到 `finishEnter` 才补，否则单词会比画面先出声。
- **复习 UI**：`modules/english/english.html` 含 `#reviewApp`，由 `english.js` 的 `initReviewApp()` 驱动（`main.js` 的 `initSubjectModule()` 动态 import）。界面为**极简全屏**风格：顶栏（沉浸模式开关 / 今日新学 / 今日复习 / 剩余待学 + 记忆元信息）、大字号单词 + 音标胶囊、底部操作区、**左下角计划小字**；揭晓后主例句目标词高亮 + 中文翻译，下面是**一条条释义块**（一个词性一块，可各带例句与译文）。键位：空格揭晓，`Q/W/E/R`（或 `1~4`）评分，`P` 读单词，`L` 读例句（`E` 被「一般」占用），`Esc` 切换沉浸模式。
- **左下角计划小字**（`#studyPlan` / `#studyPlanText`，绝对定位在 `.study-foot` 左下、11px 极淡色、`pointer-events: none`）：计划阶段显示「新词 3/5 · 抽查 2/5」，计划区走完的那一刻**带动效切换**成「计划完成 · 进入复习阶段」（`revealUp` 同族的 `planOut` / `planIn`）。
  - ⚠️ 两个坑：① 分母（`state.planTotals`）**建会话时算一次后固定**，不能每次渲染重算，否则分子涨分母也涨（会出现「新词 1/4、2/5」）；② `setPlanText` 在文案未变时**直接返回、不要清理动画类**，`markPlanDone` 也**不刷界面**——否则换卡时那次渲染会把刚起步的切换动画掐断（踩过，表现为「动画没播」）。
- **沉浸模式**：进复习页默认给 `body` 挂 `is-immersive`（隐藏站点导航栏、内容区占满整屏），样式在 `main.css` 的通用规则 + `english.css` 自己的留白里，偏好存 `reviewImmersive`。**切学科时必须摘掉**，由 `main.js` 的 `teardownSubjectModule()` 调用模块导出的 `unmount()` 完成（框架级收口点，别把清理逻辑写回 `main.js` 各学科判断里）。
  - ⚠️ **2026-09 改：隐藏导航栏不再用 `display: none`**（`display` 不可过渡，导航栏会「啪」地消失、内容区同时瞬移 120px，任何淡入淡出都会露出这一下跳变）。现在是**上滑淡出**：`transform: translateY(-100%)` + `opacity: 0` + `visibility: hidden`（配 `0s` 时长 + `460ms` 延迟，等滑完才真正隐藏；纯 `opacity:0` 仍能被 Tab 聚焦到头像）+ `pointer-events: none`，`.content-container` 的 `margin-top` / `padding` 一起过渡 —— 整屏像被「往上顶」一下。**这条是全站共享规则**，改动它会影响所有用沉浸模式的页面。
  - 归零的两条（隐藏导航栏、`.content-container` 归零）**必须成对改**：只改一条就会出现「导航栏滑走了、内容还留在 100px 下方」的错位。
- **揭晓动效**：不是整块淡入，而是「显示答案」按钮缩小淡出 → 主例句 → 译文 → 各释义块依次错开浮现（块内高亮再用 `background-size` 从左往右扫出来）→ 按钮退场动画播完的**同一刻**四个评分按钮从原位置依次顶上来（浮起 + 由小变大）。**节拍时刻**在 `english.js` 的 `playRevealAnimation()`（`REVEAL_*` 常量，靠内联 `animation-delay` 下达；`REVEAL_OUT_MS` 是换人时刻），**动作定义**在 `english.css` 的 `revealUp` / `revealOut` / `ratingIn` / `hitSweep`；`renderCardNow()` 与 `unmount()` 会调 `cancelRevealAnimation()` 把换人定时器与收尾定时器都清掉。⚠️ 两条铁律：① **单词不参与任何动画**，揭晓时必须留在原位（`.study-stage` 用 `justify-content: flex-start` 而非 `center`，否则答案变高会把单词顶上去，实测 1 条释义 53px、3 条 159px）；② **必须先藏揭晓按钮再放评分按钮**，两者是底部操作区相邻的两个块，同时显示会让操作区变高、把单词顶上去。
