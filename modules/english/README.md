# 英语模块

英语学科的全部文件都在这个目录里，便于独立管理。

```
modules/english/
├── english.html       学科页片段（被 main.js fetch 后注入 #contentContainer，可被 localStorage 缓存）
├── english.css        模块样式（只作用于英语页内的元素）
├── english.js         模块逻辑（ES module，由 main.js 动态 import；导出 initReviewApp / unmount）
├── engine/            Rust/WASM 引擎（队列编排 + FSRS 调度 + 随机化 + 词表解析 + 卡片文本）
├── admin/             词条管理后台模块（由 admin/ 的通用后台按需加载）
│   └── english-admin.js
├── FUTURE.md          后续考量（用户提过、这版刻意没做的设计）
└── README.md          本文件
```

## 职责划分

| 层 | 负责 | 不负责 |
|----|------|--------|
| `english.js` | 学生端：DOM 渲染、事件绑定、取数与提交（`fetch`）、`localStorage`、语音合成、沉浸模式开关、**每日配额的账**（今天学了几新词/抽查了几个、最近抽过哪些词） | 任何计算 |
| `engine/`（Rust） | 队列构建（**每日计划编排** / 梯度乱序 / 抽新词 / 抽查挑选）、`days_elapsed` 换算、FSRS 调度、评分决策、进度统计、**词表文本解析**、**多释义拆分与例句高亮切分** | 任何 DOM、任何 `localStorage` |
| `admin/english-admin.js` | 管理端：词条列表 / 搜索 / 分页、新增编辑删除、**多释义与例句翻译录入**、批量导入的预览与分批提交 | 任何计算 |

具体地说，**队列与游标由 Rust 侧的 `ReviewSession` 持有**，JS 只在渲染时向它索取
当前卡片的字段、在评分时拿到一个可直接 POST 的请求体。详见
[`engine/README.md`](engine/README.md)。

后台（`admin/`）是**独立入口页**，与学科页互不影响：它不经过 `main.js`，而是由根目录
`admin/` 下的通用后台框架按需 `import` 本目录的 `admin/english-admin.js`。
详见 [`../../admin/README.md`](../../admin/README.md)。

## 与主框架的接口

`main.js` 里只有两处与英语模块相关：

```js
const ENGLISH_PAGE = 'modules/english/english.html';

// 切页前先让上一个模块收回自己挂的全局状态（模块没导出 unmount 就跳过）
function teardownSubjectModule() { /* 调用 activeSubjectModule.unmount() */ }

function initSubjectModule(pageName) {
    teardownSubjectModule();
    if (pageName !== ENGLISH_PAGE) return;      // 尚无独立模块的学科直接返回
    import('./modules/english/english.js').then(function(mod) {
        activeSubjectModule = mod;
        mod.initReviewApp();
    }).catch(...);
}
```

约定：**学科模块放在 `modules/<学科>/` 下，导出一个初始化函数；`main.js` 只负责
「按需动态加载 + 调用」**。这样学科逻辑不会回流到 `main.js`，且只有真正进入该学科页
才会加载它的模块与引擎。

本模块额外导出了 `unmount()`，原因是**沉浸模式把 `is-immersive` 类挂在了 `document.body` 上**，
内容容器换成别的学科后这个类不会自己消失，导航栏会跟着一起不见。切页时由 `main.js` 统一调用，
清理职责留在模块内部（`main.js` 不需要知道任何英语相关的细节）。

## 起始页：为什么进英语页不是直接开始复习

进英语页看到的第一屏是**起始页**（`#studyStart`）：中间一颗「开始复习单词」，**导航栏保持可见**
（不套沉浸模式，用户能正常切学科 / 点头像）。点下按钮才由 `beginReview()` 放出复习界面、
加载 WASM、发那四个接口请求。

这不是为了好看，是为了修一个真实的听感问题：**同一个单词会响两遍**。

- `renderCardNow()` 末尾有 `if (isAutoSpeakOn()) speakCurrent('word')`（自动朗读默认开启）；
- 但浏览器不允许「还没有用户交互」的语音合成，所以这次朗读多半被**静默丢弃**；
- 于是 `unlockSpeech()` 在用户第一次点页面 / 按键时**补读**一遍当前单词。

两者叠起来就是「一进页面响一次、随便点一下就又响一次」。把建会话推迟到点击之后
（`beginReview`），朗读就只落在一次真实交互之后：既不重复，也不会被拦。

