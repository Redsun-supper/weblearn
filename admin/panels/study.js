// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
// 学习统计面板（roadmap 阶段 4 第一项）
//
// 挂在**个人中心**（account/），人人可见 —— 不按角色拦，所以它住在 admin/panels/ 只是
// 为了复用同一套「meta + mount(container, ctx)」契约与 panels.css；后台 admin/ 的
// SUBJECT_ADMINS 是显式列表，不会把它扫进去。
//
// 两个请求（都在 /api/reviews 组下，未登录 401）：
//   ① GET /api/reviews/history?days=84 —— 按北京自然日聚合的复习明细（本项目新增）；
//      points 已升序、缺失的天补 0，前端不补洞。热力图与评分分布都吃它。
//   ② GET /api/reviews/stats —— 原来就有的汇总数字（连续天数 / 保持率 / 已复习词数…）。
//
// 口径：
//   - 「今天 / 按天」一律按服务端 Asia/Shanghai 切（与 /api/admin/stats/trend 一致），
//     日期串（"2026-10-03"）直接当字符串用，**不要**再用 new Date() 折一次；
//   - 热力图一格 = 一个北京自然日，颜色按「当天复习过的不同词数」分四档；
//   - 评分分布 = 窗口内四次评分各自的总次数（同一天评多次都算）；
//   - 全部静态渲染，**没有任何动效**（项目红线，见 docs/launch-plan.md 第 8 节）。
//
// 风格：ES5（var/function）+ export，与另外四个面板一致。

import { fmtNum, pctText } from './util.js';

export var meta = {
    id: 'study',
    title: '学习统计',
    desc: '你的复习习惯与记忆保持'
    // 故意不写 roles：这一页人人可见（壳的导航表决定谁能看到它，与 meta 无关）
};

var ctx = null;
var root = null;
// 骨架只建一次：切换天数 / 刷新只重渲染数据区
var nodes = {};

var state = {
    days: 84,
    stats: null,
    history: null
};

// 范围切换的三个档：30 天 ≈ 5 周的细节，84 天 = 12 周（热力图标准窗口），365 天 = 全年
var RANGES = [
    { days: 30, label: '最近 30 天' },
    { days: 84, label: '最近 12 周' },
    { days: 365, label: '最近一年' }
];

// ===================== 生命周期 =====================

export function mount(container, context) {
    ctx = context;
    root = container;
    state.days = 84;
    state.stats = null;
    state.history = null;

    root.classList.add('pn-root');
    root.innerHTML = '';
    nodes = {};
    buildSkeleton();
    loadAll();
}

export function unmount() {
    ctx = null;
    root = null;
    nodes = {};
    state.stats = null;
    state.history = null;
}

// ===================== 骨架 =====================

