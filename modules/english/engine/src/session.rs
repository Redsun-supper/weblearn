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
//! - **纯计算层**（[`plan_session`]、[`days_elapsed`]、[`pick_branch`]、[`progress_percent`]
//!   以及 `ReviewSession` 的 `try_new` / `try_rate`）：只吃 epoch 毫秒与下标，错误类型是
//!   `String`，可在宿主环境直接 `cargo test`
//! - **wasm 边界层**（`#[wasm_bindgen]` 那一组方法）：只做「ISO 时间字符串 → 毫秒」的解析
//!   和「`String` 错误 → `JsValue`」的转换，不承载业务逻辑
//!
//! 这样分层是有实际原因的：`JsValue` 在非 wasm32 目标上并未实现，一旦业务逻辑里
//! 碰了它，宿主单元测试就会直接 abort 而不是给出断言失败。
//!
//! ## 关于 `fsrs` 依赖的一个事实
//!
//! `fsrs` 6.6.2 只公开 `next_states()`，其内部用 `(1..=4).map(..)` 一次性算出
//! Again/Hard/Good/Easy 四个分支，**没有单评分入口**（连私有函数也没有）。
//! 所以这里仍是「算四个取一个」；每个分支只是几十次浮点运算，开销可忽略，
//! 不值得为了省这点计算去重写 FSRS 公式（会造成算法重复与版本漂移风险）。

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::card_view::{self, ExamplePart};
use crate::fsrs_engine::{
    compute_next_states, compute_retrievability, CardState, ItemStateOut, NextStatesOut,
};
use crate::randomizer::{random_indices, random_sample, random_seed};

/// 期望记忆保持率（与原前端一致，固定 0.9）
const DESIRED_RETENTION: f32 = 0.9;

/// 每次抽取新词的默认批量（**不是每日上限**：队列抽干后前端会再抽一批，直到词库没有未学词）
const DEFAULT_NEW_BATCH: u32 = 20;

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
    /// 到期复习卡
    Due,
    /// 新词
    New,
}

/// 会话中的一张计划卡
#[derive(Debug, Clone)]
pub struct PlannedCard {
    /// 来源：到期 / 新词
    pub source: CardSource,
    /// 在原始数组中的下标（便于排查与复现）
    pub index: u32,
    /// 复习前的记忆状态；新词为 `None`
    pub state: Option<CardState>,
    /// 上次复习时间（epoch 毫秒）；新词或时间缺失时为 `None`
    pub last_ms: Option<f64>,
    /// 词条数据（由 Rust 持有，JS 不再保存一份）
    pub card: ApiCard,
}

/// 编排今日队列：到期卡整体洗牌 + 新词无放回抽样，到期卡在前、新词在后。
///
/// 与改动前 JS 的行为逐位一致：
/// - 到期顺序 = `random_indices(due.len(), seed)`
/// - 新词顺序 = `random_sample(new.len(), min(new_limit, new.len()), seed ^ 0x9E3779B9)`
///
/// `due` 需要同时给出「上次复习时间（epoch 毫秒）」，因为距上次复习的天数要在
/// 评分那一刻才换算（与改动前一致，见 [`days_elapsed`]）。
pub fn plan_session(
    due: Vec<(ApiCard, Option<f64>)>,
    new: Vec<ApiCard>,
    seed: u32,
    new_limit: u32,
) -> Vec<PlannedCard> {
    let take = new_limit.min(new.len() as u32);
    let mut cards: Vec<PlannedCard> = Vec::with_capacity(due.len() + take as usize);

    // 到期卡：Fisher–Yates 整体洗牌
    let due_order = random_indices(due.len() as u32, seed);
    for &i in due_order.iter() {
        let idx = i as usize;
        // random_indices 保证返回 0..len 的排列，越界属实现错误，跳过而非 panic
        if let Some((card, last_ms)) = due.get(idx) {
            cards.push(PlannedCard {
                source: CardSource::Due,
                index: i,
                state: match (card.stability, card.difficulty) {
                    (Some(stability), Some(difficulty)) => Some(CardState {
                        stability,
                        difficulty,
                    }),
                    // 理论上不会发生：到期卡一定带记忆状态。真出现时按新卡处理，
                    // 比改动前「拼出残缺 JSON 导致引擎报错」更稳健
                    _ => None,
                },
                last_ms: *last_ms,
                card: card.clone(),
            });
        }
    }

    // 新词：无放回随机抽 take 个
    let new_order = random_sample(new.len() as u32, take, seed ^ 0x9E37_79B9);
    for &i in new_order.iter() {
        let idx = i as usize;
        if let Some(card) = new.get(idx) {
            cards.push(PlannedCard {
                source: CardSource::New,
                index: i,
                state: None,
                last_ms: None,
                card: card.clone(),
            });
        }
    }

    cards
}

