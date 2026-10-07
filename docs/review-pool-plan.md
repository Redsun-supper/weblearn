# 单词算法重构：单一循环池（Review Pool）

> 本文是 2026-10-02 与用户逐条讨论后的**定案计划**。20 条决策全部由用户拍板，理由一并记下，
> 便于日后回看"为什么这么设计"。实施前请先读第 4 节的三处冲突与第 7 节的验收口径。

## 1. 为什么要改（问题陈述）

用户实测发现：**登录后 100 个单词只有 5 个在轮回**（详见 `TODO.md` 与 `docs/boundaries.md` 的对应条目）。

根因不是登录系统，而是**两条旧口径叠加**：

1. `modules/english/english.js:51` 把「每天 5 个新词」硬编码在客户端，配额记在浏览器 `localStorage`
   （键 `reviewDailyPlan`）。数据库实证：`words=100`，但 `word_reviews` 只有 **5 行**（全属 `user_id=1`），
   其余 95 个词**从来没进过复习队列**。
2. 复习侧有三段拼接（新词 / 抽查 / 复习区，`modules/english/engine/src/session.rs:269-276`），
   而"新词"段每天只放 5 个 —— 于是整个会话的池子就只有那 5 张卡，
   `CHUNK_SIZE = 10` 与 `SEEN_GUARD = 10` 的随机/防重复机制全部退化失效。

**更深一层**：FSRS 的正确行为是"记得越牢、间隔越长"，学得好的词会被推到几个月甚至几年后。
用户要的是"能无限学下去"，所以需要**取消批次上限 + 给间隔封顶 + 用循环池让每张卡都有机会回来**。

## 2. 新模型（一句话）

> **词库是一池水，每个用户一口自己的池子；每天先随机舀 5 个到最前面，学完了就顺着池子继续舀。
> 每张卡评完按 FSRS 算下次到期时间，插回池子。没有"已学/未学"的身份差别，只有"到没到期"。**

## 3. 定案（20 条，2026-10-02 用户逐条确认）

| # | 决策 | 说明 / 理由 |
|---|---|---|
| A1 | 池子按**未来 3500 词**规模设计，当前 100 词即样例 | 查询必须分页与索引友好，不能"一次全拉" |
| A2 | 每天**从整个池子**随机抽 5 个放最前 | 抽到已学词时按 Q11 的新卡口径重算 |
| A3 | 池子顺序**按用户随机排列**且**稳定** | 同一用户刷新/翻页顺序一致；不同用户不同。用 `(user_id, word_id, 当地日期)` 确定性哈希，**不需要新表** |
| A4 | 池子物理形态 = **`words` 全表参照 + `word_reviews` 存该用户到期状态** | 不建池表；管理员加词自动进入所有人池子 |
| B5 | 首次复习**直接复用**现有 `(None, 0)` 新卡路径 | `session.rs:794` 已验证的路径 |
| B6 | `stability`/`difficulty` **继续存库** | 跨设备要靠它算同一个下次到期时间；不存就得重放全部历史日志 |
| B7 | 首次评完 `INSERT` 进度行，二次起 `UPDATE` | 行的存在只表示"有到期时间"，**不再是身份标记** |
| B8 | 最大间隔**封顶一年**（365 天） | FSRS 满分时会把间隔拉到几个月/几年，这是"学不下去"的真正来源 |
| B9 | 整池都推到未来时，**允许无视到期继续往后翻** | 与现状一致（`queue` 给整池，只是要往下翻） |
| C10 | 优先级总序：**今日 5 个 → 已过期（越久越前）→ 未复习过 → 未到期** | 保留"优先复习已过期"的原设计 |
| C11 | 今日 5 个里若命中已学词，按**新卡口径**重算（丢状态、天数按 0） | 与抽查一致 |
| C12 | "抽查"**降级为池子里的一类排序权重**，不再独立成段 | 循环池本身已承担"让冷门词露面"的职责 |
| C13 | 未复习过的词排在**已过期之后、未到期之前** | `due_at` 为 NULL 不再排最前（现状是 `sort_key = -inf` 排最前） |
| D14 | 5 个是**起步量不是硬上限**，学完可继续 | 这是"无限学习"的关键 |
| D15 | 配额**迁到服务端** | 今日已学 = 当天 `stability_before = 0` 的日志条数，**现成字段，不需要新表** |
| D16 | 一轮过完的文案改为「**整池已过一遍，继续复习不受限**」 | 现文案「本轮已过一遍 · 可以继续」在 `english.js` |
| D17 | 顶栏改为「**今日已复习 N / 池内到期 M / 池内总数 100**」 | "剩余待学"在新模型下无意义 |
| E18 | `word_reviews` 保留 `stability/difficulty`，响应里**也下发**（~~原先删掉~~，2026-10 改回） | 原动机只有一条：修「非指针 `float64` 把 SQL NULL 扫成 0」的坑，改用**指针**就已解决；「删列」是误读 C11 —— 重置是**按卡**判定的，删列会让**所有**卡退回新卡路径（见下表后的注） |
| E19 | `review_logs` **保留全字段** | 它是"今日新学"与后续 FSRS 参数优化的唯一依据 |
| E20 | 老数据**暂时丢弃**；不加 `REVIEW_MODEL` 回退开关 | 清空 `word_reviews` 与 `review_logs`，`words` 保留（词表是资产） |

