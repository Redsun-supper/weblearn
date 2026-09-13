// 英语模块 · 单词间隔复习（FSRS）
//
// 本文件是 ES module，由 main.js 在**进入英语页时**动态 import() 加载：
// 只有真正进入英语页才会加载本模块及其 WASM 引擎，首屏不再携带英语逻辑。
//
// 职责划分（详见 modules/english/README.md）：
// - 本文件  ：DOM 渲染、事件绑定、取数与提交（fetch）、本地存储、语音合成
// - engine/ ：队列编排、FSRS 调度计算、随机化、日期换算、进度统计（Rust → WASM）
//
// 因此本文件**不保存队列、不计算间隔、不做日期运算**：
// 队列与游标由引擎里的 ReviewSession 持有，评分时引擎直接返回可提交的请求体。
//
// 模块本身是单例（ES module 的 import 缓存保证只实例化一次），因此模块级变量
// 就是应用级状态，无需再挂到 window 上。
//
// 本文件保持 ES5 写法（var / function），仅使用 export 做模块导出。

// ---------- 模块级状态 ----------

var state = {
    session: null,   // 引擎里的 ReviewSession 实例（队列、游标、记忆上下文都在 Rust 侧）
    wasm: null,      // WASM 引擎模块命名空间
    revealed: false, // 当前卡片是否已揭晓答案（主动回忆：揭晓前不允许评分）
    stats: null      // 最近一次 /api/reviews/stats 返回的数据
};

// 自动朗读开关在 localStorage 中的键名
var AUTO_SPEAK_KEY = 'reviewAutoSpeak';

// 引擎模块路径（相对本文件所在目录解析）
var WASM_MODULE_URL = './engine/pkg/guangxue_wasm.js';

// 今日新词上限（与后端默认口径一致）
var NEW_LIMIT = 5;

// 键盘监听是否已绑定（同一页面内反复切换学科只绑定一次）
var keysBound = false;

// ---------- 对外入口 ----------

// 初始化复习应用。由 main.js 在英语页 HTML 注入完成后调用。
// 重复调用是安全的：元素每次注入都会重建，重新绑定即可。
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

    // 恢复「自动朗读」开关状态
    var autoEl = document.getElementById('reviewAutoSpeak');
    if (autoEl) {
        autoEl.checked = isAutoSpeakOn();
        autoEl.addEventListener('change', function() {
            setAutoSpeak(this.checked);
        });
    }

    // 显示答案按钮与发音按钮
    bindClick('reviewRevealBtn', function() { revealAnswer(); });
    bindClick('reviewSpeakWordBtn', function() { speakCurrent('word'); });
    bindClick('reviewSpeakExampleBtn', function() { speakCurrent('example'); });

    // 评分按钮（学科页每次加载都会重建这些元素，直接绑定即可）
    var buttons = document.querySelectorAll('.review-btn');
    for (var k = 0; k < buttons.length; k++) {
        buttons[k].addEventListener('click', function() {
            this.blur(); // 主动失焦，避免回车键被按钮重复触发
            handleReviewRating(parseInt(this.getAttribute('data-rating'), 10));
        });
    }

    // 加载引擎 → 取回接口原文 → 交给引擎建会话
    loadEngine().then(function(wasm) {
        state.wasm = wasm;
        return Promise.all([
            fetchText('/api/reviews/due?limit=50'),
            fetchText('/api/reviews/new?limit=50'),
            fetchJson('/api/reviews/stats')
        ]);
    }).then(function(results) {
        var dueText = results[0];
        var newText = results[1];
        var statsRes = results[2];

        // 统计面板先渲染，进入页面即可看到今日进度
        if (statsRes && statsRes.code === 200 && statsRes.data) {
            state.stats = statsRes.data;
            renderStats(state.stats);
        }

        // 队列构建（洗牌到期卡 / 抽新词 / 换算日期）全部在引擎内完成。
        // 直接把接口返回的 JSON 原文交给引擎，本文件不再解析与保存卡片数组。
        state.session = new state.wasm.ReviewSession(dueText, newText, NEW_LIMIT);
        renderReviewCard();
    }).catch(function(err) {
        statusEl.textContent = '复习功能加载失败（需通过服务器访问，并确认已生成 WASM 引擎）: ' + err;
        console.error('复习功能初始化失败:', err);
    });
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
//    不传参时 __wbg_init 会自动 fetch 同目录下的 guangxue_wasm_bg.wasm。
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

