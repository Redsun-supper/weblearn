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
    
    // ===================== 学科模块按需加载 =====================
    // 约定：某个学科若已有独立模块，就放在 modules/<学科>/ 目录下，并在其中导出
    // 初始化函数；main.js 只负责「按需动态加载 + 调用」，不再内联任何学科的业务逻辑
    // （否则 main.js 会随学科增多而无限膨胀）。
    // 用动态 import() 的好处：只有真正进入该学科页才会加载对应模块及其 WASM 引擎，
    // 首屏不再携带学科逻辑。
    const ENGLISH_PAGE = 'modules/english/english.html';
    
    // 当前已初始化的学科模块命名空间（用于切页时调用其可选的 unmount()）
    let activeSubjectModule = null;
    
    // 切页前清理上一个学科模块：模块若导出了 unmount() 就调用它
    // 为什么框架要做这件事：学科页可以往 body / document 上挂全局状态
    // （例如英语复习页的「沉浸模式」会给 body 加 is-immersive 类来隐藏导航栏），
    // 内容容器换成别的学科后这些状态不会自己消失，必须由模块自己收回。
    // 放在这里而不是各学科里，是为了让「切页」这条路径只有一个收口点。
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
        teardownSubjectModule(); // 先收拾上一个学科留下的全局状态
        if (pageName !== ENGLISH_PAGE) return; // 目前只有英语有独立模块
        import('./modules/english/english.js').then(function(mod) {
            activeSubjectModule = mod;
            mod.initReviewApp();
        }).catch(function(err) {
            // 模块本身加载失败（路径错误 / 网络问题）：就地提示，便于定位
            const statusEl = document.getElementById('reviewStatus');
            if (statusEl) statusEl.textContent = '英语模块加载失败: ' + err;
            console.error('英语模块加载失败:', err);
        });
    }
    
    
    // 缓存配置常量定义
    // CACHE_EXPIRY: 缓存过期时间，设置为30天（30天 × 24小时 × 60分钟 × 60秒 × 1000毫秒）
    // 超过30天未访问的缓存将被自动清理，释放存储空间
    const CACHE_EXPIRY = 30 * 24 * 60 * 60 * 1000;
    // CACHE_PREFIX: 缓存键名前缀，用于区分缓存数据和其他localStorage数据
    const CACHE_PREFIX = 'pageCache_';
    // CACHE_META_KEY: 缓存元数据的键名，用于存储每个页面的最后访问时间
    const CACHE_META_KEY = 'pageCache_meta';
    // CACHE_VERSION: 页面缓存结构版本号
    // 用途：当学科页的结构或**路径**发生不兼容改动时，把版本号 +1，
    // 老用户 localStorage 里的旧页面缓存会在启动时被整体清除，
    // 避免出现「新脚本 + 旧页面结构」导致功能不可用
    // 历史：v1 初版；v2 英语页新增「显示答案」按钮；v3 英语页迁到 modules/english/；
    //       v4 英语复习界面改版（极简全屏：顶栏统计 / 大字号单词 / 例句高亮 / 粉彩评分按钮）；
    //       v5 英语页顶栏改成「沉浸模式」开关（隐藏站点导航栏），揭晓区改为每条释义一块；
    //       v6 英语页底部新增左下角「今日计划」小字
    const CACHE_VERSION = 6;
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
    // 参数：pageName - 页面文件路径（如'modules/english/english.html'）
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
    const defaultPage = ENGLISH_PAGE;
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
        // 初始化该学科的独立模块（英语有，其他学科暂无）
        initSubjectModule(defaultPage);
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
                // 初始化该学科的独立模块（英语有，其他学科暂无）
                initSubjectModule(defaultPage);
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
                // 若该学科有独立模块则初始化
                initSubjectModule(pageName);
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
                        // 若该学科有独立模块则初始化
                        initSubjectModule(pageName);
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
    // ===================== 头像 = 个人中心入口 =====================
    // 入口位置暂时从右上角的文字链接改成左上角头像（用户要求）。想改回文字入口时，
    // 把 index.html 里的 #navAccount 加回去、这段恢复成原来的写法即可。
    //
    // 登录态是服务端（Rust 认证服务 /api/auth/*，见 backend-rust/README.md）下发的
    // httpOnly Cookie，JS 读不到，所以只能问服务端一次：GET /api/auth/me
    // 账号服务没起来时静默降级：头像照样能点（账号页里可以登录），只是不点亮状态点。
    //
    // 注意：导航栏上的「退出登录」按钮随文字入口一起撤掉了，退出改在个人中心里做。

    // 过场时长（毫秒）：与 main.css 的 .avatar-zoom 过渡时长保持一致，到点即换页
    var ZOOM_MS = 460;
    var zoomPlaying = false;

    // 是否应当减少动态效果（无障碍）：系统开启时不做任何过场，直接跳转
    function prefersReducedMotion() {
        return !!(window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches);
    }

    // clip-path 圆形裁剪是否可用；不可用（老浏览器）就退化成直接跳转
    function canZoom() {
        return !!(window.CSS && window.CSS.supports && window.CSS.supports('clip-path', 'circle(0px at 0px 0px)'));
    }

    // 点击头像：从头像位置把一个圆放大到盖住整屏，然后进入个人中心。
    // 几何在这里算（CSS 只提供过渡与底色）：
    //   圆心 = 头像中心；半径 = 到四角里最远的那个角 + 一点余量 ——
    //   这样任意窗口尺寸、头像在任意位置都盖得满，不依赖 vmax 之类的近似单位。
    //
    // 分两帧写 clip-path，并且第一帧临时关掉过渡：
    //   不这么做的话，过渡的起点是样式表里的兜底值（圆心在屏幕正中），
    //   圆就会从屏幕中心长出来而不是从头像长出来（实测踩过一次）。
    function playAvatarZoom() {
        var btn = document.getElementById('navAvatar');
        var layer = document.getElementById('avatarZoom');
        var target = 'account/?from=avatar';

        if (zoomPlaying) return;
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

        zoomPlaying = true;
        btn.classList.add('is-zoom-start'); // 头像本体同时缩一下，做出「被吸进去」的手感

        // 第一帧：圆心落在头像上、半径为 0；此时把过渡关掉，确保起点精确、不参与插值
        layer.style.transition = 'none';
        layer.classList.add('is-armed'); // 可见（半径 0，屏幕上看不到东西）
        layer.style.clipPath = 'circle(0px' + origin;
        void layer.offsetWidth; // 强制重排：把这一帧真正算出来，作为过渡起点

        // 第二帧：恢复样式表里的过渡，把半径放到覆盖四角
        layer.style.transition = '';
        layer.style.clipPath = 'circle(' + radius + 'px' + origin;

        window.setTimeout(function() {
            window.location.href = target;
        }, ZOOM_MS);
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
                setTitle((user.username || user.email) + (user.role === 'admin' ? '（管理员）' : '') + ' · 个人中心');
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

    renderAvatarState();

    // 从个人中心返回时浏览器可能直接用「前进后退缓存」（bfcache）恢复页面：
    // 那时脚本不会重跑，过场遮罩会停在「已展开」的状态把整屏盖住 —— 恢复时复位，
    // 并顺手重新问一次登录态（登录/退出后返回都可能变化）。
    window.addEventListener('pageshow', function(event) {
        if (!event.persisted) return;
        var layer = document.getElementById('avatarZoom');
        if (layer) {
            layer.classList.remove('is-armed');
            layer.style.clipPath = ''; // 行内几何清掉，下次点击按当前位置重新算
            layer.style.transition = '';
        }
        var btn = document.getElementById('navAvatar');
        if (btn) btn.classList.remove('is-zoom-start');
        zoomPlaying = false;
        renderAvatarState();
    });

// 匿名函数结束，作为DOMContentLoaded事件的回调函数
});