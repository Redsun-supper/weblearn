// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
// 管理面板 · 用户管理（P1）
//
// 由壳动态 import 后调用 mount(container, ctx)。本模块**只认 ctx**，不碰任何一个页面的东西。
//
// ctx 提供：
//   api(path, {method, body}) → Promise<信封 {code,message,data}>；非 2xx 或 code!==200 会 throw
//   el(tag, attrs, children)  → DOM 构建器（{text} 走 textContent）
//   toast(msg, kind) / confirm(msg) → Promise<boolean> / escapeHtml(s) / setTitle(s) / user
//
// 用到的四个接口：
//   ① GET  /api/auth/admin/users?role=&status=&keyword=&page=&size=   —— Rust（治理数据的唯一来源），管理员+ 只读
//        响应：{items:[{id,email,username,role,status,email_verified_at,last_login_at,created_at}],
//               total, page, size, email_masked}
//   ② GET  /api/admin/users/progress?ids=1,2,3                       —— Go，管理员+（当页批量进度，一个请求）
//        响应：{items:[{user_id,learned_words,total_reviews,streak_days,last_review_at}]}
//   ③ POST /api/auth/admin/users/{id}/role        body {role}       —— **超管**：改角色
//   ④ POST /api/auth/admin/users/{id}/status      body {status}     —— **超管**：封禁 / 解封
//      POST /api/auth/admin/users/{id}/logout-all  body {}           —— **超管**：踢下线，回 {user_id,revoked}
//
// 两条容易踩的坑：
//   - **邮箱脱敏在服务端做**：普通管理员看到的 email 已经是 `22***@qq.com`，响应里
//     `email_masked=true`。这时列表上方挂一行说明，并且不提供任何「复制邮箱」按钮
//     （复制的会是打码串，拿去发信只会失败）；
//   - 进度请求的 id 来自列表响应，所以它只能排在列表**之后**发；两个请求用 Promise.all 一起等，
//     进度**只发一个批量请求**（不是每人一个 N+1）。请求里没出现的 id 按 0 显示。
//
// 权限：普通管理员整页只读（没有操作列里的三个按钮）；治理动作服务端还会再挡一道。
//
// 风格：ES5（var/function），只用 export —— 与 modules/english/admin/english-admin.js 一致。

import { fmtNum, fmtTime, idsQuery, roleLabel, userStatusLabel } from './util.js';

export var meta = {
    id: 'users',
    title: '用户管理',
    desc: '账号、角色、状态与复习进度',
    roles: ['admin', 'super_admin']
};

// 表格列数：空行 / 展开行都用它做 colspan，加列时只改这一处
var COLS = 12;

var ctx = null;
var root = null;
// 骨架只建一次：刷新只重渲染数据区（重建整块会让输入框失焦、正在填的关键词也没了）
var nodes = {};

var state = {
    keyword: '',
    role: '',
    status: '',
    size: 20,
    page: 1,
    total: 0,
    items: [],
    emailMasked: false,
    progress: {}, // 用户 id（字符串键）→ 进度条目
    detail: null, // { user_id, loading, data, error }：当前展开的那一行
    busy: false
};

// ===================== 生命周期 =====================

export function mount(container, context) {
    ctx = context;
    root = container;
    state.keyword = '';
    state.role = '';
    state.status = '';
    state.size = 20;
    state.page = 1;
    state.total = 0;
    state.items = [];
    state.emailMasked = false;
    state.progress = {};
    state.detail = null;
    state.busy = false;

    root.classList.add('pn-root');
    root.innerHTML = '';
    nodes = {};
    buildSkeleton();
    loadList();
}

export function unmount() {
    // 壳切换页面时会调它：清掉模块级引用与状态，别让上一份数据活到下一次 mount
    ctx = null;
    root = null;
    nodes = {};
    state.items = [];
    state.progress = {};
    state.detail = null;
    state.busy = false;
}

// ===================== 骨架 =====================

