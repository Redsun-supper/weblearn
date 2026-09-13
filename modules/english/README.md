# 英语模块

英语学科的全部文件都在这个目录里，便于独立管理。

```
modules/english/
├── english.html       学科页片段（被 main.js fetch 后注入 #contentContainer，可被 localStorage 缓存）
├── english.css        模块样式（只作用于英语页内的元素）
├── english.js         模块逻辑（ES module，由 main.js 动态 import）
├── engine/            Rust/WASM 引擎（队列编排 + FSRS 调度 + 随机化 + 词表解析）
├── admin/             词条管理后台模块（由 admin/ 的通用后台按需加载）
│   └── english-admin.js
└── README.md          本文件
```

## 职责划分

| 层 | 负责 | 不负责 |
|----|------|--------|
| `english.js` | 学生端：DOM 渲染、事件绑定、取数与提交（`fetch`）、`localStorage`、语音合成 | 任何计算 |
| `engine/`（Rust） | 队列构建（洗牌 / 抽新词）、`days_elapsed` 换算、FSRS 调度、评分决策、进度统计、**词表文本解析** | 任何 DOM |
| `admin/english-admin.js` | 管理端：词条列表 / 搜索 / 分页、新增编辑删除、批量导入的预览与分批提交 | 任何计算 |

具体地说，**队列与游标由 Rust 侧的 `ReviewSession` 持有**，JS 只在渲染时向它索取
当前卡片的字段、在评分时拿到一个可直接 POST 的请求体。详见
[`engine/README.md`](engine/README.md)。

后台（`admin/`）是**独立入口页**，与学科页互不影响：它不经过 `main.js`，而是由根目录
`admin/` 下的通用后台框架按需 `import` 本目录的 `admin/english-admin.js`。
详见 [`../../admin/README.md`](../../admin/README.md)。

## 与主框架的接口

`main.js` 里只有一处与英语模块相关（`initSubjectModule`）：

```js
const ENGLISH_PAGE = 'modules/english/english.html';

function initSubjectModule(pageName) {
    if (pageName !== ENGLISH_PAGE) return;      // 尚无独立模块的学科直接返回
    import('./modules/english/english.js').then(function(mod) {
        mod.initReviewApp();
    }).catch(...);
}
```

约定：**学科模块放在 `modules/<学科>/` 下，导出一个初始化函数；`main.js` 只负责
「按需动态加载 + 调用」**。这样学科逻辑不会回流到 `main.js`，且只有真正进入该学科页
才会加载它的模块与引擎。

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
   （历史：v2 加「显示答案」按钮；v3 英语页迁到本目录）

2. **`import()` 的路径不能省略 `./`**。写 `'engine/pkg/guangxue_wasm.js'` 会被浏览器
   当作 npm 包名（bare specifier）解析并直接报错。

3. **`fsrs_next_states` 返回的 `NextStatesOut` 不是给 JS 用的**。JS 只需用
   `ReviewSession`；低层导出保留是为了调试与将来复用，不要在前端重新拼装评分逻辑。
