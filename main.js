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

    // 过场时长（毫秒）：与 main.css 的 .avatar-zoom / .rounded-square.is-flying 过渡一致
    var ZOOM_MS = 460;
    var zoomPlaying = false;

    // 是否应当减少动态效果（无障碍）：系统开启时不做任何过场，直接跳转
    function prefersReducedMotion() {
        return !!(window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches);
    }

    // 导航栏那颗头像当前的几何与外观（它本身是 :hover / :active 会缩放的按钮，
    // 所以要把「正在缩放」那一下算进去，并顺手把 transform 折进宽高）。
    // 返回的 width/height 是**含白边的外框尺寸**，与目标那颗的 offsetWidth 同一口径。
    function navAvatarBox() {
        var btn = document.getElementById('navAvatar');
        if (!btn) return null;
        var r = btn.getBoundingClientRect();
        var sx = 1, sy = 1;
        var m = /matrix\(([^)]+)\)/.exec(window.getComputedStyle(btn).transform || '');
        if (m) {
            var p = m[1].split(',');
            sx = Math.abs(parseFloat(p[0])) || 1;
            sy = Math.abs(parseFloat(p[3])) || 1;
        }
        return {
            left: r.left, top: r.top,
            width: r.width * sx, height: r.height * sy,
            radius: window.getComputedStyle(btn).borderTopLeftRadius
        };
    }

    // 头像形变的目标几何 = 个人中心侧栏顶部那颗头像（.acc-sidebar-avatar）。
    //
    // 早先这里是一组写死的数字（还配了一段「必须与 account.css 对得上」的注释）；
    // 2026-10 个人中心改成侧栏布局时，顺手改成**从样式表里把规则读出来再算**：
    //   ① 侧栏内边距（.acc-sidebar 的 padding，取上下左右里最小的那个当左/上内边距）
    //   ② 那颗头像自己的尺寸、白边、圆角（.acc-sidebar-avatar）
    // 为什么会需要它：窄屏（≤920px）下侧栏横过来、头像会缩到 36×36，写死的 40 会差 4px。
    // 改 account.css 里的内边距或头像尺寸时**不用再来改这里**。
    //
    // 为什么用「临时元素 + cssRules」而不是 getComputedStyle：那个元素在另一个文档（account/）里，
    // 这边取不到；而临时元素在本页只能拿到**本页样式表**算出来的值 —— 要量的是 account.css 那两条规则，
    // 所以直接把规则文本读出来自己解析，反而更直接、也不受「探针元素继承了什么」影响。
    // 元素拿完当帧就删，不会闪。
    function morphTarget() {
        var nav = navAvatarBox();
        if (!nav) return null;

        var probe = document.createElement('div');
        try {
            probe.style.cssText = 'position:fixed;left:-9999px;top:0;visibility:hidden;pointer-events:none;';
            probe.innerHTML = '<div class="acc-sidebar" style="width:200px"></div>' +
                '<div class="acc-sidebar-avatar"><img alt=""></div>';
            document.body.appendChild(probe);

            var chunks = collectStyleText();
            var sidebar = ruleBody(chunks, '.acc-sidebar');
            var avatar = ruleBody(chunks, '.acc-sidebar-avatar');
            if (!sidebar || !avatar) return null;   // 样式表还没到（首页是预读的）→ 退化成直接跳转

            // 侧栏是 flex 纵列、内边距对称：左右内边距就是头像的左边距
            //（padding 简写：一个值 = 四边相同，两个值 = 上下、左右）
            var pad = (/(?:^|;)\s*padding\s*:\s*([^;]+)/.exec(sidebar) || [])[1] || '';
            var parts = pad.split(/\s+/).filter(function (s) { return s; });
            var padY = parseFloat(parts[0]) || 0;
            var padX = parts.length > 1 ? (parseFloat(parts[1]) || 0) : padY;

            var border = parseFloat((/(?:^|;)\s*border(?:-top)?(?:-width)?\s*:\s*([\d.]+)px/.exec(avatar) || [])[1]) || 0;
            // 外框尺寸：account.css 那边给 .acc-sidebar-avatar 写了 box-sizing: border-box，
            // 所以读到的 width/height 就是**含白边**的外框尺寸，不用再加 border * 2
            //（改那边 box-sizing 的话这里要跟着改）。
            var w = parseFloat((/(?:^|;)\s*width\s*:\s*([\d.]+)px/.exec(avatar) || [])[1]) || 0;
            var h = parseFloat((/(?:^|;)\s*height\s*:\s*([\d.]+)px/.exec(avatar) || [])[1]) || 0;
            var radius = ((/(?:^|;)\s*border-radius\s*:\s*([^;]+)/.exec(avatar) || [])[1] || '').trim();

            if (!w || !h) return null;
            return normalizeTarget(nav, {
                left: padX, top: padY, width: w, height: h, radius: radius
            }, nav.radius);
        } finally {
            // 不管走哪条路（包括上面提前 return）都要把探针摘掉
            if (probe.parentNode) probe.parentNode.removeChild(probe);
        }
    }

    // 把本页所有样式表的文本拼起来（跨域表读 cssRules 会抛，跳过即可），
    // 并带上宽度媒体查询的过滤 —— 只看**当前视口真的生效**的那些规则，
    // 否则窄屏下会把桌面尺寸也读进来。
    function collectStyleText() {
        var out = [];
        var innerWidth = window.innerWidth || document.documentElement.clientWidth || 0;
        for (var i = 0; i < document.styleSheets.length; i++) {
            var rules = null;
            try {
                rules = document.styleSheets[i].cssRules;
            } catch (e) {
                continue;   // 跨域样式表：读不了就跳过（首页那两张都是同源的）
            }
            if (!rules) continue;
            for (var k = 0; k < rules.length; k++) {
                var rule = rules[k];
                if (rule.media) {
                    // 只认「最大宽度」这一类（本项目的断点都是 max-width）；
                    // 媒体文本里没写、或写了别的（如 prefers-reduced-motion）就跳过
                    var m = /max-width\s*:\s*(\d+)px/.exec(rule.media.mediaText || '');
                    if (!m || innerWidth > parseFloat(m[1])) continue;
                    if (rule.cssRules) {
                        for (var j = 0; j < rule.cssRules.length; j++) {
                            out.push(rule.cssRules[j].cssText || '');
                        }
                    }
                    continue;
                }
                out.push(rule.cssText || '');
            }
        }
        return out;
    }

    // 从样式表文本里找出某条选择器所在的**规则体**（返回最后一个匹配的 ——
    // 同优先级下后面写的赢，与浏览器的层叠一致）。
    function ruleBody(chunks, selector) {
        var body = null;
        for (var i = 0; i < chunks.length; i++) {
            var at = chunks[i].indexOf(selector);
            if (at < 0) continue;
            // 选择器后面必须紧跟 `{`（只隔空白），避免把 `.acc-sidebar-avatar img` 之类也算进来
            var rest = chunks[i].slice(at + selector.length);
            if (!/^\s*\{/.test(rest)) continue;
            body = chunks[i].slice(chunks[i].indexOf('{', at) + 1).replace(/\}\s*$/, '');
        }
        return body;
    }

    // 把落点对齐到导航栏那颗头像的**实际外框**：
    //   · 尺寸：**原样用 CSS 里的 40×40（含白边）**，一点不缩 —— 它就是侧栏那颗
    //     头像真正的外框尺寸，人眼看过去要落在的位置
    //   · 圆角：按起点那颗的比例换算（起点 64×64 用 9px → 圆角占外框 9/64；
    //     落点 40×40 于是取 9 × 40/64 = 5.6px，与 CSS 里写的 6px 基本吻合）。
    //     这样飞行途中圆角比例恒定，看不出「圆角自己化了一下」
    //   · 位置：**横向**让两颗头像中心对齐（起点中心 x = 62，落点 12 + 20），
    //     纵向**保持 CSS 给的 top**（= 侧栏内边距 20px，与 account 页里那颗头像逐像素同位）。
    //     为什么纵向不居中：落点要盖住换页那一刻侧栏头像**真正**所在的位置，
    //     不然跳过去会看到它往上挪十几像素（实测 12px）。
    function normalizeTarget(nav, target, navRadius) {
        var r1 = parseFloat(navRadius);
        var navSide = Math.min(nav.width, nav.height);
        var usable = !isNaN(r1) && r1 > 0 && navSide > 0;
        var ratio = usable ? r1 / navSide : 0.5;   // 起点那颗的「圆角 / 外框」比例
        var side = Math.min(target.width, target.height);

        target.radius = usable ? (ratio * side) + 'px' : '0.5';
        target.ratio = ratio;

        // 横向按中心对齐（起点中心 62 = 落点 12 + 20）；纵向不动，保持 CSS 的位置
        if (Math.abs(nav.width - target.width) > 2) {
            target.left += (nav.width - target.width) / 2;
        }
        return target;
    }

    // clip-path 圆形裁剪是否可用；不可用（老浏览器）就退化成直接跳转
    function canZoom() {
        return !!(window.CSS && window.CSS.supports && window.CSS.supports('clip-path', 'circle(0px at 0px 0px)'));
    }

    // 点击头像 → 进入个人中心。整段是**一次连续的动作**，三件事同时发生：
    //   ① 遮罩（与个人中心同色）从头像正中扩散到盖满全屏
    //   ② 头像飞到最上层，边扩散边缩小 + 右移，落成个人中心侧栏顶部那颗头像
    //   ③ 460ms 后换页 —— 那时头像已在目标位置，新页面原样接着显示
    //
    // 两个实现要点：
    //   · 头像必须**临时挪到 body 下**：它原本在 .rectangle（z-index:100）里，子元素的
    //     z-index 越不过父级建立的层叠上下文，不挪就永远压在整屏遮罩（z-index:9000）下面，
    //     也就没有「背景从头像底下长出来」的效果。挪的时候用 getBoundingClientRect 把
    //     几何原样写死，所以看不出移动。
    //   · 遮罩的第一帧要把过渡关掉：否则起点会落在样式表的兜底值（圆心在屏幕正中），
    //     圆就从屏幕中心长出来了（实测踩过一次）。
    function playAvatarZoom() {
        var btn = document.getElementById('navAvatar');
        var layer = document.getElementById('avatarZoom');
        var target = 'account/?from=avatar';
        var morph = morphTarget();

        if (zoomPlaying) return;
        if (!btn || !layer || !canZoom() || prefersReducedMotion() || !morph) {
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

        zoomPlaying = true;

        // ---- ① 把头像原样「钉」到 body 上（位置一个像素都不动）----
        btn.style.cssText = 'position:fixed;left:' + rect.left + 'px;top:' + rect.top +
            'px;width:' + rect.width + 'px;height:' + rect.height + 'px;margin:0;transform:none;z-index:9001;';
        document.body.appendChild(btn);
        void btn.offsetWidth; // 让「钉住」这一帧落定，别和下面的形变并成一次

        // ---- ② 遮罩的起点：圆心落在头像正中、半径 0 ----
        layer.style.transition = 'none';
        layer.classList.add('is-armed'); // 可见（半径 0，屏幕上看不到东西）
        layer.style.clipPath = 'circle(0px' + origin;
        void layer.offsetWidth;

        // ---- ③ 形变：头像缩小并落进个人中心侧栏（与遮罩扩散同时开始）----
        // is-morphing 只提供过渡、is-flying 提供过场期间的外观（见 main.css 的注释）
        btn.classList.add('is-morphing');
        btn.classList.add('is-flying');
        void btn.offsetWidth;
        btn.style.left = morph.left + 'px';
        btn.style.top = morph.top + 'px';
        btn.style.width = morph.width + 'px';
        btn.style.height = morph.height + 'px';
        // 圆角也显式写：C 的圆角比例与起点**不是逐像素相等**（40 × 9/64 = 5.6 → 取了 6px），
        // 不写死的话过渡结束时它会跳到 6px，中途看着像「圆角自己化了一下」。
        btn.style.borderRadius = morph.radius;

        // ---- ④ 遮罩开始扩散 ----
        layer.style.transition = '';
        layer.style.clipPath = 'circle(' + radius + 'px' + origin;

        // ---- ⑤ 等形变**真正结束**再换页 ----
        // 不能只靠 setTimeout：CSS 过渡与定时器同为 460ms，浏览器最后一次绘制大约在
        // 450ms，那时位置/尺寸/白边还差一点点，换页后是 100% 状态 —— 那一点点就是
        // 肉眼看到的「跳一下」。所以听过渡结束事件，定时器只作兜底。
        goWhenSettled(btn, function() {
            window.location.href = target;
        });
    }

    // 等一个元素的过渡跑完再执行（只认 width：形变那几个属性的时长一致，等一个就够）。
    // 兜底：过渡被跳过（元素被隐藏等）或事件丢失时，ZOOM_MS + 120 后照样执行。
    function goWhenSettled(el, done) {
        var settled = false;
        var fire = function() {
            if (settled) return;
            settled = true;
            done();
        };
        el.addEventListener('transitionend', function onEnd(e) {
            if (e.propertyName !== 'width') return;
            el.removeEventListener('transitionend', onEnd);
            fire();
        });
        window.setTimeout(fire, ZOOM_MS + 120);
    }

    // 把飞出去的头像放回导航栏。正常流程用不到（换页后这份 DOM 就没了），
    // 只有浏览器用「前进后退缓存」把首页整页恢复回来时才需要复位。
    function resetAvatarFlight() {
        var layer = document.getElementById('avatarZoom');
        if (layer) {
            layer.classList.remove('is-armed');
            // 同 playReturnAnimation 的收尾：先关过渡再清几何，别让它朝兜底值再补一段
            layer.style.transition = 'none';
            layer.style.clipPath = '';
            void layer.offsetWidth;
            layer.style.transition = '';
        }
        var btn = document.getElementById('navAvatar');
        if (btn) {
            btn.classList.remove('is-flying');
            btn.classList.remove('is-morphing');
            btn.style.cssText = '';
            var nav = document.querySelector('.rectangle');
            if (nav && btn.parentNode !== nav) {
                nav.insertBefore(btn, nav.firstChild);
            }
        }
        zoomPlaying = false;
    }

    // ===================== 反向转场：从个人中心返回 =====================
    // 账号页那边先在本地把组件淡出，再带 ?from=account 跳回本页。
    // 本页要做的是「倒放」：一进来（第一帧之前）就把画面摆成**账号页最后一帧**的样子 ——
    // 遮罩整屏盖住 + 头像已经在侧栏那个位置；等页面在遮罩下面画好，再同时
    // ① 把圆圈缩回导航栏的位置与大小 ② 把侧栏那颗外观还原成导航栏那颗。
    var FROM_ACCOUNT = /[?&]from=account(&|=|$)/.test(window.location.search);

    // 前置条件：导航栏必须可见（头像才有落点）。沉浸模式下导航栏会连头像一起隐藏，
    // 这种情况本轮先跳过反向动画（原因与候选方案记在根目录 TODO.md）。
    // 判据用英语模块同一个键，同步可读，不会和它的异步初始化抢时序。
    function returnHasLandingSpot() {
        try {
            return window.localStorage.getItem('reviewImmersive') === '0';
        } catch (e) {
            return false; // 读不到 localStorage（隐私模式）→ 按「不安全」处理，退回普通跳转
        }
    }

    // 等页面在遮罩下面画好（load + 两帧）再开始倒放，否则缩圈时露出的是还没画完的页面
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
        var btn = document.getElementById('navAvatar');
        var layer = document.getElementById('avatarZoom');
        if (!btn || !layer || !canZoom() || prefersReducedMotion() || !returnHasLandingSpot()) {
            return false;
        }

        // 倒放的起点 = 正向形变的落点，两者必须是同一组数字（都从现在的 CSS 算出来）。
        // ⚠️ 必须在**挪动按钮之前**算：morphTarget() 量的是按钮在导航栏里的位置，
        //    挪走之后它就在屏幕左上角了，再量会得到一份错得离谱的结果。
        var morph = morphTarget();
        if (!morph) return false;

        // 头像在导航栏里的位置（正向过场结束时它也是回到这里）
        var rect = btn.getBoundingClientRect();
        if (!rect.width) return false; // 导航栏没排上版 → 不播
        var cx = rect.left + rect.width / 2;
        var cy = rect.top + rect.height / 2;
        var dx = Math.max(cx, window.innerWidth - cx);
        var dy = Math.max(cy, window.innerHeight - cy);
        var radius = Math.ceil(Math.sqrt(dx * dx + dy * dy)) + 32;
        var origin = ' at ' + cx + 'px ' + cy + 'px)';

        zoomPlaying = true; // 倒放期间头像不响应点击（它正被当作动画元素用）

        // ---- 第一帧：全部无过渡，直接摆成「账号页最后一帧」----
        btn.classList.add('is-morphing');
        btn.classList.add('is-flying');
        btn.style.cssText = 'position:fixed;left:' + morph.left + 'px;top:' + morph.top +
            'px;width:' + morph.width + 'px;height:' + morph.height +
            'px;margin:0;transform:none;z-index:9001;transition:none;' +
            // 圆角也一起写：与正向的落点保持一致（否则第一帧会用样式表的 9px，
            // 而 40×40 那颗实际是 6px，倒放刚开始会看到圆角「弹」一下）
            'border-radius:' + morph.radius + ';';
        document.body.appendChild(btn);

        layer.style.transition = 'none';
        layer.classList.add('is-armed');
        layer.style.clipPath = 'circle(' + radius + 'px' + origin;
        void layer.offsetWidth;

        // 遮罩已经盖住了：让页面内容正常显示（它会一直待在遮罩下面，等圆圈缩小时露出来）
        document.documentElement.classList.remove('is-returning');

        whenPageReady(function() {
            // ---- 倒放开始：圆圈缩回头像 + 侧栏头像变回导航栏那颗 ----
            btn.style.transition = '';
            void btn.offsetWidth;              // 让过渡重新生效，再改几何
            btn.classList.remove('is-flying'); // 外观变回头像（边框/底色由 is-morphing 兜住过渡）
            btn.style.left = rect.left + 'px';
            btn.style.top = rect.top + 'px';
            btn.style.width = rect.width + 'px';
            btn.style.height = rect.height + 'px';

            layer.style.transition = '';
            layer.style.clipPath = 'circle(0px' + origin;

            goWhenSettled(btn, function() {
                // 先摘过渡再清行内几何：否则「top:18px → CSS 的 50% + translateY(-50%)」
                // 会被当成一次新的过渡，头像会晃一下
                btn.classList.remove('is-morphing');
                btn.classList.remove('is-flying');
                btn.style.cssText = '';
                var nav = document.querySelector('.rectangle');
                if (nav) nav.insertBefore(btn, nav.firstChild);

                layer.classList.remove('is-armed');
                // ⚠️ 先关过渡再清几何：否则清除行内 clip-path 会立刻朝样式表的兜底值
                // （circle(0px at 50% 50%)）再补一段过渡 —— 圆心会从头像漂向屏幕正中
                // （实测过一次：calc(5.69% + 54.94px)）。半径是 0 所以看不见，
                // 但那是白白多跑一段合成，收尾就该干干净净。
                layer.style.transition = 'none';
                layer.style.clipPath = '';
                void layer.offsetWidth;
                layer.style.transition = '';
                zoomPlaying = false;

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
        navAvatarEl.addEventListener('click', playAvatarZoom);
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
    // 那时脚本不会重跑，飞出去的头像与展开的遮罩都停在原样 —— 恢复时复位，
    // 并顺手重新问一次登录态（登录/退出后返回都可能变化）。
    window.addEventListener('pageshow', function(event) {
        if (!event.persisted) return;
        resetAvatarFlight();
        renderAvatarState();
    });

    // 调试口（与 account/account.js 的 window.__guangxueAccount 同一个用意）：
    // 形变落点是**从样式表算出来的**，出问题时需要能在控制台里逐项看它是怎么算的。
    // 只读、无副作用，普通使用不会碰到。
    window.__guangxue = {
        morphTarget: morphTarget,
        navAvatarBox: navAvatarBox,
        playAvatarZoom: playAvatarZoom,
        playReturnAnimation: playReturnAnimation,
        resetAvatarFlight: resetAvatarFlight
    };

});