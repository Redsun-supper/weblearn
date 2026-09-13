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

var state = {
    session: null,   // 引擎里的 ReviewSession 实例（队列、游标、记忆上下文都在 Rust 侧）
    wasm: null,      // WASM 引擎模块命名空间
    revealed: false, // 当前卡片是否已揭晓答案（主动回忆：揭晓前不允许评分）
    stats: null,     // 最近一次 /api/reviews/stats 返回的数据
    card: null,      // 当前卡片的展示数据（渲染与揭晓共用，避免重复向引擎取数）
    pendingSubmit: null, // 最近一次复习提交的 Promise（补词前要等它落库，避免竞态）
    refilling: false,    // 是否正在补卡（防止并发补卡）
    planInfo: null,      // 引擎给的今日计划规模 {new_target, probe_target, plan_len}
    planTotals: null,    // 今日计划的分母 {new, probe}，建会话时算一次后固定
    planPhase: '',       // 左下角小字当前处于哪一段：'' / 'plan' / 'review'
    queueOffset: 0,      // 复习区已经交给引擎多少张（翻页用）
    queueTotal: 0        // 复习区一共多少张（后端给的总数）
};

// 自动朗读开关在 localStorage 中的键名
var AUTO_SPEAK_KEY = 'reviewAutoSpeak';

// 沉浸模式（隐藏站点导航栏）在 localStorage 中的键名
// 没有记录时默认**开启**：进入复习页就是要专注，导航栏先收起来
var IMMERSIVE_KEY = 'reviewImmersive';

// 今日计划（每天 5 个新词 + 5 个抽查）在 localStorage 中的键名
//
// ⚠️ 为什么配额记在浏览器上而不是服务端：现在没有登录系统，服务端只有一份共享词库，
// 若在服务端按「每天 5 个」算，等于全站每天共放 5 个新词——你先学了别人就没得学。
// 记在本地就是「每台设备各自一份计划」，换设备/清缓存会重置（已知代价，等有登录再迁走）。
var PLAN_KEY = 'reviewDailyPlan';

// 每天的新词 / 抽查配额
var DAILY_NEW_TARGET = 5;
var DAILY_PROBE_TARGET = 5;

// 抽查冷却期（天）：同一个词在这段时间内不会被再次抽中，
// 否则「每天都抽到期最远的那几个」会变成新的饥饿
var PROBE_COOLDOWN_DAYS = 7;

// 取数批量：复习区每次取多少张、新词候选取多少、抽查候选取多少
// 新词/抽查都多取一些候选，随机抽与冷却过滤才有挑选余地
var QUEUE_PAGE_SIZE = 100;
var NEW_CANDIDATE_SIZE = 20;
var PROBE_CANDIDATE_SIZE = 20;

// 引擎模块路径（相对本文件所在目录解析）
var WASM_MODULE_URL = './engine/pkg/guangxue_wasm.js';

// 换卡过渡时长（毫秒）：需与 english.css 里的离场动画时长保持一致
var TRANSITION_MS = 150;

// 左下角计划文案的切换时长（毫秒）：需与 english.css 的 planOut 动画一致
var PLAN_SWAP_OUT_MS = 160;

// 键盘监听是否已绑定（同一页面内反复切换学科只绑定一次）
var keysBound = false;

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