> ⚠️ **E18 已于 2026-10 改回下发**（`poolSelectSQL` 重新 SELECT `wr.stability` / `wr.difficulty`）。
> 实测（探针 `.tmp-test/e18-report.mjs`，直接跑构建好的 wasm）：
>
> - **删列时**：一个 `due_at=昨天`、`last_review_at=5 天前` 的成熟卡，四评分恒为
>   Again **0.2120** / Hard **1.2931** / Good **2.3065** / Easy **8.2956** 天，换一个全新会话再评，
>   数字逐位相同（= 跨会话零累积）；右上角元信息恒为 `{"status":"new"}`。
> - **加回后**：`stability=30`·5 天前 → Good **42.3071** 天；`stability=120`·40 天前 →
>   Good **204.9858** 天、Easy **279.1699** 天（这才够得着 B8 的 365 天封顶，也才让
>   「难度 / 稳定性 / 复习 / 上次 / 预计记住」五行有东西可显示）。
> - **C11 不受影响**：置顶卡（`daily=true`）即使带着状态，Good 仍是 **2.3065** 天、
>   `is_reset=true` —— 因为重置发生在引擎里（`session.rs` 的 `is_reset_card` → `try_rate` 的
>   `if reset_card { (None, 0) }`），与响应带不带状态无关。
> - **已知小瑕疵（未修）**：带状态的置顶卡会在右上角显示**旧的**难度/稳定度（`status:"probe"`），
>   而评分按新卡算 —— 显示与计算不一致。要让两者一致，须在 `plan_day` 里对 `is_daily` 的卡也置
>   `state: None`，并重建 `pkg/`（`ENGINE_VERSION` 记得 +1）。

## 4. 三处与现有实现的冲突（实施时必须处理）

### 4.1 「今日 5 个」不能用 `/api/reviews/new` 的口径

`/api/reviews/new` 喂的是"没有进度行的词"。新模型下池子 = `words` 全表，
**A2 的"抽 5 个"必须从全池抽**，抽到已学词按 C11 的新卡口径重算。
→ `NewWords` 接口在新模型下**废弃**（保留路由以免旧前端 404，返回池子首页）。

### 4.2 服务端与客户端对"今天"的口径必须一致（否则跨零点会错乱）

现状 `review_handlers.go:719` 用的是**服务器本地时区**零点：

```go
startOfToday := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())
```

香港/国内部署时该时区是 UTC+8，与国内用户一致，**当前可接受**；但它对海外用户不成立，
而且 A3 的哈希种子与 D15 的今日统计**都依赖"哪一天"**。
→ 实施时由客户端传本地日期或时区偏移（`?tz=+08:00` 或 `?today=2026-10-02`），
服务端据此算 `startOfToday` 与哈希种子；缺参数时回落到服务器本地时区（向后兼容）。

### 4.3 「重置重学」会把"今日新学"统计冲高（必须加标记位）

C11/现在的抽查都会把卡片**按新卡重算**，而日志里的 `stability_before` 是从库里读的
**真实旧值**（`session.rs:794` 的 `(None, 0)` 只是 `compute_next_states` 的入参，不是日志值）。
但另一个信号——`interval_days`——在重置时确实是 `0`（`session.rs:504` 的
`planInfo.probeIntervalDays = 0`）。

两种口径都会出问题：一旦"重置"从每天 5 个变成常态，**"今日新学"会把当天的重学也算进去**。
→ 在 `POST /api/reviews/submit` 的请求体里加**显式的重置标记**（引擎已经知道这张卡是不是重置卡），
日志写 `is_reset`；`ReviewStats` 的"今日新学"只认**真正的第一次学**（此前无任何日志/进度行）。
`ReviewLog.IsProbe`（`models.go:153`）字段语义扩展为通用的重置标记，注释同步改。