function buildSkeleton() {
    root.appendChild(
        ctx.el('div', { class: 'pn-head' }, [
            ctx.el('h2', { class: 'pn-title', text: meta.title }),
            ctx.el('p', { class: 'pn-desc', text: meta.desc })
        ])
    );

    // ---- 工具栏 ----
    var keywordInput = ctx.el('input', {
        type: 'text',
        id: 'pnUserKeyword',
        placeholder: '邮箱或昵称（子串）',
        size: '24'
    });
    keywordInput.addEventListener('keydown', function (event) {
        if (event.key === 'Enter') {
            applyKeyword();
        }
    });
    nodes.keywordInput = keywordInput;

    var queryBtn = ctx.el('button', { class: 'pn-btn', type: 'button', text: '查询' });
    queryBtn.addEventListener('click', applyKeyword);
    nodes.queryBtn = queryBtn;

    var roleSelect = ctx.el('select', { id: 'pnUserRole' }, [
        ctx.el('option', { value: '', text: '全部' }),
        ctx.el('option', { value: 'user', text: '普通用户' }),
        ctx.el('option', { value: 'admin', text: '管理员' }),
        ctx.el('option', { value: 'super_admin', text: '超级管理员' })
    ]);
    roleSelect.addEventListener('change', function () {
        // 空值 = 「全部」：**不能**把空串当筛选值传给服务端（那边只认真的角色名，
        // 传 `role=` 会让列表空掉）
        state.role = roleSelect.value;
        state.page = 1;
        loadList();
    });
    nodes.roleSelect = roleSelect;

    var statusSelect = ctx.el('select', { id: 'pnUserStatus' }, [
        ctx.el('option', { value: '', text: '全部' }),
        ctx.el('option', { value: 'active', text: '正常' }),
        ctx.el('option', { value: 'disabled', text: '已封禁' })
    ]);
    statusSelect.addEventListener('change', function () {
        state.status = statusSelect.value;
        state.page = 1;
        loadList();
    });
    nodes.statusSelect = statusSelect;

    var sizeSelect = ctx.el('select', { id: 'pnUserSize' }, [
        ctx.el('option', { value: '20', text: '每页 20' }),
        ctx.el('option', { value: '50', text: '每页 50' }),
        ctx.el('option', { value: '100', text: '每页 100' })
    ]);
    sizeSelect.addEventListener('change', function () {
        state.size = parseInt(sizeSelect.value, 10) || 20;
        state.page = 1;
        loadList();
    });
    nodes.sizeSelect = sizeSelect;

    nodes.maskHint = ctx.el('p', { class: 'pn-hint', hidden: 'hidden' });
    nodes.listMsg = ctx.el('div', { class: 'pn-msg' });
    nodes.progressMsg = ctx.el('div', { class: 'pn-msg', hidden: 'hidden' });
    nodes.tbody = ctx.el('tbody');
    nodes.pager = ctx.el('div', { class: 'pn-pager' });

    var table = ctx.el('table', { class: 'pn-table' }, [
        ctx.el('thead', {}, [
            ctx.el('tr', {}, [
                ctx.el('th', { class: 'col-num', text: 'ID' }),
                ctx.el('th', { text: '邮箱' }),
                ctx.el('th', { text: '昵称' }),
                ctx.el('th', { text: '角色' }),
                ctx.el('th', { text: '状态' }),
                ctx.el('th', { class: 'col-nowrap', text: '注册时间' }),
                ctx.el('th', { class: 'col-nowrap', text: '最后登录' }),
                ctx.el('th', { class: 'col-num', text: '已学词数' }),
                ctx.el('th', { class: 'col-num', text: '累计复习' }),
                ctx.el('th', { class: 'col-num', text: '连续天数' }),
                ctx.el('th', { class: 'col-nowrap', text: '最近复习' }),
                ctx.el('th', { class: 'col-actions', text: '操作' })
            ])
        ]),
        nodes.tbody
    ]);

    root.appendChild(
        ctx.el('div', { class: 'pn-card' }, [
            ctx.el('h3', { class: 'pn-card-title', text: '用户列表' }),
            ctx.el('div', { class: 'pn-toolbar' }, [
                field('关键词（搜邮箱或昵称）', keywordInput),
                field('角色', roleSelect),
                field('状态', statusSelect),
                field('每页', sizeSelect),
                ctx.el('div', { class: 'pn-toolbar-end' }, [queryBtn])
            ]),
            nodes.maskHint,
            nodes.listMsg,
            nodes.progressMsg,
            ctx.el('div', { class: 'pn-table-wrap' }, [table]),
            nodes.pager,
            ctx.el('p', {
                class: 'pn-hint',
                text:
                    '普通管理员整页只读；改角色 / 封禁解封 / 踢下线只对超级管理员显示（服务端也会再挡一道）。' +
                    '进度列来自复习库（Go，批量接口），账号与角色来自账号服务（Rust）—— 缺的进度按 0 显示。'
            })
        ])
    );
}

