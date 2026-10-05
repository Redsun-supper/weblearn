// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
// 管理面板 · 审计日志（P1）
//
// 由壳动态 import 后调用 mount(container, ctx)。本模块**只认 ctx**，不碰任何一个页面的东西。
//
// ctx 提供：
//   api(path, {method, body}) → Promise<信封 {code,message,data}>；非 2xx 或 code!==200 会 throw
//   el(tag, attrs, children)  → DOM 构建器（{text} 走 textContent）
//   toast(msg, kind) / confirm(msg) / escapeHtml(s) / setTitle(s) / user
//
// 一个请求（Rust 侧，**超管**）：
//   GET /api/auth/admin/audit?action=&actor=&from=&to=&page=&size=
//   响应：{items:[{id,actor_user_id,actor_email,action,target,ip,user_agent,detail,created_at}],
//          total, page, size, actions:[...]}
//
// 三条口径：
//   - 动作下拉的选项**来自响应里的 `actions`**（库里出现过的动作，服务端去重排序），
//     绝不硬编码；每次响应后要把当前选中的值保住（它可能因为保留期清理而不在列表里）；
//   - 时间筛选**按 UTC 切天**：只给 `2026-10-01` 时服务端当成 2026-10-01T00:00:00Z，
//     `to` 是「含」这个时刻（不是当天末尾）—— 北京时间比 UTC 早 8 小时，界面上必须说明；
//   - 表格里 detail 与 user_agent 先截断，点整行展开看全文（用展开行，**不要动效**）。
//
// 风格：ES5（var/function），只用 export —— 与 modules/english/admin/english-admin.js 一致。

import { fmtTime } from './util.js';

export var meta = {
    id: 'audit',
    title: '审计日志',
    desc: '谁在什么时候做了什么（保留 180 天）',
    roles: ['super_admin']
};

// 表格列数：空行 / 展开行都用它做 colspan，加列时只改这一处
var COLS = 7;

// 表格里 detail / user_agent 的截断长度：太长会把整张表撑得没法看，全文点开行看
var TRUNCATE = 60;

var ctx = null;
var root = null;
// 骨架只建一次：刷新只重渲染数据区（重建整块会让输入框失焦）
var nodes = {};

var state = {
    action: '',
    actor: '',
    from: '',
    to: '',
    size: 20,
    page: 1,
    total: 0,
    items: [],
    actions: [],
    expandedId: 0
};

// ===================== 生命周期 =====================

export function mount(container, context) {
    ctx = context;
    root = container;
    state.action = '';
    state.actor = '';
    state.from = '';
    state.to = '';
    state.size = 20;
    state.page = 1;
    state.total = 0;
    state.items = [];
    state.actions = [];
    state.expandedId = 0;

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
    state.actions = [];
    state.expandedId = 0;
}

// ===================== 骨架 =====================

