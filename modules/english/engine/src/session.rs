//! 复习会话编排（WASM 导出层）
//!
//! 把原先散落在 JavaScript 里的「队列构建 → 逐卡记忆上下文 → 评分决策 → 进度统计」
//! 收敛到 Rust，并用 [`ReviewSession`] 持有整个会话状态：
//!
//! - **JS 侧不再保存队列与游标**：只在渲染时向引擎索取当前卡片的字段（减少 JS 堆内存占用）
//! - **不再有反复的 JSON 往返**：评分一次调用即返回可直接 POST 的请求体
//! - **时间换算统一在这里**，且与原 JS 口径逐位一致：
//!   取 `last_review_at`，缺失时回退 `due_at`；`floor((now - last) / 86400000)`，负值归零
//!
//! ## 分层设计
//!
//! - **纯计算层**（[`plan_day`]、[`chunk_shuffle`]、[`days_elapsed`]、[`pick_branch`]、
//!   [`progress_percent`] 以及 `ReviewSession` 的 `try_rate`）：只吃 epoch 毫秒与下标，
//!   错误类型是 `String`，可在宿主环境直接 `cargo test`
//! - **wasm 边界层**（`#[wasm_bindgen]` 那一组方法）：只做「ISO 时间字符串 → 毫秒」的解析
//!   和「`String` 错误 → `JsValue`」的转换，不承载业务逻辑
//!
//! 这样分层是有实际原因的：`JsValue` 在非 wasm32 目标上并未实现，一旦业务逻辑里
//! 碰了它，宿主单元测试就会直接 abort 而不是给出断言失败。
//!
//! ## 每日队列的编排口径
//!
//! 一天的开局不是「把所有到期的抓过来」，而是三段拼接（详见 [`plan_day`]）：
//!
//! 1. **新词**：每天固定几个（默认 5，无放回随机抽），词库没有未学词时这段为空
//! 2. **抽查**：每天固定几个（默认 5），从「到期时间最远」的已学词里挑，最近抽过的不再抽。
//!    目的是防止只按到期时间出题时，间隔长的词永远轮不到（长期不露面就是盲区）
//! 3. **复习区**：其余已学词按紧迫度 `due_at - now` 升序；已过期的严格排序（最旧的先），
//!    未到期的每 [`CHUNK_SIZE`] 个一块、块内打乱（梯度乱序）
//!
//! 队列走完后还能继续往下翻（未到期的也允许提前复习），靠分页取数 + [`append_queue`] 追加。
//!
//! ## 关于 `fsrs` 依赖的一个事实
//!
//! `fsrs` 6.6.2 只公开 `next_states()`，其内部用 `(1..=4).map(..)` 一次性算出
//! Again/Hard/Good/Easy 四个分支，**没有单评分入口**（连私有函数也没有）。
//! 所以这里仍是「算四个取一个」；每个分支只是几十次浮点运算，开销可忽略，
//! 不值得为了省这点计算去重写 FSRS 公式（会造成算法重复与版本漂移风险）。

use std::collections::{HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::card_view::{self, ExamplePart};
use crate::fsrs_engine::{
    compute_next_states, compute_retrievability, CardState, ItemStateOut, NextStatesOut,
};
use crate::randomizer::{random_sample, random_seed};

/// 期望记忆保持率（与原前端一致，固定 0.9）
const DESIRED_RETENTION: f32 = 0.9;

/// 抽卡窗口大小：未到期的卡在「最靠前的 10 张」里随机抽一张（局部乱序、整体仍按紧迫度递减）
const CHUNK_SIZE: usize = 10;

/// 「刚抽过的不再抽」的保护张数：最近出现过的这么多张不参与下一次抽卡。
///
/// 为什么必须有它：评完的卡会按**新的到期时间**插回池子，而它的新到期时间往往就是
/// 「离现在最近」的那个 —— 于是它立刻又成了池首，抽卡会一直抽到同一张。
/// 有了这个保护，同一张卡至少要隔 10 张才会再次出现（池子小到不够 10 张时自动放行）。
const SEEN_GUARD: usize = 10;

/// 最短间隔：与服务端 `review_handlers.go` 的 `math.Max(interval*86400, 600)` 保持一致。
/// 插回池子时的到期时间在引擎里自己算，口径不一致会让排序位置与服务端实际 due_at 有偏差。
const MIN_INTERVAL_MS: f64 = 600_000.0;

/// 一天的毫秒数
const MS_PER_DAY: f64 = 86_400_000.0;

// ---------- 接口数据结构 ----------

/// 后端返回的一条释义（词条的多义项：名词一块、动词一块）
///
/// 对应 Go 侧 `models.WordSense`；整列在库里是 JSON 文本，接口里就是数组。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiSense {
    /// 词性标签，如 "n."
    #[serde(default)]
    pub pos: String,
    /// 该词性下的释义正文
    #[serde(default)]
    pub meaning: String,
    /// 该词性专属例句（可空，空了就用词条本身的例句）
    #[serde(default)]
    pub example: String,
    /// 该例句的中文翻译（可空）
    #[serde(default)]
    pub example_translation: String,
}

/// 后端 `/api/reviews/due` 与 `/api/reviews/new` 返回的词条结构。
///
/// 到期卡会带上记忆状态与时间字段，新词则没有，因此这些字段都是 `Option`。
/// 同时派生 `Serialize`：便于单元测试构造输入，也便于将来把卡片回写给 JS。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiCard {
    pub id: u32,
    #[serde(default)]
    pub word: String,
    #[serde(default)]
    pub phonetic: String,
    #[serde(default)]
    pub meaning: String,
    #[serde(default)]
    pub example: String,
    /// 例句的中文翻译（可空）
    #[serde(default)]
    pub example_translation: String,
    /// 多释义（可空）。⚠️ 必须是 `Option`：Go 侧即使保证输出 `[]`，
    /// 显式 `null` 也会让 serde 的 `#[serde(default)]` 失效（"items": null 已踩过一次）。
    #[serde(default)]
    pub senses: Option<Vec<ApiSense>>,
    #[serde(default)]
    pub subject: String,
    /// 记忆稳定度（仅到期卡有）
    #[serde(default)]
    pub stability: Option<f32>,
    /// 记忆难度 1~10（仅到期卡有）
    #[serde(default)]
    pub difficulty: Option<f32>,
    /// 下次到期时间（ISO 8601）
    #[serde(default)]
    pub due_at: Option<String>,
    /// 上次复习时间（ISO 8601）
    #[serde(default)]
    pub last_review_at: Option<String>,
    /// 累计复习次数（仅到期卡有）
    #[serde(default)]
    pub reps: Option<u32>,
}

/// `{"data":{"items":[...]}}` 外层信封
#[derive(Debug, Deserialize)]
struct ItemsEnvelope {
    /// ⚠️ 必须是 `Option`：Go 在结果为空时会把 nil 切片序列化成 `"items": null`，
    /// 而 serde 的 `#[serde(default)]` 只对「字段缺失」生效、对显式 `null` 不生效。
    /// 若写成 `Vec<ApiCard>`，「今天没有到期卡」这种正常状态会直接解析失败。
    #[serde(default)]
    items: Option<Vec<ApiCard>>,
}

#[derive(Debug, Deserialize)]
struct DataEnvelope {
    #[serde(default)]
    data: Option<ItemsEnvelope>,
}

/// 解析卡片列表：同时接受裸数组与后端信封两种形态，便于测试与复用
fn parse_items(json: &str) -> Result<Vec<ApiCard>, String> {
    if json.trim().is_empty() {
        return Ok(Vec::new());
    }
    if let Ok(list) = serde_json::from_str::<Vec<ApiCard>>(json) {
        return Ok(list);
    }
    let env: DataEnvelope =
        serde_json::from_str(json).map_err(|e| format!("解析卡片列表失败: {e}"))?;
    Ok(env.data.and_then(|d| d.items).unwrap_or_default())
}

/// 把接口返回的 ISO 时间字符串解析为 epoch 毫秒。
///
/// 直接调用 JS 引擎的 `Date.parse`（与改动前的 `new Date(x).getTime()` 是同一个解析器），
/// 因此对 Go 输出的 RFC3339 纳秒精度时间戳（如 `2026-09-13T11:47:59.1139268+08:00`）
/// 行为完全一致：解析成功并截断到毫秒，无法解析时返回 NaN。
#[cfg(target_arch = "wasm32")]
fn parse_time_ms(s: &str) -> Option<f64> {
    let t = js_sys::Date::parse(s);
    if t.is_nan() {
        None
    } else {
        Some(t)
    }
}

/// 非 wasm 目标（宿主单元测试）下不做字符串解析，时间由调用方直接以毫秒给出
#[cfg(not(target_arch = "wasm32"))]
fn parse_time_ms(_s: &str) -> Option<f64> {
    None
}

// ---------- 纯计算：计划编排 ----------

/// 卡片来源
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CardSource {
    /// 复习区的到期卡（`due_at` 已过或即将到）
    Due,
    /// 每日新词（词库里还没有复习记录的词）
    New,
    /// 每日抽查：从「到期时间最远」的已学词里提前抽出来的卡。
    ///
    /// 存在的意义：只按到期时间出题时，那些间隔被拉到几十天的词永远轮不到，
    /// 长期不露面就成了盲区。每天固定抽查几张，等于给整库做抽样体检。
    /// 评分时按**新卡**重算记忆状态（见 [`ReviewSession::try_rate`]）。
    Probe,
}

/// 队列输入：卡片 + 两个已解析成毫秒的时间。
///
/// 为什么把时间单独拎出来：宿主（`cargo test`）上拿不到 JS 的 `Date.parse`，
/// 时间串解析是 wasm 边界的事；纯计算层只认毫秒，这样排序与切分才能在宿主上测。
#[derive(Debug, Clone)]
pub struct QueueInput {
    /// 词条数据
    pub card: ApiCard,
    /// 上次复习时间（epoch 毫秒）：评分时算「距上次几天」用；新词为 `None`
    pub last_ms: Option<f64>,
    /// 下次到期时间（epoch 毫秒）：排序与「已过期 / 未到期」切分用
    pub due_ms: Option<f64>,
}

/// 每日计划参数（由 JS 用 localStorage 里的进度算好后传进来）
///
/// ⚠️ 这两个 limit 是**今天还剩多少配额**，不是每日上限本身——
/// 上限（每天 5 个新词 + 5 个抽查）记在浏览器 localStorage 里，
/// 因为现在没有登录系统，服务端分不清是谁的配额（详见 README 的说明）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PlanOptions {
    /// 今天还能学几个新词
    #[serde(default)]
    pub new_limit: u32,
    /// 今天还能抽查几个
    #[serde(default)]
    pub probe_limit: u32,
    /// 最近抽查过的词 id：这些词不再被抽中（避免连续几天抽到同一批）
    #[serde(default)]
    pub probed_ids: Vec<u32>,
    /// 当前时间（epoch 毫秒），用于切分「已过期 / 未到期」
    #[serde(default)]
    pub now_ms: f64,
}