## 5. 排序算法（服务端 SQL 口径）

四个优先级桶，桶内再排序。**全部在 SQL 里表达**，避免"客户端排序导致翻页不稳"：

```
bucket 0  今日 5 个     : word_id ∈ 今日集合(seed=hash(user_id, tz_date))  → 桶内按当日哈希
bucket 1  已过期        : due_at IS NOT NULL AND due_at <= now             → 桶内 due_at ASC
bucket 2  未复习过      : 无进度行                                          → 桶内按稳定哈希
bucket 3  未到期        : due_at IS NOT NULL AND due_at >  now             → 桶内 due_at ASC
```

- **稳定哈希**：SQLite 无 `hash()`，用整数运算的组合哈希
  （如 `((user_id*2654435761 + word_id*40503) * 1103515245) & 0x7FFFFFFF`），
  ⚠️ 必须在 Go 侧先写探针**实测**其确定性与分布（避免溢出被提升为 float 导致精度丢失）。
- **今日集合**：同一 `(user_id, 日期)` 下取哈希最小的 5 个 word_id；当天固定、翻页不重、换设备一致。
- **分页**：`ORDER BY bucket, 桶内键` + `LIMIT/OFFSET`；`total` 改为**池子总词数**（不再是"已学词数"）。
- `limit` 上限从 500 放宽（3500 词规模下前端仍需分页，但管理员/调试可能需要更大页）。

## 6. 实施步骤（按依赖顺序）

| 步 | 内容 | 产出 / 判据 |
|---|---|---|
| 0 | 写成本文档 | ✅ 本文 |
| 1 | `backend-go/cmd/discard-progress`：清空 `word_reviews` + `review_logs`（保留 `words`），**先备份、要 `--yes`** | `cmd/backup` 已存在的模式；E20 |
| 2 | Go：`learnedQuery` → 池查询（LEFT JOIN + 四桶排序 + 稳定哈希 + 分页），`stats` 改口径，`submit` 加重置标记，B8 的 365 天封顶 | `go test ./...` 61 项不回归 + 新增池排序测试 |
| 3 | WASM 引擎：`session.rs` 的 `plan_day` 去三段化、`to_queue_inputs` 对"无状态卡"的构造、D16 文案、C13 排序键 | `cargo test` 118 项不回归 |
| 4 | **重建 `pkg/`**（`cargo build --target wasm32-unknown-unknown --release` → `wasm-bindgen`） | ⚠️ 现有 `pkg/` 是 2026-09-13 的，**落后于源码**（源码最后改于 09-18），必须重生成 |
| 5 | 前端 `english.js`：删 `DAILY_NEW_TARGET`/`remainingQuota`/`loadTodayPlan` 的配额账（D14/D15），顶栏三段数字改版（D17），一轮文案（D16） | 手工端到端 |
| 6 | 验证：`pwsh scripts/verify.ps1`（三套测试）+ `pwsh scripts/verify-auth.ps1`（18 项鉴权不回归）+ 浏览器端到端 | 全绿 |
| 7 | 文档同步：`docs/review-engine.md`（三段编排口径作废）、`README.md`（接口字段）、`TODO.md`（本条结案） | 与代码一致 |

### 明确不做（本次范围外）

- 不加 `REVIEW_MODEL` 回退开关（E20）。
- 不动 `word_reviews` 表结构（E18：字段留；**响应字段 2026-10 已改回下发**，见第 3 节的注）。
- 不做管理面板的池子可视化（属 P1）。
- 不动 P0-1 的按人隔离（`idx_word_reviews_user_word` 继续用）。

## 7. 验收口径（改完必须逐条给出实测数字）

1. **新用户第一次进复习页**：顶栏 `池内到期` 为 0、`池内总数` 100；队列第一页是**随机 5 个**（不是 `abandon` 起的前 5 个）。
2. **同一天多次刷新**：这 5 个**不变**（服务端种子哈希）；`total` 恒为 100。
3. **翻页**：第 1 页与第 2 页**无重复 word_id**；`ORDER` 满足四桶顺序。
4. **评分后**：`word_reviews` **INSERT 一行**（首次）/ UPDATE（第 2 次起）；`due_at` = `now + min(FSRS 间隔, 365 天)`。
5. **今日 5 个学完**：`stats.today_new = 5`；再继续学，`today_new` **不再增长**（不虚高），`today_reviewed` 继续涨。
6. **间隔封顶**：把某个词的状态手动设成 `stability = 10000` 天，评分 Easy，`due_at` 距今**不超过 365 天**。
7. **整池都推远**：把全部进度行的 `due_at` 设成未来 → 队列**仍不空**（bucket 3 还能往下翻，B9）。
8. **不回归**：`scripts/verify-auth.ps1` 18 项、`go test ./...` 61 项、`cargo test`（引擎 118 + 后端 90）全绿。