/// 把接口返回的卡片转成计划输入：时间串 → epoch 毫秒
/// （`last_review_at` 优先，缺失时回退 `due_at`，与原 JS 一致）
fn to_due_inputs(cards: Vec<ApiCard>) -> Vec<(ApiCard, Option<f64>)> {
    cards
        .into_iter()
        .map(|card| {
            let last_ms = card
                .last_review_at
                .as_deref()
                .or(card.due_at.as_deref())
                .and_then(parse_time_ms);
            (card, last_ms)
        })
        .collect()
}

/// 把新一批卡片追加到队列尾部（**游标不动**），只返回**新增**的部分。
///
/// 去重规则：跳过 id 已经出现在「待办区」（`existing[cursor..]`，即还没评分的部分）里的卡片。
/// 游标之前已经评完的卡**不算重复**——它到期后再次被抽到属于正常复习。
/// 这样即使前端补词请求与提交存在竞态，也不会让同一张卡在待办队列里出现两次。
pub fn append_plan(
    existing: &[PlannedCard],
    cursor: usize,
    due: Vec<(ApiCard, Option<f64>)>,
    new: Vec<ApiCard>,
    new_limit: u32,
    seed: u32,
) -> Vec<PlannedCard> {
    use std::collections::HashSet;

    let mut pending: HashSet<u32> = HashSet::new();
    for card in existing.iter().skip(cursor) {
        pending.insert(card.card.id);
    }

    // insert 返回 false 说明该 id 已在待办区（或本次批次里已经出现过），直接丢弃
    plan_session(due, new, seed, new_limit)
        .into_iter()
        .filter(|card| pending.insert(card.card.id))
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
        status: "due",
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
    cards: Vec<PlannedCard>,
    cursor: usize,
    desired_retention: f32,
}

/// 纯计算实现：不参与 wasm 导出，错误类型是 `String`，可在宿主上直接单元测试。
impl ReviewSession {
    /// 由已解析的卡片建立会话（纯计算，不涉及 JSON 与时间字符串）。
    ///
    /// `due` 为 `(到期卡, 上次复习时间 epoch 毫秒)` 的列表。
    pub fn build(
        due: Vec<(ApiCard, Option<f64>)>,
        new: Vec<ApiCard>,
        new_limit: u32,
        seed: u32,
    ) -> ReviewSession {
        let limit = if new_limit == 0 {
            DEFAULT_NEW_BATCH
        } else {
            new_limit
        };
        ReviewSession {
            cards: plan_session(due, new, seed, limit),
            cursor: 0,
            desired_retention: DESIRED_RETENTION,
        }
    }

    /// 评分：用引擎算出新记忆状态，返回可直接作为 `POST /api/reviews/submit`
    /// 请求体的 JSON。只有计算成功才推进游标；出错时不移动（与改动前一致）。
    pub fn try_rate(&mut self, rating: u8, now_ms: f64) -> Result<String, String> {
        if !(1..=4).contains(&rating) {
            return Err(format!("rating 必须为 1~4，收到 {rating}"));
        }
        let card = self
            .cards
            .get(self.cursor)
            .ok_or_else(|| "复习队列已完成，没有可评分的卡片".to_string())?;

        // 距上次复习的天数在「评分这一刻」换算（与改动前一致）
        let days = match card.last_ms {
            Some(last_ms) => days_elapsed(last_ms, now_ms),
            None => 0, // 新词
        };

        let states = compute_next_states(card.state, self.desired_retention, days)
            .map_err(|e| format!("FSRS 计算失败: {e}"))?;
        let chosen = pick_branch(states, rating)
            .ok_or_else(|| format!("无对应的评分分支: {rating}"))?;

        let payload = SubmitPayload {
            word_id: card.card.id,
            rating,
            stability: chosen.memory.stability,
            difficulty: chosen.memory.difficulty,
            interval_days: chosen.interval_days,
        };

        // 计算成功后才推进游标
        self.cursor += 1;

        serde_json::to_string(&payload).map_err(|e| format!("序列化失败: {e}"))
    }
}