// ---------- 渲染 ----------

// 渲染当前卡片（主动回忆：只显示单词与音标，答案区保持隐藏）
function renderReviewCard() {
    var statusEl = document.getElementById('reviewStatus');
    var cardEl = document.getElementById('reviewCard');
    var btnEl = document.getElementById('reviewButtons');
    var revealEl = document.getElementById('reviewReveal');
    var answerEl = document.getElementById('reviewAnswer');
    if (!statusEl || !cardEl || !btnEl) return;

    // 换卡即重置揭示状态：新卡一律先隐藏答案与评分按钮
    state.revealed = false;
    if (answerEl) answerEl.style.display = 'none';
    if (btnEl) btnEl.style.display = 'none';

    var session = state.session;
    if (!session) return;

    if (session.is_finished()) {
        // 队列走完：区分「今天复习完了」与「词库为空」两种情况
        cardEl.style.display = 'none';
        if (revealEl) revealEl.style.display = 'none';
        var total = session.total();
        var stats = state.stats;
        if (total > 0) {
            statusEl.textContent = '今日完成 ✨ 共 ' + total + ' 张';
        } else if (stats && stats.total_words === 0) {
            statusEl.textContent = '词库为空：请在 backend-go 目录执行 go run ./cmd/seed 导入单词';
        } else {
            statusEl.textContent = '暂无需要复习的单词，明天再来 👋';
        }
        renderProgress();
        return;
    }

    var card = JSON.parse(session.current_json());
    // 使用 textContent 渲染词条数据，避免词条内容被当作 HTML 解析
    setTextContent('reviewWord', card.word || '');
    setTextContent('reviewPhonetic', card.phonetic || '');
    setTextContent('reviewMeaning', card.meaning || '');
    setTextContent('reviewExample', card.example || '');
    cardEl.style.display = 'block';
    if (revealEl) revealEl.style.display = 'block';
    var label = card.source === 'new' ? '新词' : '到期';
    statusEl.textContent = '第 ' + (session.done() + 1) + ' / ' + session.total() +
        ' 张（' + label + '）· 先回想，再显示答案';
    // 开启自动朗读时，每张新卡出现即朗读单词
    if (isAutoSpeakOn()) speakCurrent('word');
    renderProgress();
}

// 揭晓答案：显示释义与例句，并放出评分按钮
// 只有揭晓后才允许评分，否则「看着答案打分」会让 FSRS 的记忆状态失真
function revealAnswer() {
    if (state.revealed) return;
    var session = state.session;
    if (!session || session.is_finished()) return;
    state.revealed = true;

    var answerEl = document.getElementById('reviewAnswer');
    var revealEl = document.getElementById('reviewReveal');
    var btnEl = document.getElementById('reviewButtons');
    if (answerEl) answerEl.style.display = 'block';
    if (revealEl) revealEl.style.display = 'none';
    if (btnEl) btnEl.style.display = 'flex';

    var statusEl = document.getElementById('reviewStatus');
    if (statusEl) {
        var card = JSON.parse(session.current_json());
        var label = card.source === 'new' ? '新词' : '到期';
        statusEl.textContent = '第 ' + (session.done() + 1) + ' / ' + session.total() +
            ' 张（' + label + '）· 请根据回忆情况评分';
    }
}

// 渲染本轮队列进度条（进度百分比由引擎计算）
function renderProgress() {
    var bar = document.getElementById('statProgressBar');
    if (!bar || !state.session) return;
    bar.style.width = state.session.progress_percent() + '%';
}

// ---------- 评分与提交 ----------