## 8. 风险

| 风险 | 应对 |
|---|---|
| 老数据丢弃是**不可逆**操作 | 第 1 步的命令强制先跑 `cmd/backup`，且必须显式 `--yes`；先备份再清 |
| 稳定哈希在 SQLite 里溢出/精度丢失 | 第 2 步先写探针实测分布与确定性，再写进查询 |
| 排序键变了但 `total` 口径没跟着变 → 前端翻页算错 | `total` 与查询**同一口径**（现有代码 `review_handlers.go:549` 已有这条注释，继续遵守） |
| `pkg/` 重建后行为变化（当前浏览器跑的是 09-13 的旧引擎） | 重建后必须做一次端到端；`docs/boundaries.md:63-65` 有完整命令 |
| 3500 词规模下排序开销 | 四桶排序走 `temp b-tree`；实测单页耗时，必要时给 `due_at` 加复合索引（已有 `idx_word_reviews_user_due`） |

## 9. 实施结果（2026-10-02 完成，逐步对照第 6 节）

| 步 | 状态 | 实际产出 |
|---|---|---|
| 0 | ✅ | 本文档 |
| 1 | ✅ | `backend-go/cmd/discard-progress/main.go`（`-yes` 才真删；删前自动 `snapshot.Snapshot`；清后校验词表行数不变） |
| 2 | ✅ | `internal/stablehash`（含真 SQLite 一致性测试）+ `review_handlers.go` 四桶池查询 / stats 新口径 / `is_reset` / 365 天封顶 |
| 3 | ✅ | `session.rs`：`CardSource::Daily`、`ApiCard.is_daily`、`SubmitPayload.is_reset`、`MAX_INTERVAL_MS` 封顶、`is_reset_card()` |
| 4 | ✅ | `pkg/guangxue_wasm_bg.wasm` **289,468 B**（原 287,705 B，2026-09-13 的旧产物），2026-10-02 17:12 重建 |
| 5 | ✅ | `english.js` 删掉全部配额账（`PLAN_KEY`/`loadTodayPlan`/`markPlanDone`/`remainingQuota`/`probedIdsInCooldown` 已无引用）、顶栏三段改版、一轮文案改 D16、新增 `todayParam()` 与 `dailyProgress()` |
| 6 | ✅ | `verify.ps1` 6/6、`verify-auth.ps1` 18/18、`verify-pool.ps1 -Browser` **45/45**（28 项 HTTP + 15 项浏览器端到端 + 2 项前置自检） |
| 7 | ✅ | `docs/review-engine.md` 重写编排段、`TODO.md` 加第 6/7/8/9 条、`docs/boundaries.md` 加第 8/9/10 节、本文档加本节 |

### 第 7 节 8 条验收口径的实测

| # | 口径 | 实测 |
|---|---|---|
| 1 | 新用户看到整池、置顶是随机 5 个 | `total=100`、`items=100`、`daily=5` 且排最前 5 位；今日抽中 id `[10,31,52,73,94]`（不是 `abandon` 起的前 5 个） |
| 2 | 同一天多次刷新不变 | 两次请求 item 序列**逐字一致**（长度 291 的 id 串相同）；明天与今天置顶**重复 0 个** |
| 3 | 分页无重复、四桶有序 | 第 1 页 40 词 / 第 2 页 40 词，**重复 0**；桶序列 `0,0,0,0,0,2,2,2,2,2,2,2` 单调不减 |
| 4 | 评分落库 + `due_at` 封顶 | 首次提交 `reps=1`；提交 999 天 → 回传 `interval_days=365`、`capped=true`、due 距今 365.0 天 |
| 5 | 今日新学不被重置冲高 | 三次提交（1 次 `is_reset=false` + 2 次 `is_reset=true`）→ `today_reviewed +3` 而 `today_new **+1**` |
| 6 | 间隔封顶（引擎侧） | `cargo test interval_is_capped_at_one_year`：`stability=20000` 评 Easy → `interval_days ≤ 365`，插回池子的 `due_ms` 同样受限 |
| 7 | 整池推远也不空 | 桶 1（已过期）排在桶 2（从未复习）之前；桶 3 仍可继续翻（`items=20` 断言队列永不空） |
| 8 | 不回归 | `verify.ps1` 6 步 PASS（34.7 s）、`verify-auth.ps1` **18/18**（5.3 s）、`go test ./...` 5 包全 ok、`cargo test` **122 项**、账号服务测试 PASS |