// ---------- 会话对象：wasm 导出层（薄边界） ----------

fn js_err(msg: impl Into<String>) -> JsValue {
    JsValue::from_str(&msg.into())
}

#[wasm_bindgen]
impl ReviewSession {
    /// 建立会话：内部取一次系统随机种子。
    ///
    /// - `due_json` / `new_json`：`GET /api/reviews/due`、`GET /api/reviews/new` 的响应体原文
    /// - `new_limit`：本次抽取的新词批量（传 0 表示使用默认值 20）；**不是每日上限**
    #[wasm_bindgen(constructor)]
    pub fn new(due_json: &str, new_json: &str, new_limit: u32) -> Result<ReviewSession, JsValue> {
        let seed = random_seed().map_err(|e| js_err(format!("获取随机种子失败: {e:?}")))?;
        Self::with_seed(due_json, new_json, new_limit, seed)
    }

    /// 建立会话（显式种子）：便于复现与测试。
    #[wasm_bindgen]
    pub fn with_seed(
        due_json: &str,
        new_json: &str,
        new_limit: u32,
        seed: u32,
    ) -> Result<ReviewSession, JsValue> {
        let due_cards = parse_items(due_json).map_err(js_err)?;
        let new_cards = parse_items(new_json).map_err(js_err)?;
        let due = to_due_inputs(due_cards);

        Ok(Self::build(due, new_cards, new_limit, seed))
    }

    /// 追加一批卡片到队列尾部（**游标不动**），返回实际追加的数量。
    ///
    /// 用于「不限制每日新词、连续抽取」：队列走完后前端再取一批交进来即可。
    /// 返回 0 表示这批没有可追加的卡片（词库已无未学词）。
    pub fn append(&mut self, due_json: &str, new_json: &str, new_limit: u32) -> Result<u32, JsValue> {
        let seed = random_seed().map_err(|e| js_err(format!("获取随机种子失败: {e:?}")))?;
        self.append_with_seed(due_json, new_json, new_limit, seed)
    }

    /// 追加一批卡片（显式种子）：便于复现与单元测试。
    #[wasm_bindgen]
    pub fn append_with_seed(
        &mut self,
        due_json: &str,
        new_json: &str,
        new_limit: u32,
        seed: u32,
    ) -> Result<u32, JsValue> {
        let due_cards = parse_items(due_json).map_err(js_err)?;
        let new_cards = parse_items(new_json).map_err(js_err)?;
        let due = to_due_inputs(due_cards);

        let limit = if new_limit == 0 {
            DEFAULT_NEW_BATCH
        } else {
            new_limit
        };
        let added = append_plan(&self.cards, self.cursor, due, new_cards, limit, seed);
        let count = added.len() as u32;
        self.cards.extend(added);
        Ok(count)
    }

    /// 队列总张数
    pub fn total(&self) -> u32 {
        self.cards.len() as u32
    }

    /// 已完成张数
    pub fn done(&self) -> u32 {
        self.cursor.min(self.cards.len()) as u32
    }

    /// 是否已全部完成（队列为空也算完成）
    pub fn is_finished(&self) -> bool {
        self.cursor >= self.cards.len()
    }

    /// 本轮进度百分比 0~100
    pub fn progress_percent(&self) -> u32 {
        progress_percent(self.done(), self.total())
    }

    /// 当前卡片所属词条 id；无当前卡片时返回 0
    pub fn current_word_id(&self) -> u32 {
        self.cards.get(self.cursor).map(|c| c.card.id).unwrap_or(0)
    }