/// 今日计划 + 复习区
pub struct DayPlan {
    pub cards: Vec<PlannedCard>,
    /// 计划区长度：`cards[0..plan_len]` 是今日计划（新词 + 抽查），之后是复习区。
    /// JS 靠它判断「今日计划做完了，该切文案了」。
    pub plan_len: usize,
    /// 计划里实际放了几个新词（词库不够时会少于 `new_limit`）
    pub new_count: usize,
    /// 计划里实际放了几个抽查
    pub probe_count: usize,
}

/// 会话中的一张计划卡
#[derive(Debug, Clone)]
pub struct PlannedCard {
    /// 来源：复习区到期卡 / 每日新词 / 每日抽查
    pub source: CardSource,
    /// 在原始数组中的下标（便于排查与复现）
    pub index: u32,
    /// 复习前的记忆状态；新词为 `None`
    pub state: Option<CardState>,
    /// 上次复习时间（epoch 毫秒）；新词或时间缺失时为 `None`
    pub last_ms: Option<f64>,
    /// 下次到期时间（epoch 毫秒）。
    /// `None` 表示「今日计划卡」（新词 / 抽查），排序时按 −∞ 处理，排在最前面；
    /// 评完一次之后就会被填上新算出来的到期时间。
    pub due_ms: Option<f64>,
    /// 词条数据（由 Rust 持有，JS 不再保存一份）
    pub card: ApiCard,
}

/// 编排今日的初始池子。
///
/// 顺序（从前往后）：
/// 1. **新词**：无放回随机抽 `new_limit` 个（随机而不是按词表顺序，避免每次刷新都从同一头开始）
/// 2. **抽查**：从 `probes`（服务端按 `due_at` **倒序**给的候选）里取，跳过 `probed_ids` 里刚抽过的
/// 3. **复习区**：其余已学词，按 `due_at` **升序**（服务端 `ORDER BY due_at ASC` 保证）
///
/// ⚠️ 这里**不再打乱**：池子必须严格有序，评完的卡才能按新的到期时间插回正确位置
/// （见 [`ReviewSession::try_rate`]）。「局部乱序」改在抽卡时做——未到期的卡在
/// 最靠前的 [`CHUNK_SIZE`] 张里随机抽一张（见 [`ReviewSession::pick_index`]）。
pub fn plan_day(
    queue: Vec<QueueInput>,
    new: Vec<ApiCard>,
    probes: Vec<ApiCard>,
    opts: &PlanOptions,
    seed: u32,
) -> DayPlan {
    let mut cards: Vec<PlannedCard> = Vec::new();

    // ---- 1. 每日新词：无放回随机抽 ----
    let new_take = opts.new_limit.min(new.len() as u32);
    let new_order = random_sample(new.len() as u32, new_take, seed ^ 0x9E37_79B9);
    let mut used_ids: HashSet<u32> = HashSet::new();
    for &i in new_order.iter() {
        let idx = i as usize;
        if let Some(card) = new.get(idx) {
            used_ids.insert(card.id);
            cards.push(PlannedCard {
                source: CardSource::New,
                index: i,
                state: None,
                last_ms: None,
                due_ms: None, // 没有到期时间 → 排序键为 −∞，排在最前面
                card: card.clone(),
            });
        }
    }

    // ---- 2. 每日抽查：到期最远且最近没抽过的 ----
    // 候选已按 due_at 倒序给出，这里顺序取前 probe_limit 个
    let mut probe_ids: HashSet<u32> = HashSet::new();
    for card in probes.iter() {
        if cards.len() - new_take as usize >= opts.probe_limit as usize {
            break;
        }
        if opts.probed_ids.contains(&card.id) || used_ids.contains(&card.id) {
            continue; // 最近抽过：换下一个候选
        }
        probe_ids.insert(card.id);
        used_ids.insert(card.id);
        cards.push(PlannedCard {
            source: CardSource::Probe,
            index: 0,
            state: match (card.stability, card.difficulty) {
                (Some(stability), Some(difficulty)) => Some(CardState {
                    stability,
                    difficulty,
                }),
                _ => None,
            },
            last_ms: card
                .last_review_at
                .as_deref()
                .or(card.due_at.as_deref())
                .and_then(parse_time_ms),
            due_ms: None, // 计划卡一样排在最前面
            card: card.clone(),
        });
    }
    let plan_len = cards.len();
    let probe_count = plan_len - new_take as usize;

    // ---- 3. 复习区：按到期时间升序（保持服务端给的顺序，不再打乱）----
    for (i, item) in queue.iter().enumerate() {
        if probe_ids.contains(&item.card.id) {
            continue; // 今天已经抽查过它了，不再在复习区出现
        }
        cards.push(PlannedCard {
            source: CardSource::Due,
            index: i as u32,
            state: match (item.card.stability, item.card.difficulty) {
                (Some(stability), Some(difficulty)) => Some(CardState {
                    stability,
                    difficulty,
                }),
                // 复习区的卡一定带记忆状态；真缺失时按新卡处理，
                // 比「拼出残缺 JSON 让引擎报错」稳健
                _ => None,
            },
            last_ms: item.last_ms,
            due_ms: item.due_ms,
            card: item.card.clone(),
        });
    }

    DayPlan {
        cards,
        plan_len,
        new_count: new_take as usize,
        probe_count,
    }
}

/// 把接口返回的卡片转成队列输入：时间串 → epoch 毫秒
/// （`last_ms` 取 `last_review_at`，缺失时回退 `due_at`，与原 JS 一致）
pub fn to_queue_inputs(cards: Vec<ApiCard>) -> Vec<QueueInput> {
    cards
        .into_iter()
        .map(|card| {
            let due_ms = card.due_at.as_deref().and_then(parse_time_ms);
            let last_ms = card
                .last_review_at
                .as_deref()
                .and_then(parse_time_ms)
                .or(due_ms);
            QueueInput {
                card,
                last_ms,
                due_ms,
            }
        })
        .collect()
}

/// 距离上次复习的天数：`floor((now - last) / 86400000)`，负值归零。
///
/// 与改动前 JS 的 `Math.floor((Date.now() - new Date(last).getTime()) / 86400000)`
/// 后接 `if (days < 0) days = 0` 完全等价；同时把 NaN/Inf 一并归零，
/// 避免脏时间戳污染 FSRS 调度。
pub fn days_elapsed(last_ms: f64, now_ms: f64) -> u32 {
    let days = ((now_ms - last_ms) / MS_PER_DAY).floor();
    if !days.is_finite() || days <= 0.0 {
        return 0;
    }
    // 浮点转整数在 Rust 中是饱和转换，超出 u32 时自动截断到 u32::MAX
    days.min(u32::MAX as f64) as u32
}

/// 按评分取出对应的分支结果（1=Again 2=Hard 3=Good 4=Easy）
pub fn pick_branch(states: NextStatesOut, rating: u8) -> Option<ItemStateOut> {
    match rating {
        1 => Some(states.again),
        2 => Some(states.hard),
        3 => Some(states.good),
        4 => Some(states.easy),
        _ => None,
    }
}

/// 进度百分比：`round(done / total * 100)`，队列为空时返回 0
pub fn progress_percent(done: u32, total: u32) -> u32 {
    if total == 0 {
        return 0;
    }
    let done = done.min(total);
    (done as f64 / total as f64 * 100.0).round() as u32
}

/// 依据计划卡与当前时间生成记忆元信息。
///
/// 新词（无记忆状态）只给 `status = "new"`；到期卡给出难度、稳定度、复习次数、
/// 距上次天数，并用 FSRS 可提取率算「预计记住」。
///
/// 抽查卡给 `status = "probe"`：它同样带上**旧的**难度/稳定度/复习次数，
/// 但界面上标成「抽查」而不是「已到期」，让人看得出这个词为什么反常地提前出现。
fn card_meta(card: &PlannedCard, now_ms: f64) -> CardMetaOut {
    let state = match card.state {
        None => {
            return CardMetaOut {
                status: "new",
                ..Default::default()
            }
        }
        Some(state) => state,
    };
    let days = match card.last_ms {
        Some(last_ms) => days_elapsed(last_ms, now_ms),
        None => 0,
    };
    CardMetaOut {
        status: if card.source == CardSource::Probe {
            "probe"
        } else {
            "due"
        },
        difficulty: Some(state.difficulty),
        stability: Some(state.stability),
        reps: card.card.reps,
        days_since_last: Some(days),
        retrievability: Some(compute_retrievability(&state, days as f32)),
    }
}

// ---------- 会话对象：纯计算实现 ----------

/// 提交复习的请求体（与后端 `POST /api/reviews/submit` 对应）
#[derive(Debug, Serialize)]
struct SubmitPayload {
    word_id: u32,
    rating: u8,
    stability: f32,
    difficulty: f32,
    interval_days: f32,
    /// 是否为每日抽查卡：后端记进 `review_logs.is_probe`，
    /// 供日后做 FSRS 参数优化时排除这批「间隔被压缩」的记录
    is_probe: bool,
}

/// 卡片的记忆元信息（界面右上角展示，与参考产品的「难度/稳定性/预计记住」对应）
///
/// 新词没有记忆状态，因此除 `status` 外全部省略（`skip_serializing_if`），
/// 前端据此决定是否显示这一块。
#[derive(Debug, Default, Serialize)]
struct CardMetaOut {
    /// "due"（到期复习卡）| "new"（新词）
    status: &'static str,
    /// 记忆难度 1~10
    #[serde(skip_serializing_if = "Option::is_none")]
    difficulty: Option<f32>,
    /// 记忆稳定度（天）
    #[serde(skip_serializing_if = "Option::is_none")]
    stability: Option<f32>,
    /// 累计复习次数
    #[serde(skip_serializing_if = "Option::is_none")]
    reps: Option<u32>,
    /// 距上次复习天数
    #[serde(skip_serializing_if = "Option::is_none")]
    days_since_last: Option<u32>,
    /// 预计记住（0~1）：FSRS 可提取率，即此刻还能回忆起来的概率
    #[serde(skip_serializing_if = "Option::is_none")]
    retrievability: Option<f32>,
}

/// 一块释义（界面上一行：词性标签 + 释义正文，可带自己的例句与译文）
///
/// 空字段直接不输出，卡片 JSON 尽量小（多释义本来就比单词条重）。
#[derive(Debug, Serialize)]
struct SenseOut {
    /// 词性标签原文（"n." / "n./v."），前端负责转成大写标签
    pos: String,
    /// 释义正文
    meaning: String,
    /// 该释义自己的例句切分片段；为空表示没有独立例句
    #[serde(skip_serializing_if = "Vec::is_empty")]
    example_parts: Vec<ExamplePart>,
    /// 该释义例句的中文翻译
    #[serde(skip_serializing_if = "String::is_empty")]
    translation: String,
}

/// 当前卡片的展示数据（供 JS 直接渲染）
#[derive(Debug, Serialize)]
struct CurrentCardOut {
    word: String,
    phonetic: String,
    /// "due" | "new"
    source: CardSource,
    /// 主例句切分片段（前端据此把目标词高亮）；为空表示该词条没有例句
    example_parts: Vec<ExamplePart>,
    /// 主例句的中文翻译
    #[serde(skip_serializing_if = "String::is_empty")]
    example_translation: String,
    /// 释义块：有释义时至少一块（一个词性一块）
    senses: Vec<SenseOut>,
    /// 记忆元信息
    meta: CardMetaOut,
}

