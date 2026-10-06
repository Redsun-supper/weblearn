// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
// 管理面板 · 邀请码（P1）
//
// 由壳动态 import 后调用 mount(container, ctx)。壳可能是个人中心（account/account.js），
// 也可能是后台（admin/admin.js）—— 所以本模块**只认 ctx**，不直接碰任何一个页面的东西。
//
// ctx 提供：
//   api(path, {method, body}) → Promise<信封 {code,message,data}>；非 2xx 或 code!==200 会 throw
//   el(tag, attrs, children)  → DOM 构建器（{text} 走 textContent；{html} 只给自己拼的固定结构）
//   toast(msg, kind) / confirm(msg) → Promise<boolean> / escapeHtml(s) / setTitle(s) / user
//
// 三块功能：
//   ① 生成：明文码只在**这一次响应**里出现（列表里虽然也回码，但那要靠一次查询）；
//      要留档就在生成后当场「复制全部」或「导出 CSV」
//   ② 列表：状态筛选、分页、单张停用、整批停用、复制邀请链接
//   ③ 整批发邮件：选中的码按顺序与粘贴的邮箱一对一配对（服务端逐封独立发送，逐条回结果）
//
// 风格：ES5（var/function），只用 export —— 与 modules/english/admin/english-admin.js 一致。

import { copyText, downloadCsv, fmtTime, inviteStatusLabel, roleLabel } from './util.js';

export var meta = {
    id: 'invites',
    title: '邀请码',
    desc: '生成、查看、停用邀请码，也可以把码直接发到邮箱',
    // 谁能看到它：邀请码是超管的功能（管理员碰不到权限，见 docs/launch-plan.md 第 3 节）
    roles: ['super_admin']
};

var ctx = null;
var root = null;
// 骨架只建一次：每次刷新都重建整块会让输入框失焦、正在填的备注也没了
var nodes = {};

var state = {
    filter: 'all',
    size: 20,
    page: 1,
    total: 0,
    items: [],
    selected: {}, // id → true
    created: [], // 刚生成出来的明文码
    busy: false,
    mailOpen: false,
    mailBusy: false,
    mailResult: null
};

// ---- 生成表单的「上次填的那套值」------------------------------------------------
// 每次生成都要重填 数量 / 每张次数 / 有效期 / 角色 太烦，所以存一份在本机浏览器里。
// 存的是**这台机器这个浏览器**（localStorage），换电脑/换浏览器不带过去；
// 壳可能拿不到 localStorage（隐私模式、被策略禁用），所以读写都 try/catch，读不到就当没有。
//
// ⚠️ 备注也存，但**只用作占位提示**（「上次：给张三那批」），不自动填进输入框：
// 每批的备注基本都不一样，自动填上去最容易出的错就是「发错批次还看不出来」。
var PREF_KEY = 'guangxue.invites.create';
var PREF_DEFAULT = { count: 1, uses: 1, days: 7, role: 'user', note: '', custom: '' };

function loadPref() {
    var pref = {
        count: PREF_DEFAULT.count,
        uses: PREF_DEFAULT.uses,
        days: PREF_DEFAULT.days,
        role: PREF_DEFAULT.role,
        note: '',
        custom: ''
    };
    try {
        var raw = window.localStorage.getItem(PREF_KEY);
        if (!raw) return pref;
        var saved = JSON.parse(raw) || {};
        if (saved.count) pref.count = saved.count;
        if (saved.uses) pref.uses = saved.uses;
        // 0 是合法值（= 不过期），所以这里用 isNaN 判断而不是真值判断
        if (!isNaN(saved.days)) pref.days = saved.days;
        if (saved.role) pref.role = saved.role;
        if (saved.note) pref.note = String(saved.note);
        if (saved.custom) pref.custom = normalizeCustom(saved.custom);
    } catch (e) {
        // 读不到/坏了都当「没存过」，不影响生成功能
    }
    return pref;
}

function savePref(pref) {
    try {
        window.localStorage.setItem(PREF_KEY, JSON.stringify(pref));
    } catch (e) {
        // 存不下就算了，不值得为它报错
    }
}

function clearPref() {
    try {
        window.localStorage.removeItem(PREF_KEY);
    } catch (e) {
        // 同上
    }
}

// ---- 自定义码（超管自己指定一串码，而不是让系统随机生成）------------------------
//
// 规则与后端 `core::invite::validate_custom_code` **一字不差**地对应：
// 正好 16 位、只允许 A-Z 与 0-9（大小写不敏感）、手写的 `-` 会被抹掉。
// 前端这份校验只为「当场说清哪里不对」，**不是安全边界** —— 后端照样会再验一遍。

var CUSTOM_LEN = 16;

/// 规范化：去空白、统一大写、抹掉手写的 `-`（`abcd-efgh-jklm-npqt` → `ABCDEFGHJKLMNPQT`）
function normalizeCustom(raw) {
    return String(raw === null || raw === undefined ? '' : raw)
        .replace(/\s+/g, '')
        .replace(/-/g, '')
        .toUpperCase();
}

/// 校验：通过返回空串，不通过返回给人看的一句话
function validateCustom(code) {
    if (!code) return '自定义码不能为空';
    if (code.length !== CUSTOM_LEN) {
        return '自定义码要正好 ' + CUSTOM_LEN + ' 位字符（现在 ' + code.length + ' 位）';
    }
    if (!/^[0-9A-Z]+$/.test(code)) {
        return '自定义码只能用 0-9 与 A-Z（大小写不敏感，会统一转成大写）';
    }
    return '';
}

/// 把服务端那句「这个码已经被占用了」（409 `invite_code_taken`）解出来。
///
/// 拿到 `{ code, existing }`：`existing` 是那把码在列表里的样子（状态 / 已用次数 / 谁用过），
/// 确认框要拿它把风险说全。不是这类错误时返回 null，按普通错误提示。
/// 依赖壳把服务端信封挂在 error 上（见 `admin/admin.js` 的 apiFetch 与 `account/account.js` 的 panelApi）。
function conflictOf(err) {
    if (!err || err.error !== 'invite_code_taken') return null;
    var data = err.data || {};
    return { code: data.code || '', existing: data.existing || null };
}

