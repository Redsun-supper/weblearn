// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
// 广学 · 个人中心（账号 / 登录设备 / 操作）
//
// 交互骨架：
//   1. 打开页面先问服务端「我是谁」——GET /api/auth/me
//        200 → 显示主布局（侧栏 + 主区），内容按 hash 路由
//        401 → 显示整屏居中的「登录 / 注册」门禁
//   2. 登录态是服务端下发的 httpOnly Cookie，JS 读不到，所以只能靠这一步问服务端。
//   3. 本地开发时验证码不发邮件（AUTH_MAIL_MODE=log），发码后页面会调用
//      /api/auth/dev/codes 自动回填；生产环境该接口不存在（404），静默忽略即可。
//   4. 组件入场动效：动作定义在 account.css（accRise / accPop），
//      这里只负责「什么时候开始」—— 按可见顺序逐个写 animation-delay（playEnter）。
//      入口是首页左上角的头像（index.html 的 #navAvatar），进来时会带 ?from=avatar，
//      那种情况下跳过入场动画，跟首页那块扩散遮罩衔接成一次动画。
//
// 布局与后台 admin/ 同一套（侧栏导航 + 顶栏 + 内容区 + hash 路由），
// 所以「加一个页面」= NAV 里加一项 + HTML 里加一个 .acc-panel，不用碰样式。
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

    // 角色显示名（P0-5 三级角色）。`super_admin` 不加这层映射就会原样显示成英文。
    function roleLabel(role) {
        if (role === 'super_admin') return '超级管理员';
        if (role === 'admin') return '管理员';
        return '普通用户';
    }

    // 管理员与超管都能进后台（与 admin.js 的 canEnterAdmin 同口径）
    function canEnterAdmin(role) {
        return role === 'admin' || role === 'super_admin';
    }

    // ===================== 过渡节奏 =====================

    // 想微调「检测屏 → 真界面」那一段的手感，改这里的数字就够了
    // （一起对齐的还有 account.css 里 .acc-check 的淡出、.acc-gate-box 的弹入、
    //   .acc-sidebar 的浮起 —— 三处时长都在自己的规则旁边写了「与谁对齐」）。
    //   step          卡与卡之间的入场错开    innerStep  卡内组件的入场错开
    //   screenFade    检测屏淡出时长（动作是 account.css 的 accCheckOut）
    //   gateDelay     白卡比检测屏晚多久起手（动作是 accCardIn，缓动带一点回弹）
    //   cardIn        白卡弹入的时长（与 .acc-gate-box.is-arriving 的 animation 一致）
    //   —— 这三个数的关系就是「接力」：screenFade 要明显长于 gateDelay，
    //      白卡才会在检测屏还没淡完的时候就开始浮出来。实测过一次「生硬弹出」：
    //      gateDelay 晚于 screenFade，检测屏已经淡到 0 而白卡还是 opacity: 0，
    //      中间那几十毫秒屏幕是**空的**，看着就是「啪」地弹出一张卡。
    //      现在 40ms 起手 / 150ms 淡完 / 470ms 落定，两段有三四百毫秒是交叠的。
    //   gateFade      「检测完发现已登录」时整块门禁淡出、露出侧栏布局的时长
    //   cardHeight    切「登录 / 注册」标签时白卡长高 / 收短的时长
    var TIMING = {
        step: 30,
        innerStep: 12,
        screenFade: 400,
        gateDelay: 40,
        cardIn: 470,
        gateFade: 160,
        cardHeight: 380
    };

    var ENTER_STEP_MS = TIMING.step;
    var INNER_STEP_MS = TIMING.innerStep;
    var SCREEN_FADE_MS = TIMING.screenFade;
    var GATE_DELAY_MS = TIMING.gateDelay;
    var CARD_IN_MS = TIMING.cardIn;
    var GATE_FADE_MS = TIMING.gateFade;
    var HEIGHT_MS = TIMING.cardHeight;

    // 入场动画里最长的那一个（accRise 0.42s —— 见 account.css）：动画结束后延迟摘
    // is-animating（合成层提示）要用它算时间。
    var ANIM_LONGEST_MS = 420;
    var animatingTimer = null;
    var screenFadeTimer = null;
    var gateCardTimer = null;
    var heightTimer = null;

    // 三种入场类：默认「浮起淡入」，切标签页时按方向「轻轻浮上来」（动作在 account.css）
    var ENTER_CLASSES = ['is-enter', 'is-enter-right', 'is-enter-left'];

    // 白卡自己的弹入类（动作是 account.css 的 accCardIn，带一点回弹）
    var ARRIVING_CLASS = 'is-arriving';
    // 检测屏的淡出类（动作是 account.css 的 accCheckOut）
    var CHECK_OUT_CLASS = 'is-leaving';

    // 动画期间挂这个类（CSS 里给它 will-change: transform, opacity）：
    // 让浏览器把组件提升成合成层，整段动画只做合成、不重绘。动画跑完就摘掉。
    var ANIMATING_CLASS = 'is-animating';

    // 收集要参与入场的组件：**区块本身**（带 data-enter 的卡片）在前，块内的 [data-enter] 随后。
    // 顺序即延时顺序，所以「卡 → 标题 → 每一行」会依次浮入。
    // 每项都带上「这是第几块 / 块内第几个」——排延时时两者用的步长不一样（见 playEnter）。
    //
    // ⚠️ **藏在 [hidden] 里的组件一概跳过**：设备面板藏着时，它里面那几个 [data-enter]
    //    其实也在 DOM 里，早先它们会跟着一起跑动画（不可见，却照样占着合成与重绘的时间）。
    //    判据用 closest（祖先里只要有 hidden 就跳过）—— 只判断元素自己的 hidden 是不够的。
    function collectEnterNodes(roots) {
        var items = [];
        var blockIndex = 0;
        for (var i = 0; i < roots.length; i++) {
            var root = roots[i];
            if (!root || root.closest('[hidden]')) continue;
            if (root.hasAttribute('data-enter')) {
                items.push({ el: root, block: blockIndex, inner: 0 });
            }
            var inner = root.querySelectorAll('[data-enter]');
            var n = 0;
            for (var k = 0; k < inner.length; k++) {
                if (inner[k].closest('[hidden]')) continue;
                n += 1;
                items.push({ el: inner[k], block: blockIndex, inner: n });
            }
            blockIndex += 1;
        }
        return items;
    }

    // 给这些组件排一遍入场动效。
    //   mode：'rise' = 轻轻浮上来（默认）| 'right' / 'left' = 顺着点击方向偏一点浮上来
    //   baseDelay（毫秒，可选）：整体推迟多久开始 —— 交给调用方做「接力」用
    //    （换屏时先让检测屏淡出，真界面再起手）
    //
    // ⚠️ 性能要点：先把所有元素的旧动画类摘掉（只写不读），**只强制重排一次**，
    // 最后统一挂新类。早先的写法是「逐个元素：摘类 → 读 offsetWidth → 挂类」，
    // 每个 offsetWidth 都会让浏览器同步重算整页布局。
    function playEnter(roots, mode, baseDelay) {
        var items = collectEnterNodes(roots);
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
            items[i].el.style.animationDelay = (base + items[i].block * ENTER_STEP_MS + items[i].inner * INNER_STEP_MS) + 'ms';
            items[i].el.classList.add(cls, ANIMATING_CLASS);
        }

        // 动画跑完就把合成层的提示摘掉（最后起手的那一个 + 它的时长）。上面每次排新动画都会
        // 重新计时，所以这一条定时器不会误摘还在跑的层。
        clearTimeout(animatingTimer);
        animatingTimer = window.setTimeout(function () {
            animatingTimer = null;
            for (i = 0; i < items.length; i++) items[i].el.classList.remove(ANIMATING_CLASS);
        }, base + (items.length - 1) * INNER_STEP_MS + ENTER_STEP_MS + ANIM_LONGEST_MS + 120);
    }

    // 侧栏品牌那两行的入场：比内容稍晚一点，看起来是「侧栏先立起来，内容再填进去」
    function playSidebarIn() {
        playEnter([document.querySelector('.acc-sidebar')]);
    }

    // ===================== 两块界面：门禁 / 主布局 =====================

    // 当前登录用户（null = 未登录）；路由要靠它决定侧栏显示什么
    var currentUser = null;

    // 检测屏 → 门禁白卡。这一段的要点是「**接力**」，不是一个消失再一个冒出来：
    //   0ms        给检测屏挂 is-leaving（account.css 的 accCheckOut：淡出 + 缩到 0.94 + 模糊，
    //              像被推走；时长 TIMING.screenFade）
    //   gateDelay  白卡解除 hidden 并挂 is-arriving（accCardIn：从 0.9 倍 + 下移 10px 弹出来，
    //              缓动 c5 带一点回弹 —— 也就是「q 弹」那一下）
    //   两者交叠三四百毫秒：白卡浮到可见的时候检测屏还留着大半，看着像检测屏**变成了**白卡，
    //   而不是换了一张脸（早先的版本两段不交叠，中间那几十毫秒屏幕是空的 = 「生硬地弹出来」）
    //
    // ⚠️ 传入的是**卡片里的表单**而不是卡片本身：卡片自己已经由 is-arriving 弹入了，
    //    这里再把整张卡当成一个块去播 accRise 会两套动画打架
    //    （早先的实测就是卡片整块浮、字段又各自浮，看着糊成一团）。
    //    字段的起手排在 gateDelay + step 之后，即白卡浮到可见之后才开始「一个一个浮出来」。
    function showGate() {
        var check = $('accCheck');
        var box = $('accAuthBox');
        if (!check || !box) return;

        var form = $('formLogin');
        if (!form || form.hidden) form = $('formRegister');

        // 检测屏已经在淡出了就别重复排（refreshState 重试时会再调一次）
        var checkVisible = !check.hasAttribute('hidden') && !check.classList.contains(CHECK_OUT_CLASS);
        clearTimeout(screenFadeTimer);
        clearTimeout(gateCardTimer);

        // 白卡登场（两条路都要走这一段）
        var arrive = function () {
            box.removeAttribute('hidden');
            box.classList.add(ARRIVING_CLASS, ANIMATING_CLASS);
            if (form) playEnter([form], 'rise', GATE_DELAY_MS + ENTER_STEP_MS);
            // 弹入跑完把合成层提示摘掉（元素还留着 is-arriving，它没有 transition，
            // 留着不影响；真正让它「定格」的是 keyframes 的 forwards / backwards 填充）
            gateCardTimer = window.setTimeout(function () {
                gateCardTimer = null;
                box.classList.remove(ANIMATING_CLASS);
            }, GATE_DELAY_MS + CARD_IN_MS + 120);
        };

        if (checkVisible) {
            check.classList.add(CHECK_OUT_CLASS);
            // 淡完才藏：藏早了会在淡到一半时「啪」地消失，那正是要修掉的手感
            screenFadeTimer = window.setTimeout(function () {
                screenFadeTimer = null;
                check.setAttribute('hidden', 'hidden');
                check.classList.remove(CHECK_OUT_CLASS);
            }, SCREEN_FADE_MS);
            gateCardTimer = window.setTimeout(arrive, GATE_DELAY_MS);
            return;
        }

        // 检测屏早就没了（比如登录态重试）→ 白卡直接出现，不排队
        arrive();
    }

    // 显示主布局（侧栏 + 主区），并把路由挂上
    // 侧栏与主区（登录态已确认）：
    //   · 布局淡入（accLayoutIn，很短）—— 门禁还整屏盖着，它在下面排好版了，淡进来不换脸
    //   · 门禁整块淡出（GATE_FADE_MS）
    //   · 检测屏**必须立刻藏**：它是绝对定位在 `.acc-gate`（整屏）正中的，而侧栏布局下
    //     「正中」落在主区偏左的位置 —— 门禁淡出的那 160ms 里它会从侧栏旁边飘出来一次
    //     （实测截图里看到过「正在检查登录状态…」出现在主区空白处）。已登录这条路上
    //     它本来就没有交代，直接摘掉最干净。
    function showLayout(user) {
        var gate = $('accGate');
        var layout = $('accLayout');
        if (!gate || !layout) return;

        var check = $('accCheck');
        clearTimeout(screenFadeTimer);
        screenFadeTimer = null;
        if (check && !check.hasAttribute('hidden')) {
            check.classList.remove(CHECK_OUT_CLASS);
            check.setAttribute('hidden', 'hidden');
        }

        layout.removeAttribute('hidden');
        layout.classList.add(ANIMATING_CLASS, ARRIVING_CLASS);
        playSidebarIn();
        if (gate.hasAttribute('hidden')) return;

        // 门禁整块淡出（它只在「检测完发现已登录」这条路上会可见）
        gate.classList.add('is-animating');
        gate.style.transition = 'opacity ' + GATE_FADE_MS + 'ms linear';
        gate.style.opacity = '0';
        screenFadeTimer = window.setTimeout(function () {
            screenFadeTimer = null;
            gate.setAttribute('hidden', 'hidden');
            gate.style.transition = '';
            gate.style.opacity = '';
            gate.classList.remove('is-animating');
            layout.classList.remove(ANIMATING_CLASS, ARRIVING_CLASS);
        }, GATE_FADE_MS);
    }

    // ===================== 侧栏导航与路由 =====================

    // 加一个页面 = 这里加一项 + HTML 里加一个同 id 的 .acc-panel（顺序即侧栏顺序）
    var NAV_GROUPS = [
        { id: 'profile', name: '账号信息' },
        { id: 'devices', name: '登录过得设备' },
        { id: 'actions', name: '可用操作' }
    ];

    var DEFAULT_GROUP = NAV_GROUPS[0].id;
    var currentGroup = null;

    function findGroup(id) {
        for (var i = 0; i < NAV_GROUPS.length; i++) {
            if (NAV_GROUPS[i].id === id) return NAV_GROUPS[i];
        }
        return null;
    }

    // 分组 id → 面板元素 id（profile → panelProfile）。
    // 约定就是「panel + 首字母大写」，加页面时按这个命名即可，不用改这里。
    function panelIdOf(groupId) {
        return 'panel' + groupId.charAt(0).toUpperCase() + groupId.slice(1);
    }

    // 当前 hash（#/devices → devices）。空 / 不合法都给默认那一组。
    function groupFromHash() {
        var id = String(location.hash || '').replace(/^#\/?/, '');
        return findGroup(id) ? id : DEFAULT_GROUP;
    }

    // 渲染侧栏导航。会话台数（.acc-nav-note）由 loadSessions 填，
    // 所以重渲染后要把那个数字再补回去，别让它一闪就没了。
    function renderNav() {
        var nav = $('accNav');
        if (!nav) return;
        nav.innerHTML = '';

        NAV_GROUPS.forEach(function (entry) {
            var item = document.createElement('a');
            item.className = 'acc-nav-item';
            item.href = '#/' + entry.id;
            item.setAttribute('data-group', entry.id);
            item.textContent = entry.name;

            if (entry.id === 'devices') {
                var note = document.createElement('span');
                note.className = 'acc-nav-note';
                note.id = 'navDevicesCount';
                note.hidden = true;
                item.appendChild(note);
            }
            nav.appendChild(item);
        });
    }

    function setActiveNav(groupId) {
        var items = document.querySelectorAll('.acc-nav-item');
        for (var i = 0; i < items.length; i++) {
            var on = items[i].getAttribute('data-group') === groupId;
            items[i].className = 'acc-nav-item' + (on ? ' is-active' : '');
        }
    }

    // hash 路由：#/devices → 显示那一块面板
    function route() {
        if (!currentUser) return;              // 没登录时没有导航可点
        var id = groupFromHash();

        // 空 hash（刚进来）时补一个默认值：让地址栏与侧栏高亮始终一致
        if (String(location.hash || '').replace(/^#\/?/, '') !== id) {
            location.hash = '#/' + id;         // 会触发 hashchange 再进一次 route()
            return;
        }

        var changed = id !== currentGroup;
        currentGroup = id;

        NAV_GROUPS.forEach(function (entry) {
            var panel = $(panelIdOf(entry.id));
            if (!panel) return;
            if (entry.id === id) {
                panel.removeAttribute('hidden');
            } else {
                panel.setAttribute('hidden', 'hidden');
            }
        });

        var entry = findGroup(id);
        if ($('accPageTitle')) $('accPageTitle').textContent = entry ? entry.name : '个人中心';
        setActiveNav(id);

        // 只在真的换了页面时才播入场（同页重渲染会重跑动画，看着像闪）
        if (changed) {
            playEnter([$(panelIdOf(id))]);
        }
        if (id === 'devices') loadSessions();
    }

    // ===================== 当前账号 =====================

    function applyUser(user) {
        currentUser = user;
        if ($('accEmail')) $('accEmail').textContent = user.email || '—';
        if ($('accUsername')) $('accUsername').textContent = user.username || '（未设置）';
        if ($('accRole')) $('accRole').textContent = roleLabel(user.role);
        if ($('accCreated')) $('accCreated').textContent = user.created_at || '—';
        if ($('topUserName')) $('topUserName').textContent = user.username || user.email || '已登录';
        if ($('accAvatar')) $('accAvatar').title = (user.username || user.email || '') + ' · 个人中心';

        var adminLink = $('accAdmin');
        if (adminLink) {
            // 管理员与超管都能进后台（与 admin.js 的 canEnterAdmin 同口径）
            if (canEnterAdmin(user.role)) {
                adminLink.removeAttribute('hidden');
            } else {
                adminLink.setAttribute('hidden', 'hidden');
            }
        }
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
        });
    }

    // 会话台数：侧栏导航上一个小数字 + 「登录过得设备」标题旁的括号。
    // 列表超过可视高度时会滚动，不给个数字用户不知道自己还有几台。
    function renderSessionCount(n) {
        var label = n > 0 ? String(n) : '';
        var inNav = $('navDevicesCount');
        if (inNav) {
            inNav.textContent = label;
            if (n > 0) inNav.removeAttribute('hidden');
            else inNav.setAttribute('hidden', 'hidden');
        }
        var inTitle = $('devicesCount');
        if (inTitle) {
            inTitle.textContent = n > 0 ? '（' + n + ' 台）' : '';
            if (n > 0) inTitle.removeAttribute('hidden');
            else inTitle.setAttribute('hidden', 'hidden');
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
    // 本地 /me 只要几毫秒，一返回就换屏的话它会一闪而过，后面的表单像凭空冒出来。
    // 语义：响应晚于阈值 → 回来后立刻开始；早于阈值 → 等到阈值再开始（见 refreshState）。
    var MIN_CHECK_MS = 600;

    // 问服务端「我是谁」：决定显示门禁还是主布局
    function refreshState() {
        var t0 = Date.now();
        return api('GET', '/api/auth/me').then(function (res) {
            setBackendStatus(res);
            var wait = Math.max(0, MIN_CHECK_MS - (Date.now() - t0));
            return new Promise(function (resolve) {
                window.setTimeout(function () {
                    if (res.status === 200 && res.data && res.data.data && res.data.data.user) {
                        var user = res.data.data.user;
                        applyUser(user);
                        showLayout(user);
                        route();
                        resolve(true);
                        return;
                    }
                    // 顺序有讲究：先把门禁摆出来（此刻它还是藏着的，字段入场会被跳过），
                    // 再由 showGate 把白卡按「浮起淡入」放出来 —— 首屏是统一的入场动画。
                    currentUser = null;
                    switchTab('login', false);
                    showGate();
                    if (res.networkError) {
                        setMsg($('loginMsg'), messageOf(res), 'error');
                    }
                    resolve(false);
                }, wait);
            });
        });
    }

    // 顶栏那个服务端连通性指示（与 admin.js 的 backendStatus 同款）
    function setBackendStatus(res) {
        var el = $('backendStatus');
        if (!el) return;
        if (res && res.status === 200) {
            el.textContent = '服务正常';
            el.className = 'acc-backend is-ok';
        } else if (res && res.networkError) {
            el.textContent = '连不上账号服务';
            el.className = 'acc-backend is-bad';
        } else {
            el.textContent = '未登录';
            el.className = 'acc-backend';
        }
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

    // ===================== 切「登录 / 注册」标签 =====================

    // 白卡的高度过渡：切标签时「往下长」/「往上收」，而不是瞬间跳变。
    // 经典做法 —— 量旧高 → 换内容 → 量新高 → 从旧高过渡到新高 → 收尾还原成自动高度：
    //   · 过渡期间必须 overflow:hidden，否则新表单会先整个冲出来、再被裁下去
    //   · 收尾一定要清掉行内 height/overflow，让卡片回到「由内容决定高度」
    //     （否则报错文案、窄屏换行把内容撑高时会被裁掉）
    //   · 起点前先把 transition 关掉，别把上一次没跑完的过渡续上；再强制重排一次
    function animateCardHeight(card, from, to) {
        if (!card || from === to || prefersReducedMotion()) return;

        card.style.overflow = 'hidden';
        card.style.transition = 'none';
        card.style.height = from + 'px';
        void card.offsetHeight;

        card.style.transition = 'height ' + HEIGHT_MS + 'ms cubic-bezier(0.22, 0.61, 0.36, 1)';
        card.style.height = to + 'px';

        clearTimeout(heightTimer);
        heightTimer = window.setTimeout(function () {
            heightTimer = null;
            card.style.transition = '';
            card.style.height = '';
            card.style.overflow = '';
        }, HEIGHT_MS + 40);
    }

    // 切「登录 / 注册」标签：滑块就位 → 换表单 → 白卡高度过渡 → 新表单顺着点击方向浮入
    // （切到「注册」从下方来、切回「登录」从上方来，与滑块的移动方向一致）。
    //   animate：要不要给新表单排入场动效。首屏（门禁还没显示出来时）由 showGate 接手，
    //            不然这边的 playEnter 会因为元素还是 hidden 而整批跳过，字段就白等一场。
    // ⚠️ 只在卡片已经显示时才量高度：首屏它是 hidden，量出来是 0，
    //    那种情况交给入场动效，不要再额外播一次高度过渡。
    function switchTab(which, animate) {
        var isLogin = which === 'login';
        var card = $('accAuthBox');
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

        // animate === false 时只换内容不排动画（首屏交给 showGate，见它的注释）
        if (animate !== false) {
            playEnter([isLogin ? $('formLogin') : $('formRegister')], isLogin ? 'left' : 'right');
        }
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
                // 退出后要回到门禁：整页重来最省事，也不会留下半个已登录的界面
                window.location.href = location.pathname;
            }
        });
    }

    // ===================== 反向转场：点返回 =====================
    //
    // 顺序是刻意的（用户要的效果）：**先在本页把组件淡出**，再跳到首页并带上
    // ?from=account —— 首页那边会一进来就把画面摆成「本页最后一帧」（遮罩盖满 +
    // 头像已经在侧栏的位置），然后把那一小块缩回导航栏头像，等于把来时的过场倒放一遍。
    //
    // ⚠️ 侧栏**不参与淡出**（它不在 data-enter 的收集范围内，只在入场时用）：
    //    侧栏顶部那颗头像正是反向形变的起点，它必须留在原地，等首页那份接着演。
    var EXIT_STEP_MS = 18;   // 组件之间退场的错开间隔
    var EXIT_MS = 260;       // 单个组件的退场时长（与 account.css 的 accFall 一致）
    var leaving = false;

    // 收集当前**可见**的组件（含侧栏品牌那两行），逆序排延时
    function playExit() {
        var nodes = [];
        var all = document.querySelectorAll('.acc-layout [data-enter]');
        for (var i = 0; i < all.length; i++) {
            // 藏着的面板里的组件不参与（closest 会把它们找出来）
            if (all[i].closest('[hidden]')) continue;
            nodes.push(all[i]);
        }
        // 侧栏品牌也是可见的，但它不在面板里，单独补上（顺序放最后 = 最先退场）
        var brand = document.querySelector('.acc-brand');
        if (brand && !brand.closest('[hidden]')) nodes.push(brand);

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
        // 与本页背景同一串渐变，侧栏与内容在「遮罩下面」就该是就位的 —— 所以跳过整页入场，
        // 让「圆形铺满」和「个人中心出现」看起来是同一件事。
        if (/[?&]from=avatar(&|=|$)/.test(location.search)) {
            document.documentElement.className += ' from-avatar';
        }

        renderNav();

        // 三处「回首页」的入口都走同一条反向转场：侧栏页脚那颗、品牌、操作卡里的「回到站点」
        ['accHome', 'accBack'].forEach(function (id) {
            var el = $(id);
            if (el) el.addEventListener('click', leaveToHome);
        });
        var brand = document.querySelector('.acc-brand');
        if (brand) brand.addEventListener('click', leaveToHome);

        window.addEventListener('hashchange', route);

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

        var btnLogout = $('accLogout');
        if (btnLogout) btnLogout.addEventListener('click', function () { doLogout(false); });

        var btnLogout2 = $('btnLogout');
        if (btnLogout2) btnLogout2.addEventListener('click', function () { doLogout(false); });

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

        switchTab('login', false);   // 首屏的门禁动画由 refreshState → showGate 接手
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
        if (!currentUser) return;
        playSidebarIn();
        currentGroup = null;   // 强制重播当前页面的入场
        route();
    });

    // 暴露到 window 便于在控制台调试（playEnter 方便单独重放某个组件的入场动效）。
    // timing 是给「人工对齐转场时长」用的只读快照：改值仍要动 TIMING。
    window.__guangxueAccount = {
        api: api,
        refreshState: refreshState,
        playEnter: playEnter,
        playExit: playExit,
        leaveToHome: leaveToHome,
        route: route,
        timing: {
            step: ENTER_STEP_MS,
            innerStep: INNER_STEP_MS,
            screenFade: SCREEN_FADE_MS,
            gateDelay: GATE_DELAY_MS,
            cardIn: CARD_IN_MS,
            gateFade: GATE_FADE_MS,
            cardHeight: HEIGHT_MS
        }
    };
})();
