// 广学 · 账号中心（登录 / 注册 / 当前账号）
//
// 交互骨架：
//   1. 打开页面先问服务端「我是谁」——GET /api/auth/me
//        200 → 显示「当前账号」面板（含登录中的设备列表）
//        401 → 显示「登录 / 注册」面板
//   2. 登录态是服务端下发的 httpOnly Cookie，JS 读不到，所以只能靠这一步问服务端。
//   3. 本地开发时验证码不发邮件（AUTH_MAIL_MODE=log），发码后页面会调用
//      /api/auth/dev/codes 自动回填；生产环境该接口不存在（404），静默忽略即可。
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

    // ===================== 面板切换 =====================

    function showOnly(panelId) {
        ['panelLoading', 'panelAuth', 'panelAccount'].forEach(function (id) {
            var node = $(id);
            if (!node) return;
            if (id === panelId) {
                node.removeAttribute('hidden');
            } else {
                node.setAttribute('hidden', 'hidden');
            }
        });
    }

    function switchTab(which) {
        var isLogin = which === 'login';
        var tabLogin = $('tabLogin');
        var tabRegister = $('tabRegister');
        if (tabLogin) tabLogin.className = 'acc-tab' + (isLogin ? ' is-active' : '');
        if (tabRegister) tabRegister.className = 'acc-tab' + (isLogin ? '' : ' is-active');
        if ($('formLogin')) $('formLogin').hidden = !isLogin;
        if ($('formRegister')) $('formRegister').hidden = isLogin;
    }

    // ===================== 当前账号 =====================

    function showAccount(user) {
        showOnly('panelAccount');
        if ($('accEmail')) $('accEmail').textContent = user.email || '—';
        if ($('accUsername')) $('accUsername').textContent = user.username || '（未设置）';
        if ($('accRole')) $('accRole').textContent = user.role === 'admin' ? '管理员' : (user.role || 'user');
        if ($('accCreated')) $('accCreated').textContent = user.created_at || '—';
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
            showOnly('panelAuth');
            switchTab('login');
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

    // 暴露到 window 便于在控制台调试
    window.__guangxueAccount = { api: api, refreshState: refreshState };
})();