/// 那张已存在的码现在是什么状态、被谁用过（给确认框用的人话）
function describeTaken(existing) {
    if (!existing) return '（拿不到这张码的详情，请刷新列表后再试）';
    var status = inviteStatusLabel(existing.status).text;
    var lines = ['状态：' + status + '　已用：' + existing.used_count + ' / ' + existing.max_uses + ' 次'];
    var emails = (existing.uses || []).map(function (u) {
        return u.email;
    });
    if (emails.length) {
        lines.push('用过的人：' + emails.join('、') + (existing.used_count > emails.length ? ' 等' : ''));
    } else if (existing.used_count > 0) {
        lines.push('用过的人：（列表只带最近几条，去列表里看兑换记录）');
    }
    lines.push('级别：' + roleLabel(existing.grant_role));
    return lines.join('\n');
}

// ===================== 生命周期 =====================

export function mount(container, context) {
    ctx = context;
    root = container;
    state.filter = 'all';
    state.size = 20;
    state.page = 1;
    state.items = [];
    state.selected = {};
    state.created = [];
    state.mailOpen = false;
    state.mailResult = null;

    root.classList.add('pn-root');
    root.innerHTML = '';
    nodes = {};
    buildSkeleton();
    loadList();
}

export function unmount() {
    // 壳切换页面时会调它：把引用清掉，别让上一份状态活到下一次 mount
    ctx = null;
    root = null;
    nodes = {};
    state.items = [];
    state.selected = {};
}

// ===================== 骨架 =====================

