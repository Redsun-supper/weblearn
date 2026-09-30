// 英语模块 · 单词间隔复习（FSRS）
//
// 本文件是 ES module，由 main.js 在**进入英语页时**动态 import() 加载：
// 只有真正进入英语页才会加载本模块及其 WASM 引擎，首屏不再携带英语逻辑。
//
// 职责划分（详见 modules/english/README.md）：
// - 本文件  ：DOM 渲染、事件绑定、取数与提交（fetch）、本地存储、语音合成
// - engine/ ：队列编排、FSRS 计算、日期换算、进度统计、词性拆分与例句切分（Rust → WASM）
//
// 界面参考「极简全屏」风格：单词极大、操作区贴底、揭晓后例句里的目标词高亮。
// 本文件保持 ES5 写法（var / function），仅使用 export 做模块导出。

// ---------- 模块级状态 ----------

// 复习页的全部可变状态。队列、游标、记忆上下文都在引擎（Rust）侧，这里只放界面与今日计划相关的东西。
// ⚠️ planTotals 是今日计划的分母 {new, probe}，**建会话时算一次后固定**——每次渲染重算会让
// 分子涨、分母也跟着涨（出现过「新词 1/4、2/5」）；pendingSubmit 是最近一次提交的 Promise，
// 翻页取数前必须等它落库，否则刚评过的词会被当成到期卡再抽一次。
// started 表示「用户已经点过开始复习单词」：在此之前页面停在起始页，引擎与会话都还没建。
var state = {
    session: null,
    wasm: null,
    revealed: false,
    stats: null,
    card: null,
    pendingSubmit: null,
    refilling: false,
    planInfo: null,
    planTotals: null,
    planPhase: '',
    rounds: null,
    queueOffset: 0,
    queueTotal: 0,
    started: false,
    holdAutoSpeak: false
};

// 自动朗读开关在 localStorage 中的键名
var AUTO_SPEAK_KEY = 'reviewAutoSpeak';

// 沉浸模式（隐藏站点导航栏）在 localStorage 中的键名；没有记录时默认**开启**（进复习页就是要专注）
var IMMERSIVE_KEY = 'reviewImmersive';

// 今日计划（每天 5 个新词 + 5 个抽查）在 localStorage 中的键名。
// ⚠️ 为什么配额记在浏览器上而不是服务端：现在没有登录系统，服务端只有一份共享词库，
// 按「每天 5 个」在服务端算等于全站每天共放 5 个新词——你先学了别人就没得学。
// 记在本地就是「每台设备各自一份计划」，代价是换设备 / 清缓存会重置（等有登录再迁走）。
var PLAN_KEY = 'reviewDailyPlan';

// 每天的新词 / 抽查配额
var DAILY_NEW_TARGET = 5;
var DAILY_PROBE_TARGET = 5;

// 抽查冷却期（天）：同一个词在这段时间内不会被再次抽中，
// 否则「每天都抽到期最远的那几个」会变成新的饥饿
var PROBE_COOLDOWN_DAYS = 7;

// 取数批量：复习区每次取多少张、新词 / 抽查候选取多少
// （候选多取一些，随机抽与冷却过滤才有挑选余地）
var QUEUE_PAGE_SIZE = 100;
var NEW_CANDIDATE_SIZE = 20;
var PROBE_CANDIDATE_SIZE = 20;

// 本轮还剩这么多张没复习时，就去把复习区的下一页取回来（见 maybePrefetch）
var PREFETCH_MARGIN = 5;

// 「本轮已过一遍」提示在左下角停留多久（毫秒）
var ROUND_FLASH_MS = 2600;

// 引擎模块路径（相对本文件所在目录解析）
var WASM_MODULE_URL = './engine/pkg/guangxue_wasm.js';

// 换卡过渡 / 计划文案切换的时长（毫秒）：必须与 english.css 的 studyOut、planOut 保持一致
var TRANSITION_MS = 150;
var PLAN_SWAP_OUT_MS = 160;

// 「起始页 → 复习界面」这段过场的节拍（毫秒）。
// ⚠️ 三处必须对齐，改一处就得改三处：
//   · 起始页退场 / 复习界面入场 → english.css 的 startOut / startIn
//   · 导航栏上滑 + 内容区上移 120px → main.css 里 .rectangle 与 .content-container 的 transition
// START_TOTAL_MS 取 460ms，与「点头像进个人中心」那段过场齐平（全站最长的转场就是它）。
var START_OUT_MS = 160;               // 起始页淡出上移
var START_IN_MS = 300;                // 复习界面淡入下浮（从换幕那一刻开始算）
var START_TOTAL_MS = START_OUT_MS + START_IN_MS;

// 过场的两个定时器：换幕（藏起始页 / 放复习界面）与整段收尾
var startSwapTimer = null;
var startDoneTimer = null;

// 键盘监听是否已绑定（同一页面内反复切换学科只绑定一次）
var keysBound = false;

// 页面「世代」：每次进英语页 / 每次离开都 +1。
// 用途：beginReview 那条异步链（加载引擎 + 四个接口）在慢网下要一两秒，回来时用户
// 完全可能已经切到别的学科了 —— 那时候 enterReview() 会把 is-immersive 挂到**别人家的
// body** 上（数学页的导航栏凭空消失），还会念一个词。回调进门先对一下号，对不上就整条丢掉。
// 注意必须在**建会话之前**就拦掉，否则 ReviewSession 已经创建，WASM 内存就漏了。
var pageEpoch = 0;

// 语音是否已解锁（浏览器会拦截未经用户交互的 speechSynthesis）
var speechUnlocked = false;
var unlockBound = false;

// 词性缩写 → 界面上显示的大写标签（多词性用 " / " 连接）
var POS_LABELS = {
    'n.': 'NOUN', 'v.': 'VERB', 'adj.': 'ADJ', 'adv.': 'ADV', 'prep.': 'PREP',
    'pron.': 'PRON', 'conj.': 'CONJ', 'num.': 'NUM', 'int.': 'INT', 'art.': 'ART',
    'vt.': 'VERB', 'vi.': 'VERB', 'aux.': 'AUX', 'abbr.': 'ABBR'
};

// ---------- 对外入口 ----------

