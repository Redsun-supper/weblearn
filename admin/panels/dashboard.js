// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
// 管理面板 · 数据看板（P1）
//
// 由壳动态 import 后调用 mount(container, ctx)。本模块**只认 ctx**，不碰任何一个页面的东西
// （壳可能是个人中心 account/，也可能是后台 admin/）。
//
// ctx 提供：
//   api(path, {method, body}) → Promise<信封 {code,message,data}>；非 2xx 或 code!==200 会 throw
//   el(tag, attrs, children)  → DOM 构建器（{text} 走 textContent）
//   toast(msg, kind) / confirm(msg) / escapeHtml(s) / setTitle(s) / user
//
// 两个请求（Go 侧 /api/admin/*，整组挂 RequireAdmin：admin 与 super_admin 都能看）：
//   ① GET /api/admin/stats/overview        —— 今日 5 个 + 累计 8 个 + 账号库可用性，一个请求出全部数字
//   ② GET /api/admin/stats/trend?days=7|30 —— points 已按北京自然日升序、缺失的天补 0，前端不再补洞
//
// 口径（docs/launch-plan.md §5.0）：
//   - 「今日 / 按天」一律按**北京时间（UTC+8）**切天，与部署机、浏览器的时区无关；所以趋势的
//     日期直接用服务端给的 `2026-10-03` 字符串，不要再用 new Date() 折一次（折两次必然差 8 小时）；
//   - 账号库（auth.db）读不到时接口**仍然 200**：账号侧数字全 0 + `auth_db.available=false`。
//     这时不报错也不白屏，只在卡片下面挂一行说明 —— 账号服务没跑时登录本身也会不可用；
//   - 趋势用静态横条（.pn-bars）画：没有图表库，也**没有动效**（项目红线，见 launch-plan 第 8 节）。
//
// 风格：ES5（var/function），只用 export —— 与 modules/english/admin/english-admin.js 一致。

import { fmtNum, pctText } from './util.js';

export var meta = {
    id: 'dashboard',
    title: '数据看板',
    desc: '今日与累计的关键数字',
    roles: ['admin', 'super_admin']
};

var ctx = null;
var root = null;
// 骨架只建一次：刷新只重渲染数据区（重建整块会让人刚看的位置跳走）
var nodes = {};

var state = {
    days: 7,
    overview: null,
    trend: null
};

// ===================== 生命周期 =====================

export function mount(container, context) {
    ctx = context;
    root = container;
    state.days = 7;
    state.overview = null;
    state.trend = null;

    root.classList.add('pn-root');
    root.innerHTML = '';
    nodes = {};
    buildSkeleton();
    loadAll();
}

export function unmount() {
    // 壳切换页面时会调它：清掉模块级引用与状态，别让上一份数据活到下一次 mount
    ctx = null;
    root = null;
    nodes = {};
    state.overview = null;
    state.trend = null;
}

// ===================== 骨架 =====================