function buildSkeleton() {
    var head = ctx.el('div', { class: 'pn-head' }, [
        ctx.el('h2', { class: 'pn-title', text: meta.title }),
        ctx.el('p', { class: 'pn-desc', text: meta.desc })
    ]);
    root.appendChild(head);

    // ---- ① 生成 ----
    // 表单初值 = 上次填过的那套（没存过就是默认值），见文件上方 loadPref 的注释
    var pref = loadPref();
    var countInput = ctx.el('input', { type: 'number', id: 'pnInvCount', min: '1', max: '50', value: String(pref.count) });
    var usesInput = ctx.el('input', { type: 'number', id: 'pnInvUses', min: '1', max: '1000', value: String(pref.uses) });
    var daysInput = ctx.el('input', { type: 'number', id: 'pnInvDays', min: '0', max: '365', value: String(pref.days) });
    var noteInput = ctx.el('input', {
        type: 'text',
        id: 'pnInvNote',
        // 备注只作**提示**、不自动填：每批的备注基本都不一样，自动填最容易发错批次
        placeholder: pref.note ? '上次：' + pref.note : '给谁用的、哪一批（可空）',
        size: '28'
    });
    var roleSelect = ctx.el('select', { id: 'pnInvRole' }, [
        ctx.el('option', { value: 'user', text: '普通用户' }),
        ctx.el('option', { value: 'admin', text: '管理员' })
    ]);
    roleSelect.value = pref.role === 'admin' ? 'admin' : 'user';

    // 「永不过期」：后端 `expires_in_days <= 0` 就是不限期，这里给它一个看得见的入口
    // （勾上时天数框禁用 —— 框里留着一个数字却按「不过期」提交，最容易让人看不懂）
    var neverCb = ctx.el('input', { type: 'checkbox', id: 'pnInvNever' });
    var daysField = field('有效期（天，0=不过期）', daysInput);
    var neverLabel = ctx.el('label', { class: 'pn-check', title: '不设到期时间，只要没用完就一直能兑换' }, [
        neverCb,
        ctx.el('span', { text: '永不过期' })
    ]);
    neverCb.addEventListener('change', function () {
        daysInput.disabled = neverCb.checked;
        daysField.className = neverCb.checked ? 'pn-field is-off' : 'pn-field';
        if (neverCb.checked) {
            daysInput.value = '0';
        } else if (!value('pnInvDays') || value('pnInvDays') === '0') {
            daysInput.value = String(PREF_DEFAULT.days);
        }
    });

    // 自定义码：留空 = 照旧随机生成一批；填了 = 就用这一串（一次只出 1 张）
    var customInput = ctx.el('input', {
        type: 'text',
        id: 'pnInvCustom',
        placeholder: '留空 = 随机生成；填 16 位（A-Z、0-9）',
        size: '24',
        maxlength: '32',
        spellcheck: 'false',
        autocomplete: 'off'
    });
    customInput.addEventListener('input', function () {
        // 边打边规范化：小写立刻变大写、手写的 `-` 直接消失（不然人会以为是自己打错了）
        var cursorAtEnd = customInput.selectionStart === customInput.value.length;
        var normalized = normalizeCustom(customInput.value);
        if (normalized !== customInput.value) {
            customInput.value = normalized;
            if (cursorAtEnd) {
                try {
                    customInput.setSelectionRange(normalized.length, normalized.length);
                } catch (e) {
                    // 壳里的元素不支持选区就跳过（不影响输入）
                }
            }
        }
        renderCustomHint();
    });

    var customHint = ctx.el('p', { class: 'pn-hint' });
    function renderCustomHint() {
        var code = normalizeCustom(customInput.value);
        if (!code) {
            customHint.textContent =
                '留空就是系统随机生成（一次最多 50 张）。想自己定一串：填满 ' +
                CUSTOM_LEN +
                ' 位 A-Z 与 0-9（大小写不敏感，手写的 - 会被去掉），那一次只出 1 张。';
            customHint.className = 'pn-hint';
            return;
        }
        var bad = validateCustom(code);
        customHint.textContent = bad
            ? bad
            : '将用这一串建 1 张：' +
              code +
              '（' +
              (neverCb.checked ? '永不过期' : daysInput.value + ' 天') +
              ' · ' +
              value('pnInvUses') +
              ' 次 · ' +
              roleLabel(value('pnInvRole') || 'user') +
              '）';
        // 不合格就借 `.pn-msg.is-error` 的样式（panels.css 里已有），不再新造一个类
        customHint.className = bad ? 'pn-msg is-error' : 'pn-hint';
    }
    daysInput.addEventListener('input', renderCustomHint);
    usesInput.addEventListener('input', renderCustomHint);

    var createBtn = ctx.el('button', { class: 'pn-btn pn-btn-primary', type: 'button', text: '生成' });
    createBtn.addEventListener('click', doCreate);

    var prefBtn = ctx.el('button', { class: 'pn-btn-link', type: 'button', text: '恢复默认' });
    prefBtn.addEventListener('click', function () {
        clearPref();
        setValue('pnInvCount', PREF_DEFAULT.count);
        setValue('pnInvUses', PREF_DEFAULT.uses);
        setValue('pnInvDays', PREF_DEFAULT.days);
        setValue('pnInvNote', '');
        setValue('pnInvRole', PREF_DEFAULT.role);
        setValue('pnInvCustom', '');
        neverCb.checked = false;
        neverCb.dispatchEvent(new Event('change'));
        noteInput.placeholder = '给谁用的、哪一批（可空）';
        renderCustomHint();
        ctx.toast('已恢复默认：随机生成 1 张 · 每张 1 次 · 7 天 · 普通用户', 'ok');
    });

    nodes.created = ctx.el('div', { class: 'pn-msg' });
    nodes.createBtn = createBtn;
    nodes.neverCb = neverCb;

    // 上次填过的：天数与「永不过期」互斥，所以 0 天按勾选框还原；
    // 自定义码也带回来 —— 同一个串往往要连着调几次，重打一遍纯属折磨
    if (pref.days === 0) {
        neverCb.checked = true;
        neverCb.dispatchEvent(new Event('change'));
    }
    if (pref.custom) {
        customInput.value = pref.custom;
    }
    renderCustomHint();

    var createCard = ctx.el('div', { class: 'pn-card' }, [
        ctx.el('h3', { class: 'pn-card-title', text: '生成邀请码' }),
        ctx.el('div', { class: 'pn-toolbar' }, [
            field('数量', countInput),
            field('每张可用次数', usesInput),
            daysField,
            neverLabel,
            field('注册后等级', roleSelect),
            ctx.el('div', { class: 'pn-toolbar-end' }, [prefBtn, createBtn])
        ]),
        ctx.el('div', { class: 'pn-toolbar' }, [field('自定义码', customInput), field('备注', noteInput)]),
        customHint,
        nodes.created,
        ctx.el('p', {
            class: 'pn-hint',
            text:
                '⚠️ 明文只会出现在这次响应里：刷新或离开本页就再也拿不到。' +
                '要留档请当场「复制全部」或「导出 CSV」。等级选“管理员”生成的码带 ADMIN- 前缀（只是给人看的，判定永远看库里的 grant_role）。' +
                '「自定义码」填了就按你给的串建（别人用过的码会先问一句「是否沿用」，沿用只改额度与有效期，兑换记录不动）；' +
                '⚠️ 自定义码**不会**被加上 ADMIN- 前缀，所以自己定的码是管理员级的话，靠列表里的「等级」列分辨。' +
                '数量 / 次数 / 有效期 / 等级 / 自定义码会**记住你上次填的那套**（存在本机浏览器），旁边「恢复默认」可以清掉；备注只把上次的内容显示成提示、不会自动填。'
        })
    ]);
    root.appendChild(createCard);

    // ---- ② 列表 ----
    var filterSelect = ctx.el('select', { id: 'pnInvFilter' }, [
        ctx.el('option', { value: 'all', text: '全部' }),
        ctx.el('option', { value: 'unused', text: '未使用' }),
        ctx.el('option', { value: 'used', text: '已用完' }),
        ctx.el('option', { value: 'expired', text: '已过期' }),
        ctx.el('option', { value: 'disabled', text: '已停用' })
    ]);
    filterSelect.addEventListener('change', function () {
        state.filter = filterSelect.value;
        state.page = 1;
        loadList();
    });

    var sizeSelect = ctx.el('select', { id: 'pnInvSize' }, [
        ctx.el('option', { value: '20', text: '每页 20' }),
        ctx.el('option', { value: '50', text: '每页 50' }),
        ctx.el('option', { value: '100', text: '每页 100' })
    ]);
    sizeSelect.addEventListener('change', function () {
        state.size = parseInt(sizeSelect.value, 10) || 20;
        state.page = 1;
        loadList();
    });

    var copySelected = ctx.el('button', { class: 'pn-btn pn-btn-sm', type: 'button', text: '复制选中项' });
    copySelected.addEventListener('click', function () {
        var picked = selectedItems();
        if (!picked.length) {
            ctx.toast('先勾选要复制的邀请码', 'error');
            return;
        }
        copyAndToast(picked.map(itemLink).join('\n'), '已复制 ' + picked.length + ' 条邀请链接');
    });

    var exportBtn = ctx.el('button', { class: 'pn-btn pn-btn-sm', type: 'button', text: '导出本页 CSV' });
    exportBtn.addEventListener('click', exportCsv);

    var mailBtn = ctx.el('button', { class: 'pn-btn pn-btn-sm pn-btn-primary', type: 'button', text: '给选中项发邮件' });
    mailBtn.addEventListener('click', openMail);
    nodes.mailBtn = mailBtn;

    nodes.listMsg = ctx.el('div', { class: 'pn-msg' });
    nodes.tbody = ctx.el('tbody');
    nodes.pager = ctx.el('div', { class: 'pn-pager' });

    var table = ctx.el('table', { class: 'pn-table' }, [
        ctx.el('thead', {}, [
            ctx.el('tr', {}, [
                ctx.el('th', { class: 'col-check' }),
                ctx.el('th', { text: '邀请码' }),
                ctx.el('th', { text: '等级' }),
                ctx.el('th', { text: '状态' }),
                ctx.el('th', { class: 'col-num', text: '已用/上限' }),
                ctx.el('th', { text: '有效期' }),
                ctx.el('th', { text: '备注' }),
                ctx.el('th', { text: '兑换记录' }),
                ctx.el('th', { class: 'col-nowrap', text: '创建时间' }),
                ctx.el('th', { class: 'col-actions', text: '操作' })
            ])
        ]),
        nodes.tbody
    ]);

    var listCard = ctx.el('div', { class: 'pn-card' }, [
        ctx.el('h3', { class: 'pn-card-title', text: '邀请码列表' }),
        ctx.el('div', { class: 'pn-toolbar' }, [
            field('状态', filterSelect),
            field('每页', sizeSelect),
            ctx.el('div', { class: 'pn-toolbar-end' }, [mailBtn, copySelected, exportBtn])
        ]),
        nodes.listMsg,
        ctx.el('div', { class: 'pn-table-wrap' }, [table]),
        nodes.pager
    ]);
    root.appendChild(listCard);

    // ---- ③ 整批发邮件 ----
    nodes.mailCard = ctx.el('div', { class: 'pn-card', hidden: 'hidden' });
    root.appendChild(nodes.mailCard);

    // 生成区的结果容器（每次生成后重建）
    nodes.created.className = 'pn-msg';
}