// 初始化英语页。由 main.js 在英语页 HTML 注入完成后调用。
//
// 这一层只做「绑事件 + 决定停在起始页还是直接进复习」，**不碰引擎、不发任何请求**；
// 真正的启动在 beginReview()。顺序：释放上一次会话（引擎侧的 ReviewSession 必须显式 free，
// 否则反复进出英语页会泄漏 WASM 内存）→ 绑定监听（键盘与语音解锁都只绑一次）→
// 恢复自动朗读开关（默认开启）→ 停在起始页等用户点击。
export function initReviewApp() {
    var statusEl = document.getElementById('reviewStatus');
    if (!statusEl) return; // 当前页面不是英语页，跳过

    pageEpoch++; // 上一次留在半路上的异步链从此作废
    releaseSession();

    if (!keysBound) {
        keysBound = true;
        document.addEventListener('keydown', handleReviewKey);
    }

    var autoEl = document.getElementById('reviewAutoSpeak');
    if (autoEl) {
        autoEl.checked = isAutoSpeakOn();
        autoEl.addEventListener('change', function() {
            setAutoSpeak(this.checked);
        });
    }

    bindClick('studyNavToggle', function() { toggleImmersive(); });

    // 语音解锁要**在起始页就装上**：用户点「开始复习单词」的那一次 pointerdown
    // 正是浏览器认可的用户交互（那时 state.card 还是空的，所以补读分支不会响），
    // 之后建卡时的自动朗读才不会被拦。
    if (!unlockBound) {
        unlockBound = true;
        document.addEventListener('pointerdown', unlockSpeech);
        document.addEventListener('keydown', unlockSpeech);
    }

    bindClick('reviewRevealBtn', function() { revealAnswer(); });
    bindClick('reviewSpeakWordBtn', function() { speakCurrent('word'); });
    bindClick('reviewSpeakExampleBtn', function() { speakCurrent('example'); });

    // 评分按钮：学科页每次加载都会重建这些元素，直接逐个绑定即可
    var buttons = document.querySelectorAll('.rating');
    for (var k = 0; k < buttons.length; k++) {
        buttons[k].addEventListener('click', function() {
            this.blur(); // 主动失焦，避免回车键被按钮重复触发
            handleReviewRating(parseInt(this.getAttribute('data-rating'), 10));
        });
    }

    var startBtn = document.getElementById('studyStartBtn');
    if (!startBtn) {
        beginReview(); // 兜底：万一拿到的是旧结构的缓存页（没有起始页），直接进复习
        return;
    }

    // 起始页：刻意**不套用沉浸偏好**，站点导航栏留在原处，用户能正常切学科 / 点头像。
    // 这里也**不写 localStorage** —— 用户的沉浸偏好留到 beginReview 时再生效，别被这一下改掉。
    applyImmersive(false);
    startBtn.addEventListener('click', beginReview);
}

// 从起始页进入复习：按钮进入「准备中」→ 加载引擎、取数、建会话、渲染第一张卡 → 再播过场。
// 只允许走一次：state.started 兜一道（按钮同时会被 disabled）。
//
// ⚠️ 为什么建会话必须等到点击之后，而不是页面一挂载就建：
// renderCardNow 末尾会自动朗读单词（自动朗读默认开启），而浏览器不允许「还没有用户交互」的
// 语音合成 —— 首次朗读被静默丢弃，再由 unlockSpeech 在用户第一次点页面时补读一遍，
// 听感就是「进来响一次、随便点一下又响一次」。点了按钮之后才建卡，朗读就落在一次真实
// 交互之后：既不重复，也不会被拦。附带好处是路过英语页的人不用下载 WASM、不发请求。
//
// ⚠️ 为什么过场要等第一张卡建好才播（而不是点完立刻换屏）：
// 换屏后到卡片渲染之间，复习界面是「空单词 + 正在加载」——先换屏就会看到这么一拍空画面，
// 过场再顺也白搭。代价是按钮要顶几百毫秒的「准备中」，这是划得来的。
function beginReview() {
    if (state.started) return;
    state.started = true;

    // 记下这一路的「世代」：切走学科后 pageEpoch 会变，回调就对不上号了（见 pageEpoch 的注释）
    var epoch = pageEpoch;

    // 起始页先留在原地，只把按钮压成「准备中」（文案 + 变淡，见 english.css 的 :disabled）
    setPreparing(true);

    var plan = loadTodayPlan();
    state.planPhase = '';

    loadEngine().then(function(wasm) {
        if (epoch !== pageEpoch) return null; // 用户已经切走了，这一路整个丢掉
        state.wasm = wasm;
        return fetchDay(0);
    }).then(function(results) {
        // results 为 null = 上一步已经判定作废；再对一次号是防「取数过程中才切走」
        if (!results || epoch !== pageEpoch) return;

        // 页面已经被换成别的学科了：连会话都别建，免得白占一份 WASM 内存。
        // （enterReview 里还有同样一道闸，那道是给「失败回调迟到」这种情况用的）
        if (!document.getElementById('reviewApp')) return;

        var statsRes = results[3];

        // 顶部统计先拿到，渲染卡片时一起显示
        if (statsRes && statsRes.code === 200 && statsRes.data) {
            state.stats = statsRes.data;
        }

        // 队列编排（今日计划 + 复习区排序 + 日期换算）全部在引擎内完成：
        // 本文件只把接口返回的 JSON 原文递进去，不再自己解析与保存卡片数组。
        state.session = new state.wasm.ReviewSession(
            results[0], // 复习区：整库按紧迫度升序（含未到期）
            results[1], // 新词候选
            results[2], // 抽查候选（到期最远的）
            JSON.stringify(buildPlanOptions(plan))
        );

        state.queueOffset = parseQueueMeta(results[0]).count;
        state.queueTotal = parseQueueMeta(results[0]).total;
        state.planInfo = JSON.parse(state.session.plan_json());

        // ⚠️ 今日计划的分母在建会话时**只算一次**：今天此前已经做完的 + 本次队列里排着的。
        // 不能每次渲染时重算，否则分子涨一分母也跟着涨（会出现「新词 1/4、2/5」这种错）。
        state.planTotals = {
            new: plan.newDone + state.planInfo.new_target,
            probe: plan.probeDone + state.planInfo.probe_target
        };

        // 卡片是在起始页还盖着的时候渲染的（复习界面此时仍是 display:none），
        // 所以把这次的自动朗读压住，交给过场结束的 finishEnter 补 —— 否则单词会比画面先出声。
        state.holdAutoSpeak = true;
        renderCard();
        state.holdAutoSpeak = false;

        enterReview();
    }).catch(function(err) {
        if (epoch !== pageEpoch) return; // 用户已经切走了：别在别人家的页面上报错
        setStatus('');
        showMessage('复习功能加载失败', String(err) + '（需通过服务器访问，并确认已生成 WASM 引擎）');
        console.error('复习功能初始化失败:', err);
        // 失败也要把用户送进复习界面：卡在起始页的「准备中」上没有任何出路
        enterReview();
    });
}

// 「准备中」：按钮换文案 + 变淡，同时 disabled 挡掉重复点击
function setPreparing(on) {
    var btn = document.getElementById('studyStartBtn');
    if (!btn) return;
    btn.disabled = on;
    btn.textContent = on ? '正在准备…' : '开始复习单词';
}

// 起始页 → 复习界面的过场。
//
// 时序（总长 START_TOTAL_MS = 460ms，与「点头像进个人中心」那段齐平）：
//   0ms              套用沉浸偏好（要进沉浸的话，导航栏从这里开始上滑、内容区开始上移 120px，
//                    这两条过渡写在 main.css 里），同时给起始页挂 is-leaving 让它淡出上移
//   START_OUT_MS     换幕：藏起始页、放复习界面，给复习界面挂 is-entering 淡入下浮
//   START_TOTAL_MS   收尾：摘掉动画类、恢复按钮、补上被压住的首卡朗读
//
// ⚠️ 沉浸偏好是「用户存过就照办」：偏好是「不进沉浸」的话导航栏不动，这段过场自然退化成
// 单纯的淡出淡入 —— 别为了动画好看去覆盖用户的偏好（起始页那边同理，用的是 applyImmersive）。
function enterReview() {
    var app = document.getElementById('reviewApp');
    var start = document.getElementById('studyStart');

    // ⚠️ 复习界面都不在了 = 内容容器已经被换成别的学科了，**立刻收手**。
    // 这一条比 unmount() 的世代校验还早一步：main.js 是先 innerHTML 换页、再调 unmount()，
    // 中间有一个窗口，异步链正好在这个窗口里回来就会踩中。
    // 少了它，setImmersive 挂上去的就是别人家的 body —— 数学页的导航栏会凭空消失。
    if (!app) return;

    // 无障碍：系统开启「减少动态效果」时一步到位，业务逻辑完全一致
    if (!start || prefersReducedMotion()) {
        hide('studyStart');
        show('reviewApp');
        setImmersive(isImmersiveOn());
        finishEnter();
        return;
    }

    setImmersive(isImmersiveOn());
    start.classList.add('is-leaving');

    clearTimeout(startSwapTimer);
    clearTimeout(startDoneTimer);

    startSwapTimer = setTimeout(function() {
        startSwapTimer = null;
        hide('studyStart');
        start.classList.remove('is-leaving');
        show('reviewApp');
        app.classList.add('is-entering');
    }, START_OUT_MS);

    startDoneTimer = setTimeout(function() {
        startDoneTimer = null;
        app.classList.remove('is-entering');
        finishEnter();
    }, START_TOTAL_MS);
}

