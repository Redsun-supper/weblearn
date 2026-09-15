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

    // 统一请求：同源自动带 Cookie（credentials: 'same-origin'），自动 JSON，
    // 不抛异常而是把 { status, data } 交给调用方，便于区分 401 / 业务错误 / 网络错误。
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
                    // 非 JSON（例如网关的 502 页面）保持 null
                    data = null;
                }
                return { status: res.status, ok: res.ok, data: data };
            });
        }).catch(function (err) {
            // 网络层失败（服务没起来 / 断网）：给一个能看懂的结论，别让它变成未捕获异常
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

    // 组件之间的错开间隔（毫秒）：「什么时候开始」在这里排延时，
    // 「怎么动」写在 account.css 的 accRise（浮起淡入）/ accPop（头像回弹）。
    var ENTER_STEP_MS = 55;

    // 面板分组：一组里的卡片一起显示 / 隐藏
    // （个人中心把「账号信息 / 登录设备 / 可用操作」拆成了三张卡，所以是分组而不是单块）
    var PANEL_GROUPS = {
        loading: ['panelLoading'],
        auth: ['panelAuth'],
        account: ['panelAccount', 'panelDevices', 'panelActions']
    };

    // 收集要参与入场的组件：卡片本身（带 data-enter 的）在前，卡片内部的 [data-enter] 随后。
    // 顺序即延时顺序，所以「卡 → 标题 → 每一行」会依次浮入。
    function collectEnterNodes(cards) {
        var nodes = [];
        for (var i = 0; i < cards.length; i++) {
            var card = cards[i];
            if (!card || card.hasAttribute('hidden')) continue;
            if (card.hasAttribute('data-enter')) nodes.push(card);
            var inner = card.querySelectorAll('[data-enter]');
            for (var k = 0; k < inner.length; k++) {
                if (!inner[k].hasAttribute('hidden')) nodes.push(inner[k]);
            }
        }
        return nodes;
    }

    // 三种入场类：默认「浮起淡入」，切标签页时按方向「从侧边滑入」（动作在 account.css）
    var ENTER_CLASSES = ['is-enter', 'is-enter-right', 'is-enter-left'];

    // 给这些组件排一遍入场动效。
    //   mode：'' = 浮起淡入 | 'right' = 从右侧滑入 | 'left' = 从左侧滑入
    // 每次都先摘类再挂：① display:none 的元素不会播动画，所以必须在显示之后再挂；
    // ② 同一块面板重新显示时要能重播，所以中间强制一次重排。
    function playEnter(cards, mode) {
        var nodes = collectEnterNodes(cards);
        var cls = mode === 'right' ? 'is-enter-right' : (mode === 'left' ? 'is-enter-left' : 'is-enter');
        for (var i = 0; i < nodes.length; i++) {
            var el = nodes[i];
            for (var k = 0; k < ENTER_CLASSES.length; k++) el.classList.remove(ENTER_CLASSES[k]);
            void el.offsetWidth;
            el.style.animationDelay = (i * ENTER_STEP_MS) + 'ms';
            el.classList.add(cls);
        }
    }

    function showOnly(group) {
        var visible = PANEL_GROUPS[group] || [];
        Object.keys(PANEL_GROUPS).forEach(function (key) {
            PANEL_GROUPS[key].forEach(function (id) {
                var node = $(id);
                if (!node) return;
                if (visible.indexOf(id) >= 0) {
                    node.removeAttribute('hidden');
                } else {
                    node.setAttribute('hidden', 'hidden');
                }
            });
        });
        // 显示之后再排动效：几块面板会按顺序依次浮入
        playEnter(visible.map(function (id) { return $(id); }));
    }

    // 卡片高度过渡的时长（毫秒）：只在这里写一次，过渡是行内设的，不用和 CSS 对齐
    var HEIGHT_MS = 380;
    var heightTimer = null;

    // 白色底的高度过渡：切标签时「往下长」/「往上收」，而不是瞬间跳变。
    // 经典做法 —— 量旧高 → 换内容 → 量新高 → 从旧高过渡到新高 → 收尾还原成自动高度：
    //   · 过渡期间必须 overflow:hidden，否则新表单会先整个冲出来、再被裁下去
    //   · 收尾一定要清掉行内 height/overflow，让卡片回到「由内容决定高度」
    //     （否则报错文案、窄屏换行把内容撑高时会被裁掉）
    function animateCardHeight(card, from, to) {
        if (!card || from === to || prefersReducedMotion()) return;

        card.style.overflow = 'hidden';
        card.style.transition = 'none'; // 先把起点钉死，别把上一次没跑完的过渡续上
        card.style.height = from + 'px';
        void card.offsetHeight;         // 强制重排，这一帧就是过渡起点

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

    function switchTab(which) {
        var isLogin = which === 'login';
        var card = $('panelAuth');
        // 只在卡片已经显示时才量高度：首屏 showOnly 之前它是 hidden，量出来是 0，
        // 那种情况交给入场动效，不要再额外播一次高度过渡
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

        // 滑块滑到对应的位置（首屏就是「登录」，所以初次进来它已经在左边、不会先滑一下）
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

        // 表单顺着点击方向滑进来：切到「注册」从右边来，切回「登录」从左边来
        // （藏起来的那张会被 collectEnterNodes 跳过）
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
            role.textContent = user.role === 'admin' ? '管理员' : '普通用户';
            role.removeAttribute('hidden');
        }
    }

    function showAccount(user) {
        showOnly('account');
        setHero(user);
        if ($('accEmail')) $('accEmail').textContent = user.email || '—';
        if ($('accUsername')) $('accUsername').textContent = user.username || '（未设置）';
        if ($('accRole')) $('accRole').textContent = user.role === 'admin' ? '管理员' : (user.role || 'user');
        if ($('accCreated')) $('accCreated').textContent = user.created_at || '—';

        // 管理员多一个入口：直接进后台（非管理员看不到这个按钮）
        var adminLink = $('accAdmin');
        if (adminLink) {
            if (user.role === 'admin') {
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
            if (res.status !== 200 || !res.data || !res.data.data) {
                list.appendChild(sessionRow('读取失败：' + messageOf(res), ''));
                return;
            }
            var items = res.data.data.items || [];
            if (!items.length) {
                list.appendChild(sessionRow('没有活跃会话', ''));
                return;
            }
            items.forEach(function (item) {
                var meta = [];
                if (item.current) meta.push('当前设备');
                meta.push('最近活跃 ' + (item.last_used_at || item.created_at || '—'));
                if (item.ip && item.ip !== 'unknown') meta.push('IP ' + item.ip);
                list.appendChild(sessionRow(item.device_label || '未知设备', meta.join(' · '), item.current));
            });
        });
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

    // 问服务端「我是谁」：决定显示哪一块面板
    function refreshState() {
        return api('GET', '/api/auth/me').then(function (res) {
            if (res.status === 200 && res.data && res.data.data && res.data.data.user) {
                showAccount(res.data.data.user);
                return true;
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
            return false;
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
            setMsg($('loginMsg'), '请填写邮箱与口令', 'error');
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
            // 登录失败后口令清空，避免误以为还可以直接重试
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
        var invite = valueOf('regInvite');
        if (!email || !invite) {
            setMsg($('regMsg'), '请先填写邮箱与邀请码', 'error');
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
        var invite = valueOf('regInvite');
        var code = valueOf('regCode');
        var password = $('regPassword') ? $('regPassword').value : '';
        var password2 = $('regPassword2') ? $('regPassword2').value : '';
        var btn = $('regSubmit');

        if (!email || !invite || !code || !password) {
            setMsg($('regMsg'), '请把邮箱、邀请码、验证码、口令都填完整', 'error');
            return;
        }
        if (password !== password2) {
            setMsg($('regMsg'), '两次输入的口令不一致', 'error');
            return;
        }
        if (password.length < 8) {
            setMsg($('regMsg'), '口令至少 8 个字符，且同时包含字母与数字', 'error');
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
                // 重新问一次状态：此时应当回到「登录 / 注册」面板
                refreshState();
            }
        });
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

        // 邀请码统一成大写（服务端也会规范化，这里只是让输入框看起来干净）
        var inviteInput = $('regInvite');
        if (inviteInput) {
            inviteInput.addEventListener('blur', function () {
                inviteInput.value = inviteInput.value.replace(/[\s-]/g, '').toUpperCase();
            });
        }

        switchTab('login');
        refreshState();
    });

    // 暴露到 window 便于在控制台调试（playEnter 方便单独重放某个组件的入场动效）
    window.__guangxueAccount = { api: api, refreshState: refreshState, playEnter: playEnter };
})();
