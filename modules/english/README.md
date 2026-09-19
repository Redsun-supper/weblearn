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

## 复习页的三个细节

- **沉浸模式**：默认开启，隐藏站点导航栏（`body.is-immersive` + `main.css` 的通用规则），
  顶栏左上角开关或 `Esc` 切换，偏好存 `reviewImmersive`。
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

## 三个容易踩的坑

1. **改了 `english.html` 的结构后，必须把 `main.js` 里的 `CACHE_VERSION` +1**。
   学科页会被 localStorage 缓存 30 天，否则老用户会拿旧结构配新脚本。
   （历史：v2 加「显示答案」按钮；v3 英语页迁到本目录；v4 复习界面改版；v5 沉浸模式开关 + 多释义区）

2. **`import()` 的路径不能省略 `./`**。写 `'engine/pkg/guangxue_wasm.js'` 会被浏览器
   当作 npm 包名（bare specifier）解析并直接报错。

3. **`fsrs_next_states` 返回的 `NextStatesOut` 不是给 JS 用的**。JS 只需用
   `ReviewSession`；低层导出保留是为了调试与将来复用，不要在前端重新拼装评分逻辑。

4. **沉浸模式的状态不能留在 `body` 上过夜**。切学科、离开复习页都必须摘掉 `is-immersive`，
   否则导航栏会在别的学科页上继续消失（现在由 `unmount()` 负责，见上）。