function field(label, input) {
    return ctx.el('div', { class: 'pn-field' }, [ctx.el('label', { text: label }), input]);
}

// ===================== 列表 =====================

function applyKeyword() {
    state.keyword = nodes.keywordInput ? String(nodes.keywordInput.value || '').trim() : '';
    state.page = 1;
    loadList();
}

function queryString() {
    var parts = ['page=' + state.page, 'size=' + state.size];
    if (state.role) parts.push('role=' + encodeURIComponent(state.role));
    if (state.status) parts.push('status=' + encodeURIComponent(state.status));
    if (state.keyword) parts.push('keyword=' + encodeURIComponent(state.keyword));
    return parts.join('&');
}

function loadList() {
    state.detail = null; // 换页 / 换筛选 / 操作后刷新：展开的详情跟着收起来
    nodes.listMsg.className = 'pn-msg';
    nodes.listMsg.textContent = '正在读取…';
    nodes.progressMsg.className = 'pn-msg';
    nodes.progressMsg.setAttribute('hidden', 'hidden');
    nodes.progressMsg.textContent = '';

    var listPromise = ctx.api('/api/auth/admin/users?' + queryString());

    // 进度要用列表里的 id 才能查，所以它接在列表后面；两个请求一起 await（Promise.all），
    // 进度**只有一个批量请求**（当页 100 人也是 1 个请求，不是 100 个）。
    // 进度失败不该把整张表带走：降级成「按 0 显示」+ 一行提醒。
    var progressError = null;
    var progressPromise = listPromise
        .then(function (res) {
            var items = (res && res.data && res.data.items) || [];
            var ids = items.map(function (user) {
                return user.id;
            });
            // 空页不要发这个请求：ids 为空服务端会回 400（那是正常的参数校验，不是故障）
            if (!ids.length) return null;
            return ctx.api('/api/admin/users/progress?ids=' + encodeURIComponent(idsQuery(ids)));
        })
        .catch(function (err) {
            progressError = err;
            return null;
        });

    return Promise.all([listPromise, progressPromise])
        .then(function (results) {
            var data = (results[0] && results[0].data) || {};
            state.items = data.items || [];
            state.total = data.total || 0;
            state.emailMasked = data.email_masked === true;

            var progressData = (results[1] && results[1].data) || {};
            state.progress = {};
            (progressData.items || []).forEach(function (item) {
                state.progress[String(item.user_id)] = item;
            });

            nodes.listMsg.className = 'pn-msg';
            nodes.listMsg.textContent = '';
            renderMaskHint();
            if (progressError) {
                nodes.progressMsg.className = 'pn-msg is-warn';
                nodes.progressMsg.removeAttribute('hidden');
                nodes.progressMsg.textContent =
                    '当页复习进度没读出来：' +
                    friendlyError(progressError, '需要管理员（admin）或超级管理员') +
                    '（下面四列按 0 显示，账号与角色不受影响）';
            }
            renderList();
        })
        .catch(function (err) {
            state.items = [];
            state.total = 0;
            state.progress = {};
            nodes.tbody.innerHTML = '';
            nodes.pager.innerHTML = '';
            nodes.maskHint.setAttribute('hidden', 'hidden');
            nodes.listMsg.className = 'pn-msg is-error';
            nodes.listMsg.textContent = '读取失败：' + friendlyError(err, '需要管理员（admin）或超级管理员');
        });
}

// 普通管理员看到的邮箱是服务端打码过的（22***@qq.com）：说清楚，并且本页**没有**任何复制邮箱的按钮
function renderMaskHint() {
    if (state.emailMasked) {
        nodes.maskHint.className = 'pn-hint';
        nodes.maskHint.removeAttribute('hidden');
        nodes.maskHint.textContent =
            '⚠️ 当前身份看不到完整邮箱（超管可见）：列表里的邮箱已由服务端打码（形如 22***@qq.com），' +
            '不是真实地址，别拿去发信或对账。';
    } else {
        nodes.maskHint.className = 'pn-hint';
        nodes.maskHint.setAttribute('hidden', 'hidden');
        nodes.maskHint.textContent = '';
    }
}

function renderList() {
    nodes.tbody.innerHTML = '';
    if (!state.items.length) {
        nodes.tbody.appendChild(
            ctx.el('tr', {}, [
                ctx.el('td', { colspan: String(COLS) }, [
                    ctx.el('div', { class: 'pn-empty', text: '没有符合条件的用户' })
                ])
            ])
        );
        renderPager();
        return;
    }

    state.items.forEach(function (user) {
        nodes.tbody.appendChild(renderRow(user));
        if (state.detail && state.detail.user_id === user.id) {
            nodes.tbody.appendChild(renderDetailRow(user));
        }
    });
    renderPager();
}