附带好处：只是路过英语页的用户不用下载 WASM 引擎、也不发任何接口请求。

⚠️ 配套的两处细节：

- `#reviewApp` 默认 `display:none` **写在 HTML 标签上**，不能等 JS 去藏。`main.js` 是
  「先注入片段、再动态 import 模块」，首次进站那次 import 要走网络，中间会有一帧把复习界面
  （空单词 + 「正在加载」）画出来。
- 起始页用的是 `applyImmersive(false)`（**只改界面、不写 localStorage**），不是 `setImmersive`。
  后者会把用户的沉浸偏好永久改成「非沉浸」—— 每进一次英语页就改一次。

## 起始页 → 复习界面 的过场

总长 **460ms**，与「点头像进个人中心」那段齐平（全站最长的转场就是它）。

```
点击「开始复习单词」
  ├─ 按钮变「正在准备…」+ 变淡 + disabled（setPreparing）
  ├─ 起始页原地不动，等引擎加载 + 四个接口 + 第一张卡渲染好
  ▼
0ms     套用沉浸偏好 → 导航栏开始上滑、内容区开始上移 120px（main.css 的两条 transition）
        起始页挂 is-leaving → 淡出上移
160ms   换幕：藏起始页、放复习界面，挂 is-entering → 淡入下浮
460ms   收尾：摘动画类、恢复按钮、补上被压住的首卡朗读（finishEnter）
```

**为什么等卡就绪才换幕**：换屏到卡片渲染之间，复习界面是「空单词 + 正在加载」。
先换屏就会看到这么一拍空画面，过场再顺也白搭。代价是按钮要顶几百毫秒的「准备中」，这笔账划得来。

**四处要一起改的时长**（本项目不做 CSS 变量统一，见根 `TODO.md` 第 4 条）：

| 在哪 | 负责什么 |
|------|----------|
| `english.js` 的 `START_OUT_MS` / `START_IN_MS` / `START_TOTAL_MS` | 节拍（什么时候换幕、什么时候收尾） |
| `english.css` 的 `startOut` / `startIn` | 起始页怎么退、复习界面怎么进 |
| `main.css` 的 `.rectangle` 与 `body.is-immersive .rectangle` | 导航栏上滑淡出（460ms） |
| `main.css` 的 `.content-container` | 内容区上移 120px（460ms） |

⚠️ **首卡的自动朗读压在 `finishEnter`**（靠 `state.holdAutoSpeak`）：卡片是在起始页还盖着的时候
建好的，照常朗读的话单词会比画面早半秒念出来。

⚠️ **沉浸偏好照办，不为动画让步**：偏好是「不进沉浸」的话导航栏不动，这段过场自然退化成
单纯的淡出淡入 —— 别为了动效好看去覆盖用户的偏好。

⚠️ 系统开启「减少动态效果」时整段过场跳过（`enterReview` 里一步到位），业务逻辑完全一致。

⚠️ **三道「这条链已经作废了」的闸，别删**（都是同一类问题：异步链回来时页面可能早换了）：

1. `pageEpoch`：`initReviewApp()` 与 `unmount()` 各自 +1，`beginReview` 进门先记号，
   回调对不上号就整条丢掉。**必须在建 `ReviewSession` 之前拦**，否则 WASM 会话已经创建、
   内存就漏了。
2. `enterReview()` 开头的 `if (!app) return;`：`main.js` 是**先 `innerHTML` 换页、再调
   `unmount()`**，中间有个窗口 `unmount` 还没跑（世代没变），只能靠「复习界面还在不在」判断。
   少了它，`setImmersive` 挂上去的会是**别人家的 body** —— 数学页的导航栏凭空消失。
3. `unmount()` 清掉两个过场定时器并抹平动画类，否则定时器会在别的学科页上把复习界面「放出来」。

这三条都验证过：用 CDP 把 `/api/reviews/*` 挂起来模拟慢网、在「准备中」期间切到数学页，
再放行那条迟到的链 —— 数学页的导航栏、`margin-top`、朗读次数都必须原封不动。
⚠️ 那份脚本目前**放在仓库外的临时目录**里（没有随代码提交），要长期兜住这三条得先把它收进仓库。

## 复习页的几个细节