    /// 当前卡片的展示数据（JSON）；无当前卡片时返回空对象 `{}`
    ///
    /// `now_ms` 用于换算「距上次复习天数」与「预计记住」，所以由 JS 传 `Date.now()`。
    pub fn current_json(&self, now_ms: f64) -> String {
        match self.cards.get(self.cursor) {
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

    fn json_of(cards: &[ApiCard]) -> String {
        serde_json::to_string(cards).unwrap()
    }

    /// 构造会话：时间以毫秒直接给出（宿主环境无需 ISO 解析）
    fn session(due: &[ApiCard], times: &[Option<f64>], new: &[ApiCard], seed: u32) -> ReviewSession {
        let due = due.iter().cloned().zip(times.iter().copied()).collect();
        ReviewSession::build(due, new.to_vec(), 5, seed)
    }

    #[test]
    fn plan_puts_due_before_new_and_respects_limit() {
        let due = vec![
            (due_card(1, "a", 1.0, 5.0), Some(0.0)),
            (due_card(2, "b", 2.0, 5.0), Some(0.0)),
        ];
        let new: Vec<ApiCard> = (10..20).map(|i| card(i, "n")).collect();
        let planned = plan_session(due, new, 42, 5);

        assert_eq!(planned.len(), 7, "2 张到期 + 5 张新词");
        assert_eq!(planned[0].source, CardSource::Due);
        assert_eq!(planned[1].source, CardSource::Due);
        assert!(planned[2..].iter().all(|c| c.source == CardSource::New));
    }

    #[test]
    fn plan_is_deterministic_for_same_seed() {
        let mk = || {
            let due = vec![
                (due_card(1, "a", 1.0, 5.0), Some(0.0)),
                (due_card(2, "b", 1.0, 5.0), Some(0.0)),
                (due_card(3, "c", 1.0, 5.0), Some(0.0)),
            ];
            let new: Vec<ApiCard> = (10..30).map(|i| card(i, "n")).collect();
            plan_session(due, new, 20260913, 5)
        };
        let a = mk();
        let b = mk();
        let ida: Vec<u32> = a.iter().map(|c| c.card.id).collect();
        let idb: Vec<u32> = b.iter().map(|c| c.card.id).collect();
        assert_eq!(ida, idb, "同一种子必须得到同一队列");
    }

    #[test]
    fn plan_due_is_permutation_of_input() {
        let due: Vec<(ApiCard, Option<f64>)> = (0..8)
            .map(|i| (due_card(i, "w", 1.0, 5.0), Some(0.0)))
            .collect();
        let planned = plan_session(due, Vec::new(), 7, 5);
        let mut ids: Vec<u32> = planned.iter().map(|c| c.card.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, (0..8).collect::<Vec<u32>>(), "洗牌不应增删卡片");
    }

    #[test]
    fn plan_new_limit_capped_by_available() {
        let new: Vec<ApiCard> = (0..3).map(|i| card(i, "n")).collect();
        let planned = plan_session(Vec::new(), new, 1, 5);
        assert_eq!(planned.len(), 3, "新词不足 5 个时全部取用");
    }

    #[test]
    fn plan_falls_back_to_new_card_when_state_missing() {
        // 到期卡缺记忆状态时按新卡处理，而不是让引擎报错
        let due = vec![(card(1, "broken"), Some(0.0))];
        let planned = plan_session(due, Vec::new(), 1, 5);
        assert_eq!(planned.len(), 1);
        assert!(planned[0].state.is_none());
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
        let due = vec![due_card(1, "ability", 0.2, 9.0)];
        let new = vec![card(2, "achieve")];
        let mut s = session(&due, &[Some(0.0)], &new, 12345);

        assert_eq!(s.total(), 2);
        assert_eq!(s.done(), 0);
        assert!(!s.is_finished());
        assert_eq!(s.progress_percent(), 0);

        let first_id = s.current_word_id();
        let payload = s.try_rate(3, 0.0).expect("评分应成功");
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["word_id"].as_u64().unwrap() as u32, first_id);
        assert_eq!(v["rating"].as_u64().unwrap(), 3);
        assert!(v["stability"].as_f64().unwrap() > 0.0);
        assert!(v["interval_days"].as_f64().unwrap() > 0.0);

        assert_eq!(s.done(), 1);
        assert_eq!(s.progress_percent(), 50);
        assert_ne!(s.current_word_id(), first_id, "评分后应换到下一张");

        s.try_rate(1, 0.0).expect("第二张也应能评分");
        assert!(s.is_finished());
        assert_eq!(s.progress_percent(), 100);
        assert_eq!(s.current_json(0.0), "{}", "队列走完后不再有当前卡片");
        assert!(s.try_rate(3, 0.0).is_err(), "队列走完后评分应报错");
    }

    #[test]
    fn session_rejects_invalid_rating() {
        let new = vec![card(1, "w")];
        let mut s = session(&[], &[], &new, 1);
        assert!(s.try_rate(0, 0.0).is_err());
        assert!(s.try_rate(5, 0.0).is_err());
        assert_eq!(s.done(), 0, "非法评分不应推进游标");
    }

    #[test]
    fn session_accepts_api_envelope_and_empty_input() {
        let due = r#"{"code":200,"data":{"items":[]},"message":"获取成功"}"#;
        let new = r#"{"code":200,"data":{"items":[{"id":9,"word":"decide","phonetic":"/x/","meaning":"v. 决定","example":"e"}]},"message":"ok"}"#;
        // 走 parse_items 的信封路径，再用 build 建会话（避开宿主上不可用的 JsValue）
        let s = ReviewSession::build(
            parse_items(due)
                .unwrap()
                .into_iter()
                .map(|c| (c, None))
                .collect(),
            parse_items(new).unwrap(),
            0,
            5,
        );
        assert_eq!(s.total(), 1, "默认批量 20，但只有一个新词");
        assert_eq!(s.current_word_id(), 9);
        assert!(s.current_json(0.0).contains("decide"));
        assert!(s.current_json(0.0).contains(r#""source":"new""#));
        // 新词只给 status，不给记忆元信息
        assert!(s.current_json(0.0).contains(r#""status":"new""#));
        assert!(!s.current_json(0.0).contains("retrievability"));
    }

    #[test]
    fn current_json_exposes_senses_parts_and_meta() {
        // 到期卡：应带上释义块、例句切分与完整记忆元信息
        let due = vec![due_card(1, "purpose", 2.3, 5.0)];
        let mut s = session(&due, &[Some(10.0 * MS_PER_DAY)], &[], 1);
        // 词条文本单独构造（due_card 里没有释义与例句）
        s.cards[0].card.meaning = "n. 目的；意图".to_string();
        s.cards[0].card.example = "The purpose of this meeting is to discuss.".to_string();
        s.cards[0].card.example_translation = "这次会议的目的是讨论。".to_string();
        s.cards[0].card.reps = Some(3);

        let now = 12.0 * MS_PER_DAY; // 距上次 2 天
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
        let s = session(&[], &[], &[c], 1);

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
        let s = session(&[], &[], &[c], 1);

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
        let s = session(&[], &[], &[c], 1);

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
        let mut s = session(&[], &[], &new, 1);
        let payload = s.try_rate(3, 123456789.0).unwrap();
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let interval = v["interval_days"].as_f64().unwrap();
        assert!((interval - 2.3065).abs() < 1e-3, "得到 {interval}");
    }

    #[test]
    fn due_card_uses_elapsed_days_from_last_review() {
        // 只改变 last_ms，其它输入完全相同 → 结果必须变化（证明 last_ms 真的参与了天数换算）
        let due = vec![due_card(1, "w", 2.3065, 2.118)];
        let now = 30.0 * MS_PER_DAY;
        // fresh：1 天前复习过（逾期 29 天）
        let mut fresh = session(&due, &[Some(1.0 * MS_PER_DAY)], &[], 1);
        // overdue：7 天前复习过（逾期 23 天，离现在更近）
        let mut overdue = session(&due, &[Some(7.0 * MS_PER_DAY)], &[], 1);

        let a: serde_json::Value = serde_json::from_str(&fresh.try_rate(3, now).unwrap()).unwrap();
        let b: serde_json::Value = serde_json::from_str(&overdue.try_rate(3, now).unwrap()).unwrap();
        let ia = a["interval_days"].as_f64().unwrap();
        let ib = b["interval_days"].as_f64().unwrap();

        assert_ne!(ia, ib, "last_ms 必须影响结果");
        // FSRS 语义：逾期越久（复习时可提取率越低），同一评分的下次间隔越长
        assert!(ia > ib, "逾期更久应得到更长间隔：{ia} vs {ib}");
    }

    // ---------- 追加卡片（连续抽词，不限制每日新词） ----------

    /// 构造一个会话：待办队列里有 n 张新词
    fn session_with_new(n: u32, batch: u32) -> ReviewSession {
        let new: Vec<ApiCard> = (1..=n).map(|i| card(i, "n")).collect();
        ReviewSession::build(Vec::new(), new, batch, 42)
    }

    #[test]
    fn append_adds_cards_without_moving_cursor() {
        let s = session_with_new(3, 3);
        assert_eq!(s.total(), 3);
        assert_eq!(s.done(), 0);

        let more: Vec<ApiCard> = (100..110).map(|i| card(i, "m")).collect();
        let added = append_plan(&s.cards, s.cursor, Vec::new(), more, 5, 7);
        assert_eq!(added.len(), 5, "应按批量追加 5 张");
        assert_eq!(s.done(), 0, "追加不应推进游标");
        assert_eq!(s.total(), 3, "append_plan 只计算，不改动原队列");
    }

    #[test]
    fn append_skips_ids_already_pending() {
        // 待办区已有 id=1 → 同一张卡不应被重复追加进待办队列
        let mut s = session_with_new(1, 1);
        assert_eq!(s.total(), 1);
        let dup = vec![card(1, "n")];
        assert!(
            append_plan(&s.cards, s.cursor, Vec::new(), dup, 5, 7).is_empty(),
            "待办区已有的卡不能再追加一份"
        );

        // 评完之后（越过游标）再抽到同一 id 属于正常复习，应允许追加
        s.try_rate(3, 0.0).unwrap();
        let again = append_plan(&s.cards, s.cursor, Vec::new(), vec![card(1, "n")], 5, 7);
        assert_eq!(again.len(), 1, "已评完的卡到期后再次抽到应允许");
    }

    #[test]
    fn append_dedups_within_batch() {
        let s = session_with_new(1, 1);
        let dup = vec![card(9, "x"), card(9, "x")];
        assert_eq!(
            append_plan(&s.cards, s.cursor, Vec::new(), dup, 5, 7).len(),
            1,
            "同一批里重复的 id 只保留一张"
        );
    }

    #[test]
    fn append_does_not_limit_due_cards() {
        // 批量参数只限制新词；到期卡与 plan_session 一致是全量洗牌
        let s = session_with_new(1, 1);
        let due: Vec<(ApiCard, Option<f64>)> = (200..208)
            .map(|i| (due_card(i, "d", 1.0, 5.0), Some(0.0)))
            .collect();
        let added = append_plan(&s.cards, s.cursor, due, Vec::new(), 1, 7);
        assert_eq!(added.len(), 8, "到期卡应全部追加");
    }

    #[test]
    fn append_is_empty_for_empty_input() {
        let s = session_with_new(2, 2);
        assert!(append_plan(&s.cards, s.cursor, Vec::new(), Vec::new(), 5, 7).is_empty());
    }

    #[test]
    fn append_is_deterministic_for_same_seed() {
        let s = session_with_new(1, 1);
        let more: Vec<ApiCard> = (300..320).map(|i| card(i, "m")).collect();
        let a: Vec<u32> = append_plan(&s.cards, s.cursor, Vec::new(), more.clone(), 5, 99)
            .iter()
            .map(|c| c.card.id)
            .collect();
        let b: Vec<u32> = append_plan(&s.cards, s.cursor, Vec::new(), more, 5, 99)
            .iter()
            .map(|c| c.card.id)
            .collect();
        assert_eq!(a, b, "同种子应得到同一批");
    }

    #[test]
    fn append_with_seed_grows_queue_and_keeps_rating() {
        // 走 wasm 边界那条路径（成功路径不碰 JsValue，所以宿主上可以测）
        let mut s = session_with_new(2, 2);
        assert_eq!(s.total(), 2);

        let more: Vec<ApiCard> = (100..110).map(|i| card(i, "m")).collect();
        let added = s.append_with_seed("[]", &json_of(&more), 5, 3).unwrap();
        assert_eq!(added, 5);
        assert_eq!(s.total(), 7, "队列应增长到 2 + 5");

        // 追加之后评分流程照旧
        s.try_rate(3, 0.0).unwrap();
        assert_eq!(s.done(), 1);
        assert_eq!(s.progress_percent(), 14); // 1/7 ≈ 14%
    }

    #[test]
    fn append_returns_zero_when_nothing_available() {
        let mut s = session_with_new(1, 1);
        assert_eq!(s.append_with_seed("[]", "[]", 5, 3).unwrap(), 0);
        assert_eq!(s.total(), 1);
    }
}