function renderRow(user) {
    var statusInfo = userStatusLabel(user.status);
    var progress = progressOf(user.id);

    var emailCell = ctx.el('div', {}, [
        ctx.el('div', { class: 'pn-mono', text: user.email || '—' }),
        ctx.el('div', {
            class: 'pn-sub',
            text: user.email_verified_at ? '已验证 · ' + fmtTime(user.email_verified_at) : '未验证'
        })
    ]);

    // 「进度」对管理员也开放（只读接口）；改角色 / 封禁 / 踢下线只有超管才有
    var actions = [progressButton(user)];
    if (isSuper()) {
        actions.push(roleSelect(user));
        actions.push(statusButton(user));
        actions.push(kickButton(user));
    }

    return ctx.el('tr', {}, [
        ctx.el('td', { class: 'col-num', text: String(user.id) }),
        ctx.el('td', {}, [emailCell]),
        ctx.el('td', { text: user.username || '—' }),
        ctx.el('td', {}, [
            ctx.el('span', { class: 'pn-badge ' + roleKind(user.role), text: roleLabel(user.role) })
        ]),
        ctx.el('td', {}, [
            ctx.el('span', { class: 'pn-badge ' + statusInfo.kind, text: statusInfo.text })
        ]),
        ctx.el('td', { class: 'col-nowrap', text: fmtTime(user.created_at) }),
        ctx.el('td', { class: 'col-nowrap', text: fmtTime(user.last_login_at) }),
        ctx.el('td', { class: 'col-num', text: fmtNum(progress.learned_words) }),
        ctx.el('td', { class: 'col-num', text: fmtNum(progress.total_reviews) }),
        ctx.el('td', { class: 'col-num', text: fmtNum(progress.streak_days) }),
        ctx.el('td', { class: 'col-nowrap', text: progress.last_review_at ? fmtTime(progress.last_review_at) : '—' }),
        ctx.el('td', { class: 'col-actions' }, [ctx.el('div', { class: 'pn-actions' }, actions)])
    ]);
}

// 角色徽标配色：超管用警示色（显眼但不是红色 —— 红色留给封禁 / 危险状态）
function roleKind(role) {
    if (role === 'super_admin') return 'is-warn';
    if (role === 'admin') return 'is-info';
    return 'is-muted';
}

// 进度条目缺席（服务端可能不回这个 id）→ 当 0 看：少一行会让整张表格错位
function progressOf(userId) {
    return state.progress[String(userId)] || {};
}

function isSuper() {
    return !!(ctx && ctx.user && ctx.user.role === 'super_admin');
}

// ===================== 详情（最近 7 天） =====================

function progressButton(user) {
    var open = !!(state.detail && state.detail.user_id === user.id);
    var btn = ctx.el('button', { class: 'pn-btn-link', type: 'button', text: open ? '收起' : '进度' });
    btn.addEventListener('click', function () {
        toggleDetail(user);
    });
    return btn;
}

function toggleDetail(user) {
    if (state.detail && state.detail.user_id === user.id) {
        state.detail = null;
        renderList();
        return;
    }
    state.detail = { user_id: user.id, loading: true, data: null, error: '' };
    renderList();

    ctx
        .api('/api/admin/users/' + user.id + '/progress')
        .then(function (res) {
            // 期间又点了别的行 / 换了页：这次的结果直接丢掉，别把详情挂到别人身上
            if (!state.detail || state.detail.user_id !== user.id) return;
            state.detail.loading = false;
            state.detail.data = (res && res.data) || {};
            renderList();
        })
        .catch(function (err) {
            if (!state.detail || state.detail.user_id !== user.id) return;
            state.detail.loading = false;
            state.detail.error = friendlyError(err, '需要管理员（admin）或超级管理员');
            renderList();
        });
}