function buildSkeleton() {
    root.appendChild(
        ctx.el('div', { class: 'pn-head' }, [
            ctx.el('h2', { class: 'pn-title', text: meta.title }),
            ctx.el('p', { class: 'pn-desc', text: meta.desc })
        ])
    );

    nodes.rangeBtns = [];
    RANGES.forEach(function (range) {
        var btn = ctx.el('button', { class: 'pn-btn pn-btn-sm', type: 'button', text: range.label });
        btn.addEventListener('click', function () {
            setDays(range.days);
        });
        nodes.rangeBtns.push({ days: range.days, node: btn });
    });

    var refreshBtn = ctx.el('button', { class: 'pn-btn pn-btn-sm', type: 'button', text: '刷新' });
    refreshBtn.addEventListener('click', loadAll);
    nodes.refreshBtn = refreshBtn;

    nodes.statsMsg = ctx.el('div', { class: 'pn-msg' });
    nodes.historyMsg = ctx.el('div', { class: 'pn-msg' });

    root.appendChild(
        ctx.el('div', { class: 'pn-toolbar' }, [
            ctx.el('div', { class: 'pn-actions' }, nodes.rangeBtns.map(function (b) { return b.node; })),
            ctx.el('div', { class: 'pn-toolbar-end' }, [refreshBtn])
        ])
    );

    // ---- 关键数字 ----
    nodes.statsBox = ctx.el('div', { class: 'pn-stats' });
    root.appendChild(
        ctx.el('div', { class: 'pn-card' }, [
            ctx.el('h3', { class: 'pn-card-title', text: '关键数字' }),
            nodes.statsBox,
            nodes.statsMsg,
            ctx.el('p', {
                class: 'pn-hint',
                text:
                    '「今日」与「连续」按北京时间（UTC+8）切天，与你在哪台设备上看无关。' +
                    '记忆保持率 = 评分不是「忘记」的比例（累计口径）。'
            })
        ])
    );

    // ---- 热力图 ----
    nodes.legend = ctx.el('div', { class: 'pn-hm-legend' });
    nodes.heatmap = ctx.el('div', {});
    root.appendChild(
        ctx.el('div', { class: 'pn-card' }, [
            ctx.el('h3', { class: 'pn-card-title', text: '学习热力图' }),
            nodes.historyMsg,
            nodes.heatmap,
            nodes.legend,
            ctx.el('p', {
                class: 'pn-hint',
                text:
                    '一格 = 一个北京自然日，颜色越深当天复习的词越多（同一行是同一星期几，' +
                    '从左到右是一周一周往前走）。鼠标停在格子上可以看到那天具体复习了多少。'
            })
        ])
    );

    // ---- 评分分布 ----
    nodes.ratingBox = ctx.el('div', {});
    root.appendChild(
        ctx.el('div', { class: 'pn-card' }, [
            ctx.el('h3', { class: 'pn-card-title', text: '评分分布' }),
            nodes.ratingBox,
            ctx.el('p', {
                class: 'pn-hint',
                text:
                    '窗口内每次评分各记为一次（同一个词一天评多次也算多次）。' +
                    '「忘记」多是正常的 —— 它正是安排下次复习的依据。'
            })
        ])
    );

    syncRangeButtons();
}

// ===================== 取数 =====================

function loadAll() {
    nodes.refreshBtn.disabled = true;
    // 两个 loader 各自消化自己的错误（各自渲染 .pn-msg.is-error），所以这里不会 reject
    Promise.all([loadStats(), loadHistory()]).then(function () {
        nodes.refreshBtn.disabled = false;
    });
}

function loadStats() {
    nodes.statsMsg.className = 'pn-msg';
    nodes.statsMsg.textContent = '正在读取…';
    return ctx
        .api('/api/reviews/stats')
        .then(function (res) {
            state.stats = (res && res.data) || {};
            nodes.statsMsg.className = 'pn-msg';
            nodes.statsMsg.textContent = '';
            renderStats();
            return true;
        })
        .catch(function (err) {
            nodes.statsMsg.className = 'pn-msg is-error';
            nodes.statsMsg.textContent = '读取失败：' + friendlyError(err, '登录后即可查看');
            return false;
        });
}

function loadHistory() {
    nodes.historyMsg.className = 'pn-msg';
    nodes.historyMsg.textContent = '正在读取…';
    return ctx
        .api('/api/reviews/history?days=' + state.days)
        .then(function (res) {
            state.history = (res && res.data) || {};
            nodes.historyMsg.className = 'pn-msg';
            nodes.historyMsg.textContent = '';
            renderHeatmap();
            renderRatings();
            // 「近 7 天复习词数」要等 history 才知道：stats 先到时它没法算。
            // 所以 history 到位后把关键数字再渲染一次（幂等，值不会变第二次）
            if (state.stats) renderStats();
            return true;
        })
        .catch(function (err) {
            nodes.historyMsg.className = 'pn-msg is-error';
            nodes.historyMsg.textContent = '读取失败：' + friendlyError(err, '登录后即可查看');
            return false;
        });
}

// ===================== 渲染：关键数字 =====================