### 浏览器端到端实测（Edge 无头 + CDP，2026-10-02）

- 请求链：`index.html` → `engine/pkg/guangxue_wasm.js?v=N` + `_bg.wasm?v=N` → `queue?limit=100&offset=0&today=20261002` → `stats?today=…` → `submit` → `stats`（提交后自动刷新）。
- 界面读数：顶栏「今日已复习 N / 池内到期 0 / 池内总数 100」，左下角「今日置顶 N/5 · 继续复习不受限」；连评到置顶走完后变「整池已过一遍，继续复习不受限」；`localStorage.reviewDailyPlan` 为 **null**。
- 控制台 error **0** 条、未捕获异常 **0** 条、复习接口无 4xx/5xx。

**这一轮端到端抓到的四个真实缺陷（前三个都是纯单元测试看不见的）**：

1. **服务端字段名 `daily` 与引擎的 `is_daily` 对不上**：`#[serde(default)]` 把它静默读成 `false`，5 张置顶卡被当成普通卡埋进池子。Rust 侧测试夹具全是按**结构体字段名**手写 JSON 的，所以一路绿灯；只有走真实响应才会暴露。修法是 `#[serde(rename = "daily")]` + 新增 `api_card_reads_server_daily_key`（用真实键名写 JSON）。
2. **置顶卡被随机窗口埋掉**：`pick_index` 的窗口是「最靠前 10 张里随机抽」，里面混着大量「从未复习」的普通卡 —— 实测连评 7 张只碰到 2 张置顶卡，「今日置顶」停在 4/5。修法是新增 ①b 分支：**还没评过的置顶卡一张不落地先出完**（服务端已用稳定哈希排好它们之间的顺序，引擎不必再随机）。新增 `all_daily_cards_come_out_before_the_window_mixes_them`（40 张普通卡 + 5 张置顶卡，断言前 5 张必须全是置顶卡）。
3. **进度分子虚高**：界面用「服务端快照与本地计数取大」，两者不是同一个量 —— 连评 6 张时报 5/5「整池已过一遍」而服务端才 3/5（用户被提前告知完成）。修法是改成**增量式**：分子 = 建会话时的服务端基线 + 本会话增量，每次 stats 刷新重设基线。
4. **引擎里还留着活的三段编排**（收尾自查时发现，不是端到端抓的）：`plan_day` 仍会按 `new_limit` / `probe_limit` 把 `new` / `probes` 拼到池子最前。前端传的是空数组 + 0，所以界面上完全正常 —— 但**代码路径是活的**：`legacy_new_and_probe_inputs_are_completely_ignored` 一写就红了（池子从 3 张变 11 张）。修法是把 `plan_day` 削成「照抄队列 + 标来源」，`plan_len` / `new_count` / `probe_count` 一律 0；`plan_*` 那批测老编排的用例跟着改写成「断言老输入不再有任何效果」。

**另外四个「验证基础设施」的坑**：

- ⚠️ `fetchDay` 的返回下标必须跟着请求清单改（旧版统计在 `results[3]`，漏改后统计恒为 0 且**不报错**）；而且**新拿到的 stats 必须存进 state 并重设基线** —— 早先只返回给调用方、自己没存，于是建会话时那次同步读到的仍是上一次会话的旧快照（界面从「2/5」起步而服务端是 0/5）。
- ⚠️ `markDailyRated` 必须在 `session.rate()` 之后、`advanceCard()` 之前，否则左下角停在旧分子（实测「评完还是 2/5 不涨」）；判「是不是置顶卡」要用**引擎返回的请求体里的 `source`**，不能用界面上的 `state.card.source`（换卡过渡期间它可能已指向下一张，会漏记一张）。
- ⚠️ 手工验证时必须走 **`/index.html`**（SPA 外壳）。直接开 `/modules/english/english.html` 只有 HTML 片段、没有 `main.js`，按钮点了没反应，且 `href="modules/english/english.css"` 会被解析成 `/modules/english/modules/english/english.css`（404）。
- ⚠️ **改完引擎重建 `pkg/` 后必须把 `english.js` 的 `ENGINE_VERSION` 加一**：`dev-server.js` 给 `.wasm` 发一天强缓存，不加版本号时浏览器跑的仍是旧引擎且**不报错**（本次为此白排查了很久）。