function field(label, input) {
    return ctx.el('div', { class: 'pn-field' }, [ctx.el('label', { text: label }), input]);
}

// ===================== 生成 =====================

function doCreate() {
    if (state.busy) return;
    var count = parseInt(value('pnInvCount'), 10);
    var maxUses = parseInt(value('pnInvUses'), 10);
    // 「永不过期」勾上时天数框是禁用的，按 0（= 不过期）提交
    var days = nodes.neverCb && nodes.neverCb.checked ? 0 : parseInt(value('pnInvDays'), 10);
    var note = value('pnInvNote');
    var role = value('pnInvRole') || 'user';
    var custom = normalizeCustom(value('pnInvCustom'));

    if (custom) {
        // 自定义码：当场把格式说清（后端还会再验一遍，这里只是为了不用等一个来回）
        var bad = validateCustom(custom);
        if (bad) {
            setCreatedMsg(bad, 'error');
            return;
        }
    } else if (!count || count < 1 || count > 50) {
        setCreatedMsg('数量要在 1~50 之间', 'error');
        return;
    }
    if (!maxUses || maxUses < 1 || maxUses > 1000) {
        setCreatedMsg('每张可用次数要在 1~1000 之间', 'error');
        return;
    }
    if (!custom && (isNaN(days) || days < 0 || days > 365)) {
        setCreatedMsg('有效期要在 0~365 天之间（0 = 不过期）', 'error');
        return;
    }

    submitCreate({ count: count, maxUses: maxUses, days: days, note: note, role: role, custom: custom }, false);
}

/// 真正发请求那一步。
///
/// `allowExisting` 是「这张码已经存在，但我确认无视风险继续」（服务端要显式收到才肯沿用）——
/// 只有用户在第一轮 409 的确认框里点了「继续」才会传 true。
function submitCreate(form, allowExisting) {
    state.busy = true;
    nodes.createBtn.disabled = true;
    setCreatedMsg(allowExisting ? '正在沿用这张码…' : form.custom ? '正在建这张码…' : '正在生成…', '');

    var body = {
        max_uses: form.maxUses,
        expires_in_days: form.days,
        note: form.note,
        grant_role: form.role
    };
    if (form.custom) {
        body.custom_code = form.custom;
        body.allow_existing = !!allowExisting;
    } else {
        body.count = form.count;
    }

    ctx
        .api('/api/auth/admin/invites', { method: 'POST', body: body })
        .then(function (res) {
            var data = (res && res.data) || {};
            state.created = data.items || [];
            // 成功才记这套设置：填错了没生成出来，不该把错的记成「上次那套」
            savePref({
                count: form.count,
                uses: form.maxUses,
                days: form.days,
                role: form.role,
                note: form.note,
                custom: form.custom
            });
            if (data.reused) {
                setCreatedMsg('已沿用这张已经存在的码（额度/有效期按这次填的改，兑换记录与已注册的账号都没动）', 'ok');
                ctx.toast('已沿用「' + form.custom + '」', 'ok');
            } else {
                ctx.toast(form.custom ? '已建好这张码：' + form.custom : '已生成 ' + state.created.length + ' 张', 'ok');
            }
            renderCreated();
            state.page = 1;
            loadList();
        })
        .catch(function (err) {
            var taken = conflictOf(err);
            if (!taken) {
                setCreatedMsg('生成失败：' + messageOf(err), 'error');
                return;
            }
            // 这张码已经存在：默认**不覆盖**，先把「它是谁、被谁用过」摆出来问一句
            askReuse(form, taken);
        })
        .then(function () {
            state.busy = false;
            nodes.createBtn.disabled = false;
        });
}

/// 撞到已有码时的确认框：把那张码的现状说全，用户点「继续」才带 `allow_existing` 重发
function askReuse(form, taken) {
    var code = taken.code || form.custom;
    var existing = taken.existing;
    var usable = existing && existing.status === 'unused';
    var head = usable
        ? '这个码已经存在了（还没被用过）。'
        : '⚠️ 这个码已经存在，而且**已经被用过了**。';
    ctx
        .confirm(
            head +
                '\n\n' +
                code +
                '\n' +
                describeTaken(existing) +
                '\n\n' +
                '继续的话：**沿用它这张**（不会新建第二张，也不会覆盖成新的）：' +
                '\n· 已用次数 / 停用状态 / 兑换记录**都不动**，已经注册出来的账号不受影响；' +
                '\n· 只把「以后还能用几次」与「什么时候过期」按这次填的改（' +
                form.maxUses +
                ' 次 · ' +
                (form.days > 0 ? form.days + ' 天' : '永不过期') +
                '）。' +
                (usable ? '' : '\n\n⚠️ 这张码已经流出去过（别人可能已经看到）：沿用等于让它又变成一张可用凭据。') +
                '\n\n确定要沿用吗？'
        )
        .then(function (ok) {
            if (!ok) {
                setCreatedMsg('已取消：这张码还在，没做任何改动', '');
                return;
            }
            submitCreate(form, true);
        });
}

