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
        if (pageName !== ENGLISH_PAGE) return; // 目前只有英语有独立模块
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
    //          （原来一挂载就建会话会自动朗读，且首次交互还会补读一遍，同一个词响两次）
    const CACHE_EXPIRY = 30 * 24 * 60 * 60 * 1000;
    const CACHE_PREFIX = 'pageCache_';
    const CACHE_META_KEY = 'pageCache_meta';
    const CACHE_VERSION = 7;
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
    
    // 默认打开英语：点亮它的导航项，再按「先查缓存、miss 再 fetch」加载页面
    // （命中缓存时零网络请求，秒开；首次加载完成后写回缓存，之后同样走缓存）
    const defaultPage = ENGLISH_PAGE;
    const defaultRect = document.querySelector('.nav-rect[data-page="' + defaultPage + '"]');
    if (defaultRect) {
        defaultRect.classList.add('active');
    }
    
    const defaultCachedHtml = getCachedContent(defaultPage);
    if (defaultCachedHtml) {
        contentContainer.innerHTML = defaultCachedHtml;
        initSubjectModule(defaultPage);
    } else {
        // 加载中先给占位文案，避免内容区空着
        contentContainer.innerHTML = '<p class="placeholder-text">正在加载内容...</p>';
        fetch(defaultPage)
            .then(function(response) {
                if (!response.ok) {
                    throw new Error('文件加载失败');
                }
                return response.text();
            })
            .then(function(html) {
                saveCachedContent(defaultPage, html);
                contentContainer.innerHTML = html;
                initSubjectModule(defaultPage);
            })
            .catch(function(error) {
                console.error('加载错误:', error);
                contentContainer.innerHTML = '<p class="placeholder-text">内容加载失败，请检查文件是否存在</p>';
            });
    }
    
    // 点导航项切学科：先摘掉其他项的 active、给当前项点上，再按与默认页同一套
    // 「先查缓存、miss 再 fetch」流程换内容，并同步访问时间（供 30 天过期判断）
    navRects.forEach(function(rect) {
        rect.addEventListener('click', function() {
            navRects.forEach(function(r) {
                r.classList.remove('active');
            });
            this.classList.add('active');
            
            // data-page 存的就是该学科页的路径（如 'pages/math.html'）
            const pageName = this.getAttribute('data-page');
            updateAccessTime(pageName);
            
            const cachedHtml = getCachedContent(pageName);
            if (cachedHtml) {
                contentContainer.innerHTML = cachedHtml;
                initSubjectModule(pageName);
            } else {
                contentContainer.innerHTML = '<p class="placeholder-text">正在加载内容...</p>';
                fetch(pageName)
                    .then(function(response) {
                        if (!response.ok) {
                            throw new Error('文件加载失败');
                        }
                        return response.text();
                    })
                    .then(function(html) {
                        saveCachedContent(pageName, html);
                        contentContainer.innerHTML = html;
                        initSubjectModule(pageName);
                    })
                    .catch(function(error) {
                        console.error('加载错误:', error);
                        contentContainer.innerHTML = '<p class="placeholder-text">内容加载失败，请检查文件是否存在</p>';
                    });
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

    // 带着 ?from=account 回来（= 刚从个人中心点返回）→ 播反向过场（倒放）。
    // 播不了（沉浸模式 / 减少动效 / 不支持 clip-path）时要把 is-returning 摘掉，
    // 否则首页会一直停在 index.html 里那段内联脚本设的「先别画内容」状态。
    if (FROM_ACCOUNT && !playReturnAnimation()) {
        document.documentElement.classList.remove('is-returning');
    }

    renderAvatarState();

    // ===================== 预取个人中心 =====================
    // 主界面加载完之后（等浏览器空闲）再悄悄把个人中心的文档与资源拉进缓存，
    // 这样点头像的时候不用现等网络，过场与新页面能连成一片。
    //
    // 为什么不用 main.js 那套 localStorage 页缓存（pageCache_）：那套是给「学科页片段」
    // 用的 —— 个人中心是一份**独立文档**，而且内容依赖登录态，必须到服务端实时问
    // （GET /api/auth/me），把 HTML 缓存下来反而会把登录态缓存错。
    // 这里只是让浏览器把资源缓存住（缓存头由 dev-server / Nginx 负责发对）。
    function prefetchAccountPage() {
        ['account/', 'account/account.css', 'account/account.js'].forEach(function(href, i) {
            var link = document.createElement('link');
            link.rel = 'prefetch';
            link.href = href;
            if (i === 1) link.as = 'style';
            if (i === 2) link.as = 'script';
            document.head.appendChild(link);
        });
    }

    if (window.requestIdleCallback) {
        window.requestIdleCallback(prefetchAccountPage, { timeout: 3000 });
    } else {
        window.setTimeout(prefetchAccountPage, 1200); // 老浏览器：等首屏稳了再拉
    }

    // 从个人中心返回时浏览器可能直接用「前进后退缓存」（bfcache）恢复页面：
    // 那时脚本不会重跑，展开的遮罩还停在原样 —— 恢复时复位，
    // 并顺手重新问一次登录态（登录/退出后返回都可能变化）。
    window.addEventListener('pageshow', function(event) {
        if (!event.persisted) return;
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