// 过场收尾：把起始页的按钮恢复原样（下次进英语页还要能点），并补上被压住的首次朗读。
// 朗读放在这一刻而不是建卡那一刻，是为了让单词和画面同时出现；
// 失败路径下 state.card 是空的，这里自然什么都不做。
function finishEnter() {
    setPreparing(false);
    if (isAutoSpeakOn() && state.card) speakCurrent('word');
}

// 离开英语页时的清理。由 main.js 在切换到其他学科之前调用。
// 为什么必须显式清理：沉浸模式把 is-immersive 类挂在了 document.body 上，
// 内容容器被换成别的学科后这个类不会自己消失，导航栏会跟着一起不见。
// 顺带释放引擎侧会话、清掉揭晓动效的定时器，避免反复进出英语页时内存与声音残留。
export function unmount() {
    document.body.classList.remove('is-immersive');

    // 世代 +1：beginReview 那条还在飞的异步链从此作废（否则它回来时会把 is-immersive
    // 挂到我们已经切过去的那个学科页上）。定时器与动画类在下面一并抹平。
    pageEpoch++;

    cancelRevealAnimation();
    clearTimeout(planSwapTimer);
    clearTimeout(roundFlashTimer);
    planSwapTimer = null;
    roundFlashTimer = null;

    // 过场还没播完就被切走学科：定时器必须清掉，否则它会在别的学科页上把
    // 复习界面「放出来」（那时 DOM 早换了，虽然取不到元素，但类会挂到新页面的同名元素上）。
    // 顺带把两边的动画类与按钮状态抹平，保证下次进英语页是干净的起始页。
    clearTimeout(startSwapTimer);
    clearTimeout(startDoneTimer);
    startSwapTimer = null;
    startDoneTimer = null;
    var startEl = document.getElementById('studyStart');
    if (startEl) startEl.classList.remove('is-leaving');
    var appEl = document.getElementById('reviewApp');
    if (appEl) appEl.classList.remove('is-entering');
    setPreparing(false);

    releaseSession();
    state.card = null;
    state.revealed = false;
    state.stats = null;
    state.pendingSubmit = null;
    state.refilling = false;
    state.planInfo = null;
    state.planTotals = null;
    state.planPhase = '';
    state.rounds = null;
    state.queueOffset = 0;
    state.queueTotal = 0;
    // 回到起始页待命：下次进英语页要重新点「开始复习单词」，不会直接续上这次的复习
    state.started = false;
    state.holdAutoSpeak = false;
    if (window.speechSynthesis) {
        try {
            window.speechSynthesis.cancel();
        } catch (e) {
            // 浏览器不支持时忽略
        }
    }
}

// 取今日计划所需的数据：复习区（整库紧迫度序）/ 新词候选 / 抽查候选 / 顶部统计
function fetchDay(queueOffset) {
    return Promise.all([
        fetchText('/api/reviews/queue?limit=' + QUEUE_PAGE_SIZE + '&offset=' + queueOffset),
        fetchText('/api/reviews/new?limit=' + NEW_CANDIDATE_SIZE),
        fetchText('/api/reviews/probes?limit=' + PROBE_CANDIDATE_SIZE),
        fetchJson('/api/reviews/stats')
    ]);
}

// 只取复习区的下一页（翻到底之后用），并顺手刷新顶部统计
function fetchQueuePage(offset) {
    return Promise.all([
        fetchText('/api/reviews/queue?limit=' + QUEUE_PAGE_SIZE + '&offset=' + offset),
        fetchJson('/api/reviews/stats')
    ]);
}

// 从复习区响应里读出「这一页多少张 / 一共多少张」
// 解析失败时按 0 处理：取数异常不该让整个复习页打不开
function parseQueueMeta(queueText) {
    try {
        var parsed = JSON.parse(queueText);
        var data = (parsed && parsed.data) || {};
        var items = data.items || [];
        return { count: items.length, total: data.total || items.length };
    } catch (e) {
        return { count: 0, total: 0 };
    }
}

// ---------- 今日计划（每天 5 个新词 + 5 个抽查） ----------
//
// 配额与抽查冷却记录都放在浏览器 localStorage 里（原因见 PLAN_KEY 的注释）：
// 引擎只管「按我给的剩余额度编排队列」，额度的账在这里算。

// 本地日期键（按浏览器本地自然日，跨零点即新的一天）
function todayKey() {
    var d = new Date();
    var m = d.getMonth() + 1;
    var day = d.getDate();
    return d.getFullYear() + '-' + (m < 10 ? '0' + m : m) + '-' + (day < 10 ? '0' + day : day);
}

// 读出今日计划状态；日期变了就重置计数（抽查冷却记录保留，并按冷却期清理）
function loadTodayPlan() {
    var today = todayKey();
    var raw = null;
    try {
        raw = localStorage.getItem(PLAN_KEY);
    } catch (e) {
        raw = null;
    }

    var plan = null;
    if (raw) {
        try {
            plan = JSON.parse(raw);
        } catch (e) {
            plan = null;
        }
    }
    if (!plan || typeof plan !== 'object') plan = {};
    if (!plan.probedAt || typeof plan.probedAt !== 'object') plan.probedAt = {};

    if (plan.date !== today) {
        plan.date = today;
        plan.newDone = 0;
        plan.probeDone = 0;
    }
    if (typeof plan.newDone !== 'number' || plan.newDone < 0) plan.newDone = 0;
    if (typeof plan.probeDone !== 'number' || plan.probeDone < 0) plan.probeDone = 0;

    pruneProbedAt(plan);
    saveTodayPlan(plan);
    return plan;
}

// 丢掉超过冷却期的抽查记录（否则这个对象会无限长大）
function pruneProbedAt(plan) {
    var limit = Date.now() - PROBE_COOLDOWN_DAYS * 86400000;
    var kept = {};
    var keys = Object.keys(plan.probedAt || {});
    for (var i = 0; i < keys.length; i++) {
        var ts = Number(plan.probedAt[keys[i]]);
        if (isFinite(ts) && ts >= limit) kept[keys[i]] = ts;
    }
    plan.probedAt = kept;
}

function saveTodayPlan(plan) {
    try {
        localStorage.setItem(PLAN_KEY, JSON.stringify(plan));
    } catch (e) {
        console.error('保存今日计划失败:', e);
    }
}