// 初始化复习应用。由 main.js 在英语页 HTML 注入完成后调用。
export function initReviewApp() {
    var statusEl = document.getElementById('reviewStatus');
    if (!statusEl) return; // 当前页面不是英语页，跳过
    statusEl.textContent = '正在加载复习内容...';

    // 释放上一次会话对象，避免反复进出英语页时泄漏 WASM 侧内存
    releaseSession();

    // 键盘快捷键只绑定一次：同一页面内反复切换学科不会重复注册
    if (!keysBound) {
        keysBound = true;
        document.addEventListener('keydown', handleReviewKey);
    }

    // 恢复「自动朗读」开关状态（默认开启）
    var autoEl = document.getElementById('reviewAutoSpeak');
    if (autoEl) {
        autoEl.checked = isAutoSpeakOn();
        autoEl.addEventListener('change', function() {
            setAutoSpeak(this.checked);
        });
    }

    // 沉浸模式：把站点导航栏收起来，让复习页占满整屏（默认开启）
    bindClick('studyNavToggle', function() { toggleImmersive(); });
    setImmersive(isImmersiveOn());

    // 浏览器会拦截未经用户交互的语音：首次交互后补读一次当前单词
    if (!unlockBound) {
        unlockBound = true;
        document.addEventListener('pointerdown', unlockSpeech);
        document.addEventListener('keydown', unlockSpeech);
    }

    // 显示答案按钮与发音按钮
    bindClick('reviewRevealBtn', function() { revealAnswer(); });
    bindClick('reviewSpeakWordBtn', function() { speakCurrent('word'); });
    bindClick('reviewSpeakExampleBtn', function() { speakCurrent('example'); });

    // 评分按钮（学科页每次加载都会重建这些元素，直接绑定即可）
    var buttons = document.querySelectorAll('.rating');
    for (var k = 0; k < buttons.length; k++) {
        buttons[k].addEventListener('click', function() {
            this.blur(); // 主动失焦，避免回车键被按钮重复触发
            handleReviewRating(parseInt(this.getAttribute('data-rating'), 10));
        });
    }

    // 加载引擎 → 取回今日计划所需的四份数据 → 交给引擎编排
    var plan = loadTodayPlan();
    state.planPhase = '';

    loadEngine().then(function(wasm) {
        state.wasm = wasm;
        return fetchDay(0);
    }).then(function(results) {
        var statsRes = results[3];

        // 顶部统计先拿到，渲染卡片时一起显示
        if (statsRes && statsRes.code === 200 && statsRes.data) {
            state.stats = statsRes.data;
        }

        // 队列编排（今日计划 + 复习区排序 + 日期换算）全部在引擎内完成。
        // 直接把接口返回的 JSON 原文交给引擎，本文件不再解析与保存卡片数组。
        state.session = new state.wasm.ReviewSession(
            results[0], // 复习区：整库按紧迫度升序（含未到期）
            results[1], // 新词候选
            results[2], // 抽查候选（到期最远的）
            JSON.stringify(buildPlanOptions(plan))
        );

        // 记下复习区取到哪儿了：队列走完后按 offset 取下一页
        state.queueOffset = parseQueueMeta(results[0]).count;
        state.queueTotal = parseQueueMeta(results[0]).total;
        state.planInfo = JSON.parse(state.session.plan_json());

        // 今日计划的分母在建会话时**只算一次**：
        // = 今天此前已经做完的 + 本次队列里排着的。
        // 不能每次渲染时重算，否则分子涨一分母也跟着涨（会出现「新词 1/4、2/5」这种错）。
        state.planTotals = {
            new: plan.newDone + state.planInfo.new_target,
            probe: plan.probeDone + state.planInfo.probe_target
        };

        renderCard();
    }).catch(function(err) {
        setStatus('');
        showMessage('复习功能加载失败', String(err) + '（需通过服务器访问，并确认已生成 WASM 引擎）');
        console.error('复习功能初始化失败:', err);
    });
}

// 离开英语页时的清理。由 main.js 在切换到其他学科之前调用。
// 为什么必须显式清理：沉浸模式把 is-immersive 类挂在了 document.body 上，
// 内容容器被换成别的学科后这个类不会自己消失，导航栏会跟着一起不见。
// 顺带释放引擎侧会话、取消未读完的语音，避免反复进出英语页时内存与声音残留。
export function unmount() {
    document.body.classList.remove('is-immersive');
    cancelRevealAnimation();
    clearTimeout(planSwapTimer);
    planSwapTimer = null;
    releaseSession();
    state.card = null;
    state.revealed = false;
    state.stats = null;
    state.pendingSubmit = null;
    state.refilling = false;
    state.planInfo = null;
    state.planTotals = null;
    state.planPhase = '';
    state.queueOffset = 0;
    state.queueTotal = 0;
    if (window.speechSynthesis) {
        try {
            window.speechSynthesis.cancel();
        } catch (e) {
            // 浏览器不支持时忽略
        }
    }
}