/// 组装释义块。
///
/// 两条路径：
/// 1. 词条填了多释义（`senses`）→ 直接用，每块可以有自己的例句与译文；
/// 2. 没填 → 把 `meaning` 按词性标签**自动拆开**
///    （`"n. 好处；益处 v. 有益于"` 拆成名词、动词两块），
///    这样历史数据不用改就能显示成多块，例句仍是词条级那一条。
fn build_senses(card: &ApiCard) -> Vec<SenseOut> {
    if let Some(list) = card.senses.as_ref() {
        let out: Vec<SenseOut> = list
            .iter()
            .filter_map(|s| {
                let pos = s.pos.trim().to_string();
                let meaning = s.meaning.trim().to_string();
                if pos.is_empty() && meaning.is_empty() {
                    return None; // 空条目（后台加了一行没填）跳过
                }
                Some(SenseOut {
                    pos,
                    meaning,
                    example_parts: card_view::split_example(&s.example, &card.word),
                    translation: s.example_translation.trim().to_string(),
                })
            })
            .collect();
        if !out.is_empty() {
            return out;
        }
        // 填了但清洗后全是空的 → 退回按 meaning 自动拆
    }

    card_view::split_senses(&card.meaning)
        .into_iter()
        .map(|m| SenseOut {
            pos: m.pos,
            meaning: m.text,
            example_parts: Vec::new(),
            translation: String::new(),
        })
        .collect()
}

/// 一次复习会话：持有队列、游标与期望保持率。
///
/// JS 侧只需：建会话 → 渲染时问 `current_json()` → 评分时调 `rate()`。
/// 用完请调用 `free()` 释放（wasm-bindgen 生成），避免反复进出英语页时泄漏。
#[wasm_bindgen]
pub struct ReviewSession {
    /// 待抽池：**按到期时间升序**（`due_ms` 为 `None` 的「今日计划」卡排在最前）。
    ///
    /// 与过去的「一次性队列 + 游标」不同：评完的卡会按**新的到期时间插回这个池子**，
    /// 所以池子永远不会空，也就没有「尽头」——这正是「想一直复习就一直复习」的实现方式。
    pool: Vec<PlannedCard>,
    /// 当前卡片（已从池中取出，等待评分）
    current: Option<PlannedCard>,
    /// 本轮已评分张数（只增，用于界面进度）
    done_count: u32,
    /// 最近抽出的卡 id（最多 [`SEEN_GUARD`] 个）：防止刚评完的卡立刻又出现
    seen: VecDeque<u32>,
    /// 本轮已经见过的**不同**卡 id
    round_seen: HashSet<u32>,
    /// 已经过完整轮的次数（池里每张卡都至少见过一次 = 一轮）
    rounds: u32,
    /// 抽卡用的随机种子（每抽一张推进一次，同一种子可复现）
    rng_seed: u32,
    desired_retention: f32,
    /// 计划区长度（新词 + 抽查）；`done() >= plan_len` 就说明进入复习阶段了
    plan_len: usize,
    /// 今日计划里实际放了几个新词
    new_count: usize,
    /// 今日计划里实际放了几个抽查
    probe_count: usize,
}

/// `plan_json()` 的输出结构
#[derive(Debug, Serialize)]
struct PlanSummary {
    new_target: usize,
    probe_target: usize,
    plan_len: usize,
}

/// 解析今日计划参数；空串按「没有新词、没有抽查」处理（只复习）
fn parse_plan_options(json: &str) -> Result<PlanOptions, String> {
    let text = json.trim();
    if text.is_empty() {
        return Ok(PlanOptions::default());
    }
    serde_json::from_str::<PlanOptions>(text).map_err(|e| format!("解析今日计划参数失败: {e}"))
}

/// 池中卡片的排序键：没有到期时间的（今日计划的新词 / 抽查）排在最前
fn sort_key(card: &PlannedCard) -> f64 {
    card.due_ms.unwrap_or(f64::NEG_INFINITY)
}

/// 推进一次随机种子（LCG）。抽卡时用，保证同一种子下整轮可复现。
fn next_seed(seed: u32) -> u32 {
    seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223)
}

/// 把一张卡按到期时间插回池中（池保持升序；相同到期时间保持相对顺序）
fn insert_sorted(pool: &mut Vec<PlannedCard>, card: PlannedCard) {
    let key = sort_key(&card);
    // partition_point 返回第一个「排序键 > key」的位置，即插入点
    let at = pool.partition_point(|c| sort_key(c) <= key);
    pool.insert(at, card);
}

/// 纯计算实现：不参与 wasm 导出，错误类型是 `String`，可在宿主上直接单元测试。
impl ReviewSession {
    /// 由已解析的卡片建立会话（纯计算，不涉及 JSON 与时间字符串）。
    ///
    /// `opts.new_limit` / `opts.probe_limit` 就是本次要放几个，**不做任何兜底**：
    /// JS 会把「今天还剩多少配额」原样传进来，配额用完就传 0（这一天不再有新词）。
    /// 参数缺失或空 JSON 时两者都是 0，等价于「只复习」。
    pub fn build(
        queue: Vec<QueueInput>,
        new: Vec<ApiCard>,
        probes: Vec<ApiCard>,
        opts: PlanOptions,
        seed: u32,
    ) -> ReviewSession {
        let plan = plan_day(queue, new, probes, &opts, seed);
        let mut session = ReviewSession {
            pool: plan.cards,
            current: None,
            done_count: 0,
            seen: VecDeque::new(),
            round_seen: HashSet::new(),
            rounds: 0,
            rng_seed: seed ^ 0x2545_F491,
            desired_retention: DESIRED_RETENTION,
            plan_len: plan.plan_len,
            new_count: plan.new_count,
            probe_count: plan.probe_count,
        };
        session.advance(opts.now_ms);
        session
    }

    /// 抽下一张卡（内部使用）。
    ///
    /// 规则：
    /// 1. 池首还是「已过期 / 今日计划」的卡 → 严格按最旧优先（不打乱）
    /// 2. 池首已经全是未到期的 → 在「最靠前的 [`CHUNK_SIZE`] 张」里随机抽一张
    /// 3. **本轮已经评过的卡不再抽**（它们的到期时间往往就落在最前面，不挡的话会来回循环）；
    ///    窗口里如果全是评过的，就把窗口逐步放大去找没评过的，保证本轮每张都能轮到
    /// 4. 池子比保护窗口还小（例如只有 3 张卡）→ 放行，保证永远有卡可出
    ///
    /// 「同一张卡至少隔多久才重复」由第 3 条保证：至少要等到下一轮
    /// （词库 ≥ [`SEEN_GUARD`] 张时，一轮至少这么长），池子太小则退回到
    /// [`SEEN_GUARD`] 张的距离。
    fn pick_index(&mut self, now_ms: f64) -> Option<usize> {
        if self.pool.is_empty() {
            return None;
        }

        // 1) 池首还是「已过期 / 今日计划」的卡：严格按最旧优先，不打乱。
        //    这些卡刚评完就会带着未来到期时间插回池子，所以不存在「刚评完又抽到」。
        if sort_key(&self.pool[0]) <= now_ms {
            return Some(0);
        }

        // 2) 池首已经全是未到期的：在「最靠前的 CHUNK_SIZE 张」里随机抽一张。
        //    理想候选 = 本轮没评过、且不在最近抽过名单里的。
        let is_fresh = |s: &Self, id: u32| {
            !s.round_seen.contains(&id) && !s.seen.contains(&id)
        };

        let mut window = self.pool.len().min(CHUNK_SIZE);
        // 窗口里没有「理想候选」就逐步放大窗口（池子小的时候最终覆盖全池）
        while window < self.pool.len()
            && !self.pool[0..window]
                .iter()
                .any(|c| is_fresh(self, c.card.id))
        {
            window = (window * 2).min(self.pool.len());
        }

        let mut candidates: Vec<usize> = (0..window)
            .filter(|i| is_fresh(self, self.pool[*i].card.id))
            .collect();

        // 3) 退一步：全池都「最近抽过」了，但还有本轮没评过的 → 放宽「最近抽过」这一条。
        //    词库比 SEEN_GUARD 还小时会走到这里，此时用「没评过」来避免同一张卡立刻重复。
        if candidates.is_empty() {
            candidates = (0..window)
                .filter(|i| !self.round_seen.contains(&self.pool[*i].card.id))
                .collect();
            if candidates.is_empty() {
                // 再退一步：整个池子都是本轮评过的（一轮刚好走完的那一刻），取最靠前的
                candidates = (0..window).collect();
            }
        }

        if candidates.is_empty() {
            return Some(0);
        }

        self.rng_seed = next_seed(self.rng_seed);
        let picked = random_sample(candidates.len() as u32, 1, self.rng_seed);
        picked.first().map(|&i| candidates[i as usize])
    }

    /// 取出下一张作为当前卡，并记录「最近抽过」与「本轮见过」
    fn advance(&mut self, now_ms: f64) {
        let idx = match self.pick_index(now_ms) {
            Some(i) => i,
            None => {
                self.current = None;
                return;
            }
        };
        let card = self.pool.remove(idx);

        // 最近抽过：环形保留最近 SEEN_GUARD 张，避免刚评完的卡立刻又出现
        self.seen.push_back(card.card.id);
        while self.seen.len() > SEEN_GUARD {
            self.seen.pop_front();
        }

        self.current = Some(card);
    }

    /// 检查本轮是否走完：池子里每一张卡都**评过一次** = 一轮。
    ///
    /// 判定时机是「评完一张之后」——若改成抽到下一张时判定，
    /// 最后一轮的最后一张刚出现在屏幕上就会提示「本轮已过一遍」，那时还没复习呢。
    fn check_round(&mut self) {
        let universe = self.pool.len() + if self.current.is_some() { 1 } else { 0 };
        if universe == 0 || self.round_seen.len() < universe {
            return;
        }
        self.rounds += 1;
        self.round_seen.clear(); // 新一轮重新计数
    }

