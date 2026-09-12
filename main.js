// document表示整个HTML文档对象，addEventListener为文档添加事件监听器
// 'DOMContentLoaded'事件在HTML文档完全加载并解析完成后触发，不等待图片等外部资源加载
// 使用此事件确保DOM元素已准备好，可以安全地操作DOM
document.addEventListener('DOMContentLoaded', function() {
    // console.log向浏览器开发者工具的控制台输出日志信息，用于调试和验证代码执行
    // 此处输出提示信息，确认页面DOM已成功加载
    console.log('Page loaded successfully');
    
    // 使用querySelectorAll选中所有class为nav-rect的导航矩形项元素，返回NodeList集合
    // 这些元素是页面上显示各个学科名称的可点击按钮
    const navRects = document.querySelectorAll('.nav-rect');
    
    // 使用getElementById获取内容容器元素，用于显示加载的HTML内容
    // 该容器位于导航栏下方，用于展示用户选择的学科内容
    const contentContainer = document.getElementById('contentContainer');
    
    // ===================== 单词间隔复习（FSRS 引擎）简单版 =====================
    // 依赖 frontend-rust/pkg/guangxue_wasm.js（wasm-bindgen 生成的 ES 模块）
    // 流程：取到期卡+新词 → WASM 随机器排队列 → 用户评分 → WASM 引擎算下一状态 → 提交后端 SQL
    var reviewAppState = {
        queue: [],      // 待复习队列 [{source:'due'|'new', item:...}]
        cursor: 0,
        wasm: null,
        revealed: false, // 当前卡片是否已揭晓答案（主动回忆：揭晓前不允许评分）
        stats: null      // 最近一次 /api/reviews/stats 返回的数据
    };

    // 自动朗读开关在 localStorage 中的键名
    var AUTO_SPEAK_KEY = 'reviewAutoSpeak';

    // 动态加载 WASM 引擎模块（浏览器原生 import()）
    // 两个关键点：
    // 1. 相对路径必须写成 './xxx'：'frontend-rust/...' 会被当作 bare specifier（npm 包名）而解析失败；
    // 2. wasm-bindgen 的 --target web 产物必须先 await 默认导出（__wbg_init）完成实例化，
    //    否则模块内部变量 wasm 仍是 undefined，调用任何导出函数都会抛 TypeError。
    //    不传参时 __wbg_init 会自动 fetch 同目录下的 guangxue_wasm_bg.wasm。
    function loadReviewWasm() {
        return import('./frontend-rust/pkg/guangxue_wasm.js').then(function(mod) {
            return mod.default().then(function() {
                return mod;
            });
        });
    }

    // 加载今日复习队列：到期卡（洗牌打乱顺序）+ 新词（随机器无放回抽 5 个）
    function loadReviewQueue(wasm) {
        return Promise.all([
            fetch('/api/reviews/due?limit=50').then(function(res) { return res.json(); }),
            fetch('/api/reviews/new?limit=50').then(function(res) { return res.json(); }),
            fetch('/api/reviews/stats').then(function(res) { return res.json(); })
        ]).then(function(results) {
            var dueItems = (results[0].data && results[0].data.items) || [];
            var newItems = (results[1].data && results[1].data.items) || [];
            // 统计面板先渲染，进入页面即可看到今日进度
            if (results[2] && results[2].code === 200 && results[2].data) {
                reviewAppState.stats = results[2].data;
                renderStats(reviewAppState.stats);
            }
            var seed = wasm.random_seed();
            var queue = [];
            var dueOrder = wasm.random_indices(dueItems.length, seed);
            for (var i = 0; i < dueOrder.length; i++) {
                queue.push({ source: 'due', item: dueItems[dueOrder[i]] });
            }
            var newCount = Math.min(5, newItems.length);
            var newOrder = wasm.random_sample(newItems.length, newCount, seed ^ 0x9E3779B9);
            for (var j = 0; j < newOrder.length; j++) {
                queue.push({ source: 'new', item: newItems[newOrder[j]] });
            }
            reviewAppState.queue = queue;
            reviewAppState.cursor = 0;
            return queue;
        });
    }

    // 计算当前卡片的记忆状态 JSON 与距上次复习天数
    function reviewCardContext(entry) {
        if (entry.source === 'new') {
            return { stateJson: null, daysElapsed: 0 };
        }
        var item = entry.item;
        var last = item.last_review_at || item.due_at;
        var days = Math.floor((Date.now() - new Date(last).getTime()) / 86400000);
        if (days < 0) days = 0;
        return {
            stateJson: JSON.stringify({ stability: item.stability, difficulty: item.difficulty }),
            daysElapsed: days
        };
    }

    // 渲染当前卡片（主动回忆：只显示单词与音标，答案区保持隐藏）
    function renderReviewCard() {
        var statusEl = document.getElementById('reviewStatus');
        var cardEl = document.getElementById('reviewCard');
        var btnEl = document.getElementById('reviewButtons');
        var revealEl = document.getElementById('reviewReveal');
        var answerEl = document.getElementById('reviewAnswer');
        if (!statusEl || !cardEl || !btnEl) return;

        // 换卡即重置揭示状态：新卡一律先隐藏答案与评分按钮
        reviewAppState.revealed = false;
        if (answerEl) answerEl.style.display = 'none';
        if (btnEl) btnEl.style.display = 'none';

        var total = reviewAppState.queue.length;
        if (reviewAppState.cursor >= total) {
            cardEl.style.display = 'none';
            if (revealEl) revealEl.style.display = 'none';
            var stats = reviewAppState.stats;
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

        var entry = reviewAppState.queue[reviewAppState.cursor];
        var item = entry.item;
        // 使用 textContent 渲染词条数据，避免词条内容被当作 HTML 解析
        setTextContent('reviewWord', item.word || '');
        setTextContent('reviewPhonetic', item.phonetic || '');
        setTextContent('reviewMeaning', item.meaning || '');
        setTextContent('reviewExample', item.example || '');
        cardEl.style.display = 'block';
        if (revealEl) revealEl.style.display = 'block';
        var label = entry.source === 'new' ? '新词' : '到期';
        statusEl.textContent = '第 ' + (reviewAppState.cursor + 1) + ' / ' + total +
            ' 张（' + label + '）· 先回想，再显示答案';
        // 开启自动朗读时，每张新卡出现即朗读单词
        if (isAutoSpeakOn()) speakCurrent('word');
        renderProgress();
    }

    // 揭晓答案：显示释义与例句，并放出评分按钮
    // 只有揭晓后才允许评分，否则「看着答案打分」会让 FSRS 的记忆状态失真
    function revealAnswer() {
        if (reviewAppState.revealed) return;
        var entry = reviewAppState.queue[reviewAppState.cursor];
        if (!entry) return;
        reviewAppState.revealed = true;

        var answerEl = document.getElementById('reviewAnswer');
        var revealEl = document.getElementById('reviewReveal');
        var btnEl = document.getElementById('reviewButtons');
        if (answerEl) answerEl.style.display = 'block';
        if (revealEl) revealEl.style.display = 'none';
        if (btnEl) btnEl.style.display = 'flex';

        var statusEl = document.getElementById('reviewStatus');
        if (statusEl) {
            var label = entry.source === 'new' ? '新词' : '到期';
            statusEl.textContent = '第 ' + (reviewAppState.cursor + 1) + ' / ' +
                reviewAppState.queue.length + ' 张（' + label + '）· 请根据回忆情况评分';
        }
    }

    // 用户点击评分：WASM 引擎计算下一状态 → 提交后端持久化
    // 前置条件：必须已揭晓答案（否则视为误触，先帮用户揭晓）
    function handleReviewRating(rating) {
        var entry = reviewAppState.queue[reviewAppState.cursor];
        if (!entry || !reviewAppState.wasm) return;
        if (!reviewAppState.revealed) {
            // 未揭晓答案时忽略评分：既避免误按数字键泄漏答案，也避免凭猜测打分
            var hintEl = document.getElementById('reviewStatus');
            if (hintEl) hintEl.textContent = '请先按空格（或点「显示答案」）揭晓答案，再评分';
            return;
        }
        var ctx = reviewCardContext(entry);
        var next;
        try {
            next = JSON.parse(reviewAppState.wasm.fsrs_next_states(ctx.stateJson, 0.9, ctx.daysElapsed));
        } catch (e) {
            var errEl = document.getElementById('reviewStatus');
            if (errEl) errEl.textContent = '引擎计算失败: ' + e;
            return;
        }
        var names = { 1: 'again', 2: 'hard', 3: 'good', 4: 'easy' };
        var chosen = next[names[rating]];
        if (!chosen || !chosen.memory) {
            var badEl = document.getElementById('reviewStatus');
            if (badEl) badEl.textContent = '引擎返回数据异常，已跳过本张';
            reviewAppState.cursor++;
            renderReviewCard();
            return;
        }
        reviewAppState.cursor++;
        renderReviewCard();

        fetch('/api/reviews/submit', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                word_id: entry.item.id,
                rating: rating,
                stability: chosen.memory.stability,
                difficulty: chosen.memory.difficulty,
                interval_days: chosen.interval_days
            })
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

    // ---- 通用小工具 ----
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

    // ---- 统计面板（数据来自 GET /api/reviews/stats）----
    // 渲染今日进度、词库概览、连续天数与记忆保持率
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
        return fetch('/api/reviews/stats')
            .then(function(res) { return res.json(); })
            .then(function(data) {
                if (data && data.code === 200 && data.data) {
                    reviewAppState.stats = data.data;
                    renderStats(data.data);
                }
                return data;
            })
            .catch(function(err) {
                console.error('加载复习统计失败:', err);
                return null;
            });
    }

    // 渲染本轮队列进度条（已完成张数 / 队列总张数）
    function renderProgress() {
        var bar = document.getElementById('statProgressBar');
        if (!bar) return;
        var total = reviewAppState.queue.length;
        var done = Math.min(reviewAppState.cursor, total);
        bar.style.width = (total > 0 ? Math.round(done / total * 100) : 0) + '%';
    }

    // ---- 单词与例句发音（浏览器 Web Speech API，无需额外音频资源）----
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

    // ---- 键盘快捷键 ----
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

    // 初始化复习应用（英语页内容加载完成后调用；其他学科页无 #reviewStatus 自动跳过）
    function initReviewApp() {
        var statusEl = document.getElementById('reviewStatus');
        if (!statusEl) return; // 非英语页，跳过
        statusEl.textContent = '正在加载复习内容...';

        // 键盘快捷键只绑定一次：同一页面内反复切换学科不会重复注册
        if (!window.__guangxueReviewKeysBound) {
            window.__guangxueReviewKeysBound = true;
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

        loadReviewWasm().then(function(wasm) {
            reviewAppState.wasm = wasm;
            return loadReviewQueue(wasm); // 队列加载时已顺带渲染统计面板
        }).then(function() {
            renderReviewCard();
        }).catch(function(err) {
            statusEl.textContent = '复习功能加载失败（需通过服务器访问，并确认已生成 frontend-rust/pkg）: ' + err;
            console.error('复习功能初始化失败:', err);
        });
    }
    // ===================== 复习逻辑结束 =====================
    
    
    // 缓存配置常量定义
    // CACHE_EXPIRY: 缓存过期时间，设置为30天（30天 × 24小时 × 60分钟 × 60秒 × 1000毫秒）
    // 超过30天未访问的缓存将被自动清理，释放存储空间
    const CACHE_EXPIRY = 30 * 24 * 60 * 60 * 1000;
    // CACHE_PREFIX: 缓存键名前缀，用于区分缓存数据和其他localStorage数据
    const CACHE_PREFIX = 'pageCache_';
    // CACHE_META_KEY: 缓存元数据的键名，用于存储每个页面的最后访问时间
    const CACHE_META_KEY = 'pageCache_meta';
    // CACHE_VERSION: 页面缓存结构版本号
    // 用途：当学科页的结构发生不兼容改动时（例如英语页新增「显示答案」按钮），
    // 把版本号 +1，老用户 localStorage 里的旧页面缓存会在启动时被整体清除，
    // 避免出现「新脚本 + 旧页面结构」导致功能不可用
    const CACHE_VERSION = 2;
    // CACHE_VERSION_KEY: 记录当前缓存版本的键名
    const CACHE_VERSION_KEY = 'pageCache_version';
    
    // 获取缓存元数据函数
    // 用途：从localStorage中读取缓存元数据（记录每个页面的最后访问时间）
    // 返回值：包含页面名称和访问时间戳的对象，如果读取失败则返回空对象
    function getCacheMeta() {
        try {
            // 从localStorage中获取元数据JSON字符串
            const meta = localStorage.getItem(CACHE_META_KEY);
            // 如果存在则解析为对象，否则返回空对象
            return meta ? JSON.parse(meta) : {};
        } catch (e) {
            // 捕获解析错误（如JSON格式错误）并输出到控制台
            console.error('读取缓存元数据失败:', e);
            return {};
        }
    }
    
    // 保存缓存元数据函数
    // 用途：将缓存元数据对象序列化后存储到localStorage中
    // 参数：meta - 包含页面名称和访问时间戳的对象
    function saveCacheMeta(meta) {
        try {
            // 将对象转换为JSON字符串并存储到localStorage
            localStorage.setItem(CACHE_META_KEY, JSON.stringify(meta));
        } catch (e) {
            // 捕获存储错误（如存储空间不足）并输出到控制台
            console.error('保存缓存元数据失败:', e);
        }
    }
    
    // 清理过期缓存函数
    // 用途：遍历所有缓存记录，删除超过30天未访问的缓存内容
    // 该函数在页面加载时自动执行，确保存储空间不会被无用数据占用
    function cleanExpiredCache() {
        // 获取当前缓存元数据
        const meta = getCacheMeta();
        // 获取当前时间戳（毫秒）
        const now = Date.now();
        // 标记是否有缓存被清理
        let cleaned = false;
        
        // 遍历元数据中的每个页面记录
        for (const pageName in meta) {
            // 获取该页面的最后访问时间戳
            const lastAccess = meta[pageName];
            // 判断当前时间与最后访问时间的差值是否超过30天
            if (now - lastAccess > CACHE_EXPIRY) {
                // 从localStorage中删除该页面的缓存内容
                localStorage.removeItem(CACHE_PREFIX + pageName);
                // 从元数据对象中删除该页面的记录
                delete meta[pageName];
                // 标记已执行清理操作
                cleaned = true;
                // 输出清理日志到控制台
                console.log('清理过期缓存:', pageName);
            }
        }
        
        // 如果有缓存被清理，则更新元数据到localStorage
        if (cleaned) {
            saveCacheMeta(meta);
        }
    }
    
    // 获取缓存内容函数
    // 用途：从localStorage中读取指定页面的HTML内容
    // 参数：pageName - 页面文件路径（如'pages/english.html'）
    // 返回值：HTML内容字符串，如果不存在或读取失败则返回null
    function getCachedContent(pageName) {
        try {
            // 从localStorage中获取指定键名的缓存内容
            return localStorage.getItem(CACHE_PREFIX + pageName);
        } catch (e) {
            // 捕获读取错误并输出到控制台
            console.error('读取缓存内容失败:', e);
            return null;
        }
    }
    
    // 保存缓存内容函数
    // 用途：将HTML内容存储到localStorage中实现永久缓存
    // 参数：pageName - 页面文件路径，html - 要缓存的HTML内容字符串
    // 特性：当存储空间不足时会自动清理过期缓存后重试
    function saveCachedContent(pageName, html) {
        try {
            // 将HTML内容存储到localStorage中
            localStorage.setItem(CACHE_PREFIX + pageName, html);
        } catch (e) {
            // 判断是否为存储空间不足错误
            if (e.name === 'QuotaExceededError') {
                // 先清理过期缓存释放空间
                cleanExpiredCache();
                try {
                    // 清理后重试存储
                    localStorage.setItem(CACHE_PREFIX + pageName, html);
                } catch (e2) {
                    // 如果仍然失败则输出错误日志
                    console.error('存储空间不足，无法缓存:', pageName);
                }
            } else {
                // 其他错误直接输出日志
                console.error('保存缓存内容失败:', e);
            }
        }
    }
    
    // 更新缓存访问时间函数
    // 用途：记录用户访问某个页面的时间，用于30天过期判断
    // 参数：pageName - 页面文件路径
    // 每次用户点击学科按钮时都会调用此函数更新访问时间
    function updateAccessTime(pageName) {
        // 获取当前缓存元数据
        const meta = getCacheMeta();
        // 更新或添加该页面的访问时间为当前时间戳
        meta[pageName] = Date.now();
        // 将更新后的元数据保存到localStorage
        saveCacheMeta(meta);
    }
    
    // 缓存版本检查函数
    // 用途：缓存结构升级后，一次性清除所有旧版本缓存（含旧版遗留键），
    // 保证用户拿到的学科页结构与当前脚本匹配
    function purgeCacheIfOutdated() {
        try {
            // 版本一致则无需处理
            if (localStorage.getItem(CACHE_VERSION_KEY) === String(CACHE_VERSION)) {
                return;
            }
            // 收集所有本应用写入的缓存键（前缀 pageCache，含 pageCache_meta / pageCache_version）
            const staleKeys = [];
            for (let i = 0; i < localStorage.length; i++) {
                const key = localStorage.key(i);
                if (key && key.indexOf('pageCache') === 0) {
                    staleKeys.push(key);
                }
            }
            for (let j = 0; j < staleKeys.length; j++) {
                localStorage.removeItem(staleKeys[j]);
            }
            localStorage.setItem(CACHE_VERSION_KEY, String(CACHE_VERSION));
            console.log('缓存版本已升级到 v' + CACHE_VERSION + '，清除旧缓存 ' + staleKeys.length + ' 项');
        } catch (e) {
            console.error('缓存版本检查失败:', e);
        }
    }
    
    // 页面加载时先做缓存版本检查，再执行过期缓存清理
    // 确保每次打开页面时都会检查并清理超过30天未访问的缓存
    purgeCacheIfOutdated();
    cleanExpiredCache();
    
    // 默认加载英语页面
    // 用途：页面打开时自动显示英语学科内容，提升用户体验
    const defaultPage = 'pages/english.html';
    // 查找英语学科对应的导航按钮元素
    const defaultRect = document.querySelector('.nav-rect[data-page="' + defaultPage + '"]');
    // 如果找到该元素，则添加active类使其显示为激活状态（灰色背景）
    if (defaultRect) {
        defaultRect.classList.add('active');
    }
    
    // 检查localStorage中是否存在英语页面的缓存
    const defaultCachedHtml = getCachedContent(defaultPage);
    if (defaultCachedHtml) {
        // 如果缓存中存在，直接使用缓存的HTML内容更新内容容器
        // 这样可以实现秒开，无需等待网络请求
        contentContainer.innerHTML = defaultCachedHtml;
        // 输出日志表明使用了缓存加载
        console.log('默认加载英语（缓存）:', defaultPage);
        // 初始化英语页的单词复习应用
        initReviewApp();
    } else {
        // 如果缓存中不存在，先显示加载中的提示文字
        contentContainer.innerHTML = '<p class="placeholder-text">正在加载内容...</p>';
        // 使用fetch API发起网络请求获取英语页面内容
        // fetch兼容HTTPS协议，适合部署到服务器环境
        fetch(defaultPage)
            // 处理响应对象，检查请求是否成功
            .then(function(response) {
                // response.ok为true表示HTTP状态码在200-299范围内
                if (!response.ok) {
                    throw new Error('文件加载失败');
                }
                // 将响应体转换为文本格式
                return response.text();
            })
            // 处理获取到的HTML文本内容
            .then(function(html) {
                // 将HTML内容保存到localStorage实现永久缓存
                saveCachedContent(defaultPage, html);
                // 将HTML内容插入到内容容器中显示
                contentContainer.innerHTML = html;
                // 输出日志表明首次加载并缓存成功
                console.log('默认加载英语（首次）:', defaultPage);
                // 初始化英语页的单词复习应用
                initReviewApp();
            })
            // 捕获并处理加载过程中的错误
            .catch(function(error) {
                // 输出错误信息到控制台用于调试
                console.error('加载错误:', error);
                // 在内容容器中显示错误提示文字
                contentContainer.innerHTML = '<p class="placeholder-text">内容加载失败，请检查文件是否存在</p>';
            });
    }
    
    // 使用forEach遍历每个导航矩形项元素，为每个元素添加点击事件监听器
    // 这些导航按钮对应各个学科（语文、数学、英语等）
    navRects.forEach(function(rect) {
        // 为当前导航矩形项添加点击事件监听器
        // 当用户点击学科按钮时触发回调函数
        rect.addEventListener('click', function() {
            // 再次使用forEach遍历所有导航矩形项，移除每个元素的active类
            // 这样可以清除之前选中按钮的激活状态
            navRects.forEach(function(r) {
                // classList.remove移除指定的CSS类名
                // 移除active类后按钮背景色恢复为白色
                r.classList.remove('active');
            });
            // 为当前被点击的导航矩形项添加active类
            // active类使按钮背景色变为灰色，表示当前选中的学科
            this.classList.add('active');
            
            // 使用getAttribute获取当前元素的data-page属性值
            // data-page属性存储了对应学科HTML文件的路径（如'pages/math.html'）
            const pageName = this.getAttribute('data-page');
            
            // 更新该页面的访问时间为当前时间
            // 用于30天过期缓存清理判断
            updateAccessTime(pageName);
            
            // 检查localStorage中是否存在该页面的缓存内容
            const cachedHtml = getCachedContent(pageName);
            if (cachedHtml) {
                // 如果缓存中存在，直接使用缓存的HTML内容更新内容容器
                // 命中缓存时零带宽消耗，实现瞬间加载
                contentContainer.innerHTML = cachedHtml;
                // 输出日志表明使用了本地缓存
                console.log('使用本地缓存加载:', pageName);
                // 若为英语页则初始化单词复习应用
                initReviewApp();
            } else {
                // 如果缓存中不存在，先显示加载中的提示文字
                contentContainer.innerHTML = '<p class="placeholder-text">正在加载内容...</p>';
                
                // 使用fetch API发起网络请求获取页面内容
                // fetch是现代浏览器标准的HTTP请求方法，兼容HTTPS协议
                fetch(pageName)
                    // 处理响应对象
                    .then(function(response) {
                        // 检查HTTP响应状态是否成功
                        if (!response.ok) {
                            throw new Error('文件加载失败');
                        }
                        // 将响应体转换为文本格式
                        return response.text();
                    })
                    // 处理成功获取的HTML文本内容
                    .then(function(html) {
                        // 将HTML内容保存到localStorage实现永久缓存
                        // 下次访问该页面时可直接从缓存读取，无需网络请求
                        saveCachedContent(pageName, html);
                        // 将HTML内容插入到内容容器中显示
                        contentContainer.innerHTML = html;
                        // 输出日志表明首次加载并缓存成功
                        console.log('首次加载并永久缓存:', pageName);
                        // 若为英语页则初始化单词复习应用
                        initReviewApp();
                    })
                    // 捕获并处理加载过程中发生的错误
                    .catch(function(error) {
                        // 输出错误信息到控制台用于调试
                        console.error('加载错误:', error);
                        // 在内容容器中显示错误提示文字
                        contentContainer.innerHTML = '<p class="placeholder-text">内容加载失败，请检查文件是否存在</p>';
                    });
            }
        });
    });
// 匿名函数结束，作为DOMContentLoaded事件的回调函数
});