function renderDetailRow(user) {
    var box = ctx.el('div', {});
    var detail = state.detail;

    box.appendChild(
        ctx.el('div', {
            class: 'pn-sub',
            text: '#' + user.id + ' · ' + (user.email || '—') + ' · 最近 7 天复习量（北京自然日）'
        })
    );

    if (detail.loading) {
        box.appendChild(ctx.el('div', { class: 'pn-msg', text: '正在读取进度…' }));
    } else if (detail.error) {
        box.appendChild(ctx.el('div', { class: 'pn-msg is-error', text: '进度读取失败：' + detail.error }));
    } else {
        var data = detail.data || {};
        box.appendChild(
            ctx.el('div', { class: 'pn-actions', style: 'margin:8px 0 4px;' }, [
                ctx.el('span', { class: 'pn-sub', text: '已学词数：' + fmtNum(data.learned_words) }),
                ctx.el('span', { class: 'pn-sub', text: '累计复习：' + fmtNum(data.total_reviews) }),
                ctx.el('span', { class: 'pn-sub', text: '连续天数：' + fmtNum(data.streak_days) }),
                ctx.el('span', {
                    class: 'pn-sub',
                    text: '最近复习：' + (data.last_review_at ? fmtTime(data.last_review_at) : '—')
                })
            ])
        );
        box.appendChild(renderBars(data.last7 || []));
    }

    return ctx.el('tr', { style: 'background:#f7fafd;' }, [
        ctx.el('td', { colspan: String(COLS) }, [box])
    ]);
}

// 最近 7 天的复习量：静态横条，宽度 = 当天值 ÷ 7 天最大值（全 0 时宽度 0，别除出 NaN）
function renderBars(points) {
    var box = ctx.el('div', { style: 'margin-top:6px;' });
    if (!points.length) {
        box.appendChild(ctx.el('div', { class: 'pn-sub', text: '这段时间还没有复习记录' }));
        return box;
    }

    var max = 0;
    points.forEach(function (point) {
        var value = num(point.reviews);
        if (value > max) max = value;
    });

    var bars = ctx.el('div', { class: 'pn-bars' });
    points.forEach(function (point) {
        var value = num(point.reviews);
        var width = max > 0 ? (value / max) * 100 : 0;
        bars.appendChild(
            ctx.el('div', { class: 'pn-bar-row' }, [
                ctx.el('div', { class: 'pn-bar-label', text: point.date || '—' }),
                ctx.el('div', { class: 'pn-bar-track' }, [
                    ctx.el('div', { class: 'pn-bar-fill', style: 'width:' + width.toFixed(1) + '%;' })
                ]),
                ctx.el('div', { class: 'pn-bar-value', text: fmtNum(value) })
            ])
        );
    });
    box.appendChild(bars);
    return box;
}

// ===================== 超管操作 =====================

function roleSelect(user) {
    var select = ctx.el(
        'select',
        { style: 'padding:2px 4px;font-size:12px;max-width:110px;' },
        [
            ctx.el('option', { value: 'user', text: '普通用户' }),
            ctx.el('option', { value: 'admin', text: '管理员' }),
            ctx.el('option', { value: 'super_admin', text: '超级管理员' })
        ]
    );
    select.value = user.role;
    select.addEventListener('change', function () {
        var next = select.value;
        if (next === user.role) return;
        var who = user.email || '#' + user.id;
        ctx
            .confirm(
                '把 ' + who + ' 的角色从「' + roleLabel(user.role) + '」改成「' + roleLabel(next) + '」？\n\n' +
                '服务端规则：最后一个超管不能降级（改完就没有超管能进后台了，所以会被拒绝）。'
            )
            .then(function (ok) {
                if (!ok) {
                    // 取消时把下拉拨回当前角色：别让界面显示一个没生效的值
                    select.value = user.role;
                    return;
                }
                postRole(user, next, select);
            });
    });
    return select;
}

function postRole(user, role, select) {
    if (state.busy) {
        select.value = user.role;
        ctx.toast('上一个操作还没完成，稍等一下', 'error');
        return;
    }
    state.busy = true;
    ctx
        .api('/api/auth/admin/users/' + user.id + '/role', { method: 'POST', body: { role: role } })
        .then(function () {
            ctx.toast('已把 ' + (user.email || '#' + user.id) + ' 的角色改为「' + roleLabel(role) + '」', 'ok');
            loadList();
        })
        .catch(function (err) {
            select.value = user.role;
            ctx.toast('改角色失败：' + friendlyError(err, '需要超管权限'), 'error');
        })
        .then(function () {
            state.busy = false;
        });
}