    /// 评分：用引擎算出新记忆状态，返回可直接作为 `POST /api/reviews/submit`
    /// 请求体的 JSON。
    ///
    /// 除了算状态，还会把这张卡**按新的到期时间插回池中**——
    /// 池子因此永远不会空，可以一直复习下去。出错时不改任何状态（与改动前一致）。
    pub fn try_rate(&mut self, rating: u8, now_ms: f64) -> Result<String, String> {
        if !(1..=4).contains(&rating) {
            return Err(format!("rating 必须为 1~4，收到 {rating}"));
        }
        let card = self
            .current
            .as_ref()
            .ok_or_else(|| "复习池已空，没有可评分的卡片".to_string())?;

        // 距上次复习的天数在「评分这一刻」换算（与改动前一致）
        let days = match card.last_ms {
            Some(last_ms) => days_elapsed(last_ms, now_ms),
            None => 0, // 新词
        };

        // 抽查卡按**新卡**重算：丢掉原来的 stability/difficulty，天数按 0 算。
        // 这是「重新体检」——它的间隔会被压缩回几天，从而很快回来重新标定。
        let (state, days) = if card.source == CardSource::Probe {
            (None, 0)
        } else {
            (card.state, days)
        };

        let states = compute_next_states(state, self.desired_retention, days)
            .map_err(|e| format!("FSRS 计算失败: {e}"))?;
        let chosen = pick_branch(states, rating)
            .ok_or_else(|| format!("无对应的评分分支: {rating}"))?;

        let payload = SubmitPayload {
            word_id: card.card.id,
            rating,
            stability: chosen.memory.stability,
            difficulty: chosen.memory.difficulty,
            interval_days: chosen.interval_days,
            is_probe: card.source == CardSource::Probe,
        };

        // ---- 算完了才动状态：把这张卡按新的到期时间插回池子 ----
        let mut done = self.current.take().expect("上面已确认有当前卡");
        let done_id = done.card.id;
        // ⚠️ 到期时间在这里自己算：now + 间隔，并镜像服务端「最短 10 分钟」的下限
        //    （backend-go/handlers/review_handlers.go 的 dueSeconds）。
        //    两处口径必须一致，否则插回池里的位置会和服务端实际 due_at 有偏差。
        let interval_ms = ((chosen.interval_days as f64) * MS_PER_DAY).max(MIN_INTERVAL_MS);
        done.due_ms = Some(now_ms + interval_ms);
        // 记忆状态也更新成刚算出来的，这样本轮再次抽到它时元信息、天数换算都基于新状态
        done.state = Some(CardState {
            stability: chosen.memory.stability,
            difficulty: chosen.memory.difficulty,
        });
        done.last_ms = Some(now_ms);
        insert_sorted(&mut self.pool, done);

        self.done_count += 1;
        self.round_seen.insert(done_id); // 本轮「评过」的卡
        // ⚠️ 顺序很重要：先结算「过完一轮」再抽下一张。
        // 反过来的话，新一轮的第一张是在「整池都评过」的状态下抽的，
        // 候选全被挡掉会退化到兜底分支，于是刚评过的卡可能隔 8 张就重复出现。
        self.check_round();
        self.advance(now_ms);

        serde_json::to_string(&payload).map_err(|e| format!("序列化失败: {e}"))
    }

    /// 把复习区的一页追加进池子（按到期时间插入，池保持有序）。返回实际新增张数。
    pub fn append_page(&mut self, queue: Vec<QueueInput>, now_ms: f64, seed: u32) -> u32 {
        let mut pending: HashSet<u32> = HashSet::new();
        for card in self.pool.iter() {
            pending.insert(card.card.id);
        }
        if let Some(cur) = self.current.as_ref() {
            pending.insert(cur.card.id);
        }

        let mut added = 0u32;
        for item in queue {
            if !pending.insert(item.card.id) {
                continue; // 池里或手上已经有了
            }
            insert_sorted(
                &mut self.pool,
                PlannedCard {
                    source: CardSource::Due,
                    index: 0,
                    state: match (item.card.stability, item.card.difficulty) {
                        (Some(stability), Some(difficulty)) => Some(CardState {
                            stability,
                            difficulty,
                        }),
                        _ => None,
                    },
                    last_ms: item.last_ms,
                    due_ms: item.due_ms,
                    card: item.card,
                },
            );
            added += 1;
        }

        // 池里原本一张都没有（例如刚进来就只给了空队列）：抽一张出来
        if self.current.is_none() {
            self.rng_seed ^= seed;
            self.advance(now_ms);
        }
        added
    }
}

// ---------- 会话对象：wasm 导出层（薄边界） ----------

fn js_err(msg: impl Into<String>) -> JsValue {
    JsValue::from_str(&msg.into())
}

#[wasm_bindgen]
impl ReviewSession {
    /// 建立今日会话：内部取一次系统随机种子。
    ///
    /// - `queue_json`：`GET /api/reviews/queue` 的响应体原文（**整库按紧迫度升序**，含未到期）
    /// - `new_json`：`GET /api/reviews/new` 的响应体原文（未学词候选）
    /// - `probe_json`：`GET /api/reviews/probes` 的响应体原文（**到期最远**的已学词候选）
    /// - `plan_json`：今日计划参数，形如
    ///   `{"new_limit":5,"probe_limit":5,"probed_ids":[12,34],"now_ms":1757000000000}`
    ///   （两个 limit 是**今天还剩多少配额**，由 JS 从 localStorage 里的今日进度算出来）
    #[wasm_bindgen(constructor)]
    pub fn new(
        queue_json: &str,
        new_json: &str,
        probe_json: &str,
        plan_json: &str,
    ) -> Result<ReviewSession, JsValue> {
        let seed = random_seed().map_err(|e| js_err(format!("获取随机种子失败: {e:?}")))?;
        Self::with_seed(queue_json, new_json, probe_json, plan_json, seed)
    }

    /// 建立会话（显式种子）：便于复现与测试。
    #[wasm_bindgen]
    pub fn with_seed(
        queue_json: &str,
        new_json: &str,
        probe_json: &str,
        plan_json: &str,
        seed: u32,
    ) -> Result<ReviewSession, JsValue> {
        let queue = to_queue_inputs(parse_items(queue_json).map_err(js_err)?);
        let new_cards = parse_items(new_json).map_err(js_err)?;
        let probe_cards = parse_items(probe_json).map_err(js_err)?;
        let opts = parse_plan_options(plan_json).map_err(js_err)?;

        Ok(Self::build(queue, new_cards, probe_cards, opts, seed))
    }

    /// 追加复习区的一页到池子里（按到期时间插好，池保持有序），返回实际新增的数量。
    ///
    /// 「整库往下翻」靠它：前端判断本轮快走完（`seen_count()` 逼近 `pending_count()`）时再取一页。
    /// **只追加复习区**：新词与抽查受每日配额限制，不会因为翻页而变多。
    /// 返回 0 表示这一页没有新的卡（都在池子里了，或已到词库末尾）。
    pub fn append(&mut self, queue_json: &str, plan_json: &str) -> Result<u32, JsValue> {
        let seed = random_seed().map_err(|e| js_err(format!("获取随机种子失败: {e:?}")))?;
        self.append_with_seed(queue_json, plan_json, seed)
    }

    /// 追加复习区的一页（显式种子）：便于复现与单元测试。
    #[wasm_bindgen]
    pub fn append_with_seed(
        &mut self,
        queue_json: &str,
        plan_json: &str,
        seed: u32,
    ) -> Result<u32, JsValue> {
        let queue = to_queue_inputs(parse_items(queue_json).map_err(js_err)?);
        let opts = parse_plan_options(plan_json).map_err(js_err)?;
        Ok(self.append_page(queue, opts.now_ms, seed))
    }

    /// 今日计划的规模：`{"new_target":5,"probe_target":5,"plan_len":10}`
    ///
    /// JS 用它渲染左下角的计划进度，并判断「计划区走完了没有」（`done() >= plan_len`）。
    /// `*_target` 是**实际放进池子的数量**，词库不够时会小于每日上限。
    pub fn plan_json(&self) -> String {
        let plan = PlanSummary {
            new_target: self.new_count,
            probe_target: self.probe_count,
            plan_len: self.plan_len,
        };
        serde_json::to_string(&plan).unwrap_or_else(|_| "{}".to_string())
    }

    /// 池中待抽 + 手上这一张的总张数（**永远不会归零**：评完的卡会插回池子）
    pub fn total(&self) -> u32 {
        (self.pool.len() + if self.current.is_some() { 1 } else { 0 }) as u32
    }

    /// 池中还有多少张没抽（不含手上这一张）
    pub fn pending_count(&self) -> u32 {
        self.pool.len() as u32
    }

    /// 本轮已经见过多少张**不同**的卡（用于判断「本轮快过完了，可以再取一页」）
    pub fn seen_count(&self) -> u32 {
        self.round_seen.len() as u32
    }

    /// 已经过完整轮的次数（每张在池里的卡都至少见过一次 = 一轮）。
    /// 前端用它的变化来提示「本轮已过一遍」。
    pub fn rounds(&self) -> u32 {
        self.rounds
    }

    /// 本轮已评分张数（只增）
    pub fn done(&self) -> u32 {
        self.done_count
    }

    /// 是否已经没卡可出（池子空且手上没有）——正常词库下不会发生
    pub fn is_finished(&self) -> bool {
        self.current.is_none()
    }

    /// 本轮进度百分比 0~100（无限复习下这个数字意义有限，保留给调试与旧调用）
    pub fn progress_percent(&self) -> u32 {
        progress_percent(self.done(), self.done().saturating_add(self.total()))
    }

    /// 当前卡片所属词条 id；无当前卡片时返回 0
    pub fn current_word_id(&self) -> u32 {
        self.current.as_ref().map(|c| c.card.id).unwrap_or(0)
    }

    /// 当前卡片的展示数据（JSON）；无当前卡片时返回空对象 `{}`
    ///
    /// `now_ms` 用于换算「距上次复习天数」与「预计记住」，所以由 JS 传 `Date.now()`。
    pub fn current_json(&self, now_ms: f64) -> String {
        match self.current.as_ref() {
            Some(c) => {
                serde_json::to_string(&CurrentCardOut {
                    word: c.card.word.clone(),
                    phonetic: c.card.phonetic.clone(),
                    source: c.source,
                    example_parts: card_view::split_example(&c.card.example, &c.card.word),
                    example_translation: c.card.example_translation.trim().to_string(),
                    senses: build_senses(&c.card),
                    meta: card_meta(c, now_ms),
                })
                .unwrap_or_else(|_| "{}".to_string())
            }
            None => "{}".to_string(),
        }
    }

    /// 评分：返回可直接作为 `POST /api/reviews/submit` 请求体的 JSON。
    ///
    /// - `rating`：1=Again 2=Hard 3=Good 4=Easy
    /// - `now_ms`：当前时间（epoch 毫秒，由 JS 传 `Date.now()`）
    pub fn rate(&mut self, rating: u8, now_ms: f64) -> Result<String, JsValue> {
        self.try_rate(rating, now_ms).map_err(js_err)
    }
}