// 今天还剩多少额度（0 表示今天不再放这一类卡片）
function remainingQuota(plan, kind) {
    var target = kind === 'probe' ? DAILY_PROBE_TARGET : DAILY_NEW_TARGET;
    var done = kind === 'probe' ? plan.probeDone : plan.newDone;
    return Math.max(0, target - done);
}

// 冷却期内抽过的词 id 列表：交给引擎跳过，避免连续几天抽到同一批
function probedIdsInCooldown(plan) {
    var ids = [];
    var keys = Object.keys(plan.probedAt || {});
    for (var i = 0; i < keys.length; i++) {
        var id = parseInt(keys[i], 10);
        if (isFinite(id) && id > 0) ids.push(id);
    }
    return ids;
}

// 组装给引擎的计划参数：两个 limit 都是「今天还剩多少」，不是每日上限本身
function buildPlanOptions(plan) {
    return {
        new_limit: remainingQuota(plan, 'new'),
        probe_limit: remainingQuota(plan, 'probe'),
        probed_ids: probedIdsInCooldown(plan),
        now_ms: Date.now()
    };
}

// 记一次计划完成（评分成功后调用）。
// ⚠️ 这里**不刷界面**：评分紧接着就是换卡过渡（150ms），界面刷新交给换卡时的 renderPlanProgress。
// 若在这里先刷一次，切换动画刚起步就会被换卡那次渲染掐断（踩过，表现为「动画没播」）。
function markPlanDone(kind, wordId) {
    var plan = loadTodayPlan();
    if (kind === 'probe') {
        plan.probeDone += 1;
        if (wordId) plan.probedAt[String(wordId)] = Date.now();
    } else {
        plan.newDone += 1;
    }
    saveTodayPlan(plan);
}

// 释放引擎侧的会话对象（wasm-bindgen 为导出结构体生成的 free()）
function releaseSession() {
    if (state.session) {
        try {
            state.session.free();
        } catch (e) {
            console.error('释放复习会话失败:', e);
        }
        state.session = null;
    }
}

// ---------- 引擎加载与取数 ----------

// 动态加载 WASM 引擎模块（浏览器原生 import()）。两个关键点：
// 1. 相对路径必须写成 './xxx'：'engine/pkg/...' 会被当作 bare specifier（npm 包名）而解析失败；
// 2. wasm-bindgen 的 --target web 产物必须先 await 默认导出（__wbg_init）完成实例化，
//    否则模块内部变量 wasm 仍是 undefined，调用任何导出函数都会抛 TypeError。
function loadEngine() {
    return import(WASM_MODULE_URL).then(function(mod) {
        return mod.default().then(function() {
            return mod;
        });
    });
}

// 复习接口在阶段 2 之后全部要登录：Cookie 缺失 / 过期时后端统一回 401。
// 这里做「401 → 去登录页」的最小处理：只跳转并提示一句，不改任何动画。
// 用一次性的标记位避免并发请求各跳一次（首屏会同时发 4 个请求）。
var authRedirecting = false;
function redirectToLogin() {
    if (authRedirecting) return;
    authRedirecting = true;
    setStatus('登录状态已失效，正在前往登录页…');
    // next 用当前页地址，登录后 account/account.js 的 nextUrl() 会把用户送回来；
    // 它只接受站内相对路径，所以这里传 pathname + search（不带协议与域名）。
    var next = location.pathname + location.search;
    location.href = 'account/?next=' + encodeURIComponent(next);
}

// 统一的响应检查：401 说明没登录（或登录已过期），引到登录页；其余非 2xx 交给调用方按原逻辑报错。
function ensureOk(res, url) {
    if (res.status === 401) {
        redirectToLogin();
        throw new Error(url + ' 需要登录：HTTP 401');
    }
    if (!res.ok) {
        throw new Error(url + ' 请求失败：HTTP ' + res.status);
    }
    return res;
}

// 取回响应原文（不做 JSON 解析，交给引擎处理）
function fetchText(url) {
    return fetch(url).then(function(res) {
        return ensureOk(res, url).text();
    });
}

function fetchJson(url) {
    return fetch(url).then(function(res) {
        return ensureOk(res, url).json();
    });
}

// ---------- 渲染：卡片 ----------

// 首屏渲染（不做过渡，避免刚进页面就闪一下）
function renderCard() {
    renderCardNow();
}

// 评分后切换卡片：先上锁 → 播离场动画 → 更新内容 → 播入场动画。
// ⚠️ 上锁必须发生在过渡**开始前**：过渡期间旧卡的答案与评分按钮还在屏幕上，
// 若用户连点或按住数字键，第二次评分会落到下一张卡上（引擎游标已经前进）。
function advanceCard() {
    state.revealed = false; // 评分守卫立刻失效
    state.card = null;      // 键盘处理器靠 !state.card 直接 return
    hide('reviewButtons');
    withCardTransition(function() {
        renderCardNow();
    });
}

// 换卡过渡：给舞台加离场 / 入场动画类；系统开启「减少动态效果」时直接跳过动画（业务逻辑完全一致）
function withCardTransition(update) {
    var stage = document.getElementById('studyStage');
    if (!stage || prefersReducedMotion()) {
        update();
        return;
    }
    stage.classList.remove('is-in');
    stage.classList.add('is-out');
    setTimeout(function() {
        update();
        stage.classList.remove('is-out');
        void stage.offsetWidth; // 强制重排，保证入场动画能重新播放
        stage.classList.add('is-in');
        setTimeout(function() {
            stage.classList.remove('is-in');
        }, 260);
    }, TRANSITION_MS);
}

// 是否应当减少动态效果（无障碍）
function prefersReducedMotion() {
    return !!(window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches);
}

// 把当前卡片画到界面上（幂等；不负责过渡动画）。
// 换卡即重置：新卡一律先藏答案与评分按钮，并掐掉上一张卡可能还在收尾的揭晓动效。
// ⚠️ 计划小字要在「队列走完」这条分支**之前**刷新：最后一张计划卡评完时队列可能同时见底，
// 否则小字会停在旧文案上不再切换。
function renderCardNow() {
    var session = state.session;
    if (!session) return;

    state.revealed = false;
    state.card = null;
    cancelRevealAnimation();
    hide('reviewAnswer');
    hide('reviewButtons');
    hide('reviewReveal');

    renderStats(state.stats);
    renderPlanProgress();

    if (session.is_finished()) {
        hideCardBody();
        setStatus('');
        showFinishMessage();
        maybePrefetch(); // 池子是空的：看看还有没有下一页可取，取到了就能继续
        return;
    }

    // 一次取回当前卡片的全部展示数据（含词性拆分、例句切分与记忆元信息）
    state.card = JSON.parse(session.current_json(Date.now()));

    hideMessage();
    setText('reviewWord', state.card.word || '');
    setText('reviewPhonetic', state.card.phonetic || '');
    show('reviewWord');
    // 没有音标时整行藏起来，避免留个空胶囊
    if (state.card.phonetic) {
        show('reviewPhoneticRow');
    } else {
        hide('reviewPhoneticRow');
    }

    renderMeta(state.card.meta);
    renderAnswer(state.card);
    renderPlanProgress();
    checkRounds(session.rounds());

    // 无限复习：评完的卡会按新到期时间插回池子，所以不再有「剩余张数」的概念，
    // 这里显示的是本轮已经复习了多少张（越往下翻越大）
    var sourceLabel = sourceLabelOf(state.card.source);
    setStatus('本轮已复习 ' + session.done() + ' 张 · ' + sourceLabel);
    show('reviewReveal');

    maybePrefetch();

    // holdAutoSpeak：首卡是在起始页还盖着的时候建的，朗读要压到过场结束（见 finishEnter）
    if (isAutoSpeakOn() && !state.holdAutoSpeak) speakCurrent('word');
}

