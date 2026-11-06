// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
// 页面入口：DOMContentLoaded 在 DOM 树解析完、外部资源还没加载时就触发，比 load 早，
// 能保证取元素 / 挂监听时节点都已就位。
document.addEventListener('DOMContentLoaded', function() {
    
    // 导航项（学科按钮）与内容容器：前者负责切页，后者负责接住加载回来的 HTML
    const navRects = document.querySelectorAll('.nav-rect');
    
    const contentContainer = document.getElementById('contentContainer');
    
    // ===================== 学科模块按需加载 =====================
    // 约定：有独立模块的学科放在 modules/<学科>/ 并导出初始化函数，main.js 只负责
    // 「按需动态加载 + 调用」，不再内联任何学科的业务逻辑（否则它会随学科增多无限膨胀）。
    // 用动态 import() 的好处：只有真正进入该学科页才加载对应模块及其 WASM 引擎，首屏不带学科逻辑。
    const ENGLISH_PAGE = 'modules/english/english.html';
    const MATH_PAGE = 'modules/math/math.html';
    
    // 当前已初始化的学科模块命名空间（切页时用它调可选的 unmount()）
    let activeSubjectModule = null;
    
    // 切页前清理上一个学科模块（导出了 unmount() 才调）。
    // 为什么必须由框架做：学科页会往 body / document 上挂全局状态（例如英语复习页的
    // 「沉浸模式」给 body 挂 is-immersive 隐藏导航栏），换成别的学科后这些状态不会自己
    // 消失，只能由模块自己收回；收口放在这里，是为了让「切页」只有一条路径。
    function teardownSubjectModule() {
        const mod = activeSubjectModule;
        activeSubjectModule = null;
        if (mod && typeof mod.unmount === 'function') {
            try {
                mod.unmount();
            } catch (err) {
                console.error('学科模块清理失败:', err);
            }
        }
    }
    
    // 按 pageName 加载并初始化对应学科模块；尚无独立模块的学科直接返回
    function initSubjectModule(pageName) {
        teardownSubjectModule();
        if (pageName === MATH_PAGE) {
            // 数学模块：数学页的 DOM 结构由 math.html 注入，模块自己负责渲染分支树、
            // 注入 math.css 并绑定交互（initMathApp 幂等，切页回来会按当前 DOM 重建）
            import('./modules/math/math.js').then(function(mod) {
                activeSubjectModule = mod;
                mod.initMathApp();
            }).catch(function(err) {
                const app = document.getElementById('mathApp');
                if (app) app.innerHTML = '<p class="placeholder-text">数学模块加载失败: ' + err + '</p>';
                console.error('数学模块加载失败:', err);
            });
            return;
        }
        if (pageName !== ENGLISH_PAGE) return; // 其余学科仍是 pages/ 下的静态占位页
        import('./modules/english/english.js').then(function(mod) {
            activeSubjectModule = mod;
            mod.initReviewApp();
        }).catch(function(err) {
            // 模块加载失败（路径错 / 网络问题）：就地提示，便于定位
            const statusEl = document.getElementById('reviewStatus');
            if (statusEl) statusEl.textContent = '英语模块加载失败: ' + err;
            console.error('英语模块加载失败:', err);
        });
    }
    
    
    // ===================== 学科页 localStorage 缓存配置 =====================
    // 学科页 HTML 存在 localStorage：键前缀 pageCache_，超过 30 天未访问自动清理。
    // CACHE_VERSION 是「结构与路径」的版本号：当学科页的结构或**路径**发生不兼容改动时 +1，
    // 老用户 localStorage 里的旧页面缓存会在启动时被整体清除，
    // 避免出现「新脚本 + 旧页面结构」导致功能不可用。
    // 历史：v1 初版；v2 英语页新增「显示答案」按钮；v3 英语页迁到 modules/english/；
    //       v4 英语复习界面改版（极简全屏：顶栏统计 / 大字号单词 / 例句高亮 / 粉彩评分按钮）；
    //       v5 英语页顶栏改成「沉浸模式」开关（隐藏站点导航栏），揭晓区改为每条释义一块；
    //       v6 英语页底部新增左下角「今日计划」小字；
    //       v7 英语页新增起始页（中间一颗「开始复习单词」），复习会话改为点击后才建
    //          （原来一挂载就建会话会自动朗读，且首次交互还会补读一遍，同一个词响两次）；
    //       v8 数学页接入独立模块：路径由 pages/math.html 改为 modules/math/math.html；
    //       v9 占位学科页统一包 .page-stub（居中空状态），导航项补键盘语义
    const CACHE_EXPIRY = 30 * 24 * 60 * 60 * 1000;
    const CACHE_PREFIX = 'pageCache_';
    const CACHE_META_KEY = 'pageCache_meta';
    const CACHE_VERSION = 9;
    const CACHE_VERSION_KEY = 'pageCache_version';
    
    // 缓存元数据 { 页面路径: 最后访问时间戳 } 的读写；读失败（脏 JSON / 隐私模式）按「没有元数据」处理
    function getCacheMeta() {
        try {
            const meta = localStorage.getItem(CACHE_META_KEY);
            return meta ? JSON.parse(meta) : {};
        } catch (e) {
            console.error('读取缓存元数据失败:', e);
            return {};
        }
    }
    
    function saveCacheMeta(meta) {
        try {
            localStorage.setItem(CACHE_META_KEY, JSON.stringify(meta));
        } catch (e) {
            console.error('保存缓存元数据失败:', e);
        }
    }
    
    // 启动时清理超过 CACHE_EXPIRY 未访问的学科页缓存，并同步删掉元数据里的对应记录
    function cleanExpiredCache() {
        const meta = getCacheMeta();
        const now = Date.now();
        let cleaned = false;
        
        for (const pageName in meta) {
            const lastAccess = meta[pageName];
            if (now - lastAccess > CACHE_EXPIRY) {
                localStorage.removeItem(CACHE_PREFIX + pageName);
                delete meta[pageName];
                cleaned = true;
            }
        }
        
        if (cleaned) {
            saveCacheMeta(meta);
        }
    }
    
    // 读某个学科页的缓存 HTML；没有缓存 / localStorage 读不到（隐私模式）时返回 null
    function getCachedContent(pageName) {
        try {
            return localStorage.getItem(CACHE_PREFIX + pageName);
        } catch (e) {
            console.error('读取缓存内容失败:', e);
            return null;
        }
    }
    
    // 写学科页缓存；配额满（QuotaExceededError）时先清过期缓存再重试一次
    function saveCachedContent(pageName, html) {
        try {
            localStorage.setItem(CACHE_PREFIX + pageName, html);
        } catch (e) {
            if (e.name === 'QuotaExceededError') {
                cleanExpiredCache();
                try {
                    localStorage.setItem(CACHE_PREFIX + pageName, html);
                } catch (e2) {
                    console.error('存储空间不足，无法缓存:', pageName);
                }
            } else {
                console.error('保存缓存内容失败:', e);
            }
        }
    }
    
    // 记一次访问时间，供 30 天过期判断（每次点学科按钮都会调）
    function updateAccessTime(pageName) {
        const meta = getCacheMeta();
        meta[pageName] = Date.now();
        saveCacheMeta(meta);
    }
    
    // 缓存版本检查：版本号对不上（= 结构或路径变过）就一次性清掉所有本应用写入的键
    // （前缀 pageCache，含 pageCache_meta / pageCache_version 这类旧版遗留键），
    // 保证用户拿到的学科页结构与当前脚本匹配
    function purgeCacheIfOutdated() {
        try {
            if (localStorage.getItem(CACHE_VERSION_KEY) === String(CACHE_VERSION)) {
                return;
            }
            // 先收集再删：边遍历 localStorage 边删会漏项
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
        } catch (e) {
            console.error('缓存版本检查失败:', e);
        }
    }
    
    // 启动时先做版本失效（可能整体清空），再清理超过 30 天未访问的缓存
    purgeCacheIfOutdated();
    cleanExpiredCache();
    
    // ===================== 学科页加载 =====================
    // 当前显示的学科页路径与请求序号：
    //   · currentPage 是「已经渲染出来」的页，pendingPage 是「最近一次请求的目标」。
    //     两者要分开：数学请求还在飞的时候点回英语，currentPage 仍是英语，
    //     若只看 currentPage 就会把这次点击当成「重复点当前页」吞掉，
    //     随后数学的慢响应回来反而把英语页覆盖掉（实测踩过）。
    //   · pageSeq 用来丢弃过期响应——先点数学再点英语时，数学的慢请求不能覆盖英语页。
    let currentPage = null;
    let pendingPage = null;
    let pageSeq = 0;

    // 学科名映射（导航项文字 → 页面标题），用于 document.title
    const pageTitles = {};
    navRects.forEach(function(rect, i) {
        pageTitles[rect.getAttribute('data-page')] = rect.textContent.trim();
        // 给每个标签一个稳定 id，供 tabpanel 的 aria-labelledby 指过来
        if (!rect.id) rect.id = 'navSubject' + i;
    });

    // 同步导航高亮与读屏状态（视觉上的 .active 与 aria-selected 必须始终一致）
    function setActiveNav(pageName) {
        const navBox = document.querySelector('.nav-items');
        navRects.forEach(function(rect) {
            const on = rect.getAttribute('data-page') === pageName;
            rect.classList.toggle('active', on);
            rect.setAttribute('aria-selected', on ? 'true' : 'false');
            // roving tabindex：整组标签只保留一个 Tab 停靠点（当前选中项），
            // 组内用左右方向键移动 —— WAI-ARIA 对 tablist 的推荐做法，
            // 否则 9 个学科会把 Tab 键的第一段路撑得很长。
            rect.tabIndex = on ? 0 : -1;
            if (on) {
                contentContainer.setAttribute('aria-labelledby', rect.id);
                // 窄屏导航条是横向滚动的：选中项可能滚出可视区，这里把它带回可视范围
                if (navBox && navBox.scrollWidth > navBox.clientWidth + 1) {
                    rect.scrollIntoView({ block: 'nearest', inline: 'nearest' });
                }
            }
        });
    }

    // 把 HTML 注入内容区并初始化模块；淡入动画（main.css 的 pageIn）与滚动复位都在这里，
    // 保证「缓存命中」与「网络加载」两条路径的表现完全一致。
    function applyPageHtml(pageName, html) {
        currentPage = pageName;
        pendingPage = pageName;
        // 先让上一页收回全局状态（如英语复习页挂在 body 上的 is-immersive），再替换 DOM：
        // 否则新页面会先按沉浸布局（导航隐藏、内容贴顶）画一帧，再滑回正常位置。
        teardownSubjectModule();
        contentContainer.removeAttribute('aria-busy');
        contentContainer.innerHTML = html;
        // 移除再强制重排后重新加类：同一段动画才能连续播放
        contentContainer.classList.remove('is-page-in');
        void contentContainer.offsetWidth;
        contentContainer.classList.add('is-page-in');
        document.title = (pageTitles[pageName] || '学科') + ' · 大冬呱';
        window.scrollTo(0, 0);
        initSubjectModule(pageName);
    }

    // 统一的取页流程：缓存优先 → fetch → 注入。
    // 只有「目标页 = 正在请求页 = 已渲染页」时才早退（真正的重复点击），
    // 这样英语复习不会被重建，而请求进行中的回切仍然生效。
    function loadPage(pageName, force) {
        if (!force && pageName === currentPage && pageName === pendingPage) return;
        const seq = ++pageSeq;
        pendingPage = pageName;
        setActiveNav(pageName);
        updateAccessTime(pageName);

        const cachedHtml = getCachedContent(pageName);
        if (cachedHtml) {
            applyPageHtml(pageName, cachedHtml);
            return;
        }

        contentContainer.innerHTML = '<p class="placeholder-text">正在加载内容...</p>';
        contentContainer.setAttribute('aria-busy', 'true');
        fetch(pageName)
            .then(function(response) {
                if (!response.ok) {
                    throw new Error('文件加载失败');
                }
                return response.text();
            })
            .then(function(html) {
                if (seq !== pageSeq) return; // 已切到别的学科，丢弃这次结果
                saveCachedContent(pageName, html);
                applyPageHtml(pageName, html);
            })
            .catch(function(error) {
                if (seq !== pageSeq) return;
                console.error('加载错误:', error);
                contentContainer.removeAttribute('aria-busy');
                // 失败时给一条能自救的出路。原来只丢一句「请检查文件是否存在」：
                // 用户既看不懂，也没有下一步（整页刷新会把其他学科的状态一起丢掉），
                // 而且错误对象只该进控制台，不该出现在界面上。
                contentContainer.innerHTML =
                    '<div class="load-error">' +
                        '<p class="load-error-title">这个科目的内容没能加载出来</p>' +
                        '<p class="load-error-hint">可能是网络断了，或者文件被移动过。稍等再试一次，通常就好了。</p>' +
                        '<button type="button" class="load-error-retry">重新加载</button>' +
                    '</div>';
                var retryBtn = contentContainer.querySelector('.load-error-retry');
                if (retryBtn) {
                    // force=true：绕过「目标页 = 当前页」的早退，否则重试会被当成重复点击吞掉
                    retryBtn.addEventListener('click', function() {
                        loadPage(pageName, true);
                    });
                }
            });
    }

    // 默认打开英语：按「先查缓存、miss 再 fetch」加载
    // （命中缓存时零网络请求，秒开；首次加载完成后写回缓存，之后同样走缓存）
    loadPage(ENGLISH_PAGE, true);

    // 点导航项切学科：加载对应页面；键盘 Enter / 空格等价于点击，
    // 左右方向键在同组标签之间移动焦点（读屏与纯键盘用户的常规预期）
    navRects.forEach(function(rect) {
        rect.addEventListener('click', function() {
            loadPage(this.getAttribute('data-page'), false);
        });

        rect.addEventListener('keydown', function(e) {
            if (e.key === 'Enter' || e.key === ' ' || e.key === 'Spacebar') {
                e.preventDefault();
                loadPage(this.getAttribute('data-page'), false);
                return;
            }
            if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
                e.preventDefault();
                const list = Array.prototype.slice.call(navRects);
                const i = list.indexOf(this);
                const step = e.key === 'ArrowRight' ? 1 : -1;
                const next = list[(i + step + list.length) % list.length];
                if (next) next.focus();
            }
        });
    });
    // ===================== 头像 = 个人中心入口 =====================
    // 入口位置暂时从右上角的文字链接改成左上角头像（用户要求）。想改回文字入口时，
    // 把 index.html 里的 #navAccount 加回去、这段恢复成原来的写法即可。
    //
    // 登录态是服务端（Rust 认证服务 /api/auth/*，见 backend-rust/README.md）下发的
    // httpOnly Cookie，JS 读不到，所以只能问服务端一次：GET /api/auth/me
    // 账号服务没起来时静默降级：头像照样能点（账号页里可以登录），只是不点亮状态点。
    //
    // 注意：导航栏上的「退出登录」按钮随文字入口一起撤掉了，退出改在个人中心里做。

    // 过场时长（毫秒）：与 main.css 的 .avatar-zoom 的过渡一致
    var ZOOM_MS = 460;

    // 是否应当减少动态效果（无障碍）：系统开启时不做任何过场，直接跳转
    function prefersReducedMotion() {
        return !!(window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches);
    }

    // clip-path 圆形裁剪是否可用；不可用（老浏览器）就退化成直接跳转
    function canZoom() {
        return !!(window.CSS && window.CSS.supports && window.CSS.supports('clip-path', 'circle(0px at 0px 0px)'));
    }

    // 点击头像 → 进入个人中心。整段只有一件事：**白底从头像正中扩散开**。
    //
    // 头像自己**不动**：它在首页导航栏里是 64×64 @ (30, 18)，而个人中心侧栏那颗
    // （account.css 的 .acc-sidebar，padding 18px 30px + 64×64）也是 64×64 @ (30, 18) ——
    // 两边逐像素重合，所以换页时它就在原地，不需要任何形变或位移。
    // 2026-10 之前这里还有一段「缩小飞进侧栏」的形变（morphTarget / normalizeTarget
    // 那一整套，从 account.css 里读几何再折算），几何对齐之后整段都删了。
    //
    // ⚠️ 遮罩的第一帧要把过渡关掉：否则起点会落在样式表的兜底值（圆心在屏幕正中），
    //    圆就从屏幕中心长出来了（实测踩过一次）。
    function spreadOverlay() {
        var btn = document.getElementById('navAvatar');
        var layer = document.getElementById('avatarZoom');
        var target = 'account/?from=avatar';

        if (!btn || !layer || !canZoom() || prefersReducedMotion()) {
            window.location.href = target; // 直接跳，别让人白等
            return;
        }

        var rect = btn.getBoundingClientRect();
        var cx = rect.left + rect.width / 2;
        var cy = rect.top + rect.height / 2;
        var dx = Math.max(cx, window.innerWidth - cx);
        var dy = Math.max(cy, window.innerHeight - cy);
        var radius = Math.ceil(Math.sqrt(dx * dx + dy * dy)) + 32;
        var origin = ' at ' + cx + 'px ' + cy + 'px)';

        // 起点：圆心落在头像正中、半径 0（屏幕上看不到东西）
        layer.style.transition = 'none';
        layer.classList.add('is-armed');
        layer.style.clipPath = 'circle(0px' + origin;
        void layer.offsetWidth;

        // 扩散。换页等它的过渡**真正结束**（而不是掐个 460ms 的定时器）：
        // 两者同为 ZOOM_MS，浏览器最后一次绘制大约在 450ms —— 那时还差一点点，
        // 换页后是 100% 状态，那一点点就是肉眼看到的「跳一下」。
        layer.style.transition = '';
        layer.style.clipPath = 'circle(' + radius + 'px' + origin;
        goWhenSettled(layer, function() {
            window.location.href = target;
        });
    }

    // 等一个元素的过渡跑完再执行（只认一个属性名：那几条的时长一致，等一个就够）。
    // 兜底：过渡被跳过（元素被隐藏等）或事件丢失时，ZOOM_MS + 120 后照样执行。
    function goWhenSettled(el, done) {
        var settled = false;
        var fire = function() {
            if (settled) return;
            settled = true;
            done();
        };
        el.addEventListener('transitionend', function onEnd(e) {
            if (e.propertyName !== 'clip-path') return;
            el.removeEventListener('transitionend', onEnd);
            fire();
        });
        window.setTimeout(fire, ZOOM_MS + 120);
    }

    // 只把遮罩复位（头像不参与过场，没什么要还原的）。
    // 换页之后这份 DOM 就没了，所以只有浏览器用「前进后退缓存」（bfcache）把首页
    // 整页恢复回来时才走得到这里 —— 那时遮罩还停在「盖满全屏」的状态上。
    function resetAvatarOverlay() {
        var layer = document.getElementById('avatarZoom');
        if (!layer) return;
        layer.classList.remove('is-armed');
        // 先关过渡再清几何：否则清除行内 clip-path 会朝样式表的兜底值再补一段过渡
        layer.style.transition = 'none';
        layer.style.clipPath = '';
        void layer.offsetWidth;
        layer.style.transition = '';
    }

    // ===================== 反向转场：从个人中心返回 =====================
    // 账号页那边先在本地把组件淡出，再带 ?from=account 跳回本页。
    // 本页要做的只是「倒放那块遮罩」：一进来（第一帧之前）就用它盖住整屏，等页面在
    // 遮罩下面画好，再把圆圈从**首页头像那颗的位置**缩到 0、露出主界面。
    //
    // 为什么这里比原来短得多：头像在两边都是 64×64 @ (30, 18)，换页时它根本没动过，
    // 所以不需要「把侧栏那颗还原成导航栏那颗」那一整套几何搬运（2026-10 随形变一起删了）。
    var FROM_ACCOUNT = /[?&]from=account(&|=|$)/.test(window.location.search);

    // 前置条件：导航栏得**真的**看得见（缩圈的圆心要落在头像上）。
    //
    // 判据不是「沉没沉浸」这个偏好本身，而是那份偏好**有没有被写下来过**：
    // 英语模块只在「进过复习页」时才动 `reviewImmersive` 这个键（applyImmersive 挂
    // body.is-immersive 的那一刻就带着这个偏好走，见 modules/english/english.js），
    // 而导航栏藏不藏是**跟着这个键一起**发生的。所以：
    //   · 键不存在（多数情况：没进过英语复习页）→ 导航栏就是显示着的 → 有落点；
    //   · 键存在且不是 '1' → 用户自己关掉了沉浸 → 导航栏显示 → 有落点；
    //   · 键就是 '1' → 沉浸开着 → 导航栏连头像一起隐藏 → 没落点，别播。
    // ⚠️ 别再往「null 也算没有落点」那边写。上一版就是 `raw === null → false`，
    //    注释里明写着「没有记录 = 默认开启沉浸」，可导航栏在没有记录时恰恰是**显示**的 ——
    //    于是全新会话（localStorage 干净）点「返回主页面」永远退化成普通跳转
    //    （实测：首页的遮罩始终没加过 is-armed）。空串同理：等于没记录。
    //    真到了沉浸状态、导航栏没排上版时，下面那道 rect.width 的检查会兜住。
    function returnHasLandingSpot() {
        var raw = null;
        try {
            raw = window.localStorage.getItem('reviewImmersive');
        } catch (e) {
            raw = null; // 隐私模式下读不到 → 与「没有记录」同义：导航栏显示着
        }
        return raw !== '1';
    }

    // 等页面在遮罩下面画好（load + 两帧）再开始收圈，否则缩圈时露出的是还没画完的页面
    function whenPageReady(fn) {
        var run = function() {
            window.requestAnimationFrame(function() {
                window.requestAnimationFrame(fn);
            });
        };
        if (document.readyState === 'complete') run();
        else window.addEventListener('load', run);
    }

    // 返回 false 表示「这次不播倒放」（调用方负责把 is-returning 摘掉，恢复正常显示）
    function playReturnAnimation() {
        var layer = document.getElementById('avatarZoom');
        if (!layer || !canZoom() || prefersReducedMotion() || !returnHasLandingSpot()) {
            return false;
        }

        // 圆心 = 首页头像那颗的中心。取不到就说明导航栏还没排上版，不播。
        var btn = document.getElementById('navAvatar');
        var rect = btn ? btn.getBoundingClientRect() : null;
        if (!rect || !rect.width) return false;
        var cx = rect.left + rect.width / 2;
        var cy = rect.top + rect.height / 2;
        var dx = Math.max(cx, window.innerWidth - cx);
        var dy = Math.max(cy, window.innerHeight - cy);
        var radius = Math.ceil(Math.sqrt(dx * dx + dy * dy)) + 32;
        var origin = ' at ' + cx + 'px ' + cy + 'px)';

        // ---- 第一帧：无过渡，直接摆成「账号页最后一帧」——遮罩整屏盖住 ----
        layer.style.transition = 'none';
        layer.classList.add('is-armed');
        layer.style.clipPath = 'circle(' + radius + 'px' + origin;
        void layer.offsetWidth;

        // 遮罩已经盖住了：让页面内容正常显示（它会一直待在遮罩下面，等圆圈缩小时露出来）
        document.documentElement.classList.remove('is-returning');

        whenPageReady(function() {
            // ---- 收圈 ----
            layer.style.transition = '';
            layer.style.clipPath = 'circle(0px' + origin;

            goWhenSettled(layer, function() {
                resetAvatarOverlay();

                // 参数用完就抹掉：刷新时不会再播一遍（URL 也干净）
                if (window.history && window.history.replaceState) {
                    window.history.replaceState(null, '', window.location.pathname);
                }
            });
        });

        return true;
    }

    // 问一次登录态：只用来决定头像的状态点与提示文案（点了都能进个人中心）
    function renderAvatarState() {
        var btn = document.getElementById('navAvatar');
        if (!btn) return;

        function setTitle(text) {
            btn.title = text;
            btn.setAttribute('aria-label', text);
        }

        setTitle('个人中心'); // 请求还没回来 / 失败时的兜底

        fetch('/api/auth/me', { credentials: 'same-origin' })
            .then(function(response) {
                if (!response.ok) {
                    throw new Error('未登录');
                }
                return response.json();
            })
            .then(function(payload) {
                var user = payload && payload.data ? payload.data.user : null;
                if (!user) {
                    throw new Error('响应里没有用户信息');
                }
                btn.classList.add('is-signed');
                // 管理员与超管都要标出来（P0-5 之前只认 'admin'，超管会看不到标记）
                var roleTag = user.role === 'super_admin' ? '（超级管理员）'
                    : (user.role === 'admin' ? '（管理员）' : '');
                setTitle((user.username || user.email) + roleTag + ' · 个人中心');
            })
            .catch(function() {
                btn.classList.remove('is-signed');
                setTitle('登录 / 注册 · 个人中心');
            });
    }

    var navAvatarEl = document.getElementById('navAvatar');
    if (navAvatarEl) {
        navAvatarEl.addEventListener('click', spreadOverlay);
    }

    // ===================== 站名 → 法律声明浮层 =====================
    // 2026-10 用户要求转场「直接覆盖在原有画面上」：法律页不再整页跳走，而是以 iframe
    // 浮层滑进来 —— 原页面原样垫在下面，浮层里法律页的背衬淡入盖住它、声明侧栏从右
    // 滑入、滑到 3/4 时左栏从它下面同向滑出（动画在 legal.css，与独立打开法律页时
    // 同一套 keyframes）。浮层里点「← 返回首页」→ 播退场 → legal.js 调这里的
    // __closeLegalOverlay 关掉浮层，原页面露出来，一步网络都没走。
    //
    // ⚠️ 站名的 href 保持 legal/index.html?from=logo：JS 挂了 / 浮层还没就绪时
    //    openLegalOverlay 返回 false、不拦点击，链接按原生方式跳过去（NOTICE 附加
    //    条款第 3 条：入口必须能直达法律页）。
    // ⚠️ 只拦普通左键：带修饰键（Ctrl / Cmd / Shift / Alt，用户要新开标签页）、
    //    非左键一律放行。
    var legalFrame = document.getElementById('legalFrame');
    var legalFrameReady = false;
    if (legalFrame) {
        // visibility:hidden 的 iframe 照常加载渲染：页面打开时它就备好了
        legalFrame.addEventListener('load', function() {
            legalFrameReady = true;
        });
    }

    // 打开时把宿主内容设为 inert：键盘焦点不再落到垫在下面的导航与学科页上
    function setHostInert(on) {
        var hosts = [document.querySelector('.rectangle'), document.getElementById('contentContainer')];
        for (var i = 0; i < hosts.length; i++) {
            if (!hosts[i]) continue;
            if (on) hosts[i].setAttribute('inert', '');
            else hosts[i].removeAttribute('inert');
        }
    }

    // 返回 false = 这次没开成（浮层没就绪），调用方放行原生跳转
    function openLegalOverlay() {
        if (!legalFrame || !legalFrameReady) return false;
        var doc = legalFrame.contentDocument;
        if (!doc || !doc.documentElement) return false;
        // 复位上次退场留下的类：is-staging 摘掉再加重播入场；is-leaving 不摘会停在
        // 退场终帧（声明滑走、左栏透明）—— 上次关闭时已清，这里双保险
        doc.documentElement.classList.remove('is-staging', 'is-leaving');
        // 复位 iframe 内 legal.js 的防重入标记：iframe 不重新加载，它的 IIFE 状态会
        // 一直留着 —— 不复位的话第二次打开浮层后两个返回链接永久失效（压力测试 S3
        // 踩过：连开第二轮点返回毫无反应，浮层只能靠外部关）
        try {
            if (doc.defaultView && typeof doc.defaultView.__legalReset === 'function') {
                doc.defaultView.__legalReset();
            }
        } catch (e) { /* 跨域 / 老浏览器：忽略 */ }
        setHostInert(true);
        legalFrame.classList.add('is-open');
        // 加 is-staging 触发两段式入场（与 legal/index.html 头里的内联脚本同一个类）
        doc.documentElement.classList.add('is-staging');
        try { legalFrame.focus(); } catch (e) { /* 老浏览器不抛也行 */ }
        return true;
    }

    // 浮层里播完退场后由 legal.js 调用（同源，直接调）：关掉浮层、放回原页面焦点，
    // 必要时再跳转 —— 「在首页开浮层却点了返回个人中心」，关掉浮层后父页面跳去 account/。
    window.__closeLegalOverlay = function(navigateTo) {
        if (legalFrame) {
            legalFrame.classList.remove('is-open');
            var doc = legalFrame.contentDocument;
            if (doc && doc.documentElement) {
                doc.documentElement.classList.remove('is-staging', 'is-leaving');
            }
        }
        setHostInert(false);
        if (navigateTo) {
            window.location.href = navigateTo;
        }
    };

    var navLogoEl = document.querySelector('.nav-logo');
    if (navLogoEl) {
        navLogoEl.addEventListener('click', function(e) {
            if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
            if (openLegalOverlay()) e.preventDefault();
        });
    }

    // 带着 ?from=account 回来（= 刚从个人中心点返回）→ 播反向过场（倒放）。
    // 播不了（沉浸模式 / 减少动效 / 不支持 clip-path）时要把 is-returning 摘掉，
    // 否则首页会一直停在 index.html 里那段内联脚本设的「先别画内容」状态。
    if (FROM_ACCOUNT && !playReturnAnimation()) {
        document.documentElement.classList.remove('is-returning');
    }

    renderAvatarState();

    // ===================== 预取个人中心与法律声明页 =====================
    // 主界面加载完之后（等浏览器空闲）再悄悄把这两个特殊页的文档与资源拉进缓存，
    // 这样点头像 / 点站名的时候不用现等网络，过场与新页面能连成一片。
    //
    // 为什么不用 main.js 那套 localStorage 页缓存（pageCache_）：那套是给「学科页片段」
    // 用的 —— 这两个都是**独立文档**，个人中心的内容还依赖登录态，必须到服务端实时问
    // （GET /api/auth/me），把 HTML 缓存下来反而会把登录态缓存错。
    // 这里只是让浏览器把资源缓存住（缓存头由 dev-server / Nginx 负责发对）。
    // 法律页也要预取：点站名的过场是「先淡出再跳」，跳过去那一下文档还在路上的话，
    // 淡入会淡在一张还没画出来的白纸上。
    function prefetchSpecialPages() {
        [
            { href: 'account/', as: '' },
            { href: 'account/account.css', as: 'style' },
            { href: 'account/account.js', as: 'script' },
            { href: 'legal/index.html', as: '' },
            { href: 'legal/legal.css', as: 'style' },
            { href: 'legal/legal.js', as: 'script' }
        ].forEach(function(item) {
            var link = document.createElement('link');
            link.rel = 'prefetch';
            link.href = item.href;
            if (item.as) link.as = item.as;
            document.head.appendChild(link);
        });
    }

    if (window.requestIdleCallback) {
        window.requestIdleCallback(prefetchSpecialPages, { timeout: 3000 });
    } else {
        window.setTimeout(prefetchSpecialPages, 1200); // 老浏览器：等首屏稳了再拉
    }

    // 从个人中心返回时浏览器可能直接用「前进后退缓存」（bfcache）恢复页面：
    // 那时脚本不会重跑，展开的遮罩还停在原样 —— 恢复时复位，并顺手重新问一次登录态
    // （登录/退出后返回都可能变化）。浮层开着时被恢复也一并关掉：恢复出来的页面
    // 不该被一页已经播完入场动画的浮层盖着。
    window.addEventListener('pageshow', function(event) {
        if (!event.persisted) return;
        if (legalFrame && legalFrame.classList.contains('is-open')) {
            window.__closeLegalOverlay(null);
        }
        resetAvatarOverlay();
        renderAvatarState();
    });

    // 调试口（与 account/account.js 的 window.__guangxueAccount 同一个用意）：
    // 过场只剩「遮罩扩散 / 收圈」这一步，出问题时需要能在控制台里手动播一遍看。只读、无副作用。
    window.__guangxue = {
        spreadOverlay: spreadOverlay,
        playReturnAnimation: playReturnAnimation,
        resetAvatarOverlay: resetAvatarOverlay
    };

});