function renderStats() {
    var s = state.stats || {};
    // history 可能还没到（两个 loader 并发）：那格先显示「—」，history 到位后会整体重渲染一次
    var points = (state.history && state.history.points) || [];
    var historyReady = !!state.history;
    // 近 7 天复习词数：直接由 history 的最后 7 天算，不多发一个请求
    var recentWords = 0;
    for (var i = Math.max(0, points.length - 7); i < points.length; i++) {
        recentWords += num(points[i].words);
    }

    fillStats(nodes.statsBox, [
        stat('连续学习', fmtNum(s.streak_days) + ' 天', '有复习记录的天连成的链，断了重新算'),
        stat('记忆保持率', pctText(s.retention_rate), '评分不是「忘记」的比例（累计）'),
        stat('已复习词数', fmtNum(s.reviewed_words), '有过进度行的词（池子共 ' + fmtNum(s.pool_size) + ' 个）'),
        stat('近 7 天复习词数', historyReady ? fmtNum(recentWords) : '—', '按当前窗口的最后 7 天算'),
        stat('今日复习', fmtNum(s.today_reviewed) + ' 次', '其中新学 ' + fmtNum(s.today_new) + ' 个词'),
        stat('累计复习', fmtNum(s.total_reviews) + ' 次', '顽固词（忘得特别勤）' + fmtNum(s.leech_count) + ' 个')
    ]);
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

// ===================== 渲染：学习热力图 =====================

// 日期串（"2026-10-03"）→ 星期几的索引，周一 = 0。
// 按 UTC 解析：这串本来就是日历日，按本地时区解析会整体偏一天。
function weekdayIndex(dateStr) {
    var d = new Date(dateStr + 'T00:00:00Z');
    return (d.getUTCDay() + 6) % 7;
}

function renderHeatmap() {
    var points = (state.history && state.history.points) || [];
    nodes.heatmap.innerHTML = '';
    renderLegend(points);
    if (!points.length) {
        nodes.heatmap.appendChild(ctx.el('div', { class: 'pn-empty', text: '这段时间还没有数据' }));
        return;
    }

    // 颜色分档：按窗口内「单日最多词数」归一成四档；全 0 时所有格子都是空的
    var max = 0;
    points.forEach(function (p) {
        if (num(p.words) > max) max = num(p.words);
    });
    function levelOf(words) {
        if (words <= 0) return 0;
        if (max <= 1) return 1;
        var ratio = words / max;
        if (ratio > 0.75) return 4;
        if (ratio > 0.5) return 3;
        if (ratio > 0.25) return 2;
        return 1;
    }

    var weekdays = ['一', '', '三', '', '五', '', '日'];
    var board = ctx.el('div', { class: 'pn-hm' });
    var dayCol = ctx.el('div', { class: 'pn-hm-days' });
    weekdays.forEach(function (label) {
        dayCol.appendChild(ctx.el('div', { text: label }));
    });
    board.appendChild(dayCol);

    // 按「周」分列：第一列前面补空格，让每一天都落在正确的星期几行上
    var offset = weekdayIndex(points[0].date);
    var week = null;
    var weekCells = null;
    var pending = offset;
    var prevMonth = -1;

    // 新起一列（一周）。列首那个月与上一列不同才显示月份，否则顶部会被重复的月份糊满
    function startWeek(month) {
        week = ctx.el('div', { class: 'pn-hm-week' });
        var label = month !== prevMonth ? month + '月' : '';
        prevMonth = month;
        week.appendChild(ctx.el('div', { class: 'pn-hm-month', text: label }));
        weekCells = ctx.el('div', { class: 'pn-hm-cells' });
        week.appendChild(weekCells);
        board.appendChild(week);
    }

    startWeek(Number(String(points[0].date).slice(5, 7)));
    // 第一周前面的空位（例如窗口从周三开始，周一 / 周二两格留空）
    for (var b = 0; b < offset; b++) {
        weekCells.appendChild(ctx.el('div', { class: 'pn-hm-cell is-off' }));
    }

    for (var i = 0; i < points.length; i++) {
        var p = points[i];
        // pending 归零 = 上一列这周排满了，该开新列（i=0 那一格上面已经开过，别再开）
        if (pending === 0 && i > 0) {
            startWeek(Number(String(p.date).slice(5, 7)));
        }
        weekCells.appendChild(cellEl(p, levelOf(num(p.words))));
        pending = (pending + 1) % 7;
    }
    // 最后一列补占位：窗口末尾不一定正好压在周日，缺的格子补满，每一列才都是 7 行
    while (pending !== 0) {
        weekCells.appendChild(ctx.el('div', { class: 'pn-hm-cell is-off' }));
        pending = (pending + 1) % 7;
    }

    nodes.heatmap.appendChild(ctx.el('div', { class: 'pn-hm-scroll' }, [board]));
}

function cellEl(point, level) {
    var words = num(point.words);
    var reviews = num(point.reviews);
    var title = monthDay(point.date) + ' · 复习 ' + words + ' 个词 · ' + reviews + ' 次评分';
    return ctx.el('div', {
        class: 'pn-hm-cell' + (level > 0 ? ' is-lv' + level : ''),
        'data-date': point.date,
        'data-level': String(level),
        title: title
    });
}

// "2026-10-03" → "10月3日"（不本地化：服务端给的就是北京自然日）
function monthDay(dateStr) {
    var parts = String(dateStr).split('-');
    return Number(parts[1]) + '月' + Number(parts[2]) + '日';
}

function renderLegend(points) {
    nodes.legend.innerHTML = '';
    nodes.legend.appendChild(ctx.el('span', { text: '少' }));
    [0, 1, 2, 3, 4].forEach(function (level) {
        nodes.legend.appendChild(
            ctx.el('div', { class: 'pn-hm-cell' + (level > 0 ? ' is-lv' + level : ''), title: '' })
        );
    });
    nodes.legend.appendChild(ctx.el('span', { text: '多' }));
    if (points.length) {
        var totalWords = 0;
        points.forEach(function (p) {
            totalWords += num(p.words);
        });
        nodes.legend.appendChild(
            ctx.el('span', { text: '　窗口内共复习 ' + totalWords + ' 个词' })
        );
    }
}

// ===================== 渲染：评分分布 =====================

var RATINGS = [
    { key: 'again', label: '忘记', cls: 'is-again', note: 'rating 1 · 重新安排' },
    { key: 'hard', label: '困难', cls: 'is-hard', note: 'rating 2 · 间隔缩短' },
    { key: 'good', label: '一般', cls: 'is-good', note: 'rating 3 · 按记忆进度' },
    { key: 'easy', label: '简单', cls: 'is-easy', note: 'rating 4 · 间隔拉长' }
];

function renderRatings() {
    var points = (state.history && state.history.points) || [];
    nodes.ratingBox.innerHTML = '';
    if (!points.length) {
        nodes.ratingBox.appendChild(ctx.el('div', { class: 'pn-empty', text: '这段时间还没有数据' }));
        return;
    }

    var totals = {};
    var max = 0;
    RATINGS.forEach(function (r) {
        var sum = 0;
        points.forEach(function (p) {
            sum += num(p[r.key]);
        });
        totals[r.key] = sum;
        if (sum > max) max = sum;
    });

    var totalAll = 0;
    RATINGS.forEach(function (r) {
        totalAll += totals[r.key];
    });
    if (totalAll === 0) {
        nodes.ratingBox.appendChild(ctx.el('div', { class: 'pn-empty', text: '这段时间还没有评分记录' }));
        return;
    }

    var bars = ctx.el('div', { class: 'pn-bars' });
    RATINGS.forEach(function (r) {
        var value = totals[r.key];
        var width = max > 0 ? (value / max) * 100 : 0;
        // 备注走 title 而不是可见文字：横条行里要塞第四个孩子会把轨道挤成一条缝
        bars.appendChild(
            ctx.el('div', {
                class: 'pn-bar-row',
                title: r.label + '（' + r.note + '）'
            }, [
                ctx.el('div', { class: 'pn-bar-label', text: r.label }),
                ctx.el('div', { class: 'pn-bar-track' }, [
                    ctx.el('div', {
                        class: 'pn-bar-fill ' + r.cls,
                        style: 'width:' + width.toFixed(1) + '%;'
                    })
                ]),
                ctx.el('div', {
                    class: 'pn-bar-value',
                    text: fmtNum(value) + '（' + pctText(totalAll > 0 ? value / totalAll : 0) + '）'
                })
            ])
        );
    });
    nodes.ratingBox.appendChild(bars);
}

// ===================== 小工具 =====================

function setDays(days) {
    if (state.days === days) return;
    state.days = days;
    syncRangeButtons();
    loadHistory();
}

function syncRangeButtons() {
    nodes.rangeBtns.forEach(function (item) {
        item.node.className = 'pn-btn pn-btn-sm' + (state.days === item.days ? ' pn-btn-primary' : '');
    });
}

function num(value) {
    var n = Number(value);
    return isFinite(n) ? n : 0;
}

function messageOf(err) {
    if (!err) return '未知错误';
    if (err.message) return err.message;
    return String(err);
}

// 401 / 403 要说人话：壳在 401 时会自己提示并跳登录页，这里则显示成一句能看懂的话，
// 绝不把裸错误抛到控制台。判据是壳的兜底文案 'HTTP <状态码> <路径>'（与 admin.js 的 apiFetch 同款）。
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