// ---------- 宿主单元测试（纯计算部分，不依赖浏览器） ----------

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: u32, word: &str) -> ApiCard {
        ApiCard {
            id,
            word: word.to_string(),
            phonetic: String::new(),
            meaning: String::new(),
            example: String::new(),
            example_translation: String::new(),
            senses: None,
            subject: "english".to_string(),
            stability: None,
            difficulty: None,
            due_at: None,
            last_review_at: None,
            reps: None,
        }
    }

    fn due_card(id: u32, word: &str, stability: f32, difficulty: f32) -> ApiCard {
        ApiCard {
            stability: Some(stability),
            difficulty: Some(difficulty),
            ..card(id, word)
        }
    }

    /// 已学词（复习区 / 抽查候选共用）：带记忆状态与到期时间（毫秒直接给）
    fn learned(id: u32, word: &str, stability: f32, difficulty: f32, due_ms: f64) -> QueueInput {
        QueueInput {
            card: due_card(id, word, stability, difficulty),
            // 上次复习时间随便给一个过去的值（本组测试只关心排序与编排）
            last_ms: Some(due_ms - MS_PER_DAY),
            due_ms: Some(due_ms),
        }
    }

    /// 今日计划参数
    fn plan_opts(new_limit: u32, probe_limit: u32, now_ms: f64) -> PlanOptions {
        PlanOptions {
            new_limit,
            probe_limit,
            probed_ids: Vec::new(),
            now_ms,
        }
    }

    /// 构造会话：`now` 固定为第 10 天，计划为「5 个新词 + 5 个抽查」
    fn session(
        queue: Vec<QueueInput>,
        new: &[ApiCard],
        probes: &[ApiCard],
        seed: u32,
    ) -> ReviewSession {
        ReviewSession::build(
            queue,
            new.to_vec(),
            probes.to_vec(),
            plan_opts(5, 5, 10.0 * MS_PER_DAY),
            seed,
        )
    }

    // ---------- 今日编排：新词 → 抽查 → 复习区 ----------

    #[test]
    fn plan_puts_new_then_probe_then_review() {
        // 复习区给 3 张已到期的卡（第 5 天到期，now = 第 10 天）
        let queue: Vec<QueueInput> = (1..=3)
            .map(|i| learned(i, "d", 1.0, 5.0, 5.0 * MS_PER_DAY))
            .collect();
        let new: Vec<ApiCard> = (10..15).map(|i| card(i, "n")).collect();
        let probes: Vec<ApiCard> = (20..23).map(|i| due_card(i, "p", 8.0, 3.0)).collect();

        let plan = plan_day(queue, new, probes, &plan_opts(5, 5, 10.0 * MS_PER_DAY), 42);

        // 新词 5 个（不足则全取）+ 抽查 3 个（候选只有 3 个）+ 复习区 3 张
        assert_eq!(plan.new_count, 5);
        assert_eq!(plan.probe_count, 3);
        assert_eq!(plan.plan_len, 8, "计划区 = 新词 + 抽查");
        assert_eq!(plan.cards.len(), 11);

        let sources: Vec<CardSource> = plan.cards.iter().map(|c| c.source).collect();
        assert!(sources[..5].iter().all(|s| *s == CardSource::New), "最前面是新词");
        assert!(sources[5..8].iter().all(|s| *s == CardSource::Probe), "接着是抽查");
        assert!(sources[8..].iter().all(|s| *s == CardSource::Due), "最后是复习区");
    }

    #[test]
    fn plan_respects_remaining_quota() {
        // 今天已经学过 3 个新词、2 个抽查 → 只剩 2 个新词与 3 个抽查的额度
        let new: Vec<ApiCard> = (10..20).map(|i| card(i, "n")).collect();
        let probes: Vec<ApiCard> = (20..30).map(|i| due_card(i, "p", 8.0, 3.0)).collect();
        let plan = plan_day(
            Vec::new(),
            new,
            probes,
            &plan_opts(2, 3, 10.0 * MS_PER_DAY),
            7,
        );
        assert_eq!(plan.new_count, 2);
        assert_eq!(plan.probe_count, 3);
        assert_eq!(plan.plan_len, 5);
    }

    #[test]
    fn plan_skips_recently_probed_words() {
        // 候选按「到期最远」倒序给出：[1,2,3,4]；1、2 最近抽过 → 应换成 3、4
        let probes: Vec<ApiCard> = (1..=4).map(|i| due_card(i, "p", 8.0, 3.0)).collect();
        let mut opts = plan_opts(0, 2, 10.0 * MS_PER_DAY);
        opts.probed_ids = vec![1, 2];

        let plan = plan_day(Vec::new(), Vec::new(), probes, &opts, 1);
        assert_eq!(plan.probe_count, 2);
        let ids: Vec<u32> = plan.cards.iter().map(|c| c.card.id).collect();
        assert_eq!(ids, vec![3, 4], "刚抽过的词要被跳过");
    }

    #[test]
    fn plan_probe_target_capped_by_available_candidates() {
        let probes: Vec<ApiCard> = vec![due_card(1, "p", 8.0, 3.0)];
        let plan = plan_day(Vec::new(), Vec::new(), probes, &plan_opts(0, 5, 0.0), 1);
        assert_eq!(plan.probe_count, 1, "候选不够时只放现有的");
    }

    #[test]
    fn plan_probed_card_is_not_repeated_in_review_region() {
        // 抽查卡同时也是已学词（复习区里本来有它）→ 不应出现两次
        let queue = vec![learned(1, "p", 8.0, 3.0, 30.0 * MS_PER_DAY)];
        let probes = vec![due_card(1, "p", 8.0, 3.0)];
        let plan = plan_day(
            queue,
            Vec::new(),
            probes,
            &plan_opts(0, 5, 10.0 * MS_PER_DAY),
            1,
        );
        assert_eq!(plan.cards.len(), 1, "同一张卡只出现一次");
        assert_eq!(plan.cards[0].source, CardSource::Probe);
    }

    #[test]
    fn plan_overdue_keeps_urgency_order_strictly() {
        // 已过期（第 1、3、5 天到期，now = 第 10 天）应严格按到期时间升序，不打乱
        let queue = vec![
            learned(1, "a", 1.0, 5.0, 1.0 * MS_PER_DAY),
            learned(2, "b", 1.0, 5.0, 3.0 * MS_PER_DAY),
            learned(3, "c", 1.0, 5.0, 5.0 * MS_PER_DAY),
        ];
        let plan = plan_day(
            queue,
            Vec::new(),
            Vec::new(),
            &plan_opts(0, 0, 10.0 * MS_PER_DAY),
            99,
        );
        let ids: Vec<u32> = plan.cards.iter().map(|c| c.card.id).collect();
        assert_eq!(ids, vec![1, 2, 3], "已过期的最旧优先，且顺序不受种子影响");
    }

    #[test]
    fn plan_keeps_pool_sorted_by_due_date() {
        // 初始池子必须严格按到期时间升序：评完的卡要能按新到期时间插回正确位置
        let queue: Vec<QueueInput> = (1..=12)
            .map(|i| learned(i as u32, "u", 1.0, 5.0, (100 + i) as f64 * MS_PER_DAY))
            .collect();
        let plan = plan_day(
            queue,
            Vec::new(),
            Vec::new(),
            &plan_opts(0, 0, 10.0 * MS_PER_DAY),
            20260913,
        );
        let ids: Vec<u32> = plan.cards.iter().map(|c| c.card.id).collect();
        assert_eq!(ids, (1..=12).collect::<Vec<u32>>(), "顺序即到期顺序，不再打乱");
    }

    #[test]
    fn plan_cards_sort_to_the_front() {
        // 今日计划卡没有到期时间（排序键 −∞）→ 一定排在复习区前面
        let queue = vec![learned(1, "d", 1.0, 5.0, 1.0 * MS_PER_DAY)];
        let new = vec![card(10, "n")];
        let plan = plan_day(
            queue,
            new,
            Vec::new(),
            &plan_opts(1, 0, 10.0 * MS_PER_DAY),
            1,
        );
        assert_eq!(plan.cards[0].source, CardSource::New);
        assert!(plan.cards[0].due_ms.is_none());
    }

    #[test]
    fn insert_sorted_keeps_pool_ordered() {
        let mut pool: Vec<PlannedCard> = Vec::new();
        let mk = |id: u32, due: f64| PlannedCard {
            source: CardSource::Due,
            index: 0,
            state: None,
            last_ms: None,
            due_ms: Some(due),
            card: card(id, "w"),
        };
        insert_sorted(&mut pool, mk(1, 30.0));
        insert_sorted(&mut pool, mk(2, 10.0));
        insert_sorted(&mut pool, mk(3, 20.0));
        insert_sorted(&mut pool, mk(4, 10.0)); // 与 id=2 同时到期 → 排在它后面（稳定）
        let ids: Vec<u32> = pool.iter().map(|c| c.card.id).collect();
        assert_eq!(ids, vec![2, 4, 3, 1]);
    }

    #[test]
    fn plan_is_deterministic_for_same_seed() {
        let mk = || {
            let queue: Vec<QueueInput> = (1..=20)
                .map(|i| learned(i, "u", 1.0, 5.0, (20 + i) as f64 * MS_PER_DAY))
                .collect();
            let new: Vec<ApiCard> = (100..120).map(|i| card(i, "n")).collect();
            let probes: Vec<ApiCard> = (200..220).map(|i| due_card(i, "p", 8.0, 3.0)).collect();
            plan_day(queue, new, probes, &plan_opts(5, 5, 10.0 * MS_PER_DAY), 20260913)
        };
        let a: Vec<u32> = mk().cards.iter().map(|c| c.card.id).collect();
        let b: Vec<u32> = mk().cards.iter().map(|c| c.card.id).collect();
        assert_eq!(a, b, "同一种子必须得到同一个池子");
    }

    #[test]
    fn plan_due_is_permutation_of_input() {
        let queue: Vec<QueueInput> = (0..8)
            .map(|i| learned(i, "w", 1.0, 5.0, 3.0 * MS_PER_DAY))
            .collect();
        let plan = plan_day(
            queue,
            Vec::new(),
            Vec::new(),
            &plan_opts(0, 0, 10.0 * MS_PER_DAY),
            7,
        );
        let mut ids: Vec<u32> = plan.cards.iter().map(|c| c.card.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, (0..8).collect::<Vec<u32>>(), "编排不应增删卡片");
    }

    #[test]
    fn plan_new_limit_capped_by_available() {
        let new: Vec<ApiCard> = (0..3).map(|i| card(i, "n")).collect();
        let plan = plan_day(Vec::new(), new, Vec::new(), &plan_opts(5, 0, 0.0), 1);
        assert_eq!(plan.new_count, 3, "新词不足时全部取用");
    }

    #[test]
    fn plan_falls_back_to_new_card_when_state_missing() {
        // 复习区的卡缺记忆状态时按新卡处理，而不是让引擎报错
        let queue = vec![QueueInput {
            card: card(1, "broken"),
            last_ms: Some(0.0),
            due_ms: Some(0.0),
        }];
        let plan = plan_day(
            queue,
            Vec::new(),
            Vec::new(),
            &plan_opts(0, 0, 10.0 * MS_PER_DAY),
            1,
        );
        assert_eq!(plan.cards.len(), 1);
        assert!(plan.cards[0].state.is_none());
    }

    // ---------- 抽卡规则 ----------

    /// 造一个只有复习区的会话：`n` 张卡，到期时间分别在第 `1..=n` 天，`now` 在第 0 天
    fn session_with_due_days(n: u32) -> ReviewSession {
        let queue: Vec<QueueInput> = (1..=n)
            .map(|i| learned(i, "q", 2.0, 5.0, i as f64 * MS_PER_DAY))
            .collect();
        ReviewSession::build(
            queue,
            Vec::new(),
            Vec::new(),
            plan_opts(0, 0, 0.0),
            20260913,
        )
    }

    #[test]
    fn pick_takes_oldest_first_when_overdue() {
        // now = 第 5 天：id=1..4 已过期 → 必须按最旧优先依次出
        let mut s = ReviewSession::build(
            (1..=6)
                .map(|i| learned(i, "q", 2.0, 5.0, i as f64 * MS_PER_DAY))
                .collect(),
            Vec::new(),
            Vec::new(),
            plan_opts(0, 0, 5.0 * MS_PER_DAY),
            7,
        );
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(s.current_word_id());
            s.try_rate(3, 5.0 * MS_PER_DAY).unwrap();
        }
        assert_eq!(seen, vec![1, 2, 3, 4], "过期的严格最旧优先");
    }

    #[test]
    fn pick_draws_from_the_front_block_when_not_due() {
        // 全部未到期（now = 第 0 天）→ 从最靠前的 10 张里随机抽，不会跳到第 11 张之后
        let mut s = session_with_due_days(30);
        for _ in 0..10 {
            let id = s.current_word_id();
            assert!(id >= 1 && id <= 10, "只应在前 10 张里抽，抽到 {id}");
            s.try_rate(3, 0.0).unwrap();
        }
    }

    #[test]
    fn pick_does_not_repeat_recent_cards() {
        // 全部未到期时，同一张卡至少要隔 SEEN_GUARD 张才会再次出现（防死循环的关键）
        let mut s = session_with_due_days(30);
        let mut seen = Vec::new();
        for _ in 0..30 {
            seen.push(s.current_word_id());
            s.try_rate(3, 0.0).unwrap();
        }
        for i in 0..seen.len() {
            for j in (i + 1)..seen.len() {
                if j - i <= SEEN_GUARD {
                    assert_ne!(
                        seen[i], seen[j],
                        "第 {i} 张与第 {j} 张间隔只有 {}，不该重复出现：{:?}",
                        j - i,
                        seen
                    );
                }
            }
        }
    }

    #[test]
    fn pick_repeats_when_pool_is_smaller_than_guard() {
        // 池子只有 3 张（比保护窗口还小）→ 必须放行，否则会卡死没有卡可出
        let mut s = session_with_due_days(3);
        for _ in 0..6 {
            assert!(s.current_word_id() > 0, "池子小也必须一直有卡可出");
            s.try_rate(3, 0.0).unwrap();
        }
        assert!(!s.is_finished());
    }

    // ---------- 评完插回池子（无限复习的核心） ----------

    #[test]
    fn rate_reinserts_card_and_pool_never_empties() {
        let mut s = session_with_due_days(5);
        assert_eq!(s.total(), 5);
        for i in 1..=40u32 {
            assert!(!s.is_finished(), "第 {i} 次评分前池子就空了");
            assert!(s.current_word_id() > 0, "第 {i} 次没有卡可评");
            s.try_rate(3, 0.0).unwrap();
            assert_eq!(s.done(), i, "done 应逐次累加");
        }
        assert_eq!(s.total(), 5, "池子大小不变：卡被插回来了而不是被丢掉");
    }

    #[test]
    fn rate_reinserts_by_new_due_date() {
        // 评完的卡要带着「now + 引擎算出的间隔」回到池子里，并且池子仍然有序
        let mut s = session_with_due_days(5);
        let first = s.current_word_id();
        let now = 0.0;
        let payload: serde_json::Value = serde_json::from_str(&s.try_rate(3, now).unwrap()).unwrap();
        let interval_ms = payload["interval_days"].as_f64().unwrap() * MS_PER_DAY;

        let due = s
            .pool
            .iter()
            .find(|c| c.card.id == first)
            .and_then(|c| c.due_ms)
            .expect("评完的卡应带着新到期时间回到池子里");
        assert!(
            (due - (now + interval_ms)).abs() < 1.0,
            "新到期时间应为 now + 间隔（{} 天），得到 {} 天",
            interval_ms / MS_PER_DAY,
            due / MS_PER_DAY
        );
        // 池子仍按到期时间升序
        let keys: Vec<f64> = s.pool.iter().map(sort_key).collect();
        let mut sorted = keys.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(keys, sorted, "插回后池子必须仍是有序的");
    }

    #[test]
    fn rate_respects_minimum_interval() {
        // 评「陌生」时 FSRS 给的间隔很短，但插回池子的到期时间不能早于 now + 10 分钟
        // （与服务端 review_handlers.go 的 600 秒下限一致）
        let mut s = session_with_due_days(3);
        let id = s.current_word_id();
        let now = 100.0 * MS_PER_DAY;
        s.try_rate(1, now).unwrap();
        let due = s
            .pool
            .iter()
            .find(|c| c.card.id == id)
            .and_then(|c| c.due_ms)
            .unwrap();
        assert!(due >= now + MIN_INTERVAL_MS, "到期时间不能早于 now + 10 分钟");
    }

    #[test]
    fn rate_updates_card_state_for_next_time() {
        // 插回池子时记忆状态也要更新：本轮再次抽到它时，元信息与天数换算都基于新状态
        let mut s = session_with_due_days(3);
        let id = s.current_word_id();
        let now = 50.0 * MS_PER_DAY;
        let payload: serde_json::Value = serde_json::from_str(&s.try_rate(3, now).unwrap()).unwrap();
        let card = s.pool.iter().find(|c| c.card.id == id).unwrap();
        let state = card.state.expect("插回时应带上新的记忆状态");
        assert!(
            (state.stability as f64 - payload["stability"].as_f64().unwrap()).abs() < 1e-3,
            "插回的状态应等于引擎刚算出来的"
        );
        assert_eq!(card.last_ms, Some(now), "上次复习时间应更新为此刻");
    }

    #[test]
    fn rounds_increase_after_a_full_pass() {
        // 池子里每张卡都评过一次 = 过完一轮；一轮之内不会重复出现同一张
        let mut s = session_with_due_days(4);
        assert_eq!(s.rounds(), 0);
        let mut seen = Vec::new();
        for i in 0..4 {
            assert_eq!(s.rounds(), 0, "第 {i} 张时还没过完一轮");
            seen.push(s.current_word_id());
            s.try_rate(3, 0.0).unwrap();
        }
        assert_eq!(s.rounds(), 1, "4 张都评过了 → 过完一轮");
        seen.sort_unstable();
        assert_eq!(seen, vec![1, 2, 3, 4], "一轮之内每张恰好出现一次");
        assert_eq!(s.seen_count(), 0, "新一轮的计数从头开始");
    }

    #[test]
    fn round_completes_even_when_some_cards_are_far_in_the_future() {
        // 复现过的坑：卡片到期时间跨度很大时，「前 10 张随机抽」的窗口若永远不往后放宽，
        // 排在后面的卡会被饿死，轮次永远凑不齐。这条测试锁住这个行为。
        let mut s = ReviewSession::build(
            (1..=12)
                .map(|i| learned(i, "q", 2.0, 5.0, i as f64 * MS_PER_DAY))
                .collect(),
            Vec::new(),
            Vec::new(),
            plan_opts(0, 0, 0.0),
            20260913,
        );
        let mut seen = Vec::new();
        for _ in 0..12 {
            seen.push(s.current_word_id());
            s.try_rate(3, 0.0).unwrap();
        }
        seen.sort_unstable();
        assert_eq!(
            seen,
            (1..=12).collect::<Vec<u32>>(),
            "一轮之内 12 张都要轮到（不能有卡被饿死）"
        );
        assert_eq!(s.rounds(), 1);
    }

    #[test]
    fn repeat_gap_holds_across_round_boundaries() {
        // 复现过的坑：跨轮边界时「刚评过的卡」保护会失效（实测只隔了 8 张就重复）。
        // 12 张池子 → 每轮 12 次评分，同一张卡之间的间隔必须 ≥ SEEN_GUARD。
        let mut s = ReviewSession::build(
            (1..=12)
                .map(|i| learned(i, "q", 2.0, 5.0, i as f64 * MS_PER_DAY))
                .collect(),
            Vec::new(),
            Vec::new(),
            plan_opts(0, 0, 0.0),
            20260913,
        );
        let mut seen = Vec::new();
        for _ in 0..48 {
            seen.push(s.current_word_id());
            s.try_rate(3, 0.0).unwrap();
        }
        for i in 0..seen.len() {
            for j in (i + 1)..seen.len() {
                if j - i < SEEN_GUARD {
                    assert_ne!(
                        seen[i], seen[j],
                        "第 {i} 张与第 {j} 张只隔 {} 张就重复了：{:?}",
                        j - i,
                        seen
                    );
                }
            }
        }
    }

    #[test]
    fn days_elapsed_floors_and_clamps() {
        // 0.7 天 → 0
        assert_eq!(days_elapsed(0.0, 0.7 * MS_PER_DAY), 0);
        // 恰好 2 天 → 2
        assert_eq!(days_elapsed(0.0, 2.0 * MS_PER_DAY), 2);
        // 2.99 天 → 2（向下取整）
        assert_eq!(days_elapsed(0.0, 2.99 * MS_PER_DAY), 2);
        // 未来时间（负值）→ 0
        assert_eq!(days_elapsed(10.0 * MS_PER_DAY, 1.0 * MS_PER_DAY), 0);
        // NaN 与无穷 → 0
        assert_eq!(days_elapsed(f64::NAN, 0.0), 0);
        assert_eq!(days_elapsed(0.0, f64::INFINITY), 0);
    }

    #[test]
    fn progress_percent_rounds_and_handles_empty() {
        assert_eq!(progress_percent(0, 0), 0);
        assert_eq!(progress_percent(0, 5), 0);
        assert_eq!(progress_percent(1, 5), 20);
        assert_eq!(progress_percent(2, 3), 67); // 66.67 四舍五入
        assert_eq!(progress_percent(9, 5), 100); // 越界时截断
    }

    #[test]
    fn pick_branch_maps_rating_to_slot() {
        let states = compute_next_states(None, 0.9, 0).unwrap();
        assert_eq!(pick_branch(states.clone(), 1).unwrap().interval_days,
                   states.again.interval_days);
        assert_eq!(pick_branch(states.clone(), 2).unwrap().interval_days,
                   states.hard.interval_days);
        assert_eq!(pick_branch(states.clone(), 3).unwrap().interval_days,
                   states.good.interval_days);
        assert_eq!(pick_branch(states, 4).unwrap().interval_days,
                   crate::fsrs_engine::compute_next_states(None, 0.9, 0).unwrap().easy.interval_days);
        assert!(pick_branch(compute_next_states(None, 0.9, 0).unwrap(), 0).is_none());
        assert!(pick_branch(compute_next_states(None, 0.9, 0).unwrap(), 5).is_none());
    }

    #[test]
    fn session_rates_in_order_and_returns_submit_payload() {
        let queue = vec![learned(1, "ability", 0.2, 9.0, 9.0 * MS_PER_DAY)];
        let new = vec![card(2, "achieve")];
        let mut s = session(queue, &new, &[], 12345);

        assert_eq!(s.total(), 2);
        assert_eq!(s.done(), 0);
        assert!(!s.is_finished());

        // 计划卡（新词）排在最前：先评它，再评复习区的卡
        assert_eq!(s.current_word_id(), 2, "今日计划卡优先");
        let payload = s.try_rate(3, 0.0).expect("评分应成功");
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["word_id"].as_u64().unwrap(), 2);
        assert_eq!(v["rating"].as_u64().unwrap(), 3);
        assert!(v["stability"].as_f64().unwrap() > 0.0);
        assert!(v["interval_days"].as_f64().unwrap() > 0.0);

        assert_eq!(s.done(), 1);
        assert_eq!(s.current_word_id(), 1, "接着是复习区的卡");
        assert_eq!(s.total(), 2, "评完的卡插回池子，总数不变");

        s.try_rate(1, 0.0).expect("第二张也应能评分");
        assert_eq!(s.done(), 2);
        // 无限复习：池子不会空，永远有下一张
        assert!(!s.is_finished(), "评完不该结束——卡会按新到期时间插回池子");
        assert!(s.current_word_id() > 0);
        assert!(s.try_rate(3, 0.0).is_ok(), "还能继续评");
    }

    #[test]
    fn session_rejects_invalid_rating() {
        let new = vec![card(1, "w")];
        let mut s = session(Vec::new(), &new, &[], 1);
        assert!(s.try_rate(0, 0.0).is_err());
        assert!(s.try_rate(5, 0.0).is_err());
        assert_eq!(s.done(), 0, "非法评分不应算作已复习");
        assert_eq!(s.current_word_id(), 1, "非法评分不应换卡");
    }

    #[test]
    fn session_accepts_api_envelope_and_empty_input() {
        let queue = r#"{"code":200,"data":{"items":[]},"message":"获取成功"}"#;
        let new = r#"{"code":200,"data":{"items":[{"id":9,"word":"decide","phonetic":"/x/","meaning":"v. 决定","example":"e"}]},"message":"ok"}"#;
        // 走 parse_items 的信封路径，再用 build 建会话（避开宿主上不可用的 JsValue）
        let s = ReviewSession::build(
            to_queue_inputs(parse_items(queue).unwrap()),
            parse_items(new).unwrap(),
            Vec::new(),
            plan_opts(5, 0, 0.0),
            7,
        );
        assert_eq!(s.total(), 1, "只有一个新词候选");
        assert_eq!(s.current_word_id(), 9);
        assert!(s.current_json(0.0).contains("decide"));
        assert!(s.current_json(0.0).contains(r#""source":"new""#));
        // 新词只给 status，不给记忆元信息
        assert!(s.current_json(0.0).contains(r#""status":"new""#));
        assert!(!s.current_json(0.0).contains("retrievability"));
    }

    #[test]
    fn session_reports_plan_targets() {
        let new: Vec<ApiCard> = (10..20).map(|i| card(i, "n")).collect();
        let probes: Vec<ApiCard> = (20..30).map(|i| due_card(i, "p", 8.0, 3.0)).collect();
        let s = ReviewSession::build(
            Vec::new(),
            new,
            probes,
            plan_opts(5, 5, 10.0 * MS_PER_DAY),
            3,
        );
        let v: serde_json::Value = serde_json::from_str(&s.plan_json()).unwrap();
        assert_eq!(v["new_target"], 5);
        assert_eq!(v["probe_target"], 5);
        assert_eq!(v["plan_len"], 10);
        assert_eq!(s.total(), 10, "没有复习区时队列就是计划区");
    }

    // ---------- 抽查：按新卡重置 ----------

    #[test]
    fn probe_rating_resets_memory_state() {
        // 一张已经稳定到 60 天的卡：正常复习会得到很长的间隔，
        // 但被抽查时应按新卡重算（Good → 初始稳定度 2.3065 天）
        let matured = 60.0f32;
        let queue = vec![learned(1, "mature", matured, 3.0, 100.0 * MS_PER_DAY)];
        let probes = vec![due_card(1, "mature", matured, 3.0)];
        let mut s = ReviewSession::build(
            queue,
            Vec::new(),
            probes,
            plan_opts(0, 5, 10.0 * MS_PER_DAY),
            5,
        );
        assert_eq!(
            s.current.as_ref().map(|c| c.source),
            Some(CardSource::Probe),
            "抽查卡应排在复习区的卡之前（计划卡优先）"
        );

        let payload = s.try_rate(3, 10.0 * MS_PER_DAY).unwrap();
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let interval = v["interval_days"].as_f64().unwrap();
        assert!(
            (interval - 2.3065).abs() < 1e-3,
            "抽查应按新卡重算（Good → 2.3065 天），得到 {interval}"
        );
        assert_eq!(v["is_probe"], true, "抽查必须在请求体里标记，供后端记进日志");
    }

    #[test]
    fn normal_review_keeps_memory_state() {
        // 同一张卡走正常复习：间隔应远大于新卡（证明上面那条不是巧合）
        let matured = 60.0f32;
        let queue = vec![learned(1, "mature", matured, 3.0, 5.0 * MS_PER_DAY)];
        let mut s = ReviewSession::build(
            queue,
            Vec::new(),
            Vec::new(),
            plan_opts(0, 0, 10.0 * MS_PER_DAY),
            5,
        );
        let payload = s.try_rate(3, 10.0 * MS_PER_DAY).unwrap();
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(
            v["interval_days"].as_f64().unwrap() > 30.0,
            "稳定到 60 天的卡正常复习应拿到长间隔，得到 {}",
            v["interval_days"]
        );
        assert_eq!(v["is_probe"], false);
    }

    #[test]
    fn probe_meta_is_marked_as_probe() {
        let probes = vec![ApiCard {
            stability: Some(60.0),
            difficulty: Some(3.0),
            reps: Some(9),
            ..card(1, "mature")
        }];
        let s = ReviewSession::build(
            Vec::new(),
            Vec::new(),
            probes,
            plan_opts(0, 5, 10.0 * MS_PER_DAY),
            5,
        );
        let v: serde_json::Value = serde_json::from_str(&s.current_json(10.0 * MS_PER_DAY)).unwrap();
        assert_eq!(v["meta"]["status"], "probe", "界面要能看出这是抽查而不是到期");
        assert_eq!(v["meta"]["reps"], 9, "抽查仍展示原有的复习次数供参考");
        assert!((v["meta"]["stability"].as_f64().unwrap() - 60.0).abs() < 1e-3);
    }

    #[test]
    fn parse_plan_options_handles_empty_and_missing() {
        assert_eq!(parse_plan_options("").unwrap().new_limit, 0);
        assert_eq!(parse_plan_options("  ").unwrap().probe_limit, 0);
        let o = parse_plan_options(r#"{"new_limit":3}"#).unwrap();
        assert_eq!(o.new_limit, 3);
        assert_eq!(o.probe_limit, 0, "缺失字段按 0 处理");
        assert!(o.probed_ids.is_empty());
        assert!(parse_plan_options("{坏JSON").is_err());
    }

    #[test]
    fn current_json_exposes_senses_parts_and_meta() {
        // 已到期的复习卡：应带上释义块、例句切分与完整记忆元信息
        let queue = vec![learned(1, "purpose", 2.3, 5.0, 5.0 * MS_PER_DAY)];
        let mut s = ReviewSession::build(
            queue,
            Vec::new(),
            Vec::new(),
            plan_opts(0, 0, 10.0 * MS_PER_DAY),
            1,
        );
        // 词条文本单独构造（learned 里没有释义与例句）。
        // 注意：当前卡已经不在池子里了（抽出来放在 current 上），所以改的是 current
        s.current.as_mut().unwrap().card.meaning = "n. 目的；意图".to_string();
        s.current.as_mut().unwrap().card.example = "The purpose of this meeting is to discuss.".to_string();
        s.current.as_mut().unwrap().card.example_translation = "这次会议的目的是讨论。".to_string();
        s.current.as_mut().unwrap().card.reps = Some(3);
        // learned() 把上次复习时间设成「到期前一天」，这里改成「到期前 7 天」，
        // 于是 now = 第 10 天时正好距上次 2 天（与断言一致）
        s.current.as_mut().unwrap().last_ms = Some(8.0 * MS_PER_DAY);

        let now = 10.0 * MS_PER_DAY; // 距上次 2 天
        let v: serde_json::Value = serde_json::from_str(&s.current_json(now)).unwrap();

        // 没填多释义时，由 meaning 自动拆成一块
        let senses = v["senses"].as_array().unwrap();
        assert_eq!(senses.len(), 1);
        assert_eq!(senses[0]["pos"], "n.");
        assert_eq!(senses[0]["meaning"], "目的；意图");
        assert!(senses[0].get("example_parts").is_none(), "没有独立例句时不该输出该字段");

        assert_eq!(v["example_translation"], "这次会议的目的是讨论。");

        let parts = v["example_parts"].as_array().unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[1]["text"], "purpose");
        assert_eq!(parts[1]["hit"], true);
        assert_eq!(parts[0]["hit"], false);

        assert_eq!(v["meta"]["status"], "due");
        assert_eq!(v["meta"]["days_since_last"], 2);
        assert_eq!(v["meta"]["reps"], 3);
        assert!((v["meta"]["stability"].as_f64().unwrap() - 2.3).abs() < 1e-3);
        let r = v["meta"]["retrievability"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&r), "可提取率应在 0~1：{r}");
        assert!(r < 1.0, "过了 2 天，预计记住应小于 1：{r}");
    }

    #[test]
    fn current_json_splits_legacy_multi_sense_meaning() {
        // 历史数据把两个义项写在一行：应拆成名词、动词两块
        let mut c = card(1, "benefit");
        c.meaning = "n. 好处；益处 v. 有益于".to_string();
        let s = session(Vec::new(), &[c], &[], 1);

        let v: serde_json::Value = serde_json::from_str(&s.current_json(0.0)).unwrap();
        let senses = v["senses"].as_array().unwrap();
        assert_eq!(senses.len(), 2);
        assert_eq!(senses[0]["pos"], "n.");
        assert_eq!(senses[0]["meaning"], "好处；益处");
        assert_eq!(senses[1]["pos"], "v.");
        assert_eq!(senses[1]["meaning"], "有益于");
        // 词条级例句为空 → 不输出 example_parts，前端整块不渲染
        assert!(v.get("example_parts").is_none() || v["example_parts"].as_array().unwrap().is_empty());
    }

    #[test]
    fn current_json_prefers_explicit_senses() {
        // 词条填了多释义时，直接用它们（每块可带自己的例句与译文），不再按 meaning 拆
        let mut c = card(1, "benefit");
        c.meaning = "n. 好处；益处 v. 有益于".to_string();
        c.example = "Exercise has many benefits.".to_string();
        c.senses = Some(vec![
            ApiSense {
                pos: "n.".to_string(),
                meaning: "好处；益处".to_string(),
                example: "Exercise has many benefits.".to_string(),
                example_translation: "锻炼有很多好处。".to_string(),
            },
            ApiSense {
                pos: "v.".to_string(),
                meaning: "有益于".to_string(),
                example: String::new(),
                example_translation: String::new(),
            },
        ]);
        let s = session(Vec::new(), &[c], &[], 1);

        let v: serde_json::Value = serde_json::from_str(&s.current_json(0.0)).unwrap();
        let senses = v["senses"].as_array().unwrap();
        assert_eq!(senses.len(), 2);
        assert_eq!(senses[0]["translation"], "锻炼有很多好处。");
        let parts = senses[0]["example_parts"].as_array().unwrap();
        assert_eq!(parts.iter().find(|p| p["hit"] == true).unwrap()["text"], "benefits");
        // 第二块没有独立例句与译文 → 两个字段都不输出
        assert!(senses[1].get("example_parts").is_none());
        assert!(senses[1].get("translation").is_none());
    }

    #[test]
    fn current_json_skips_blank_senses_and_falls_back() {
        // 后台加了一行没填 → 整条空释义被跳过
        let mut c = card(1, "apply");
        c.meaning = "v. 申请；应用".to_string();
        c.senses = Some(vec![ApiSense::default(), ApiSense::default()]);
        let s = session(Vec::new(), &[c], &[], 1);

        let v: serde_json::Value = serde_json::from_str(&s.current_json(0.0)).unwrap();
        let senses = v["senses"].as_array().unwrap();
        assert_eq!(senses.len(), 1, "全是空条目时应退回按 meaning 自动拆");
        assert_eq!(senses[0]["meaning"], "申请；应用");
    }

    #[test]
    fn parse_items_tolerates_null_senses() {
        // Go 侧即使输出 "senses": null 也不能让整批卡片解析失败
        let json = r#"{"data":{"items":[{"id":1,"word":"apply","senses":null}]}}"#;
        let items = parse_items(json).unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].senses.is_none());
    }

    #[test]
    fn parses_senses_from_api_payload() {
        let json = r#"{"data":{"items":[{"id":1,"word":"benefit",
            "example_translation":"锻炼有很多好处。",
            "senses":[{"pos":"n.","meaning":"好处","example":"Benefits here.","example_translation":"这里的好处。"}]}]}}"#;
        let items = parse_items(json).unwrap();
        assert_eq!(items[0].example_translation, "锻炼有很多好处。");
        let senses = items[0].senses.as_ref().unwrap();
        assert_eq!(senses.len(), 1);
        assert_eq!(senses[0].pos, "n.");
        assert_eq!(senses[0].example_translation, "这里的好处。");
    }

    #[test]
    fn parse_items_handles_both_shapes() {
        assert_eq!(parse_items("[]").unwrap().len(), 0);
        assert_eq!(parse_items("").unwrap().len(), 0);
        assert_eq!(parse_items(r#"[{"id":1,"word":"a"}]"#).unwrap().len(), 1);
        assert_eq!(
            parse_items(r#"{"data":{"items":[{"id":2,"word":"b"}]}}"#)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(parse_items(r#"{"code":200,"data":null}"#).unwrap().len(), 0);
        // Go 在结果为空时会把 nil 切片序列化成 null（真实接口会这样返回）
        assert_eq!(
            parse_items(r#"{"code":200,"data":{"items":null},"message":"获取成功"}"#)
                .unwrap()
                .len(),
            0,
            "items 为 null 时必须当作空列表，而不是解析失败"
        );
        assert_eq!(
            parse_items(r#"{"code":200,"data":{},"message":"获取成功"}"#)
                .unwrap()
                .len(),
            0,
            "items 字段缺失时也必须当作空列表"
        );
        assert!(parse_items("{坏JSON").is_err());
    }

    #[test]
    fn new_word_uses_zero_days_elapsed() {
        // 新词没有 last_ms，days 一律按 0 处理；新卡 Good 的首个间隔等于初始稳定度 2.3065
        let new = vec![card(1, "w")];
        let mut s = session(Vec::new(), &new, &[], 1);
        let payload = s.try_rate(3, 123456789.0).unwrap();
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let interval = v["interval_days"].as_f64().unwrap();
        assert!((interval - 2.3065).abs() < 1e-3, "得到 {interval}");
    }

    #[test]
    fn due_card_uses_elapsed_days_from_last_review() {
        // 只改变 last_ms，其它输入完全相同 → 结果必须变化（证明 last_ms 真的参与了天数换算）
        let now = 30.0 * MS_PER_DAY;
        let mk = |last_days: f64| {
            let mut item = learned(1, "w", 2.3065, 2.118, 10.0 * MS_PER_DAY);
            item.last_ms = Some(last_days * MS_PER_DAY);
            ReviewSession::build(
                vec![item],
                Vec::new(),
                Vec::new(),
                plan_opts(0, 0, now),
                1,
            )
        };
        // fresh：1 天前复习过（逾期 29 天）
        let mut fresh = mk(1.0);
        // overdue：7 天前复习过（逾期 23 天，离现在更近）
        let mut overdue = mk(7.0);

        let a: serde_json::Value = serde_json::from_str(&fresh.try_rate(3, now).unwrap()).unwrap();
        let b: serde_json::Value = serde_json::from_str(&overdue.try_rate(3, now).unwrap()).unwrap();
        let ia = a["interval_days"].as_f64().unwrap();
        let ib = b["interval_days"].as_f64().unwrap();

        assert_ne!(ia, ib, "last_ms 必须影响结果");
        // FSRS 语义：逾期越久（复习时可提取率越低），同一评分的下次间隔越长
        assert!(ia > ib, "逾期更久应得到更长间隔：{ia} vs {ib}");
    }

    // ---------- 追加复习区（整库往下翻，不限量） ----------

    /// 构造一个只有复习区的会话：n 张已到期（第 1 天到期）
    fn session_with_queue(n: u32) -> ReviewSession {
        let queue: Vec<QueueInput> = (1..=n)
            .map(|i| learned(i, "q", 1.0, 5.0, 1.0 * MS_PER_DAY))
            .collect();
        ReviewSession::build(
            queue,
            Vec::new(),
            Vec::new(),
            plan_opts(0, 0, 10.0 * MS_PER_DAY),
            42,
        )
    }

    #[test]
    fn append_inserts_page_into_pool() {
        let mut s = session_with_queue(3);
        assert_eq!(s.total(), 3);
        assert_eq!(s.done(), 0);

        let more: Vec<QueueInput> = (100..110)
            .map(|i| learned(i, "m", 1.0, 5.0, 20.0 * MS_PER_DAY))
            .collect();
        let added = s.append_page(more, 10.0 * MS_PER_DAY, 7);
        assert_eq!(added, 10, "复习区一页全量追加（不受新词批量限制）");
        assert_eq!(s.done(), 0, "追加不算已复习");
        assert_eq!(s.total(), 13, "池子应增长到 3 + 10");
        // 池子仍按到期时间有序
        let keys: Vec<f64> = s.pool.iter().map(sort_key).collect();
        let mut sorted = keys.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(keys, sorted);
    }

    #[test]
    fn append_skips_ids_already_in_pool() {
        let mut s = session_with_queue(2);
        assert_eq!(s.total(), 2);
        // id=1 已经在池子里（或手上）→ 不应重复追加
        let dup: Vec<QueueInput> = vec![learned(1, "q", 1.0, 5.0, 1.0 * MS_PER_DAY)];
        assert_eq!(s.append_page(dup, 10.0 * MS_PER_DAY, 7), 0, "已有这张卡了");

        // 手上那张也算「已有」
        let cur = s.current_word_id();
        let again: Vec<QueueInput> = vec![learned(cur, "q", 1.0, 5.0, 1.0 * MS_PER_DAY)];
        assert_eq!(s.append_page(again, 10.0 * MS_PER_DAY, 7), 0, "当前卡不能重复进池");
    }

    #[test]
    fn append_dedups_within_batch() {
        let mut s = session_with_queue(1);
        let dup: Vec<QueueInput> = vec![
            learned(9, "x", 1.0, 5.0, 20.0 * MS_PER_DAY),
            learned(9, "x", 1.0, 5.0, 20.0 * MS_PER_DAY),
        ];
        assert_eq!(s.append_page(dup, 10.0 * MS_PER_DAY, 7), 1, "同一批里重复的 id 只留一张");
    }

    #[test]
    fn append_only_adds_review_cards() {
        // 每日配额不因为翻页而变多：追加进来的都是复习区的卡
        let mut s = session_with_queue(1);
        let more: Vec<QueueInput> = (200..208)
            .map(|i| learned(i, "d", 1.0, 5.0, 20.0 * MS_PER_DAY))
            .collect();
        assert_eq!(s.append_page(more, 10.0 * MS_PER_DAY, 7), 8);
        assert!(s.pool.iter().all(|c| c.source == CardSource::Due));
        assert_eq!(s.plan_json().contains(r#""plan_len":0"#), true, "计划区长度不受追加影响");
    }

    #[test]
    fn append_is_noop_for_empty_input() {
        let mut s = session_with_queue(2);
        assert_eq!(s.append_page(Vec::new(), 10.0 * MS_PER_DAY, 7), 0);
    }

    #[test]
    fn append_with_seed_grows_pool_and_keeps_rating() {
        // 走 wasm 边界那条路径（成功路径不碰 JsValue，所以宿主上可以测）
        let mut s = session_with_queue(2);
        assert_eq!(s.total(), 2);

        let more: Vec<QueueInput> = (100..105)
            .map(|i| learned(i, "m", 1.0, 5.0, 20.0 * MS_PER_DAY))
            .collect();
        let more_json =
            serde_json::to_string(&more.iter().map(|m| m.card.clone()).collect::<Vec<ApiCard>>())
                .unwrap();
        let added = s
            .append_with_seed(&more_json, r#"{"now_ms":864000000}"#, 3)
            .unwrap();
        assert_eq!(added, 5);
        assert_eq!(s.total(), 7, "池子应增长到 2 + 5");

        // 追加之后评分流程照旧
        s.try_rate(3, 10.0 * MS_PER_DAY).unwrap();
        assert_eq!(s.done(), 1);
    }

    #[test]
    fn append_returns_zero_when_nothing_available() {
        let mut s = session_with_queue(1);
        assert_eq!(s.append_with_seed("[]", "{}", 3).unwrap(), 0);
        assert_eq!(s.total(), 1);
    }

    #[test]
    fn append_fills_current_when_session_started_empty() {
        // 进来时池子是空的（词库还没加载）：追加一页之后必须能出题
        let mut s = ReviewSession::build(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            plan_opts(0, 0, 10.0 * MS_PER_DAY),
            1,
        );
        assert!(s.is_finished(), "空池子就是没卡可出");
        assert_eq!(s.current_json(0.0), "{}");
        let more: Vec<QueueInput> = (1..=3)
            .map(|i| learned(i, "q", 1.0, 5.0, 5.0 * MS_PER_DAY))
            .collect();
        assert_eq!(s.append_page(more, 10.0 * MS_PER_DAY, 1), 3);
        assert!(!s.is_finished(), "追加之后应能出题");
        assert_eq!(s.current_word_id(), 1, "过期的先出");
    }
}