- **沉浸模式**：默认开启，隐藏站点导航栏（`body.is-immersive` + `main.css` 的通用规则），
  顶栏左上角开关或 `Esc` 切换，偏好存 `reviewImmersive`。
  ⚠️ 它**从点了「开始复习单词」那一刻起才生效**：起始页恒为「显示导航栏」，
  而且走的是不写偏好的 `applyImmersive(false)`（原因见上一节）。
  ⚠️ **2026-09 起，隐藏导航栏不再是 `display:none`**，而是上滑淡出
  （`transform: translateY(-100%)` + `opacity: 0` + `visibility: hidden` + `pointer-events: none`），
  同时 `.content-container` 的 `margin-top` / `padding` 一起过渡 —— 这条是 `main.css` 里的
  **全站共享规则**，改它会影响所有用沉浸模式的页面；两条（隐藏导航栏 + 内容区归零）必须成对改，
  只改一条会出现「导航栏滑走了、内容还留在 100px 下方」的错位。
- **多释义**：揭晓区是「主例句（高亮 + 中文翻译）+ 一条条释义块」。
  释义块由引擎给：词条填了 `senses` 就用它，没填则按 `meaning` 里的词性标签自动拆
  （所以 `n. 好处；益处 v. 有益于` 这种老数据也能显示成两块）。
- **左下角计划小字**：`新词 3/5 · 抽查 2/5`，计划走完后带动效切换成
  「计划完成 · 进入复习阶段」；过完一整轮（池里每张都评过一次）时会闪一句
  「本轮已过一遍 · 可以继续」，2.6 秒后切回。每日配额的账记在 `localStorage`（键 `reviewDailyPlan`）：
  日期变了就重置计数，`probedAt` 里的抽查记录保留 7 天（冷却期内不再被抽中）。
  ⚠️ 冷却是「反饥饿」的另一半：没有它，今天抽的明天大概率还是那几个（它们的 `due_at` 最远）。
- **无限复习**：评完的卡会按「现在 + 新间隔」插回池子，所以没有结束页、可以一直复习。
  引擎里的抽卡规则与两个踩过的坑见 [`engine/README.md`](engine/README.md) 的「无限复习」一节。

## 本地开发

```powershell
# 1. 首次需要先构建 WASM 引擎（pkg/ 与 target/ 都被 gitignore）
cd modules/english/engine
cargo build --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir pkg --out-name guangxue_wasm `
  target/wasm32-unknown-unknown/release/guangxue_wasm.wasm

# 2. 改引擎后跑单元测试
cargo test

# 3. 起站点（仓库根目录），详见根 README 的「本地启动测试」
node dev-server.js
```

改完 `english.js` / `english.css` 刷新即生效；改完 Rust 需要重新执行上面的构建命令。

## 几个容易踩的坑

1. **改了 `english.html` 的结构后，必须把 `main.js` 里的 `CACHE_VERSION` +1**。
   学科页会被 localStorage 缓存 30 天，否则老用户会拿旧结构配新脚本。
   （历史：v2 加「显示答案」按钮；v3 英语页迁到本目录；v4 复习界面改版；
   v5 沉浸模式开关 + 多释义区；v6 左下角计划小字；v7 起始页 + 会话改为点击后才建）

2. **`import()` 的路径不能省略 `./`**。写 `'engine/pkg/guangxue_wasm.js'` 会被浏览器
   当作 npm 包名（bare specifier）解析并直接报错。

3. **`fsrs_next_states` 返回的 `NextStatesOut` 不是给 JS 用的**。JS 只需用
   `ReviewSession`；低层导出保留是为了调试与将来复用，不要在前端重新拼装评分逻辑。

4. **沉浸模式的状态不能留在 `body` 上过夜**。切学科、离开复习页都必须摘掉 `is-immersive`，
   否则导航栏会在别的学科页上继续消失（现在由 `unmount()` 负责，见上）。

5. **起始页的两道守卫别顺手删掉**：

   - `handleReviewKey` 开头的 `if (!state.started) return;` —— 少了它，在起始页按 `Esc`
     会把导航栏藏起来（起始页刻意要留着导航栏）；起始页的键盘操作交给那颗 `<button>` 自己
     （聚焦后回车 / 空格就是原生点击）。
   - `show()` 里登记了 `reviewApp` / `studyStart` 走 `display: flex` —— 这个兜底分支写的是
     `display: block`，忘了登记就会把整页的纵向布局连居中一起打散。以后新增整页级
     flex 容器同样要在那里登记。