function statusButton(user) {
    var disabled = user.status === 'disabled';
    var btn = ctx.el('button', { class: 'pn-btn-link', type: 'button', text: disabled ? '解封' : '封禁' });
    btn.addEventListener('click', function () {
        var who = user.email || '#' + user.id;
        var message = disabled
            ? '把 ' + who + ' 解封？\n\n解封后他可以重新登录（之前被吊销的会话不会恢复，需要重新登录）。'
            : '封禁 ' + who + '？\n\n⚠️ 封禁会立刻吊销他所有会话：他所有设备上的登录当场失效，并且不能再登录。';
        ctx.confirm(message).then(function (ok) {
            if (!ok) return;
            doSetStatus(user, disabled ? 'active' : 'disabled');
        });
    });
    return btn;
}

function doSetStatus(user, status) {
    if (state.busy) {
        ctx.toast('上一个操作还没完成，稍等一下', 'error');
        return;
    }
    state.busy = true;
    ctx
        .api('/api/auth/admin/users/' + user.id + '/status', { method: 'POST', body: { status: status } })
        .then(function () {
            ctx.toast(status === 'disabled' ? '已封禁' : '已解封', 'ok');
            loadList();
        })
        .catch(function (err) {
            ctx.toast((status === 'disabled' ? '封禁失败：' : '解封失败：') + friendlyError(err, '需要超管权限'), 'error');
        })
        .then(function () {
            state.busy = false;
        });
}

function kickButton(user) {
    var btn = ctx.el('button', { class: 'pn-btn-link', type: 'button', text: '踢下线' });
    btn.addEventListener('click', function () {
        var who = user.email || '#' + user.id;
        ctx
            .confirm('把 ' + who + ' 全部踢下线？\n\n他所有设备上的登录都会立刻失效，需要重新登录。')
            .then(function (ok) {
                if (!ok) return;
                doKick(user);
            });
    });
    return btn;
}

function doKick(user) {
    if (state.busy) {
        ctx.toast('上一个操作还没完成，稍等一下', 'error');
        return;
    }
    state.busy = true;
    ctx
        .api('/api/auth/admin/users/' + user.id + '/logout-all', { method: 'POST', body: {} })
        .then(function (res) {
            var revoked = (res && res.data && res.data.revoked) || 0;
            ctx.toast(revoked > 0 ? '已吊销 ' + revoked + ' 个会话' : '他当前没有可吊销的会话', 'ok');
            loadList();
        })
        .catch(function (err) {
            ctx.toast('踢下线失败：' + friendlyError(err, '需要超管权限'), 'error');
        })
        .then(function () {
            state.busy = false;
        });
}

// ===================== 分页 =====================

function renderPager() {
    nodes.pager.innerHTML = '';
    var pages = Math.max(1, Math.ceil(state.total / state.size));
    nodes.pager.appendChild(
        ctx.el('span', { text: '共 ' + state.total + ' 个 · 第 ' + state.page + ' / ' + pages + ' 页' })
    );

    var btns = ctx.el('div', { class: 'pn-pager-btns' });
    var prev = ctx.el('button', { class: 'pn-btn pn-btn-sm', type: 'button', text: '上一页' });
    prev.disabled = state.page <= 1;
    prev.addEventListener('click', function () {
        if (state.page > 1) {
            state.page -= 1;
            loadList();
        }
    });
    var next = ctx.el('button', { class: 'pn-btn pn-btn-sm', type: 'button', text: '下一页' });
    next.disabled = state.page >= pages;
    next.addEventListener('click', function () {
        if (state.page < pages) {
            state.page += 1;
            loadList();
        }
    });
    btns.appendChild(prev);
    btns.appendChild(next);
    nodes.pager.appendChild(btns);
}

// ===================== 小工具 =====================

function num(value) {
    var n = Number(value);
    return isFinite(n) ? n : 0;
}

// 壳的 api 失败时抛出的 Error：拿它的 message 就够看了
function messageOf(err) {
    if (!err) return '未知错误';
    if (err.message) return err.message;
    return String(err);
}

// 401 / 403 要说人话：壳在 401 时会自己提示并跳登录页，这一块则显示成一句能看懂的话，
// 绝不把裸错误抛到控制台。判据是壳的兜底文案 'HTTP <状态码> <路径>'（见 admin/admin.js 的
// apiFetch）与后端可能回的中文；别的情况原样显示后端的 message。
function friendlyError(err, needText) {
    var raw = messageOf(err);
    if (/HTTP\s*401|unauthenticated|未登录|请先登录|登录状态/.test(raw)) {
        return '登录状态已失效，请重新登录后再打开这一页。';
    }
    if (/HTTP\s*403|forbidden|权限/.test(raw)) {
        return '没有权限看这块内容（' + needText + '）。';
    }
    return raw;
}