// 用户点击评分：交给引擎算新记忆状态 → 引擎返回可直接提交的请求体 → POST 持久化
// 前置条件：必须已揭晓答案；揭晓前的评分一律忽略（避免误触键盘泄漏答案、也避免凭猜测打分）
function handleReviewRating(rating) {
    var session = state.session;
    if (!session || session.is_finished()) return;
    if (!state.revealed) {
        var hintEl = document.getElementById('reviewStatus');
        if (hintEl) hintEl.textContent = '请先按空格（或点「显示答案」）揭晓答案，再评分';
        return;
    }

    // 引擎内部完成：换算距上次复习天数 → FSRS 计算 → 取对应分支 → 推进游标
    var payload;
    try {
        payload = session.rate(rating, Date.now());
    } catch (e) {
        var errEl = document.getElementById('reviewStatus');
        if (errEl) errEl.textContent = '引擎计算失败: ' + e;
        return;
    }

    renderReviewCard(); // 乐观推进到下一张

    fetch('/api/reviews/submit', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: payload // 引擎返回的 JSON 即请求体，无需在前端重新拼装
    }).then(function(res) {
        return res.json();
    }).then(function(data) {
        if (data && data.code === 200) {
            // 提交成功后刷新统计（今日已复习 / 到期数 / 连续天数）
            loadReviewStats();
        } else {
            console.error('提交复习失败:', data);
        }
    }).catch(function(err) {
        console.error('提交复习失败:', err);
    });
}

// ---------- 统计面板 ----------

// 渲染 /api/reviews/stats 的数据：今日进度、词库概览、连续天数与记忆保持率
function renderStats(stats) {
    var panel = document.getElementById('reviewStats');
    if (!panel || !stats) return;
    setTextContent('statToday', stats.today_reviewed || 0);
    setTextContent('statDue', stats.due_cards || 0);
    setTextContent('statNew', stats.new_words || 0);
    setTextContent('statStreak', stats.streak_days || 0);
    var rate = '—';
    if (stats.total_reviews > 0 && typeof stats.retention_rate === 'number') {
        rate = Math.round(stats.retention_rate * 100) + '%';
    }
    setTextContent('statRetention', rate);
    setTextContent('statTotal', '词库共 ' + (stats.total_words || 0) + ' 词 · 已学 ' +
        (stats.reviewed_words || 0) + ' 词 · 累计复习 ' + (stats.total_reviews || 0) + ' 次');
    panel.style.display = 'flex';
    renderProgress();
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
function isAutoSpeakOn() {
    try {
        return localStorage.getItem(AUTO_SPEAK_KEY) === '1';
    } catch (e) {
        return false;
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
    var el = document.getElementById(kind === 'example' ? 'reviewExample' : 'reviewWord');
    if (el) speakText(el.textContent);
}

// ---------- 键盘快捷键 ----------

// 空格/回车：显示答案；1~4：评分（仅揭晓后生效）；P：朗读单词；E：朗读例句
function handleReviewKey(e) {
    // 不在英语复习页时直接忽略
    if (!document.getElementById('reviewStatus')) return;
    var target = e.target;
    if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable)) {
        return; // 正在操作输入控件时不拦截按键
    }
    var cardEl = document.getElementById('reviewCard');
    if (!cardEl || cardEl.style.display === 'none') return;

    var key = e.key;
    if (key === ' ' || key === 'Spacebar' || key === 'Enter') {
        e.preventDefault(); // 阻止空格滚动页面
        revealAnswer();
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
    if (key === 'e' || key === 'E') {
        speakCurrent('example');
    }
}

// ---------- 通用小工具 ----------

// 设置元素文本（不解析 HTML），元素不存在时静默跳过
function setTextContent(id, value) {
    var el = document.getElementById(id);
    if (el) el.textContent = (value === null || value === undefined) ? '' : String(value);
}

// 绑定点击事件（元素不存在时静默跳过）
function bindClick(id, handler) {
    var el = document.getElementById(id);
    if (el) el.addEventListener('click', handler);
}