// 卡片来源 → 界面文案
function sourceLabelOf(source) {
    if (source === 'new') return '新词';
    if (source === 'probe') return '抽查';
    return '复习';
}

// 结束文案：池子空了才会出现（词库没词、或接口什么都没给）
function showFinishMessage() {
    var session = state.session;
    var stats = state.stats || {};
    var learned = session ? session.done() : 0;

    if (learned > 0) {
        showMessage('今天复习完成 ✨', '本轮共学 ' + learned + ' 张');
    } else if ((stats.total_words || 0) === 0) {
        showMessage('词库还是空的', '可以到后台的「批量导入」粘贴词表添加词条');
    } else {
        showMessage('暂时没有可复习的词', '稍后再来 👋');
    }
}

// 本轮快过完时，预先取复习区的下一页塞进池子。
//
// 为什么不是「池子空了才取」：评完的卡会按新的到期时间插回池子，池子永远不会空，
// 所以「空了再取」的分页条件再也触发不了——词库有几千词时会永远只在前一页里打转。
// 改成看**本轮的进度**：本轮已复习的卡数逼近池子总量时，提前把下一页取回来。
function maybePrefetch() {
    if (state.refilling || !state.session) return;
    if (state.queueOffset >= state.queueTotal) return; // 整库都取回来了

    // universe = 池中待抽 + 手上这一张；rated = 本轮已评分数
    var universe = state.session.pending_count() + 1;
    var rated = state.session.seen_count();
    if (universe - rated > PREFETCH_MARGIN) return;

    state.refilling = true;

    // 必须等上一次提交落库：否则刚评过的词在服务端 due_at 还没更新，
    // 取回来的那一页可能又把它当成「早就到期」的卡
    var wait = state.pendingSubmit || Promise.resolve();

    wait.then(function() {
        return fetchQueuePage(state.queueOffset);
    }).then(function(results) {
        state.refilling = false;

        var statsRes = results[1];
        if (statsRes && statsRes.code === 200 && statsRes.data) {
            state.stats = statsRes.data;
        }

        var meta = parseQueueMeta(results[0]);
        var added = 0;
        try {
            added = state.session.append(results[0], JSON.stringify(buildPlanOptions(loadTodayPlan())));
        } catch (e) {
            setStatus('取下一页失败：' + e);
            console.error('取下一页失败:', e);
            return;
        }
        state.queueOffset += meta.count;

        // 追加只是把池子变大，当前这张卡不变，所以不需要重绘；
        // 本轮因此自然延长，可以一直复习下去
        if (added === 0) {
            // 这一页全是池子里已有的卡（例如刚评完又插回去的）：跳过它继续往后取
            if (state.queueOffset < state.queueTotal && meta.count > 0) {
                maybePrefetch();
            }
        }
    }).catch(function(err) {
        state.refilling = false;
        console.error('取下一页失败:', err);
    });
}

// 顶栏三个数字：今日新学 / 今日复习 / 剩余待学
function renderStats(stats) {
    if (!stats) return;
    setText('statTodayNew', stats.today_new || 0);
    setText('statTodayReview', stats.today_review || 0);
    setText('statRest', stats.new_words || 0);
}

// ---------- 左下角：今日计划进度 ----------

// 计划文案的切换定时器（换卡 / 离开页面时要清掉）
var planSwapTimer = null;
// 「本轮已过一遍」提示的定时器
var roundFlashTimer = null;

// 画出左下角小字。
// 两个阶段：
//   plan   —— 今日计划还没走完：「新词 3/5 · 抽查 2/5」
//   review —— 计划区的卡全部评完了：「计划完成 · 进入复习阶段」（带动效切换）
// 阶段切换只在真的跨过去时播一次动画。
function renderPlanProgress(forceAnimate) {
    var session = state.session;
    var info = state.planInfo;
    if (!session || !info) return;

    var done = session.done();
    var phase = done >= info.plan_len ? 'review' : 'plan';
    var text;

    if (phase === 'review') {
        text = '计划完成 · 进入复习阶段';
    } else {
        var plan = loadTodayPlan();
        var totals = state.planTotals || { new: 0, probe: 0 };
        var parts = [];
        if (totals.new > 0) parts.push('新词 ' + plan.newDone + '/' + totals.new);
        if (totals.probe > 0) parts.push('抽查 ' + plan.probeDone + '/' + totals.probe);
        // 两类都没有（例如词库空了）：小字不显示，保持界面干净
        text = parts.join(' · ');
    }

    setPlanText(text, !!forceAnimate || (phase !== state.planPhase && state.planPhase !== ''));
    state.planPhase = phase;
}

// 过完一整轮时的轻提示：左下角小字闪一句「本轮已过一遍」，过一会儿自动切回计划文案
function checkRounds(rounds) {
    // 首次进入只记基线，不提示
    if (state.rounds === null || state.rounds === undefined) {
        state.rounds = rounds;
        return;
    }
    if (rounds <= state.rounds) return;
    state.rounds = rounds;

    setPlanText('本轮已过一遍 · 可以继续', true);
    clearTimeout(roundFlashTimer);
    roundFlashTimer = setTimeout(function() {
        roundFlashTimer = null;
        renderPlanProgress(true); // 切回计划文案（也带动效）
    }, ROUND_FLASH_MS);
}

// 设置左下角文案：阶段变化时先淡出旧文案、再浮入新文案（见 english.css 的 planOut / planIn）。
// ⚠️ 文案没变时**直接返回、不要顺手清动画类**：切换动画播到一半时如果有一次多余的渲染
// （换卡、统计刷新等）进来，就会把动画掐断，看上去像「没播」（踩过）。
function setPlanText(text, animate) {
    var el = document.getElementById('studyPlanText');
    if (!el) return;

    if (el.textContent === text) return;

    clearTimeout(planSwapTimer);
    planSwapTimer = null;

    if (!animate) {
        el.textContent = text;
        el.classList.remove('is-out', 'is-in');
        return;
    }

    el.classList.remove('is-in');
    void el.offsetWidth;
    el.classList.add('is-out');
    planSwapTimer = setTimeout(function() {
        el.textContent = text;
        el.classList.remove('is-out');
        void el.offsetWidth; // 强制重排，保证入场动画能重新播放
        el.classList.add('is-in');
        planSwapTimer = setTimeout(function() {
            el.classList.remove('is-in');
            planSwapTimer = null;
        }, 260);
    }, PLAN_SWAP_OUT_MS);
}

// 右上角记忆元信息：难度 / 稳定性 / 状态 / 复习次数 / 上次 / 预计记住
function renderMeta(meta) {
    var box = document.getElementById('studyMeta');
    if (!box) return;
    box.textContent = '';
    if (!meta) return;

    function line(label, value) {
        var row = document.createElement('span');
        row.appendChild(document.createTextNode(label + ' '));
        var em = document.createElement('em');
        em.textContent = value;
        row.appendChild(em);
        box.appendChild(row);
    }

    if (meta.status === 'new') {
        line('状态', '新词');
        return;
    }
    if (typeof meta.difficulty === 'number') line('难度', formatNumber(meta.difficulty, 1));
    if (typeof meta.stability === 'number') line('稳定性', formatNumber(meta.stability, 1) + ' 天');
    // 抽查：这个词本来是几十天后才轮到，被提前抽出来「体检」的
    line('状态', meta.status === 'probe' ? '抽查' : '复习');
    if (typeof meta.reps === 'number') line('复习', meta.reps + ' 次');
    if (typeof meta.days_since_last === 'number') line('上次', meta.days_since_last + ' 天');
    if (typeof meta.retrievability === 'number') {
        line('预计记住', Math.round(meta.retrievability * 100) + '%');
    }
}