// 取今日计划所需的数据。
// 四份数据各司其职：复习区（整库紧迫度序）/ 新词候选 / 抽查候选 / 顶部统计。
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
// 配额和抽查冷却记录都放在浏览器 localStorage 里（原因见 PLAN_KEY 的注释）。
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

// 记一次计划完成（评分成功后调用）
//
// 这里**不刷界面**：评分紧接着就是换卡过渡（150ms），界面刷新交给换卡时的
// renderPlanProgress。若在这里先刷一次，切换动画刚起步就会被换卡那次渲染掐断。
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

// 动态加载 WASM 引擎模块（浏览器原生 import()）
// 两个关键点：
// 1. 相对路径必须写成 './xxx' 或 '../xxx'：'engine/pkg/...' 会被当作 bare specifier
//    （npm 包名）而解析失败；
// 2. wasm-bindgen 的 --target web 产物必须先 await 默认导出（__wbg_init）完成实例化，
//    否则模块内部变量 wasm 仍是 undefined，调用任何导出函数都会抛 TypeError。
function loadEngine() {
    return import(WASM_MODULE_URL).then(function(mod) {
        return mod.default().then(function() {
            return mod;
        });
    });
}

// 取回响应原文（不做 JSON 解析，交给引擎处理）
function fetchText(url) {
    return fetch(url).then(function(res) {
        if (!res.ok) {
            throw new Error(url + ' 请求失败：HTTP ' + res.status);
        }
        return res.text();
    });
}

// 取回并解析 JSON
function fetchJson(url) {
    return fetch(url).then(function(res) {
        if (!res.ok) {
            throw new Error(url + ' 请求失败：HTTP ' + res.status);
        }
        return res.json();
    });
}

// ---------- 渲染：卡片 ----------

// 首屏渲染（不做过渡，避免刚进页面就闪一下）
function renderCard() {
    renderCardNow();
}

// 评分后切换卡片：先上锁 → 播离场动画 → 更新内容 → 播入场动画
//
// 上锁必须发生在过渡**开始前**：过渡期间旧卡的答案与评分按钮还在屏幕上，
// 若用户连点或按住数字键，第二次评分会落到下一张卡上（引擎游标已经前进）。
function advanceCard() {
    state.revealed = false; // 评分守卫立刻失效
    state.card = null;      // 键盘处理器靠 !state.card 直接 return
    hide('reviewButtons');  // 视觉上让按钮先消失
    withCardTransition(function() {
        renderCardNow();
    });
}