function renderCreated() {
    var box = nodes.created;
    box.className = 'pn-msg';
    box.innerHTML = '';
    if (!state.created.length) return;

    box.appendChild(
        ctx.el('div', {
            class: 'pn-msg is-ok',
            text: '已生成 ' + state.created.length + ' 张（明文只显示这一次，请立即留档）'
        })
    );

    var codes = ctx.el('div', { class: 'pn-codes' });
    state.created.forEach(function (item) {
        codes.appendChild(ctx.el('span', { class: 'pn-code', text: item.code }));
    });
    box.appendChild(codes);

    var actions = ctx.el('div', { class: 'pn-actions', style: 'margin-top:10px;' });
    var copyBtn = ctx.el('button', { class: 'pn-btn pn-btn-sm', type: 'button', text: '复制全部' });
    copyBtn.addEventListener('click', function () {
        copyAndToast(
            state.created
                .map(function (i) {
                    return i.code;
                })
                .join('\n'),
            '已复制全部邀请码'
        );
    });
    var csvBtn = ctx.el('button', { class: 'pn-btn pn-btn-sm', type: 'button', text: '导出 CSV' });
    csvBtn.addEventListener('click', function () {
        // ⚠️ 表头 + **每一张**码都要铺开：早先这里把 map 出来的数组又取了 [0]，
        // 于是生成 20 张也只导出 1 张（数据没丢，是导出写错了）
        var rows = [['邀请码', '等级', '每张可用次数', '有效期', '备注']];
        state.created.forEach(function (item) {
            rows.push([
                item.code,
                roleLabel(item.grant_role),
                item.max_uses,
                item.expires_at ? fmtTime(item.expires_at) : '不过期',
                item.note || ''
            ]);
        });
        downloadCsv('invites-' + stamp() + '.csv', rows);
        ctx.toast('已导出 ' + state.created.length + ' 张', 'ok');
    });
    var mailBtn = ctx.el('button', { class: 'pn-btn pn-btn-sm', type: 'button', text: '把这一批发到邮箱' });
    mailBtn.addEventListener('click', function () {
        // 刚生成的这一批直接进发信流程（省得再去列表里勾一遍）
        state.selected = {};
        state.created.forEach(function (i) {
            state.selected[i.id] = true;
        });
        openMail();
    });
    actions.appendChild(copyBtn);
    actions.appendChild(csvBtn);
    actions.appendChild(mailBtn);
    box.appendChild(actions);

    // 便捷：生成后列表里那一批的链接也能一次复制
    var linkBtn = ctx.el('button', { class: 'pn-btn pn-btn-sm', type: 'button', text: '复制邀请链接（全部）' });
    linkBtn.addEventListener('click', function () {
        copyAndToast(
            state.created
                .map(function (i) {
                    return itemLink(i);
                })
                .join('\n'),
            '已复制 ' + state.created.length + ' 条邀请链接'
        );
    });
    actions.appendChild(linkBtn);
}

function setCreatedMsg(text, kind) {
    nodes.created.className = 'pn-msg' + (kind ? ' ' + kind : '');
    nodes.created.innerHTML = '';
    nodes.created.textContent = text;
}

// ===================== 列表 =====================

function loadList() {
    nodes.listMsg.className = 'pn-msg';
    nodes.listMsg.textContent = '正在读取…';
    var url =
        '/api/auth/admin/invites?status=' +
        encodeURIComponent(state.filter) +
        '&page=' +
        state.page +
        '&size=' +
        state.size;
    ctx
        .api(url)
        .then(function (res) {
            var data = (res && res.data) || {};
            state.items = data.items || [];
            state.total = data.total || 0;
            nodes.listMsg.textContent = '';
            renderList();
        })
        .catch(function (err) {
            nodes.listMsg.className = 'pn-msg is-error';
            nodes.listMsg.textContent = '读取失败：' + messageOf(err);
        });
}

function renderList() {
    nodes.tbody.innerHTML = '';
    if (!state.items.length) {
        nodes.tbody.appendChild(
            ctx.el('tr', {}, [ctx.el('td', { colspan: '10' }, [ctx.el('div', { class: 'pn-empty', text: '没有符合条件的邀请码' })])])
        );
        renderPager();
        return;
    }

    state.items.forEach(function (item) {
        var status = inviteStatusLabel(item.status);

        var check = ctx.el('input', { type: 'checkbox' });
        check.checked = !!state.selected[item.id];
        check.addEventListener('change', function () {
            if (check.checked) state.selected[item.id] = true;
            else delete state.selected[item.id];
        });

        var linkBtn = ctx.el('button', { class: 'pn-btn-link', type: 'button', text: '链接' });
        linkBtn.addEventListener('click', function () {
            copyAndToast(itemLink(item), '已复制邀请链接');
        });
        var copyBtn = ctx.el('button', { class: 'pn-btn-link', type: 'button', text: '复制码' });
        copyBtn.addEventListener('click', function () {
            copyAndToast(item.code, '已复制邀请码');
        });

        var actions = [copyBtn, linkBtn];
        // 「已用完」的码可以**重新启用**（把已用次数清零）：这是一个主动降安全的动作，
        // 所以按钮文案保持中性，风险全写在确认框里（见 doReset）
        if (item.status === 'used') {
            var resetBtn = ctx.el('button', { class: 'pn-btn-link', type: 'button', text: '重新启用' });
            resetBtn.addEventListener('click', function () {
                doReset(item);
            });
            actions.push(resetBtn);
        }
        if (item.status === 'unused' || item.status === 'expired') {
            var disableBtn = ctx.el('button', { class: 'pn-btn-link', type: 'button', text: '停用' });
            disableBtn.addEventListener('click', function () {
                doDisable(item);
            });
            actions.push(disableBtn);
        }
        // 整批停用：同一批的码通常一起回收，逐张点太累
        if (item.batch_id) {
            var batchBtn = ctx.el('button', { class: 'pn-btn-link', type: 'button', text: '停用整批' });
            batchBtn.addEventListener('click', function () {
                doDisableBatch(item);
            });
            actions.push(batchBtn);
        }

        nodes.tbody.appendChild(
            ctx.el('tr', {}, [
                ctx.el('td', { class: 'col-check' }, [check]),
                ctx.el('td', { class: 'pn-mono col-nowrap', text: item.code }),
                ctx.el('td', { text: roleLabel(item.grant_role) }),
                ctx.el('td', {}, [ctx.el('span', { class: 'pn-badge ' + status.kind, text: status.text })]),
                ctx.el('td', { class: 'col-num', text: item.used_count + ' / ' + item.max_uses }),
                ctx.el('td', { class: 'col-nowrap', text: item.expires_at ? fmtTime(item.expires_at) : '不过期' }),
                ctx.el('td', { class: 'col-text', text: item.note || '—' }),
                ctx.el('td', { class: 'col-text' }, [usesCell(item)]),
                ctx.el('td', { class: 'col-nowrap', text: fmtTime(item.created_at) }),
                ctx.el('td', { class: 'col-actions' }, [ctx.el('div', { class: 'pn-actions' }, actions)])
            ])
        );
    });

    renderPager();
}