// 揭晓区：主例句（目标词高亮，配中文翻译）+ 一条条释义块。
// 引擎已把例句切成「命中 / 未命中」片段、也把「一个词性一块」的释义拆好了
// （词条填了多释义就用它，没填则按词性标签自动拆），这里只负责拼成 DOM。
// 词条没有主例句时不渲染引文块——多释义各自带例句的情况很常见，不能留个空壳。
function renderAnswer(card) {
    var quote = document.getElementById('reviewExample');
    var parts = card.example_parts || [];
    if (quote) {
        quote.textContent = '';
        if (parts.length === 0) {
            hide('reviewExample');
        } else {
            quote.appendChild(document.createTextNode('“'));
            for (var i = 0; i < parts.length; i++) {
                if (parts[i].hit) {
                    var mark = document.createElement('mark');
                    mark.className = 'study-hit';
                    mark.textContent = parts[i].text;
                    quote.appendChild(mark);
                } else {
                    quote.appendChild(document.createTextNode(parts[i].text));
                }
            }
            quote.appendChild(document.createTextNode('”'));
            show('reviewExample');
        }
    }

    var quoteCn = document.getElementById('reviewExampleCn');
    if (quoteCn) {
        if (card.example_translation && parts.length > 0) {
            quoteCn.textContent = card.example_translation;
            show('reviewExampleCn');
        } else {
            quoteCn.textContent = '';
            hide('reviewExampleCn');
        }
    }

    var sensesBox = document.getElementById('reviewSenses');
    if (!sensesBox) return;
    sensesBox.textContent = '';

    var senses = card.senses || [];
    for (var s = 0; s < senses.length; s++) {
        sensesBox.appendChild(renderSense(senses[s]));
    }
}

// 画一块释义：词性标签 + 释义正文，若这块带自己的例句则再画例句与译文
function renderSense(sense) {
    var block = document.createElement('div');
    block.className = 'sense-block';

    var head = document.createElement('div');
    head.className = 'sense-head';

    var posText = formatPos(sense.pos);
    if (posText) {
        var posEl = document.createElement('span');
        posEl.className = 'sense-pos';
        posEl.textContent = posText;
        head.appendChild(posEl);
    }

    var meaningEl = document.createElement('span');
    meaningEl.className = 'sense-meaning';
    meaningEl.textContent = sense.meaning || '';
    head.appendChild(meaningEl);
    block.appendChild(head);

    // 该释义专属例句：缩进对齐到释义正文那一列（缩进在 english.css 的 .sense-example 里）
    var parts = sense.example_parts || [];
    if (parts.length > 0) {
        var ex = document.createElement('p');
        ex.className = 'sense-example';
        appendHighlighted(ex, parts);
        block.appendChild(ex);
    }
    if (sense.translation) {
        var cn = document.createElement('p');
        cn.className = 'sense-translation';
        cn.textContent = sense.translation;
        block.appendChild(cn);
    }
    return block;
}

// 把引擎切好的片段拼进元素：命中片段用 <mark> 包起来
function appendHighlighted(target, parts) {
    for (var i = 0; i < parts.length; i++) {
        if (parts[i].hit) {
            var mark = document.createElement('mark');
            mark.className = 'study-hit';
            mark.textContent = parts[i].text;
            target.appendChild(mark);
        } else {
            target.appendChild(document.createTextNode(parts[i].text));
        }
    }
}

// 把 "n./v." 这类词性缩写转成界面上的大写标签
function formatPos(pos) {
    if (!pos) return '';
    return String(pos).split('/').map(function(one) {
        var key = one.trim().toLowerCase();
        if (!key) return '';
        if (key.indexOf('.') < 0) key += '.';
        return POS_LABELS[key] || key.toUpperCase();
    }).filter(function(s) { return s; }).join(' / ');
}

// 数字格式化：固定小数位（引擎给的是 f32，直接显示会出现一长串小数）
function formatNumber(value, digits) {
    var n = Number(value);
    if (!isFinite(n)) return '—';
    return n.toFixed(digits);
}

// ---------- 揭晓与评分 ----------

// 揭晓答案：显示例句与释义，并放出评分按钮。
// 只有揭晓后才允许评分——「看着答案打分」会让 FSRS 的记忆状态失真。
// 揭晓按钮的退场与评分按钮的入场都挂在 playRevealAnimation 的时间线上，所以这里不再直接 hide/show 那两个块。
function revealAnswer() {
    if (state.revealed) return;
    if (!state.session || state.session.is_finished()) return;
    state.revealed = true;

    show('reviewAnswer');
    playRevealAnimation();

    var sourceLabel = sourceLabelOf(state.card ? state.card.source : '');
    setStatus('已学 ' + state.session.done() + ' 张 · ' + sourceLabel + ' · 请根据回忆情况评分');
}

// ---------- 揭晓动效 ----------
//
// 一眼看完的「整块淡入」很死板，这里排了一个节拍：
//   揭晓按钮缩小淡出 ─┐
//   主例句淡入上浮 ───┤
//   译文跟上 ─────────┼─→ 揭晓按钮消失的同一刻，四个评分按钮从它原来的位置依次顶上来
//   一条条释义块浮现 ─┘   （每块里的目标词高亮再晚一点扫过去，像荧光笔划过去）
//
// 「动画定义」与「延时」分离：什么时候开始由这里的常量决定（好调），
// 动画怎么动（时长 / 缓动 / 起止状态）写在 english.css 里，两边只靠这几个常量对齐。
// ⚠️ 单词本身**不参与任何动画**：揭晓时它必须待在原位不动（之前整体居中导致单词被顶上去，已改掉）。

// 释义块之间的错开间隔
var REVEAL_STEP_MS = 60;
// 首块释义之前留一点时间，让主例句先浮现
var REVEAL_HEAD_MS = 110;
// 译文比主例句晚一点
var REVEAL_CN_MS = 60;
// 高亮比它所在的那一块再晚一点（例句先出来，荧光笔才扫过去）
var REVEAL_HIT_MS = 100;
// 「显示答案」按钮退场时长。**换人的时刻就是它**：退场一播完立刻藏掉它、放出评分按钮，中间不留空档也不重叠
var REVEAL_OUT_MS = 170;
// 评分按钮：相对「被放出来」那一刻的起步延后，以及它们之间的错开间隔
var REVEAL_BTN_BASE_MS = 30;
var REVEAL_BTN_STEP_MS = 45;
// 文本块动画时长（与 english.css 的 revealUp / hitSweep 保持一致）
var REVEAL_DURATION_MS = 300;
// 评分按钮动画时长（与 english.css 的 ratingIn 保持一致）
var REVEAL_RATING_MS = 320;

// 两个定时器：换人（藏揭晓按钮 / 放评分按钮）与整轮收尾
// 换卡或离开页面时必须都清掉，否则会把下一张卡的动效提前掐断
var revealSwapTimer = null;
var revealTimer = null;