function buildSkeleton() {
    root.appendChild(
        ctx.el('div', { class: 'pn-head' }, [
            ctx.el('h2', { class: 'pn-title', text: meta.title }),
            ctx.el('p', { class: 'pn-desc', text: meta.desc })
        ])
    );

    var refreshBtn = ctx.el('button', { class: 'pn-btn pn-btn-sm', type: 'button', text: '刷新' });
    refreshBtn.addEventListener('click', loadAll);
    nodes.refreshBtn = refreshBtn;

    // 看板的整体消息行（overview 的进度与错误都放这儿）
    nodes.msg = ctx.el('div', { class: 'pn-msg' });

    root.appendChild(
        ctx.el('div', { class: 'pn-toolbar' }, [
            ctx.el('div', { class: 'pn-toolbar-end' }, [refreshBtn])
        ])
    );
    root.appendChild(nodes.msg);

    // ---- 今日 ----
    // 账号侧的数字（今日新增用户）排在业务侧前面，账号库读不到时下面那行警告就紧贴着它
    nodes.todayStats = ctx.el('div', { class: 'pn-stats' });
    nodes.authWarn = ctx.el('div', { class: 'pn-msg', hidden: 'hidden' });
    root.appendChild(
        ctx.el('div', { class: 'pn-card' }, [
            ctx.el('h3', { class: 'pn-card-title', text: '今日' }),
            nodes.todayStats,
            nodes.authWarn,
            ctx.el('p', {
                class: 'pn-hint',
                text:
                    '「今日」按北京时间（UTC+8）切天：auth.db 存的是 UTC 文本、复习库存的是带时区的时间，' +
                    '两边都先折成北京自然日再统计 —— 与部署机、浏览器的时区无关。'
            })
        ])
    );

    // ---- 累计 ----
    nodes.totalStats = ctx.el('div', { class: 'pn-stats' });
    root.appendChild(
        ctx.el('div', { class: 'pn-card' }, [
            ctx.el('h3', { class: 'pn-card-title', text: '累计' }),
            nodes.totalStats
        ])
    );

    // ---- 趋势 ----
    var btn7 = ctx.el('button', { class: 'pn-btn pn-btn-sm', type: 'button', text: '最近 7 天' });
    var btn30 = ctx.el('button', { class: 'pn-btn pn-btn-sm', type: 'button', text: '最近 30 天' });
    btn7.addEventListener('click', function () {
        setDays(7);
    });
    btn30.addEventListener('click', function () {
        setDays(30);
    });
    nodes.btn7 = btn7;
    nodes.btn30 = btn30;

    nodes.trendMsg = ctx.el('div', { class: 'pn-msg' });
    nodes.trendBox = ctx.el('div', {});

    root.appendChild(
        ctx.el('div', { class: 'pn-card' }, [
            ctx.el('h3', { class: 'pn-card-title', text: '趋势' }),
            ctx.el('div', { class: 'pn-toolbar' }, [
                ctx.el('div', { class: 'pn-actions' }, [btn7, btn30]),
                ctx.el('div', { class: 'pn-toolbar-end' }, [
                    ctx.el('span', { class: 'pn-sub', text: '每条 = 一个北京自然日' })
                ])
            ]),
            nodes.trendMsg,
            nodes.trendBox,
            ctx.el('p', {
                class: 'pn-hint',
                text:
                    '柱长按「该指标自己」的最大值归一（三个指标互不比较，也不跨天累加）：' +
                    '某天的值 ÷ 该指标区间内的最大值；整个区间都是 0 时柱长为 0。'
            })
        ])
    );

    syncDayButtons();
}

// ===================== 取数 =====================

// 刷新按钮：两个接口一起重拉（趋势按当前选中的 7 / 30 天）
function loadAll() {
    nodes.refreshBtn.disabled = true;
    // 两个 loader 各自消化自己的错误（各自渲染 .pn-msg.is-error），所以这里不会 reject
    Promise.all([loadOverview(), loadTrend()]).then(function () {
        nodes.refreshBtn.disabled = false;
    });
}

function loadOverview() {
    nodes.msg.className = 'pn-msg';
    nodes.msg.textContent = '正在读取…';
    return ctx
        .api('/api/admin/stats/overview')
        .then(function (res) {
            state.overview = (res && res.data) || {};
            nodes.msg.className = 'pn-msg';
            nodes.msg.textContent = '';
            renderOverview();
            return true;
        })
        .catch(function (err) {
            nodes.msg.className = 'pn-msg is-error';
            nodes.msg.textContent = '读取失败：' + friendlyError(err, '需要管理员（admin）或超级管理员');
            return false;
        });
}

// 切 7 / 30 天只重拉趋势：overview 与天数无关，不多发一个请求
function loadTrend() {
    nodes.trendMsg.className = 'pn-msg';
    nodes.trendMsg.textContent = '正在读取…';
    return ctx
        .api('/api/admin/stats/trend?days=' + state.days)
        .then(function (res) {
            state.trend = (res && res.data) || {};
            nodes.trendMsg.className = 'pn-msg';
            nodes.trendMsg.textContent = '';
            renderTrend();
            return true;
        })
        .catch(function (err) {
            // 失败时保留上一次的柱子，只在上面挂一行错误
            nodes.trendMsg.className = 'pn-msg is-error';
            nodes.trendMsg.textContent = '趋势读取失败：' + friendlyError(err, '需要管理员（admin）或超级管理员');
            return false;
        });
}

// ===================== 渲染：今日 / 累计 =====================