function usesCell(item) {
    var uses = item.uses || [];
    if (!uses.length) return ctx.el('span', { class: 'pn-sub', text: '—' });
    var box = ctx.el('div', {});
    uses.forEach(function (use) {
        box.appendChild(
            ctx.el('div', { class: 'pn-sub', text: use.email + ' · ' + fmtTime(use.used_at) })
        );
    });
    if (item.used_count > uses.length) {
        box.appendChild(
            ctx.el('div', { class: 'pn-sub', text: '（还有 ' + (item.used_count - uses.length) + ' 条未显示）' })
        );
    }
    return box;
}

function renderPager() {
    nodes.pager.innerHTML = '';
    var pages = Math.max(1, Math.ceil(state.total / state.size));
    nodes.pager.appendChild(
        ctx.el('span', {
            text: '共 ' + state.total + ' 张 · 第 ' + state.page + ' / ' + pages + ' 页'
        })
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

// ===================== 停用 / 重新启用 =====================

/// 「已经被谁用掉几次」的人话摘要（停用与重置的风险提示都要用）
function usedSummary(item) {
    var emails = (item.uses || []).map(function (u) {
        return u.email;
    });
    var who = emails.length
        ? '：' + emails.join('、') + (item.used_count > emails.length ? ' 等' : '')
        : '';
    return '已用过 ' + item.used_count + ' 次' + who;
}

function doDisable(item) {
    // 已经被人用过的码：停用**回收不了**已经注册出来的账号，这一点必须写在确认框里，
    // 否则很容易以为「停用 = 把人赶走」
    var extra =
        item.used_count > 0
            ? '\n\n⚠️ 这张码' +
              usedSummary(item) +
              '：停用只拦住还没用掉的次数，已经注册出来的账号不会被回收（要收回权限请去「用户」面板封禁）。'
            : '';
    ctx.confirm('停用这张邀请码？\n\n' + item.code + '\n停用后不能再用来注册，已经注册的账号不受影响。' + extra).then(function (ok) {
        if (!ok) return;
        ctx
            .api('/api/auth/admin/invites/' + item.id + '/disable', { method: 'POST', body: {} })
            .then(function () {
                ctx.toast('已停用', 'ok');
                loadList();
            })
            .catch(function (err) {
                ctx.toast('停用失败：' + messageOf(err), 'error');
            });
    });
}

function doDisableBatch(item) {
    // 同一批里可能混着已经用过的码：把**本页能看到**的先数出来，并说清「停用 ≠ 收回账号」
    var sameBatch = state.items.filter(function (i) {
        return i.batch_id === item.batch_id;
    });
    var usedOnes = sameBatch.filter(function (i) {
        return i.used_count > 0;
    });
    var usedTimes = usedOnes.reduce(function (n, i) {
        return n + i.used_count;
    }, 0);
    var extra = usedOnes.length
        ? '\n\n⚠️ 本页这一批里已经有 ' +
          usedOnes.length +
          ' 张被用过（共 ' +
          usedTimes +
          ' 次）：' +
          usedOnes
              .slice(0, 3)
              .map(function (i) {
                  return i.code;
              })
              .join('、') +
          (usedOnes.length > 3 ? ' 等' : '') +
          '\n停用只影响还没用掉的次数 —— 已经注册出来的账号不会被回收，要收回权限请去「用户」面板封禁。'
        : '';
    ctx
        .confirm(
            '把这一批全部停用？\n\n批次：' + item.batch_id + '\n已经停用的不会重复计数，已注册的账号不受影响。' + extra
        )
        .then(function (ok) {
            if (!ok) return;
            ctx
                .api('/api/auth/admin/invite-batches/' + encodeURIComponent(item.batch_id) + '/disable', {
                    method: 'POST',
                    body: {}
                })
                .then(function (res) {
                    var n = (res && res.data && res.data.disabled) || 0;
                    ctx.toast(n > 0 ? '已停用 ' + n + ' 张' : '这一批已经全部停用了', 'ok');
                    loadList();
                })
                .catch(function (err) {
                    ctx.toast('整批停用失败：' + messageOf(err), 'error');
                });
        });
}

/// 「重新启用一张已用过的码」：把已用次数清零，让它还能被兑换。
///
/// ⚠️ 这是**主动降安全**的动作（码已经流出去过），所以确认框里要把三件事说全：
/// 这张码是什么等级（重置后还能再换出一个什么账号）、之前是谁用的、
/// 以及重置之后会发生什么（兑换记录不删、操作进审计 invite_reset）。
function doReset(item) {
    var role = roleLabel(item.grant_role);
    var emails = (item.uses || []).map(function (u) {
        return u.email;
    });
    var who = emails.length ? emails.join('、') : '（见列表里的兑换记录）';
    ctx
        .confirm(
            '重新启用这张已用过的邀请码？\n\n' +
                item.code +
                '\n等级：' +
                role +
                '（重置后谁拿到都能再兑换出一个' +
                role +
                '账号）\n' +
                '已用：' +
                item.used_count +
                ' / ' +
                item.max_uses +
                '　用过的人：' +
                who +
                '\n\n' +
                '⚠️ 这张码已经发出去过，别人可能已经看到：「清零」等于让它重新变成一张可用凭据。\n' +
                '· 兑换记录不会删除，仍然留在列表里；\n' +
                '· 这次操作会记进审计日志（动作 invite_reset）。\n\n' +
                '确定要重新启用吗？'
        )
        .then(function (ok) {
            if (!ok) return;
            ctx
                .api('/api/auth/admin/invites/' + item.id + '/reset', { method: 'POST', body: {} })
                .then(function (res) {
                    var n = (res && res.data && res.data.cleared) || 0;
                    ctx.toast(n > 0 ? '已重新启用（清掉 ' + n + ' 次使用记录）' : '这张码本来就没被用过', 'ok');
                    loadList();
                })
                .catch(function (err) {
                    ctx.toast('重新启用失败：' + messageOf(err), 'error');
                });
        });
}

// ===================== 整批发邮件 =====================

function openMail() {
    var all = selectedItems();
    if (!all.length) {
        ctx.toast('先勾选要发邮件的邀请码', 'error');
        return;
    }
    var picked = all.filter(function (item) {
        return item.status === 'unused';
    });
    var skipped = all.filter(function (item) {
        return item.status !== 'unused';
    });

    if (!picked.length) {
        ctx.toast(
            '勾选的 ' + all.length + ' 张都不是「未使用」的（已用完 / 已过期 / 已停用的发出去也兑换不了）',
            'error'
        );
        return;
    }

    var open = function () {
        state.mailOpen = true;
        state.mailResult = null;
        renderMail();
    };

    // 勾选里混着「发出去也没用」的码：**别默默丢掉**（默默地少发几张，最容易出的事是
    // 「以为发出去了其实没发」）。把是哪几张、为什么说清楚，让人自己决定要不要继续。
    if (!skipped.length) {
        open();
        return;
    }
    ctx
        .confirm(
            '勾选的 ' + all.length + ' 张里，有 ' + skipped.length + ' 张发出去也兑换不了：\n\n' +
            describeSkipped(skipped) +
            '\n\n只会把剩下 ' + picked.length + ' 张与邮箱配对发送。仍要继续吗？'
        )
        .then(function (ok) {
            if (ok) open();
        });
}

/// 把「发不出去的」按状态归类成几行人话（最多列 3 个码，其余只报数量）
function describeSkipped(items) {
    var byKind = {};
    items.forEach(function (item) {
        var kind = inviteStatusLabel(item.status).text;
        if (!byKind[kind]) byKind[kind] = [];
        byKind[kind].push(item.code);
    });
    return Object.keys(byKind)
        .map(function (kind) {
            var codes = byKind[kind];
            var shown = codes.slice(0, 3).join('、');
            if (codes.length > 3) shown += ' 等';
            return '· ' + kind + '（' + codes.length + ' 张）：' + shown;
        })
        .join('\n');
}

function renderMail() {
    var card = nodes.mailCard;
    card.innerHTML = '';
    if (!state.mailOpen) {
        card.setAttribute('hidden', 'hidden');
        return;
    }
    card.removeAttribute('hidden');

    var picked = selectedItems().filter(function (item) {
        return item.status === 'unused';
    });

    card.appendChild(ctx.el('h3', { class: 'pn-card-title', text: '把邀请码发到邮箱' }));
    card.appendChild(
        ctx.el('p', {
            class: 'pn-desc',
            text:
                '选中的 ' +
                picked.length +
                ' 张码会**按顺序**与下面粘贴的邮箱一对一配对：第 1 行邮箱拿第 1 张码，依次类推。' +
                '数量必须一致。'
        })
    );

    var list = ctx.el('ol', { class: 'pn-sub', style: 'margin:8px 0 0 18px;' });
    picked.forEach(function (item) {
        list.appendChild(ctx.el('li', { class: 'pn-mono', text: item.code }));
    });
    card.appendChild(list);

    var textarea = ctx.el('textarea', {
        id: 'pnInvMailTo',
        placeholder: '每行一个邮箱，例如：\nfriend1@qq.com\nfriend2@163.com'
    });
    nodes.mailTo = textarea;
    card.appendChild(ctx.el('div', { class: 'pn-field pn-field-wide', style: 'margin-top:10px;' }, [
        ctx.el('label', { text: '收件邮箱（每行一个）' }),
        textarea
    ]));

    nodes.mailMsg = ctx.el('div', { class: 'pn-msg' });
    card.appendChild(nodes.mailMsg);

    if (state.mailResult) {
        card.appendChild(renderMailResult(state.mailResult));
    }

    var sendBtn = ctx.el('button', { class: 'pn-btn pn-btn-primary', type: 'button', text: '发送' });
    sendBtn.addEventListener('click', function () {
        doSendMail(picked);
    });
    nodes.mailSend = sendBtn;

    var cancelBtn = ctx.el('button', { class: 'pn-btn', type: 'button', text: '收起' });
    cancelBtn.addEventListener('click', function () {
        state.mailOpen = false;
        state.mailResult = null;
        renderMail();
    });

    card.appendChild(
        ctx.el('div', { class: 'pn-actions', style: 'margin-top:10px;' }, [sendBtn, cancelBtn])
    );
    card.appendChild(
        ctx.el('p', {
            class: 'pn-hint',
            text:
                '⚠️ 本机若没配 SMTP（AUTH_MAIL_MODE=log），服务端只会把邮件内容写进日志、并不真的发信；' +
                '响应里的 mail_mode 会告诉你当前是哪种模式。'
        })
    );
}

function renderMailResult(result) {
    var box = ctx.el('div', { class: 'pn-card', style: 'margin-top:12px;background:#f7fafd;' });
    box.appendChild(
        ctx.el('div', {
            class: 'pn-msg' + (result.failed ? ' is-warn' : ' is-ok'),
            text: '发送结果：成功 ' + result.sent + ' 封，失败 ' + result.failed + ' 封（mail_mode=' + result.mail_mode + '）'
        })
    );
    var table = ctx.el('table', { class: 'pn-table', style: 'margin-top:8px;' }, [
        ctx.el('thead', {}, [
            ctx.el('tr', {}, [
                ctx.el('th', { text: '邮箱' }),
                ctx.el('th', { text: '结果' }),
                ctx.el('th', { text: '说明' })
            ])
        ])
    ]);
    var tbody = ctx.el('tbody');
    (result.items || []).forEach(function (item) {
        tbody.appendChild(
            ctx.el('tr', {}, [
                ctx.el('td', { text: item.email }),
                ctx.el('td', {}, [
                    ctx.el('span', {
                        class: 'pn-badge ' + (item.ok ? 'is-ok' : 'is-bad'),
                        text: item.ok ? '已发送' : '失败'
                    })
                ]),
                ctx.el('td', { class: 'col-text', text: item.error || '—' })
            ])
        );
    });
    table.appendChild(tbody);
    box.appendChild(ctx.el('div', { class: 'pn-table-wrap' }, [table]));
    return box;
}

function doSendMail(picked) {
    if (state.mailBusy) return;
    var emails = String(nodes.mailTo && nodes.mailTo.value ? nodes.mailTo.value : '')
        .split(/[\s,;，；]+/)
        .map(function (s) {
            return s.trim();
        })
        .filter(function (s) {
            return !!s;
        });

    if (!emails.length) {
        nodes.mailMsg.className = 'pn-msg is-error';
        nodes.mailMsg.textContent = '请先粘贴收件邮箱（每行一个）';
        return;
    }
    if (emails.length !== picked.length) {
        nodes.mailMsg.className = 'pn-msg is-error';
        nodes.mailMsg.textContent =
            '邮箱数量（' + emails.length + '）与选中的邀请码数量（' + picked.length + '）不一致：必须一对一配对';
        return;
    }
    if (emails.length > 50) {
        nodes.mailMsg.className = 'pn-msg is-error';
        nodes.mailMsg.textContent = '单次最多 50 封';
        return;
    }

    state.mailBusy = true;
    nodes.mailSend.disabled = true;
    nodes.mailMsg.className = 'pn-msg';
    nodes.mailMsg.textContent = '正在发送…';

    var pairs = picked.map(function (item, index) {
        return { invite_id: item.id, email: emails[index] };
    });

    ctx
        .api('/api/auth/admin/invite-mail', { method: 'POST', body: { pairs: pairs } })
        .then(function (res) {
            var data = (res && res.data) || {};
            state.mailResult = data;
            nodes.mailMsg.className = 'pn-msg is-ok';
            nodes.mailMsg.textContent = '成功 ' + data.sent + ' 封，失败 ' + data.failed + ' 封';
            renderMail();
            loadList();
        })
        .catch(function (err) {
            nodes.mailMsg.className = 'pn-msg is-error';
            nodes.mailMsg.textContent = '发送失败：' + messageOf(err);
        })
        .then(function () {
            state.mailBusy = false;
        });
}

// ===================== 小工具 =====================

function value(id) {
    var el = document.getElementById(id);
    return el ? String(el.value || '').trim() : '';
}

function setValue(id, v) {
    var el = document.getElementById(id);
    if (el) el.value = String(v);
}

// 勾选可能来自两处：列表里的行、以及刚生成出来的那一批（生成后列表会刷新，同一批码会再出现一次），
// 所以合并后按 id 去重 —— 否则「发邮件」会在配对表里把同一张码列两遍，邮箱数量也就对不上了
function selectedItems() {
    var seen = {};
    var picked = [];
    state.created.concat(state.items).forEach(function (item) {
        if (!item || !state.selected[item.id] || seen[item.id]) return;
        seen[item.id] = true;
        picked.push(item);
    });
    return picked;
}

function itemLink(item) {
    return location.origin + '/account/?invite=' + encodeURIComponent(item.code);
}

function copyAndToast(text, okMessage) {
    copyText(text).then(function (ok) {
        if (ok) ctx.toast(okMessage || '已复制', 'ok');
        else ctx.toast('复制失败，请手动选中后复制', 'error');
    });
}

function exportCsv() {
    if (!state.items.length) {
        ctx.toast('当前页没有可导出的数据', 'error');
        return;
    }
    var rows = [['邀请码', '等级', '状态', '已用/上限', '有效期', '备注', '批次', '创建时间', '邀请链接']];
    state.items.forEach(function (item) {
        var status = inviteStatusLabel(item.status);
        rows.push([
            item.code,
            roleLabel(item.grant_role),
            status.text,
            item.used_count + '/' + item.max_uses,
            item.expires_at ? fmtTime(item.expires_at) : '不过期',
            item.note || '',
            item.batch_id || '',
            fmtTime(item.created_at),
            itemLink(item)
        ]);
    });
    downloadCsv('invites-page' + state.page + '-' + stamp() + '.csv', rows);
    ctx.toast('已导出当前页 ' + state.items.length + ' 条', 'ok');
}

function stamp() {
    var d = new Date();
    function pad(n) {
        return n < 10 ? '0' + n : String(n);
    }
    return (
        d.getFullYear() + pad(d.getMonth() + 1) + pad(d.getDate()) + '-' + pad(d.getHours()) + pad(d.getMinutes())
    );
}

// 壳的 api 失败时抛出的 Error：拿它的 message 就够看了
function messageOf(err) {
    if (!err) return '未知错误';
    if (err.message) return err.message;
    return String(err);
}