// 给元素设动画延时（内联样式，优先级高于样式表里的默认值）
function setRevealDelay(el, ms) {
    if (el) el.style.animationDelay = ms + 'ms';
}

// 播放揭晓动效：给各元素排好延时，再给根节点挂 revealing 类触发动画；返回这一轮动效的总时长（毫秒）
function playRevealAnimation() {
    var root = document.getElementById('reviewApp');
    if (!root) return 0;

    // 系统开启「减少动态效果」：不放动画，直接一步到位换人
    if (prefersReducedMotion()) {
        cancelRevealAnimation();
        hide('reviewReveal');
        show('reviewButtons');
        return 0;
    }

    setRevealDelay(document.getElementById('reviewExample'), 0);
    setRevealDelay(document.getElementById('reviewExampleCn'), REVEAL_CN_MS);

    var i, k;

    var quoteHits = document.querySelectorAll('#reviewExample .study-hit');
    for (k = 0; k < quoteHits.length; k++) {
        setRevealDelay(quoteHits[k], REVEAL_HIT_MS);
    }

    // 一条条释义块，每块里自己的高亮再晚一点
    var blocks = document.querySelectorAll('#reviewSenses .sense-block');
    for (i = 0; i < blocks.length; i++) {
        var delay = REVEAL_HEAD_MS + i * REVEAL_STEP_MS;
        setRevealDelay(blocks[i], delay);
        var hits = blocks[i].querySelectorAll('.study-hit');
        for (k = 0; k < hits.length; k++) {
            setRevealDelay(hits[k], delay + REVEAL_HIT_MS);
        }
    }

    // 揭晓按钮退场：延时留 0，跟着 revealing 类立刻开始缩小淡出
    setRevealDelay(document.getElementById('reviewRevealBtn'), 0);

    // 评分按钮的延时是相对「被放出来的那一刻」算的（它们在 REVEAL_OUT_MS 之后才 show），
    // 所以起步延后从 0 附近开始，才是紧接着揭晓按钮的退场
    var buttons = document.querySelectorAll('.rating');
    for (i = 0; i < buttons.length; i++) {
        setRevealDelay(buttons[i], REVEAL_BTN_BASE_MS + i * REVEAL_BTN_STEP_MS);
    }

    // 整轮时长取两条线里更晚结束的那条：文本块那条，和「退场 → 换人 → 按钮入场」那条
    var textEnd = REVEAL_HEAD_MS + blocks.length * REVEAL_STEP_MS + REVEAL_DURATION_MS;
    var buttonEnd = REVEAL_OUT_MS + REVEAL_BTN_BASE_MS + buttons.length * REVEAL_BTN_STEP_MS + REVEAL_RATING_MS;
    var total = Math.max(textEnd, buttonEnd);

    // 先摘类再挂：强制一次重排，保证同一张卡重播（或换卡后又回来）时动画能重新开始
    root.classList.remove('revealing');
    void root.offsetWidth;
    root.classList.add('revealing');

    clearTimeout(revealSwapTimer);
    clearTimeout(revealTimer);

    // 揭晓按钮退场动画播完的同一刻换人。
    // ⚠️ 顺序铁律：必须先藏掉揭晓按钮再放评分按钮。两者是底部操作区里相邻的两个块，
    // 同时显示会让操作区变高、把上面的单词顶上去（那正是之前修掉的毛病）。
    revealSwapTimer = setTimeout(function() {
        revealSwapTimer = null;
        hide('reviewReveal');
        show('reviewButtons');
    }, REVEAL_OUT_MS);

    revealTimer = setTimeout(function() {
        root.classList.remove('revealing');
        revealTimer = null;
    }, total);

    return total;
}

// 取消揭晓动效：换卡时调用，避免上一张卡的动画与定时器残留到新卡上
function cancelRevealAnimation() {
    clearTimeout(revealSwapTimer);
    clearTimeout(revealTimer);
    revealSwapTimer = null;
    revealTimer = null;
    var root = document.getElementById('reviewApp');
    if (root) root.classList.remove('revealing');
}

// 用户点击评分：交给引擎算新记忆状态 → 引擎返回可直接提交的请求体 → POST 持久化。
// 前置条件（铁律）：必须已揭晓答案，揭晓前的评分一律忽略——
// 既是避免误触键盘泄漏答案，也是避免「凭猜测打分」污染记忆状态。
// 提交本身不 await：先换卡保证手感，Promise 记进 state.pendingSubmit 供翻页前等待。
function handleReviewRating(rating) {
    var session = state.session;
    if (!session || session.is_finished()) return;
    if (!state.revealed) {
        setStatus('请先按空格（或点「显示答案」）揭晓答案，再评分');
        return;
    }

    // 引擎内部完成：换算距上次复习天数 → FSRS 计算 → 取对应分支 → 推进游标；
    // 抽查卡在引擎里按「新卡」重算（返回体里带 is_probe 标记）
    var cardSource = state.card ? state.card.source : '';
    var payload;
    try {
        payload = session.rate(rating, Date.now());
    } catch (e) {
        setStatus('引擎计算失败: ' + e);
        return;
    }

    // 算一次今日计划的账：新词 / 抽查各自用掉一个额度，抽查还要记下冷却时间
    var body = null;
    try {
        body = JSON.parse(payload);
    } catch (e) {
        body = null;
    }
    if (cardSource === 'new') {
        markPlanDone('new');
    } else if (cardSource === 'probe' || (body && body.is_probe)) {
        markPlanDone('probe', body ? body.word_id : 0);
    }

    advanceCard(); // 带动效推进到下一张（内部会先上锁，防止过渡期间重复评分）

    // 记下这次提交：翻页前要等它落库，否则刚评过的卡会被当成到期卡再抽一次
    state.pendingSubmit = fetch('/api/reviews/submit', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: payload // 引擎返回的 JSON 即请求体，无需在前端重新拼装
    }).then(function(res) {
        if (res.status === 401) {
            // Cookie 在复习中途失效（过期 / 被清掉）：提示并去登录，别无提示地停在原地
            redirectToLogin();
            return null;
        }
        return res.json();
    }).then(function(data) {
        if (!data) return;
        if (data && data.code === 200) {
            loadReviewStats();
        } else {
            console.error('提交复习失败:', data);
        }
    }).catch(function(err) {
        console.error('提交复习失败:', err);
    });
}

// 刷新统计（每次提交复习成功后调用）
function loadReviewStats() {
    return fetchJson('/api/reviews/stats')
        .then(function(data) {
            if (data && data.code === 200 && data.data) {
                state.stats = data.data;
                renderStats(data.data);
            }
            return data;
        })
        .catch(function(err) {
            console.error('加载复习统计失败:', err);
            return null;
        });
}

// ---------- 语音合成 ----------

// 读取「自动朗读」开关（持久化在 localStorage）。
// **默认开启**：只有用户显式关掉过（存了 '0'）才是关闭状态。
function isAutoSpeakOn() {
    try {
        var saved = localStorage.getItem(AUTO_SPEAK_KEY);
        return saved === null ? true : saved === '1';
    } catch (e) {
        return true; // 读不到 localStorage 时也按默认开启处理
    }
}

