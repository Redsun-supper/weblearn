// 广学 · 个人中心（账号 / 登录设备 / 操作）
//
// 交互骨架：
//   1. 打开页面先问服务端「我是谁」——GET /api/auth/me
//        200 → 显示「账号信息 + 登录设备 + 可用操作」三张卡（都是个人中心的一部分）
//        401 → 显示「登录 / 注册」面板
//   2. 登录态是服务端下发的 httpOnly Cookie，JS 读不到，所以只能靠这一步问服务端。
//   3. 本地开发时验证码不发邮件（AUTH_MAIL_MODE=log），发码后页面会调用
//      /api/auth/dev/codes 自动回填；生产环境该接口不存在（404），静默忽略即可。
//   4. 组件入场动效：动作定义在 account.css（accRise / accPop），
//      这里只负责「什么时候开始」—— 按可见顺序逐个写 animation-delay（playEnter）。
//      入口是首页左上角的头像（index.html 的 #navAvatar），进来时会带 ?from=avatar，
//      那种情况下跳过身份卡的入场，跟首页那块扩散遮罩衔接成一次动画。
//
// ⚠️ 本文件保持 ES5 写法（var / function），与 main.js、admin.js 一致。

(function () {
    'use strict';

    // ===================== 通用小工具 =====================

    function $(id) {
        return document.getElementById(id);
    }

    // 统一请求：同源自动带 Cookie（credentials: 'same-origin'），自动 JSON。
    // 不抛异常，而是把 { status, data } 交给调用方 —— 便于区分 401 / 业务错误 / 网络错误；
    // 非 JSON 响应（网关的 502 页面）与网络层失败（服务没起来 / 断网）都降级成能看懂的结论，
    // 不让它们变成未捕获异常。
    function api(method, path, body) {
        var init = { method: method, credentials: 'same-origin', headers: {} };
        if (body !== undefined && body !== null) {
            init.headers['Content-Type'] = 'application/json';
            init.body = JSON.stringify(body);
        }
        return fetch(path, init).then(function (res) {
            return res.text().then(function (text) {
                var data = null;
                try {
                    data = text ? JSON.parse(text) : null;
                } catch (e) {
                    data = null;
                }
                return { status: res.status, ok: res.ok, data: data };
            });
        }).catch(function (err) {
            return { status: 0, ok: false, data: null, networkError: String(err) };
        });
    }

    // 从响应里取后端的中文提示；取不到就给一个兜底文案
    function messageOf(res, fallback) {
        if (res && res.data && res.data.message) return res.data.message;
        if (res && res.networkError) return '无法连接账号服务，请确认它已启动（backend-rust: cargo run --release）';
        return fallback || '操作失败，请稍后再试';
    }

    function setMsg(node, text, kind) {
        if (!node) return;
        node.textContent = text || '';
        node.className = 'acc-msg' + (kind ? ' is-' + kind : '');
    }

    function setBusy(btn, busy, busyText) {
        if (!btn) return;
        if (busy) {
            btn.dataset.label = btn.textContent;
            btn.textContent = busyText || '处理中…';
            btn.disabled = true;
        } else {
            if (btn.dataset.label) btn.textContent = btn.dataset.label;
            btn.disabled = false;
        }
    }

    function valueOf(id) {
        var node = $(id);
        return node ? String(node.value || '').trim() : '';
    }

    // 是否应当减少动态效果（无障碍）：系统开启时不做高度过渡
    function prefersReducedMotion() {
        return !!(window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches);
    }

    // 认证成功后要跳回哪一页：只接受站内相对路径，避免被 ?next= 带去外站
    function nextUrl() {
        var match = /[?&]next=([^&#]+)/.exec(location.search);
        if (!match) return '../index.html';
        var raw = decodeURIComponent(match[1]);
        if (raw.indexOf('://') >= 0 || raw.indexOf('//') === 0 || raw.charAt(0) === '\\') {
            return '../index.html';
        }
        return raw;
    }

    // ===================== 面板切换与入场动效 =====================

    // 过渡节奏，集中放一处 —— 想微调「加载屏 → 真面板」这一段的手感，改这里的数字就够了
    // （一起对齐的还有 account.css 里 .acc-loading 的 opacity 过渡时长，见下面那条注释）。
    //   step          卡与卡之间的入场错开    innerStep  卡内组件的入场错开
    //   loadingFade   加载卡淡出时长（也是真面板起手的延时：两者接力，不重叠）
    //   cardHeight    切「登录 / 注册」标签时白卡长高 / 收短的时长
    var TIMING = {
        step: 30,
        innerStep: 12,
        loadingFade: 160,
        cardHeight: 380
    };

    // 入场动画里最长的那一个（accRise 0.42s —— 见 account.css）：动画结束后延迟摘
    // is-animating（合成层提示）要用它算时间。
    var ANIM_LONGEST_MS = 420;
    var animatingTimer = null;

    // 组件之间的错开间隔（毫秒）：「什么时候开始」在这里排延时，
    // 「怎么动」写在 account.css 的 accRise（浮起淡入）/ accPop（头像回弹）。
    // 取 30ms 而不是更大：个人中心有 30 来个组件，间隔一大整体要 1.5 秒才装配完，
    // 看上去像「页面一直在慢慢冒东西」；30ms 能让最后一组在 1 秒内到位。
    var ENTER_STEP_MS = TIMING.step;

    // 卡片**内部**组件的错开间隔（毫秒）：比卡与卡之间的 30ms 小一半。
    // 卡里最多的登录表单有 10 个字段，都按 30ms 排的话最后一个要等 300ms 才开始往上浮，
    // 看起来像「表单一直在往外冒」。12ms 既保留从左到右的次序感，整块又在 120ms 内到位。
    var INNER_STEP_MS = TIMING.innerStep;

    // 面板分组：一组里的卡片一起显示 / 隐藏
    // （个人中心把「账号信息 / 登录设备 / 可用操作」拆成了三张卡，所以是分组而不是单块）
    var PANEL_GROUPS = {
        loading: ['panelLoading'],
        auth: ['panelAuth'],
        account: ['panelAccount', 'panelDevices', 'panelActions']
    };

    // 收集要参与入场的组件：卡片本身（带 data-enter 的）在前，卡片内部的 [data-enter] 随后。
    // 顺序即延时顺序，所以「卡 → 标题 → 每一行」会依次浮入。
    // 每项都带上「这是第几张卡 / 卡内第几个」——排延时时两者用的步长不一样（见 playEnter）。
    //
    // ⚠️ **藏在 [hidden] 里的组件一概跳过**：切到登录面板时，注册表单那 8 个 [data-enter]
    //    其实也在面板里，早先它们会跟着一起跑动画（不可见，却照样占着合成与重绘的时间），
    //    正好挤在可见字段那一批上。判据用 closest（祖先里只要有 hidden 就跳过），
    //    与 playExit 的口径一致 —— 只判断元素自己的 hidden 属性是不够的。
    function collectEnterNodes(cards) {
        var items = [];
        var cardIndex = 0;
        for (var i = 0; i < cards.length; i++) {
            var card = cards[i];
            if (!card || card.hasAttribute('hidden')) continue;
            if (card.hasAttribute('data-enter')) items.push({ el: card, card: cardIndex, inner: 0 });
            var inner = card.querySelectorAll('[data-enter]');
            var n = 0;
            for (var k = 0; k < inner.length; k++) {
                if (inner[k].closest('[hidden]')) continue;
                n += 1;
                items.push({ el: inner[k], card: cardIndex, inner: n });
            }
            cardIndex += 1;
        }
        return items;
    }

    // 三种入场类：默认「浮起淡入」，切标签页时按方向「轻轻浮上来」（动作在 account.css）
    var ENTER_CLASSES = ['is-enter', 'is-enter-right', 'is-enter-left'];

    // 动画期间挂这个类（CSS 里给它 will-change: transform, opacity）：
    // 让浏览器把组件提升成合成层，整段动画只做合成、不重绘 —— 卡片那圈 26px 模糊的阴影
    // 就是靠这一步免掉每帧重绘的。动画跑完就摘掉，别让几十个层常驻占显存。
    var ANIMATING_CLASS = 'is-animating';

    // 给这些组件排一遍入场动效。
    //   mode：'rise' = 轻轻浮上来（默认）| 'right' / 'left' = 顺着点击方向偏一点浮上来
    //   baseDelay（毫秒，可选）：整体推迟多久开始 —— 交给调用方做「接力」用
    //    （换面板时先让加载卡淡出，真面板再起手，见 showOnly）
    //
    // ⚠️ 性能要点：先把所有元素的旧动画类摘掉（只写不读），**只强制重排一次**，
    // 最后统一挂新类。早先的写法是「逐个元素：摘类 → 读 offsetWidth → 挂类」，
    // 每个 offsetWidth 都会让浏览器同步重算整页布局 —— 30 个组件就是 30 次布局，
    // 正好卡在进场那一两帧上。
    function playEnter(cards, mode, baseDelay) {
        var items = collectEnterNodes(cards);
        if (!items.length) return;

        var cls = mode === 'right' ? 'is-enter-right' : (mode === 'left' ? 'is-enter-left' : 'is-enter');
        var base = baseDelay || 0;
        var i, k;

        for (i = 0; i < items.length; i++) {
            for (k = 0; k < ENTER_CLASSES.length; k++) {
                items[i].el.classList.remove(ENTER_CLASSES[k]);
            }
        }
        void document.body.offsetHeight;

        for (i = 0; i < items.length; i++) {
            items[i].el.style.animationDelay = (base + items[i].card * ENTER_STEP_MS + items[i].inner * INNER_STEP_MS) + 'ms';
            items[i].el.classList.add(cls, ANIMATING_CLASS);
        }

        // 动画跑完就把合成层的提示摘掉（最后起手的那一个 + 它的时长）。上面每次排新动画都会
        // 重新计时，所以这一条定时器不会误摘还在跑的层。用 > 号而不是 =：多出来的余量是留给
        // 「定时器抖动」和「浏览器晚一帧才结束动画」的，摘早了反而会多一次重绘。
        clearTimeout(animatingTimer);
        animatingTimer = window.setTimeout(function () {
            animatingTimer = null;
            for (i = 0; i < items.length; i++) items[i].el.classList.remove(ANIMATING_CLASS);
        }, base + (items.length - 1) * INNER_STEP_MS + ENTER_STEP_MS + ANIM_LONGEST_MS + 120);
    }

    // 一次只显示一组面板（组内的卡片一起显示 / 隐藏）。
    // 加载卡单独处理：它是**绝对定位的浮层**，要「淡出」而不是立刻藏掉 —— 新面板这时
    // 已经在它下面渲染好了，让它在上面淡完再藏（时长见 LOADING_FADE_MS，与 account.css 的
    // .acc-loading 过渡一致），否则会看到一次布局高度跳变。
    // 交接是**接力**的：加载卡先淡出，真面板等它淡完（playEnter 的 baseDelay）再起手。
    function showOnly(group) {
        var visible = PANEL_GROUPS[group] || [];
        Object.keys(PANEL_GROUPS).forEach(function (key) {
            PANEL_GROUPS[key].forEach(function (id) {
                if (id === 'panelLoading') return;
                var node = $(id);
                if (!node) return;
                if (visible.indexOf(id) >= 0) {
                    node.removeAttribute('hidden');
                } else {
                    node.setAttribute('hidden', 'hidden');
                }
            });
        });

        // 舞台高度**一次落定**（不给过渡）：加载卡就是按这个高度铺的浮层，真面板一显示
        // 就已经是这个高度了，中间那一小截零头（换行/小数取整，几像素）如果还走 260ms 的
        // 过渡，就会在交叉淡出期间被看见 —— 那正是「切到下一个动画时也有问题」。
        // 放在 playEnter 之前：入场动效带 transform，先量准再动。
        applyStageHeight(group === 'loading' ? reservedStageHeight() : targetStageHeight($('panelLoading')));

        var loading = $('panelLoading');
        if (loading) {
            if (group === 'loading') {
                clearTimeout(loadingFadeTimer);
                loadingFadeTimer = null;
                loading.classList.remove('is-leaving');
                loading.removeAttribute('hidden');
            } else if (!loading.hasAttribute('hidden')) {
                // 加载卡淡出 → 真面板起手，两者接力而不是同时动（后者看起来是两屏叠在一起互相穿模）
                loading.classList.add('is-animating', 'is-leaving');
                clearTimeout(loadingFadeTimer);
                loadingFadeTimer = window.setTimeout(function() {
                    loadingFadeTimer = null;
                    loading.setAttribute('hidden', 'hidden');
                    loading.classList.remove('is-leaving', 'is-animating');
                    // 清掉把它撑到整屏高的行内值：只在**藏起来之后**清 —
                    // 淡出期间它还可见，清掉会当场缩回 81px
                    loading.style.minHeight = '';
                }, LOADING_FADE_MS);
            }
        }

        playEnter(visible.map(function (id) { return $(id); }), 'rise', group === 'loading' ? 0 : LOADING_FADE_MS);
    }

    // 舞台该有多高：当前**可见**卡片里最高的那张（按 scrollHeight 量，含被 max-height 裁掉的内容）。
    // 注意每次都要**重新量**：卡片内容是入场后才填的（设备列表要等接口），量早了高度就停在旧值。
    function targetStageHeight(loading) {
        var stage = document.querySelector('.acc-stage');
        if (!stage) return 0;
        var best = 0;
        for (var i = 0; i < stage.children.length; i++) {
            var el = stage.children[i];
            if (el === loading || el.hasAttribute('hidden')) continue;
            if (el.scrollHeight > best) best = el.scrollHeight;
        }
        return best;
    }

    // 把**藏着**的候选面板临时放出来量一遍（量完立刻藏回去），返回最高的那张。
    // 为什么要量：登录后要显示的卡（账号 204 + 设备 366 + 操作 205 ≈ 775）比加载卡高得多，
    // 这一屏的高度得提前定下来，加载卡才有地方铺（见 reservedStageHeight）。
    // ⚠️ 量完必须恢复 hidden：这里改的是真实 DOM，漏掉一张就会让面板提前露出来。
    function hiddenPanelsHeight(stage, loading) {
        var best = 0;
        for (var i = 0; i < stage.children.length; i++) {
            var el = stage.children[i];
            if (el === loading || !el.hasAttribute('hidden')) continue;
            el.removeAttribute('hidden');
            if (el.scrollHeight > best) best = el.scrollHeight;
            el.setAttribute('hidden', 'hidden');
        }
        return best;
    }

    // 加载屏该占的高度（每打开一次页面只量一次，之后一直用这个值）。
    //
    // 为什么要**提前定死**：加载卡是绝对定位的浮层，而舞台原来是首帧之后自己长上去的
    //（87px → 700px 上下）。这一长，卡里居中的「正在检查登录状态…」就被一路往下带 300 多像素，
    // 顺带把页脚顶出屏幕、切面板时页面又缩回来。现在改成：一上来就按「下一屏的高度」铺好，
    // 加载卡自己撑满这块地方，整段过渡里**没有任何东西移动**。
    //
    // 量的是「藏着的那些面板里最高的那张」而不是「下面所有屏里最高的那张」：后者会让
    // 加载期间凭空多出一条更长的滚动条。取**原值**（不加余量）也很重要 —— 加载屏与真面板
    // 高度一致时页面总高完全不变，用户就算提前往下滚了，交接时也不会有一次滚动回弹。
    // ⚠️ 已知小瑕疵：如果最终显示的是「账号信息」那组（更高），真面板浮上来时会往下抻一点。
    //    这是刻意的取舍 —— 首屏那个瞬间还不知道登录态，猜错方向时宁可下面多出来。
    function reservedStageHeight() {
        if (reserved !== null) return reserved;
        var stage = document.querySelector('.acc-stage');
        var loading = $('panelLoading');
        if (!stage || !loading) return 0;
        var natural = naturalLoadingHeight(loading);
        var next = hiddenPanelsHeight(stage, loading);
        reserved = next > 0 ? Math.max(natural, next) : natural;
        return reserved;
    }

    // 加载卡「本来多高」（自然高度）：量之前先把自己行内的 min-height 摘掉 ——
    // 那个值正是它被撑到的当前高度，不摘就会把上一轮的高度当成自然高度。
    function naturalLoadingHeight(loading) {
        var saved = loading.style.minHeight;
        loading.style.minHeight = '0';
        var h = Math.round(loading.getBoundingClientRect().height);
        loading.style.minHeight = saved;
        return h;
    }

    // 写舞台高度。CSS 的 .acc-stage 上有 height 过渡，所以这里是「动过去」而不是跳。
    // 只有「舞台高度已经和当前内容对齐过」之后的变化才给过渡（is-sized 就是这件事的标记）：
    // 首屏那一趟是**一次性落位**，就该是跳的 —— 否则会看到页面开场时高度自己长出来。
    // loading 传进来时，顺便把加载卡也撑到同一高度：这样「加载中 → 真面板」看起来是
    // 同一次呼吸，而不是一张小卡浮在大卡片里。
    // ⚠️ 不在这里清加载卡的行内 min-height：它淡出的那 160ms 里还是可见的，
    //    清掉会让这张卡当场缩回 81px 再淡出（比位移更显眼）。清理由淡出结束的回调接手。
    function applyStageHeight(value, loading) {
        var stage = document.querySelector('.acc-stage');
        if (!stage) return;
        var settled = stage.classList.contains('is-sized');
        stage.style.transition = settled ? '' : 'none';
        stage.style.height = value > 0 ? value + 'px' : '';
        if (loading) {
            loading.style.minHeight = value > 0 ? value + 'px' : '';
        }
        if (!settled) {
            void stage.offsetHeight;          // 让「无过渡」这件事立刻生效
            stage.classList.add('is-sized');
            stage.style.transition = '';
        }
        stage.dataset.h = value;
    }

    // 把舞台高度对齐到「当前真正该显示的内容」。
    // 三个时机调用：① 首帧（把加载屏的高度定死）② 卡片内容填完后
    //（设备列表是异步来的，填之前那张卡只有几十像素高，不补量的话高度就停在旧值上）。
    function syncStageHeight() {
        var stage = document.querySelector('.acc-stage');
        var loading = $('panelLoading');
        if (!stage) return;

        var isLoading = !!(loading && !loading.hasAttribute('hidden'));
        var target = isLoading ? reservedStageHeight() : targetStageHeight(loading);
        if (String(target) === stage.dataset.h) return;
        applyStageHeight(target, isLoading ? loading : null);
    }

    // 首帧画完之后落位。用两帧：一帧太早（还没排过版，量出来的高度不作数），
    // 两帧之后布局已经稳定。落位是**没有过渡**的跳变，所以看不到高度自己长出来；
    // 用户看到的第一帧就是最终布局。
    function scheduleStageFit() {
        window.requestAnimationFrame(function () {
            window.requestAnimationFrame(function () { syncStageHeight(); });
        });
    }

    // 加载卡的交叉淡出时长（毫秒）：与 account.css 的 .acc-loading 过渡一致。
    // 真面板的入场（playEnter 的 baseDelay）也用它 —— 于是「加载卡淡完」和「真面板起手」
    // 正好接上：接力而不是两屏同时动。改 TIMING.loadingFade 记得同步 CSS 那个 0.16s。
    var LOADING_FADE_MS = TIMING.loadingFade;
    var loadingFadeTimer = null;

    // 加载屏占的高度（reservedStageHeight 量一次就缓存，null = 还没量过）
    var reserved = null;

    // 视口尺寸变了（浏览器缩放、手机转屏）之前量的高度就不作数了：清掉缓存，
    // 下一次 syncStageHeight 重新量。不清的话加载屏会按旧尺寸铺，真面板上来时还得抻一下。
    window.addEventListener('resize', function () { reserved = null; });

    // 顶栏的入场：品牌与徽章各错开一点（动作仍是 accRise，画风不变）。
    // 为什么单独排而不是并进面板那套：顶栏不在面板分组里，而且它必须在遮罩溶掉的
    // 那一刻"到位"—— 之前没有动画，两个文字是凭空出现的（用户反馈很突兀）。
    // ⚠️ 返回按钮（.acc-home-btn）永远不参与：它是正向形变的落点，动了就前功尽弃。
    function playTopbarIn() {
        [['.acc-brand', 60], ['.acc-brand-sub', 90]].forEach(function(pair) {
            var el = document.querySelector(pair[0]);
            if (!el) return;
            for (var k = 0; k < ENTER_CLASSES.length; k++) el.classList.remove(ENTER_CLASSES[k]);
            el.classList.remove('is-exit');
            el.style.animationDelay = pair[1] + 'ms';
            el.classList.add('is-enter');
        });
    }

    // 卡片高度过渡的时长（毫秒）：只在这里写一次，过渡是行内设的，不用和 CSS 对齐
    var HEIGHT_MS = TIMING.cardHeight;
    var heightTimer = null;

    // 白色底的高度过渡：切标签时「往下长」/「往上收」，而不是瞬间跳变。
    // 经典做法 —— 量旧高 → 换内容 → 量新高 → 从旧高过渡到新高 → 收尾还原成自动高度：
    //   · 过渡期间必须 overflow:hidden，否则新表单会先整个冲出来、再被裁下去
    //   · 收尾一定要清掉行内 height/overflow，让卡片回到「由内容决定高度」
    //     （否则报错文案、窄屏换行把内容撑高时会被裁掉）
    //   · 起点前先把 transition 关掉，别把上一次没跑完的过渡续上；
    //     再强制重排一次，这一帧才是真正的过渡起点
    function animateCardHeight(card, from, to) {
        if (!card || from === to || prefersReducedMotion()) return;

        card.style.overflow = 'hidden';
        card.style.transition = 'none';
        card.style.height = from + 'px';
        void card.offsetHeight;

        card.style.transition = 'height ' + HEIGHT_MS + 'ms cubic-bezier(0.22, 0.61, 0.36, 1)';
        card.style.height = to + 'px';

        clearTimeout(heightTimer);
        heightTimer = setTimeout(function() {
            heightTimer = null;
            card.style.transition = '';
            card.style.height = '';
            card.style.overflow = '';
        }, HEIGHT_MS + 40);
    }

    // 切「登录 / 注册」标签：滑块就位 → 换表单 → 白卡高度过渡 → 新表单顺着点击方向滑入
    // （切到「注册」从右边来、切回「登录」从左边来，与滑块的移动方向一致）。
    // ⚠️ 只在卡片已经显示时才量高度：首屏 showOnly 之前它是 hidden，量出来是 0，
    //    那种情况交给入场动效，不要再额外播一次高度过渡。
    function switchTab(which) {
        var isLogin = which === 'login';
        var card = $('panelAuth');
        var measure = !!(card && !card.hasAttribute('hidden'));
        var fromHeight = 0;

        if (measure) {
            clearTimeout(heightTimer);
            heightTimer = null;
            // 上一次高度过渡可能还在跑，此刻的当前高度正好是新的起点
            fromHeight = card.getBoundingClientRect().height;
        }

        var tabLogin = $('tabLogin');
        var tabRegister = $('tabRegister');
        if (tabLogin) tabLogin.className = 'acc-tab' + (isLogin ? ' is-active' : '');
        if (tabRegister) tabRegister.className = 'acc-tab' + (isLogin ? '' : ' is-active');
        if ($('formLogin')) $('formLogin').hidden = !isLogin;
        if ($('formRegister')) $('formRegister').hidden = isLogin;

        var thumb = $('tabsThumb');
        if (thumb) {
            if (isLogin) {
                thumb.classList.remove('is-second');
            } else {
                thumb.classList.add('is-second');
            }
        }

        // 内容换完了才量新高。量之前先清掉上一轮遗留的行内高度，
        // 否则 overflow:hidden + 旧高度会把新内容裁掉，量到的是被裁过的值。
        if (measure) {
            clearTimeout(heightTimer);
            heightTimer = null;
            card.style.transition = 'none';
            card.style.height = '';
            card.style.overflow = '';
            animateCardHeight(card, fromHeight, card.getBoundingClientRect().height);
        }

        playEnter([isLogin ? $('formLogin') : $('formRegister')], isLogin ? 'left' : 'right');
    }

    // ===================== 当前账号 =====================

    // 身份卡：昵称 / 邮箱 + 一句话状态 + 角色徽章（未登录时给登录入口的文案）
    function setHero(user) {
        var name = $('heroName');
        var sub = $('heroSub');
        var role = $('heroRole');
        if (!name || !sub) return;

        if (!user) {
            name.textContent = '登录 / 注册';
            sub.textContent = '登录后可以管理账号与登录设备';
            if (role) role.setAttribute('hidden', 'hidden');
            return;
        }
        name.textContent = user.username || user.email || '已登录';
        sub.textContent = user.username ? (user.email || '') : '欢迎回来';
        if (role) {
            role.textContent = roleLabel(user.role);
            role.removeAttribute('hidden');
        }
    }

    // 角色显示名（P0-5 三级角色）。`super_admin` 不加这层映射就会原样显示成英文。
    function roleLabel(role) {
        if (role === 'super_admin') return '超级管理员';
        if (role === 'admin') return '管理员';
        return '普通用户';
    }

    function showAccount(user) {
        showOnly('account');
        setHero(user);
        if ($('accEmail')) $('accEmail').textContent = user.email || '—';
        if ($('accUsername')) $('accUsername').textContent = user.username || '（未设置）';
        if ($('accRole')) $('accRole').textContent = roleLabel(user.role);
        if ($('accCreated')) $('accCreated').textContent = user.created_at || '—';

        var adminLink = $('accAdmin');
        if (adminLink) {
            // 管理员与超管都能进后台（与 admin.js 的 canEnterAdmin 同口径）
            if (user.role === 'admin' || user.role === 'super_admin') {
                adminLink.removeAttribute('hidden');
            } else {
                adminLink.setAttribute('hidden', 'hidden');
            }
        }

        loadSessions();
    }

    function loadSessions() {
        var list = $('accSessions');
        if (!list) return;
        list.innerHTML = '';
        api('GET', '/api/auth/sessions').then(function (res) {
            var items = (res.status === 200 && res.data && res.data.data && res.data.data.items) || [];
            var failed = res.status !== 200 || !res.data || !res.data.data;
            renderSessionCount(failed ? 0 : items.length);
            if (failed) {
                list.appendChild(sessionRow('读取失败：' + messageOf(res), ''));
            } else if (!items.length) {
                list.appendChild(sessionRow('没有活跃会话', ''));
            } else {
                items.forEach(function (item) {
                    var meta = [];
                    if (item.current) meta.push('当前设备');
                    meta.push('最近活跃 ' + (item.last_used_at || item.created_at || '—'));
                    if (item.ip && item.ip !== 'unknown') meta.push('IP ' + item.ip);
                    list.appendChild(sessionRow(item.device_label || '未知设备', meta.join(' · '), item.current));
                });
            }
            // 列表填完再量一次高度：这之前那张卡只有「正在读取…」那么高（见 syncStageHeight）
            syncStageHeight();
        });
    }

    // 标题旁边的会话数。列表超过可视高度时会滚动，不给个数字用户不知道自己还有几台
    function renderSessionCount(n) {
        var el = $('devicesCount');
        if (!el) return;
        if (n > 0) {
            el.textContent = '（' + n + ' 台）';
            el.removeAttribute('hidden');
        } else {
            el.textContent = '';
            el.setAttribute('hidden', 'hidden');
        }
    }

    function sessionRow(name, meta, isCurrent) {
        var li = document.createElement('li');
        li.className = 'acc-session';

        var title = document.createElement('span');
        title.className = 'acc-session-name';
        title.textContent = name;
        li.appendChild(title);

        if (isCurrent) {
            var badge = document.createElement('span');
            badge.className = 'acc-badge';
            badge.textContent = '本机';
            li.appendChild(badge);
        }

        if (meta) {
            var text = document.createElement('span');
            text.className = 'acc-session-meta';
            text.textContent = meta;
            li.appendChild(text);
        }
        return li;
    }

    // 「正在检查登录状态」这一屏的**最短显示时长**（毫秒）。
    // 本地 /me 只要几毫秒，一返回就切面板的话它会一闪而过，后面的表单像凭空冒出来。
    // 语义：响应晚于阈值 → 回来后立刻开始；早于阈值 → 等到阈值再开始（见 refreshState）。
    var MIN_CHECK_MS = 600;

    // 问服务端「我是谁」：决定显示哪一块面板
    function refreshState() {
        var t0 = Date.now();
        return api('GET', '/api/auth/me').then(function (res) {
            var wait = Math.max(0, MIN_CHECK_MS - (Date.now() - t0));
            return new Promise(function (resolve) {
                window.setTimeout(function () {
                    if (res.status === 200 && res.data && res.data.data && res.data.data.user) {
                        showAccount(res.data.data.user);
                        resolve(true);
                        return;
                    }
                    // 顺序有讲究：先 switchTab 让滑块就位（此刻面板还藏着，字段入场会被跳过），
                    // 再由 showOnly 把整块面板按「浮起淡入」放出来 —— 这样首屏是统一的入场动画，
                    // 而不是登录表单单独从左边滑进来。
                    setHero(null);
                    switchTab('login');
                    showOnly('auth');
                    if (res.networkError) {
                        setMsg($('loginMsg'), messageOf(res), 'error');
                    }
                    resolve(false);
                }, wait);
            });
        });
    }

    // ===================== 登录 =====================

    function doLogin(event) {
        if (event) event.preventDefault();
        var email = valueOf('loginEmail');
        var password = $('loginPassword') ? $('loginPassword').value : '';
        var device = valueOf('loginDevice');
        var btn = $('loginSubmit');

        if (!email || !password) {
            setMsg($('loginMsg'), '请填写邮箱与密码', 'error');
            return;
        }
        setMsg($('loginMsg'), '正在登录…');
        setBusy(btn, true, '登录中…');

        api('POST', '/api/auth/login', {
            email: email,
            password: password,
            device_label: device || undefined
        }).then(function (res) {
            setBusy(btn, false);
            if (res.status === 200) {
                setMsg($('loginMsg'), '登录成功，正在跳转…', 'ok');
                location.href = nextUrl();
                return;
            }
            setMsg($('loginMsg'), messageOf(res, '登录失败'), 'error');
            // 清空密码框，避免误以为还能直接重试
            if ($('loginPassword')) $('loginPassword').value = '';
        });
    }

    // ===================== 注册 =====================

    var codeTimer = null;

    function startCountdown(seconds) {
        var btn = $('btnSendCode');
        if (!btn) return;
        var left = seconds;
        btn.disabled = true;
        btn.textContent = left + ' 秒后可重发';
        if (codeTimer) clearInterval(codeTimer);
        codeTimer = setInterval(function () {
            left -= 1;
            if (left <= 0) {
                clearInterval(codeTimer);
                codeTimer = null;
                btn.disabled = false;
                btn.textContent = '重新获取';
                return;
            }
            btn.textContent = left + ' 秒后可重发';
        }, 1000);
    }

    function sendCode() {
        var email = valueOf('regEmail');
        // 邀请码可选：不填也能发码（开放注册）。填了就一起发过去，
        // 让服务端在这一步先把「码不对」挡下来，而不是等注册时才失败。
        var invite = valueOf('regInvite');
        if (!email) {
            setMsg($('regMsg'), '请先填写邮箱', 'error');
            return;
        }
        setMsg($('regMsg'), '正在发送验证码…');
        setBusy($('btnSendCode'), true, '发送中…');

        api('POST', '/api/auth/email-code', { email: email, invite_code: invite }).then(function (res) {
            setBusy($('btnSendCode'), false);
            if (res.status !== 200) {
                setMsg($('regMsg'), messageOf(res, '验证码发送失败'), 'error');
                return;
            }
            startCountdown(60);
            setMsg($('regMsg'), '验证码已发送，请查收邮件（10 分钟内有效）', 'ok');
            // 开发模式：验证码不会真的发信，这里直接从调试接口取回来填进表单
            api('GET', '/api/auth/dev/codes?email=' + encodeURIComponent(email)).then(function (devRes) {
                if (devRes.status === 200 && devRes.data && devRes.data.data && devRes.data.data.code) {
                    if ($('regCode')) $('regCode').value = devRes.data.data.code;
                    setMsg($('regMsg'), '开发模式：验证码已自动填入（生产环境会真的发邮件）', 'ok');
                    if ($('codeHint')) {
                        $('codeHint').textContent = '这是本地开发模式，验证码取自服务端调试接口，并没有真的发邮件。';
                    }
                }
            });
        });
    }

    function doRegister(event) {
        if (event) event.preventDefault();
        var email = valueOf('regEmail');
        // 邀请码可选：留空 = 普通用户；填了 = 注册后升级为管理员（多个用空格分隔）
        var invite = valueOf('regInvite');
        var code = valueOf('regCode');
        var password = $('regPassword') ? $('regPassword').value : '';
        var password2 = $('regPassword2') ? $('regPassword2').value : '';
        var btn = $('regSubmit');

        if (!email || !code || !password) {
            setMsg($('regMsg'), '请把邮箱、验证码、密码都填完整', 'error');
            return;
        }
        if (password !== password2) {
            setMsg($('regMsg'), '两次输入的密码不一致', 'error');
            return;
        }
        if (password.length < 8) {
            setMsg($('regMsg'), '密码至少 8 个字符，且同时包含字母与数字', 'error');
            return;
        }

        setMsg($('regMsg'), '正在注册…');
        setBusy(btn, true, '注册中…');
        api('POST', '/api/auth/register', {
            email: email,
            email_code: code,
            invite_code: invite,
            password: password
        }).then(function (res) {
            setBusy(btn, false);
            if (res.status === 200) {
                setMsg($('regMsg'), '注册成功，正在进入…', 'ok');
                location.href = nextUrl();
                return;
            }
            setMsg($('regMsg'), messageOf(res, '注册失败'), 'error');
        });
    }

    // ===================== 退出 =====================

    function doLogout(keepCurrent) {
        var payload = keepCurrent ? { keep_current: true } : {};
        var path = keepCurrent ? '/api/auth/logout-all' : '/api/auth/logout';
        var msgNode = $('accountMsg');
        setMsg(msgNode, '正在处理…');
        api('POST', path, payload).then(function (res) {
            if (res.status !== 200) {
                setMsg(msgNode, messageOf(res, '操作失败'), 'error');
                return;
            }
            if (keepCurrent) {
                setMsg(msgNode, '已登出其他设备，本机保持登录', 'ok');
                loadSessions();
            } else {
                setMsg(msgNode, '已退出登录', 'ok');
                refreshState();
            }
        });
    }

    // ===================== 反向转场：点返回 =====================
    //
    // 顺序是刻意的（用户要的效果）：**先在本页把组件淡出**，再跳到首页并带上
    // ?from=account —— 首页那边会一进来就把画面摆成「本页最后一帧」（遮罩盖满 +
    // 头像已经是胶囊的样子），然后把圆圈缩回头像，等于把来时的过场倒放一遍。
    //
    // ⚠️ 返回按钮（.acc-home-btn）与顶栏**不参与淡出**：那颗按钮是反向形变的起点，
    // 它必须留在原地，等首页的胶囊接着它继续演。
    var EXIT_STEP_MS = 18;   // 组件之间退场的错开间隔
    var EXIT_MS = 260;       // 单个组件的退场时长（与 account.css 的 accFall 一致）
    var leaving = false;

    // 收集当前**可见**面板里的所有组件（含身份卡），逆序排延时
    function playExit() {
        var nodes = [];
        var all = document.querySelectorAll('.acc-wrap [data-enter]');
        for (var i = 0; i < all.length; i++) {
            // 藏着的面板里的组件不参与（closest 会把它们找出来）
            if (all[i].closest('[hidden]')) continue;
            nodes.push(all[i]);
        }

        for (var k = 0; k < nodes.length; k++) {
            // 逆序：最后进场的最先退场，看起来像「收拢回去」
            var el = nodes[nodes.length - 1 - k];
            for (var c = 0; c < ENTER_CLASSES.length; c++) el.classList.remove(ENTER_CLASSES[c]);
            el.style.animationDelay = (k * EXIT_STEP_MS) + 'ms';
            el.classList.add('is-exit');
        }
        return EXIT_MS + nodes.length * EXIT_STEP_MS;
    }

    function leaveToHome(event) {
        if (event) event.preventDefault();
        if (leaving) return;
        leaving = true;
        var wait = playExit();
        window.setTimeout(function () {
            window.location.href = '../index.html?from=account';
        }, wait);
    }

    // ===================== 启动 =====================

    document.addEventListener('DOMContentLoaded', function () {
        // 从首页点头像过来的（main.js 会带 ?from=avatar）：那时遮罩已经把整屏铺成
        // 与本页背景同一串渐变，身份卡在「遮罩下面」就该是就位的 —— 所以跳过它的入场动画，
        // 让「圆形铺满」和「个人中心出现」看起来是同一件事。
        if (/[?&]from=avatar(&|=|$)/.test(location.search)) {
            document.documentElement.className += ' from-avatar';
        } else {
            playEnter([document.querySelector('.acc-hero')]);
        }

        playTopbarIn();

        // 三处「回首页」的入口都走同一条反向转场：左上角按钮、品牌、操作卡里的「回到站点」
        ['accHome', 'accBack'].forEach(function (id) {
            var el = $(id);
            if (el) el.addEventListener('click', leaveToHome);
        });
        var brand = document.querySelector('.acc-brand');
        if (brand) brand.addEventListener('click', leaveToHome);

        var tabLogin = $('tabLogin');
        var tabRegister = $('tabRegister');
        if (tabLogin) tabLogin.addEventListener('click', function () { switchTab('login'); });
        if (tabRegister) tabRegister.addEventListener('click', function () { switchTab('register'); });

        var formLogin = $('formLogin');
        if (formLogin) formLogin.addEventListener('submit', doLogin);

        var formRegister = $('formRegister');
        if (formRegister) formRegister.addEventListener('submit', doRegister);

        var btnSendCode = $('btnSendCode');
        if (btnSendCode) btnSendCode.addEventListener('click', sendCode);

        var btnLogout = $('btnLogout');
        if (btnLogout) btnLogout.addEventListener('click', function () { doLogout(false); });

        var btnLogoutOthers = $('btnLogoutOthers');
        if (btnLogoutOthers) btnLogoutOthers.addEventListener('click', function () { doLogout(true); });

        // ⚠️ 不要再规范化邀请码输入框的内容：**空格是「多个邀请码」的分隔符**，
        // **`-` 是邀请码自身的格式**（后台靠它区分用途），大小写交给服务端处理。
        // （这里以前会把 `[\s-]` 全抹掉再转大写 —— 那正好把这两种语义都破坏了。）

        // 登录表单小字里的那个「注册」：点了直接切到注册标签页，并把光标放进邮箱框
        var linkToRegister = $('linkToRegister');
        if (linkToRegister) {
            linkToRegister.addEventListener('click', function () {
                switchTab('register');
                if ($('regEmail')) $('regEmail').focus();
            });
        }

        switchTab('login');
        // 把加载屏的高度**在首帧就定死**（此时各面板都藏着，量出来的是它们的自然高度）：
        // 加载卡按这个高度铺好之后，整段等待与交接期间页面不再有任何位移。
        scheduleStageFit();
        refreshState();
    });

    // 浏览器「前进后退缓存」（bfcache）把本页整页恢复回来时：退场动画已经把组件透明掉了，
    // 脚本又不会重跑 —— 不复位的话回来就是一片空白。这里把退场类摘掉并重播入场。
    window.addEventListener('pageshow', function (event) {
        if (!event.persisted) return;
        leaving = false;
        document.querySelectorAll('.is-exit').forEach(function (el) {
            el.classList.remove('is-exit');
            el.style.animationDelay = '';
        });
        playTopbarIn();
        // 按当前可见的那一组重播入场
        var group = 'auth';
        Object.keys(PANEL_GROUPS).forEach(function (key) {
            if (key === 'loading') return;
            PANEL_GROUPS[key].forEach(function (id) {
                var card = $(id);
                if (card && !card.hasAttribute('hidden')) group = key;
            });
        });
        showOnly(group);
    });

    // 暴露到 window 便于在控制台调试（playEnter 方便单独重放某个组件的入场动效）。
    // timing 是给「人工对齐转场时长」（见 TODO.md 第 4 条）用的只读快照：改值仍要动 TIMING。
    window.__guangxueAccount = {
        api: api,
        refreshState: refreshState,
        playEnter: playEnter,
        playExit: playExit,
        leaveToHome: leaveToHome,
        timing: {
            step: ENTER_STEP_MS,
            innerStep: INNER_STEP_MS,
            loadingFade: LOADING_FADE_MS,
            cardHeight: HEIGHT_MS
        }
    };
})();