function renderOverview() {
    var data = state.overview || {};
    var today = data.today || {};
    var totals = data.totals || {};

    fillStats(nodes.todayStats, [
        stat('今日新增用户', fmtNum(today.new_users), '账号库 · 按北京时间切天'),
        stat('今日活跃用户', fmtNum(today.active_users), '复习库 · 当天有复习记录的人数'),
        stat('今日复习次数', fmtNum(today.reviews), '复习库 · 当天的复习日志条数'),
        stat('今日新学词数', fmtNum(today.new_words), '复习库 · stability_before = 0 的日志数'),
        stat('今日抽查', fmtNum(today.probes), '复习库 · is_probe 的日志数')
    ]);

    fillStats(nodes.totalStats, [
        stat('用户总数', fmtNum(totals.users), '账号库 · 含管理员'),
        stat('管理员', fmtNum(totals.admins), '账号库 · role = admin'),
        stat('超级管理员', fmtNum(totals.super_admins), '账号库 · role = super_admin'),
        stat('发出邀请码', fmtNum(totals.invites_issued), '账号库 · 累计生成张数'),
        stat('已兑换邀请码', fmtNum(totals.invites_redeemed), '账号库 · 至少用过一次的张数'),
        stat('邀请码兑换率', pctText(totals.invite_conversion), '账号库 · 已兑换 ÷ 发出（张数口径）'),
        stat('词条总数', fmtNum(totals.words), '复习库'),
        stat('复习总数', fmtNum(totals.reviews), '复习库 · 日志累计条数')
    ]);

    renderAuthWarn(data.auth_db || {});
}

function stat(label, value, note) {
    return ctx.el('div', { class: 'pn-stat' }, [
        ctx.el('div', { class: 'pn-stat-label', text: label }),
        ctx.el('div', { class: 'pn-stat-value', text: value }),
        note ? ctx.el('div', { class: 'pn-stat-note', text: note }) : null
    ]);
}

function fillStats(box, cards) {
    box.innerHTML = '';
    cards.forEach(function (card) {
        box.appendChild(card);
    });
}

// 账号库读不到：不报错、不白屏，只在今日那几张卡下面说明「哪些数字按 0 显示、为什么」。
// 文案里带上服务端给的 error：它是后端拼好的原因（路径没配 / 库被搬走 / 打不开只读）。
function renderAuthWarn(authDb) {
    if (authDb && authDb.available === false) {
        var reason = authDb.error ? String(authDb.error) : '服务端没给出原因';
        nodes.authWarn.className = 'pn-msg is-warn';
        nodes.authWarn.removeAttribute('hidden');
        nodes.authWarn.textContent =
            '账号库（auth.db）读不到：' +
            reason +
            '\n账号侧的数字现在都按 0 显示 —— 上面的「今日新增用户」，以及下面累计里的用户总数 / 管理员 / 超管 / 发码 / 兑换 / 兑换率；复习侧的数字照常准确。' +
            '账号库是只读挂载的，账号服务没在跑时会这样（登录也会不可用）。';
    } else {
        nodes.authWarn.className = 'pn-msg';
        nodes.authWarn.setAttribute('hidden', 'hidden');
        nodes.authWarn.textContent = '';
    }
}

// ===================== 渲染：趋势横条 =====================

function setDays(days) {
    if (state.days === days) return;
    state.days = days;
    syncDayButtons();
    loadTrend();
}

function syncDayButtons() {
    if (!nodes.btn7 || !nodes.btn30) return;
    nodes.btn7.className = 'pn-btn pn-btn-sm' + (state.days === 7 ? ' pn-btn-primary' : '');
    nodes.btn30.className = 'pn-btn pn-btn-sm' + (state.days === 30 ? ' pn-btn-primary' : '');
}

function renderTrend() {
    var data = state.trend || {};
    var points = data.points || [];
    nodes.trendBox.innerHTML = '';
    if (!points.length) {
        nodes.trendBox.appendChild(
            ctx.el('div', { class: 'pn-empty', text: '这段时间还没有数据' })
        );
        return;
    }
    // 三个指标各一段：每段自己归一（新增用户 3 个和复习 300 次不该共用一根刻度）
    nodes.trendBox.appendChild(barGroup('新增用户', points, 'new_users'));
    nodes.trendBox.appendChild(barGroup('活跃用户', points, 'active_users'));
    nodes.trendBox.appendChild(barGroup('复习次数', points, 'reviews'));
}

function barGroup(title, points, key) {
    var max = 0;
    points.forEach(function (point) {
        var value = num(point[key]);
        if (value > max) max = value;
    });

    var box = ctx.el('div', {}, [
        ctx.el('div', { class: 'pn-sub', style: 'margin:12px 0 6px;', text: title })
    ]);

    var bars = ctx.el('div', { class: 'pn-bars' });
    points.forEach(function (point) {
        var value = num(point[key]);
        // 最大值为 0 时宽度给 0：不能除出 NaN（'NaN%' 会让整条轨道空掉）
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