// 首次用户交互后解锁语音：浏览器会拦截未经交互的 speechSynthesis，
// 因此首屏那次自动朗读可能被静默丢弃，这里补读一次当前单词
function unlockSpeech() {
    if (speechUnlocked) return;
    speechUnlocked = true;
    document.removeEventListener('pointerdown', unlockSpeech);
    document.removeEventListener('keydown', unlockSpeech);
    if (isAutoSpeakOn() && state.card) {
        speakCurrent('word');
    }
}

// 保存「自动朗读」开关（'1' / '0'，见 isAutoSpeakOn 的默认值约定）
function setAutoSpeak(on) {
    try {
        localStorage.setItem(AUTO_SPEAK_KEY, on ? '1' : '0');
    } catch (e) {
        console.error('保存自动朗读设置失败:', e);
    }
}

// ---------- 沉浸模式（隐藏站点导航栏） ----------

// 是否开启沉浸模式；没有记录时默认开启
function isImmersiveOn() {
    var raw = null;
    try {
        raw = localStorage.getItem(IMMERSIVE_KEY);
    } catch (e) {
        raw = null; // 隐私模式下读不到，按默认值走
    }
    if (raw === null || raw === undefined || raw === '') return true;
    return raw !== '0';
}

// 应用沉浸模式：给 body 挂 is-immersive 类，站点级样式（main.css）据此隐藏导航栏并让内容区占满整屏。
// 这里只负责挂/摘类与更新按钮文案，样式一律写在 CSS 里；**顺带把偏好存下来**。
function setImmersive(on) {
    applyImmersive(on);
    try {
        localStorage.setItem(IMMERSIVE_KEY, on ? '1' : '0');
    } catch (e) {
        // 存不下也不影响本次使用
    }
}

// 只改界面、**不写偏好**。
// 起始页用它强制显示导航栏（起始页刻意不进沉浸），但那不代表用户想把沉浸偏好关掉 ——
// 要是这里用 setImmersive，用户每进一次英语页就会被永久改成「非沉浸」。
function applyImmersive(on) {
    if (on) {
        document.body.classList.add('is-immersive');
    } else {
        document.body.classList.remove('is-immersive');
    }

    var btn = document.getElementById('studyNavToggle');
    if (btn) {
        btn.textContent = on ? '显示导航栏' : '沉浸模式';
        btn.title = on ? '显示站点导航栏（Esc）' : '隐藏站点导航栏，全屏专注复习（Esc）';
    }
}

// 切换沉浸模式（顶栏按钮与 Esc 键共用）
function toggleImmersive() {
    setImmersive(!isImmersiveOn());
}

// 朗读一段英文文本（lang 默认美式英语）
function speakText(text, lang) {
    if (!text) return;
    if (!window.speechSynthesis || typeof window.SpeechSynthesisUtterance !== 'function') {
        return; // 浏览器不支持语音合成时静默跳过
    }
    try {
        window.speechSynthesis.cancel(); // 打断上一条，避免叠读
        var utter = new window.SpeechSynthesisUtterance(text);
        utter.lang = lang || 'en-US';
        utter.rate = 0.9;
        window.speechSynthesis.speak(utter);
    } catch (e) {
        console.error('朗读失败:', e);
    }
}

// 朗读当前卡片上的单词或例句（kind: 'word' | 'example'）
function speakCurrent(kind) {
    if (kind === 'example') {
        var answerEl = document.getElementById('reviewAnswer');
        if (!answerEl || answerEl.style.display === 'none') {
            setStatus('先显示答案才能朗读例句');
            return;
        }
    }
    var el = document.getElementById(kind === 'example' ? 'reviewExample' : 'reviewWord');
    if (el) speakText(el.textContent);
}

// ---------- 键盘快捷键 ----------

// 空格/回车：显示答案；Q/W/E/R（或 1~4）：评分（仅揭晓后生效）；
// P：朗读单词；L：朗读例句（E 已被「一般」评分占用）；Esc：切换沉浸模式
function handleReviewKey(e) {
    // 不在英语复习页时直接忽略
    if (!document.getElementById('reviewStatus')) return;

    // 还停在起始页：一个复习按键都不处理。
    // 尤其 Esc —— 它会切成沉浸模式把导航栏藏起来，而起始页刻意是要留着导航栏的。
    // 起始页的键盘操作交给那颗 <button> 自己：聚焦后回车 / 空格就是原生点击。
    if (!state.started) return;

    var target = e.target;
    if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable)) {
        return; // 正在操作输入控件时不拦截按键
    }

    // Esc 与卡片状态无关：加载中、已学完时也应该能切换沉浸模式
    if (e.key === 'Escape') {
        toggleImmersive();
        return;
    }

    var wordEl = document.getElementById('reviewWord');
    if (!wordEl || wordEl.style.display === 'none' || !state.card) return;

    var key = e.key;
    if (key === ' ' || key === 'Spacebar' || key === 'Enter') {
        e.preventDefault(); // 阻止空格滚动页面
        revealAnswer();
        return;
    }

    // 评分：Q/W/E/R 与 1~4 都支持（依次对应 陌生/困难/一般/简单）
    var byLetter = { q: 1, w: 2, e: 3, r: 4 };
    var lower = key.length === 1 ? key.toLowerCase() : '';
    if (byLetter[lower]) {
        handleReviewRating(byLetter[lower]);
        return;
    }
    if (key.length === 1 && key >= '1' && key <= '4') {
        handleReviewRating(parseInt(key, 10));
        return;
    }

    if (key === 'p' || key === 'P') {
        speakCurrent('word');
        return;
    }
    if (key === 'l' || key === 'L') {
        speakCurrent('example');
    }
}

// ---------- 通用小工具 ----------

// 设置元素文本（不解析 HTML），元素不存在时静默跳过
function setText(id, value) {
    var el = document.getElementById(id);
    if (el) el.textContent = (value === null || value === undefined) ? '' : String(value);
}

function setStatus(text) {
    setText('reviewStatus', text);
}

// 显示元素（按各自的布局类型还原 display）。
// ⚠️ 布局是 flex 的必须在这里登记：show() 兜底那句是 display:block，
// 用错会把纵向布局连居中一起打散（reviewApp / studyStart 整页都是 flex 列）。
function show(id) {
    var el = document.getElementById(id);
    if (!el) return;
    if (id === 'reviewPhoneticRow') {
        el.style.display = 'inline-flex';
    } else if (id === 'reviewButtons' || id === 'reviewReveal' || id === 'reviewAnswer' ||
               id === 'reviewApp' || id === 'studyStart') {
        el.style.display = 'flex';
    } else {
        el.style.display = 'block';
    }
}

function hide(id) {
    var el = document.getElementById(id);
    if (el) el.style.display = 'none';
}

// 主体里的提示（加载中 / 已完成 / 出错）
function showMessage(main, sub) {
    var box = document.getElementById('studyMessage');
    if (!box) return;
    box.textContent = '';
    var mainEl = document.createElement('div');
    mainEl.textContent = main;
    box.appendChild(mainEl);
    if (sub) {
        var subEl = document.createElement('div');
        subEl.className = 'study-message-sub';
        subEl.textContent = sub;
        box.appendChild(subEl);
    }
    box.style.display = 'block';
}

function hideMessage() {
    hide('studyMessage');
}

// 隐藏单词、音标与揭晓区（无卡片时）
function hideCardBody() {
    hide('reviewWord');
    hide('reviewPhoneticRow');
    hide('reviewAnswer');
}

function bindClick(id, handler) {
    var el = document.getElementById(id);
    if (el) el.addEventListener('click', handler);
}