// 换卡过渡：给舞台加离场/入场动画类
// 系统开启「减少动态效果」时直接跳过动画（业务逻辑完全一致）
function withCardTransition(update) {
    var stage = document.getElementById('studyStage');
    if (!stage || prefersReducedMotion()) {
        update();
        return;
    }
    stage.classList.remove('is-in');
    stage.classList.add('is-out');
    setTimeout(function() {
        update(); // 真正换内容
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

// 把当前卡片画到界面上（幂等；不负责过渡动画）
function renderCardNow() {
    var session = state.session;
    if (!session) return;

    // 换卡即重置揭示状态：新卡一律先隐藏答案与评分按钮
    state.revealed = false;
    state.card = null;
    cancelRevealAnimation(); // 上一张卡的揭晓动效可能还在收尾，先掐掉
    hide('reviewAnswer');
    hide('reviewButtons');
    hide('reviewReveal');

    renderStats(state.stats);
    // 计划小字要在「队列走完」这条分支之前刷新：
    // 最后一张计划卡评完时队列可能同时见底，否则小字会停在旧文案上不再切换
    renderPlanProgress();

    if (session.is_finished()) {
        hideCardBody();
        setStatus('');
        showFinishMessage();
        // 队列抽干：还有下一页就接着取，取不到就停在上面的结束文案
        maybeRefill();
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

    // 无限学习的场景下，「队列剩余」比「第 N / M 张」更能反映进度（M 会随翻页增长）
    var sourceLabel = sourceLabelOf(state.card.source);
    setStatus('已学 ' + session.done() + ' 张 · 队列剩余 ' +
        (session.total() - session.done()) + ' 张 · ' + sourceLabel);
    show('reviewReveal');

    // 开启自动朗读时，每张新卡出现即朗读单词
    if (isAutoSpeakOn()) speakCurrent('word');
}

// 卡片来源 → 界面文案
function sourceLabelOf(source) {
    if (source === 'new') return '新词';
    if (source === 'probe') return '抽查';
    return '复习';
}

// 结束文案：区分「本轮学过」「词库学完」「词库为空」三种情况
function showFinishMessage() {
    var session = state.session;
    var stats = state.stats || {};
    var learned = session ? session.done() : 0;

    if (learned > 0) {
        showMessage('今天复习完成 ✨', '本轮共学 ' + learned + ' 张');
    } else if ((stats.total_words || 0) === 0) {
        showMessage('词库还是空的', '可以到后台的「批量导入」粘贴词表添加词条');
    } else if ((stats.new_words || 0) === 0) {
        showMessage('词库都学完了 🎉', '共 ' + stats.total_words + ' 个词条；可以到后台继续添加');
    } else {
        showMessage('暂无需要复习的单词', '明天再来 👋');
    }
}

// 队列走完后接着往下翻：复习区还分页没取完就再取一页，取完了就停在结束文案。
// 注意这里只追加**复习区**：新词与抽查受每日配额限制，不因为翻页而变多。
function maybeRefill() {
    if (state.refilling || !state.session) return;

    // 整库都翻完了：没有更多卡片了
    if (state.queueOffset >= state.queueTotal) {
        showFinishMessage();
        return;
    }

    state.refilling = true;

    // 必须等上一次提交落库：否则刚评过的词在服务端 due_at 还没更新，
    // 会被当成到期卡再抽一次（引擎侧的待办去重挡不住它，因为它已越过游标）
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
            setStatus('补卡失败：' + e);
            console.error('补卡失败:', e);
            return;
        }
        state.queueOffset += meta.count;

        if (added > 0) {
            renderCardNow(); // 接着往下翻
            return;
        }
        // 这一页没有新卡（例如全都在待办区里了）：直接看还有没有下一页
        if (state.queueOffset < state.queueTotal && meta.count > 0) {
            state.refilling = false;
            maybeRefill();
            return;
        }
        showFinishMessage();
    }).catch(function(err) {
        state.refilling = false;
        setStatus('补卡失败（刷新页面可重试）：' + err);
        console.error('自动补卡失败:', err);
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

// 画出左下角小字。
// 两个阶段：
//   plan   —— 今日计划还没走完：「新词 3/5 · 抽查 2/5」
//   review —— 计划区的卡全部评完了：「计划完成 · 进入复习阶段」（带动效切换）
// 阶段切换只在真的跨过去时播一次动画。
function renderPlanProgress() {
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

    setPlanText(text, phase !== state.planPhase && state.planPhase !== '');
    state.planPhase = phase;
}

// 设置左下角文案：阶段变化时先淡出旧文案、再浮入新文案（见 english.css 的 planOut / planIn）
function setPlanText(text, animate) {
    var el = document.getElementById('studyPlanText');
    if (!el) return;

    // 文案没变就什么都不做。
    // ⚠️ 这里**不能**顺手清掉动画类：切换动画播到一半时如果有一次多余的渲染
    // （换卡、统计刷新等）进来，就会把动画掐断，看上去像「没播」。
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

// 揭晓区：主例句（目标词高亮，配中文翻译）+ 一条条释义块
function renderAnswer(card) {
    // ---------- 主例句 ----------
    // 引擎已切成「命中 / 未命中」片段，这里只负责拼成 DOM。
    // 词条没有例句时不渲染引文块（多释义各自带例句的情况很常见，不能留个空壳）
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

    // 主例句的中文翻译（没填就不显示这一行）
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

    // ---------- 释义块 ----------
    // 引擎已经把「一个词性一块」拆好了（词条填了多释义就用它，没填就按词性标签自动拆）
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

    // 该释义专属例句（缩进对齐到释义正文那一列）
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

// 揭晓答案：显示例句与释义，并放出评分按钮
// 只有揭晓后才允许评分，否则「看着答案打分」会让 FSRS 的记忆状态失真
function revealAnswer() {
    if (state.revealed) return;
    if (!state.session || state.session.is_finished()) return;
    state.revealed = true;

    show('reviewAnswer');

    // 揭晓动效：整块淡入太死板，改成按顺序浮现（见 playRevealAnimation）
    // 「揭晓按钮退场 → 评分按钮入场」也在这条时间线上，所以这里不再直接 hide/show 按钮
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
// 为什么分离「动画定义」与「延时」：什么时候开始由这里的数字决定（好调），
// 动画怎么动（时长/缓动/起止状态）写在 english.css 里，两边只靠这几个常量对齐。
// 单词本身**不参与动画**：揭晓时它必须待在原位不动（之前整体居中导致上移，已经改掉了）。

// 释义块之间的错开间隔
var REVEAL_STEP_MS = 60;
// 首块释义之前留一点时间，让主例句先浮现
var REVEAL_HEAD_MS = 110;
// 译文比主例句晚一点
var REVEAL_CN_MS = 60;
// 高亮比它所在的那一块再晚一点（例句先出来，荧光笔才扫过去）
var REVEAL_HIT_MS = 100;
// 「显示答案」按钮退场时长。**换人的时刻就是它**：退场动画一播完，
// 立刻藏掉它、放出评分按钮，中间不留空档也不重叠
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

// 播放揭晓动效：给各元素排好延时，再给根节点挂 revealing 类触发动画
// 返回这一轮动效的总时长（毫秒）
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

    // 主例句与它的译文
    setRevealDelay(document.getElementById('reviewExample'), 0);
    setRevealDelay(document.getElementById('reviewExampleCn'), REVEAL_CN_MS);

    var i, k;

    // 主例句里的目标词高亮
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
    // ⚠️ 必须先藏掉揭晓按钮再放评分按钮：两者是底部操作区里相邻的两个块，
    // 同时显示会让操作区变高，把上面的单词顶上去（那正是之前修掉的毛病）。
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

// 用户点击评分：交给引擎算新记忆状态 → 引擎返回可直接提交的请求体 → POST 持久化
// 前置条件：必须已揭晓答案；揭晓前的评分一律忽略（避免误触键盘泄漏答案、也避免凭猜测打分）
function handleReviewRating(rating) {
    var session = state.session;
    if (!session || session.is_finished()) return;
    if (!state.revealed) {
        setStatus('请先按空格（或点「显示答案」）揭晓答案，再评分');
        return;
    }

    // 引擎内部完成：换算距上次复习天数 → FSRS 计算 → 取对应分支 → 推进游标
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
        return res.json();
    }).then(function(data) {
        if (data && data.code === 200) {
            // 提交成功后刷新统计（今日新学 / 今日复习 / 剩余待学）
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

// 读取「自动朗读」开关（持久化在 localStorage）
// **默认开启**：只有用户显式关掉过（存了 '0'）才是关闭状态
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

// 保存「自动朗读」开关
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

// 应用沉浸模式：给 body 挂 is-immersive 类，站点级样式（main.css）据此隐藏导航栏并让内容区占满整屏
// 这里只负责挂/摘类与更新按钮文案，样式一律写在 CSS 里
function setImmersive(on) {
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

    try {
        localStorage.setItem(IMMERSIVE_KEY, on ? '1' : '0');
    } catch (e) {
        // 存不下也不影响本次使用
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

// 设置底部状态文字
function setStatus(text) {
    setText('reviewStatus', text);
}

// 显示元素（按各自的布局类型还原 display）
function show(id) {
    var el = document.getElementById(id);
    if (!el) return;
    if (id === 'reviewPhoneticRow') {
        el.style.display = 'inline-flex';
    } else if (id === 'reviewButtons' || id === 'reviewReveal' || id === 'reviewAnswer') {
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

// 隐藏单词与音标（无卡片时）
function hideCardBody() {
    hide('reviewWord');
    hide('reviewPhoneticRow');
    hide('reviewAnswer');
}

function bindClick(id, handler) {
    var el = document.getElementById(id);
    if (el) el.addEventListener('click', handler);
}