function buildSkeleton() {
    root.appendChild(
        ctx.el('div', { class: 'pn-head' }, [
            ctx.el('h2', { class: 'pn-title', text: meta.title }),
            ctx.el('p', { class: 'pn-desc', text: meta.desc })
        ])
    );

    // 动作下拉：先只放「全部」，真实选项在每次响应后用 actions 填（见 renderActionOptions）
    var actionSelect = ctx.el('select', { id: 'pnAuditAction' }, [
        ctx.el('option', { value: '', text: '全部' })
    ]);
    actionSelect.addEventListener('change', function () {
        state.action = actionSelect.value;
        state.page = 1;
        loadList();
    });
    nodes.actionSelect = actionSelect;

    var actorInput = ctx.el('input', {
        type: 'number',
        id: 'pnAuditActor',
        min: '1',
        placeholder: '如 12（可空）',
        size: '10'
    });
    actorInput.addEventListener('keydown', function (event) {
        if (event.key === 'Enter') {
            applyFilters();
        }
    });
    nodes.actorInput = actorInput;

    var fromInput = ctx.el('input', { type: 'date', id: 'pnAuditFrom' });
    var toInput = ctx.el('input', { type: 'date', id: 'pnAuditTo' });
    nodes.fromInput = fromInput;
    nodes.toInput = toInput;

    var sizeSelect = ctx.el('select', { id: 'pnAuditSize' }, [
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

    var queryBtn = ctx.el('button', { class: 'pn-btn', type: 'button', text: '查询' });
    queryBtn.addEventListener('click', applyFilters);
    nodes.queryBtn = queryBtn;

    nodes.msg = ctx.el('div', { class: 'pn-msg' });
    nodes.tbody = ctx.el('tbody');
    nodes.pager = ctx.el('div', { class: 'pn-pager' });

    var table = ctx.el('table', { class: 'pn-table' }, [
        ctx.el('thead', {}, [
            ctx.el('tr', {}, [
                ctx.el('th', { class: 'col-nowrap', text: '时间' }),
                ctx.el('th', { text: '动作' }),
                ctx.el('th', { text: '操作者' }),
                ctx.el('th', { text: '目标' }),
                ctx.el('th', { text: 'IP' }),
                ctx.el('th', { text: '详情（点行看全文）' }),
                ctx.el('th', { text: 'User-Agent（点行看全文）' })
            ])
        ]),
        nodes.tbody
    ]);

    root.appendChild(
        ctx.el('div', { class: 'pn-card' }, [
            ctx.el('h3', { class: 'pn-card-title', text: '日志' }),
            ctx.el('p', {
                class: 'pn-hint',
                text: '日志只记动作与目标，绝不记密码、验证码、邀请码明文（这条是后端写库时的红线，不是界面过滤）。'
            }),
            ctx.el('div', { class: 'pn-toolbar' }, [
                field('动作', actionSelect),
                field('操作者（user id）', actorInput),
                field('起（UTC 日期）', fromInput),
                field('止（UTC 日期）', toInput),
                field('每页', sizeSelect),
                ctx.el('div', { class: 'pn-toolbar-end' }, [queryBtn])
            ]),
            ctx.el('p', {
                class: 'pn-hint',
                text:
                    '⚠️ 起止日期按 UTC 切天，与本地时区可能差 8 小时：只填 2026-10-01 时服务端当成 2026-10-01T00:00:00Z，' +
                    '结束日期是「含」这个时刻（不是当天 23:59:59）。北京时间比 UTC 早 8 小时 —— ' +
                    '本地 10-02 早上 7 点写入的日志，UTC 还是 10-01。要看完整的一天，结束日期往后填一天。'
            }),
            nodes.msg,
            ctx.el('div', { class: 'pn-table-wrap' }, [table]),
            nodes.pager,
            ctx.el('p', {
                class: 'pn-hint',
                text: '动作下拉的选项来自服务端返回的 actions（库里出现过的动作），不硬编码；保留 180 天后旧动作可能消失。'
            })
        ])
    );
}

function field(label, input) {
    return ctx.el('div', { class: 'pn-field' }, [ctx.el('label', { text: label }), input]);
}

// ===================== 取数 =====================

// 操作者与起止日期由「查询」按钮落地（输一半就发请求会白查一堆）；
// 动作与每页是下拉，选完立刻生效。
function applyFilters() {
    var actor = nodes.actorInput ? String(nodes.actorInput.value || '').trim() : '';
    var from = nodes.fromInput ? String(nodes.fromInput.value || '').trim() : '';
    var to = nodes.toInput ? String(nodes.toInput.value || '').trim() : '';

    if (actor && !/^[0-9]+$/.test(actor)) {
        setMsg('操作者要填数字 user id（也可以留空）', true);
        return;
    }
    if (from && to && from > to) {
        setMsg('起始日期不能晚于结束日期', true);
        return;
    }

    state.actor = actor;
    state.from = from;
    state.to = to;
    state.page = 1;
    loadList();
}

function queryString() {
    var parts = ['page=' + state.page, 'size=' + state.size];
    // 空值一律不发：服务端把空的 action 当「没这个筛选」，但少一个参数最不容易出岔子
    if (state.action) parts.push('action=' + encodeURIComponent(state.action));
    if (state.actor) parts.push('actor=' + encodeURIComponent(state.actor));
    if (state.from) parts.push('from=' + encodeURIComponent(state.from));
    if (state.to) parts.push('to=' + encodeURIComponent(state.to));
    return parts.join('&');
}

function loadList() {
    state.expandedId = 0; // 换页 / 换筛选后，展开的那一行跟着收起来
    setMsg('正在读取…', false);

    ctx
        .api('/api/auth/admin/audit?' + queryString())
        .then(function (res) {
            var data = (res && res.data) || {};
            state.items = data.items || [];
            state.total = data.total || 0;
            state.actions = data.actions || [];
            setMsg('', false);
            renderActionOptions();
            renderList();
        })
        .catch(function (err) {
            state.items = [];
            state.total = 0;
            // 拉失败时把表格与分页清掉：留着上一页的行会让人以为筛选生效了
            nodes.tbody.innerHTML = '';
            nodes.pager.innerHTML = '';
            setMsg('读取失败：' + friendlyError(err, '需要超管（super_admin）'), true);
        });
}

function setMsg(text, isError) {
    nodes.msg.className = 'pn-msg' + (isError ? ' is-error' : '');
    nodes.msg.textContent = text;
}

// ===================== 渲染 =====================

function renderActionOptions() {
    var list = state.actions || [];
    var select = nodes.actionSelect;
    select.innerHTML = '';
    select.appendChild(ctx.el('option', { value: '', text: '全部' }));
    list.forEach(function (action) {
        select.appendChild(ctx.el('option', { value: action, text: action }));
    });
    // 当前选中的动作要保住：它可能已不在 actions 里（180 天保留期清理过），
    // 这时补一个选项，否则下拉会悄悄跳回「全部」，而用户以为筛选还生效
    if (state.action && list.indexOf(state.action) === -1) {
        select.appendChild(
            ctx.el('option', { value: state.action, text: state.action + '（已不在当前动作列表里）' })
        );
    }
    select.value = state.action;
}

function renderList() {
    nodes.tbody.innerHTML = '';
    if (!state.items.length) {
        nodes.tbody.appendChild(
            ctx.el('tr', {}, [
                ctx.el('td', { colspan: String(COLS) }, [
                    ctx.el('div', { class: 'pn-empty', text: '没有符合条件的日志' })
                ])
            ])
        );
        renderPager();
        return;
    }

    state.items.forEach(function (item) {
        var open = state.expandedId === item.id;
        var row = ctx.el('tr', { style: 'cursor:pointer;' }, [
            ctx.el('td', { class: 'col-nowrap', text: fmtTime(item.created_at) }),
            ctx.el('td', {}, [
                ctx.el('span', { class: 'pn-badge ' + actionKind(item.action), text: item.action || '—' })
            ]),
            actorCell(item),
            ctx.el('td', { class: 'col-text', text: item.target || '—' }),
            ctx.el('td', { class: 'pn-mono col-nowrap', text: item.ip || '—' }),
            ctx.el('td', { class: 'col-text', text: truncate(item.detail, TRUNCATE) }),
            ctx.el('td', { class: 'col-text', text: truncate(item.user_agent, TRUNCATE) })
        ]);
        row.addEventListener('click', function () {
            state.expandedId = open ? 0 : item.id;
            renderList();
        });
        nodes.tbody.appendChild(row);
        if (open) {
            nodes.tbody.appendChild(renderDetailRow(item));
        }
    });

    renderPager();
}

function actorCell(item) {
    var box = ctx.el('div', {}, [ctx.el('div', { text: item.actor_email || '系统' })]);
    if (item.actor_user_id !== null && item.actor_user_id !== undefined) {
        // 带上数字 id：想按操作者筛选时，「操作者」那一栏要填的就是它
        box.appendChild(ctx.el('div', { class: 'pn-sub', text: 'user id ' + item.actor_user_id }));
    } else {
        box.appendChild(ctx.el('div', { class: 'pn-sub', text: '没有操作者（服务端自己干的）' }));
    }
    return ctx.el('td', {}, [box]);
}

function renderDetailRow(item) {
    var box = ctx.el('div', {}, [
        ctx.el('div', {
            class: 'pn-sub',
            text:
                '日志 #' + item.id + ' · ' + fmtTime(item.created_at) + ' · 动作 ' + (item.action || '—') +
                ' · 操作者 ' + (item.actor_email || '系统')
        }),
        ctx.el('div', {
            class: 'pn-msg',
            style: 'word-break:break-all;',
            text: '详情：' + (item.detail || '（空）')
        }),
        ctx.el('div', {
            class: 'pn-msg',
            style: 'word-break:break-all;',
            text: 'User-Agent：' + (item.user_agent || '（空）')
        })
    ]);
    return ctx.el('tr', { style: 'background:#f7fafd;' }, [
        ctx.el('td', { colspan: String(COLS) }, [box])
    ]);
}

// 动作 → 徽标配色。
//
// **显式列出**服务端目前会写进 audit_logs 的 15 个动作名，未知动作一律退化到灰色 is-muted：
// 后端以后新增动作时，界面既不会报错、也不会把没见过的名字误标成危险动作。
// 分类口径：注册 / 登录成功 = 绿；发码 / 用码 / 发信 / 种子 / 改密 = 中性蓝；
// 正常登出 = 灰；停用 / 登录失败 / 被踢下线 = 橙（警示）；改角色 / 封禁解封 = 红（危险）。
function actionKind(action) {
    var table = {
        register: 'is-ok',
        login_ok: 'is-ok',
        invite_create: 'is-info',
        invite_use: 'is-info',
        invite_email: 'is-info',
        admin_seed: 'is-info',
        password_change: 'is-info',
        logout: 'is-muted',
        logout_all: 'is-muted',
        invite_disable: 'is-warn',
        invite_disable_batch: 'is-warn',
        login_fail: 'is-warn',
        user_logout_all: 'is-warn',
        role_change: 'is-bad',
        user_status: 'is-bad'
    };
    return table[action] || 'is-muted';
}

function renderPager() {
    nodes.pager.innerHTML = '';
    var pages = Math.max(1, Math.ceil(state.total / state.size));
    nodes.pager.appendChild(
        ctx.el('span', { text: '共 ' + state.total + ' 条 · 第 ' + state.page + ' / ' + pages + ' 页' })
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

// 表格里先截断（换行 + 长文本会把行高撑爆）；点开行看全文
function truncate(text, max) {
    var value = text === null || text === undefined ? '' : String(text);
    if (!value) return '—';
    return value.length > max ? value.slice(0, max) + '…' : value;
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
