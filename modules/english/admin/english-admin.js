// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
// 英语后台 · 词条管理
//
// 由通用后台框架（admin/admin.js）按需动态 import，并调用本模块的 mount(container, ctx)。
// 通用能力（请求 / 提示 / 确认 / DOM 构建）由 ctx 注入，本模块不重复实现。
//
// 三块功能：
//   1. 词条列表：关键词搜索、词书/单元筛选、分页
//   2. 新增 / 编辑 / 删除词条（删除会连带复习状态与日志，界面会提示）
//   3. 批量导入：粘贴文本 → 交给引擎解析（Rust，容错各种格式）→ 预览 → 分批导入
//
// 计算都放在引擎里：解析词表用的是 WASM 的 parse_word_list，本文件只做渲染与请求。
// 本文件保持 ES5 写法（var / function），仅用 export 做模块导出。

// 引擎模块路径（相对本文件所在目录解析）
var ENGINE_URL = '../engine/pkg/guangxue_wasm.js';

// 每次 POST 的词条数：太大单请求过重，太小请求次数多
var IMPORT_CHUNK = 200;

// 框架注入的通用能力 / 挂载容器 / 引擎模块（提供 parse_word_list）
var ctx = null;
var root = null;
var engine = null;

// 后台视图状态：当前视图 + 列表的筛选与分页 + 编辑中的词条 + 导入面板的原文与解析结果。
// view 三态：list | editor | import；editing.id 为 0 表示新增。
var state = {
    view: 'list',   // list | editor | import
    search: '',
    book: '',
    unit: '',
    limit: 20,
    offset: 0,
    total: 0,
    items: [],
    options: { books: [], units: [] },
    editing: null,
    pasteText: '',
    parse: null,
    importBook: '',
    importUnit: '',
    importing: false
};

// ===================== 生命周期 =====================

export function mount(container, context) {
    ctx = context;
    root = container;
    state.view = 'list';
    state.editing = null;
    state.parse = null;
    state.offset = 0;
    loadOptions();
    loadList();
}

export function unmount() {
    // 没有定时器/全局监听需要清理，重置视图状态即可
    ctx = null;
    root = null;
    state.editing = null;
    state.parse = null;
    state.items = [];
}

// ===================== 数据加载 =====================

// 已有词书/单元（下拉提示用）
function loadOptions() {
    return ctx.api('/api/word-options').then(function (res) {
        var d = (res && res.data) || {};
        state.options = { books: d.books || [], units: d.units || [] };
    }).catch(function (err) {
        console.error('加载词书/单元选项失败:', err);
    });
}

function loadList() {
    var params = ['limit=' + state.limit, 'offset=' + state.offset];
    if (state.search) params.push('search=' + encodeURIComponent(state.search));
    if (state.book) params.push('book=' + encodeURIComponent(state.book));
    if (state.unit) params.push('unit=' + encodeURIComponent(state.unit));

    renderMessage('正在加载词条…');

    return ctx.api('/api/words?' + params.join('&')).then(function (res) {
        var d = (res && res.data) || {};
        state.items = d.items || [];
        state.total = d.total || 0;
        renderList();
    }).catch(function (err) {
        renderMessage('加载失败', String(err));
        ctx.toast('加载词条失败：' + err, 'error');
    });
}

// ===================== 列表视图 =====================

function renderList() {
    var el = ctx.el;
    root.textContent = '';

    var searchInput = el('input', { type: 'text', value: state.search, placeholder: '单词或释义关键词' });
    var bookInput = el('input', { type: 'text', value: state.book, placeholder: '不限', list: 'admBooks' });
    var unitInput = el('input', { type: 'text', value: state.unit, placeholder: '不限', list: 'admUnits' });

    function applyFilter() {
        state.search = searchInput.value.trim();
        state.book = bookInput.value.trim();
        state.unit = unitInput.value.trim();
        state.offset = 0;
        loadList();
    }
    [searchInput, bookInput, unitInput].forEach(function (input) {
        input.addEventListener('keydown', function (e) {
            if (e.key === 'Enter') applyFilter();
        });
    });

    // ---- 工具条 ----
    root.appendChild(el('div', { class: 'admin-card' }, [
        el('div', { class: 'admin-actions' }, [
            el('div', { class: 'admin-field', style: 'flex:2 1 180px;margin:0' }, [
                el('label', { text: '搜索' }), searchInput
            ]),
            el('div', { class: 'admin-field', style: 'flex:1 1 120px;margin:0' }, [
                el('label', { text: '词书' }), bookInput
            ]),
            el('div', { class: 'admin-field', style: 'flex:1 1 120px;margin:0' }, [
                el('label', { text: '单元' }), unitInput
            ]),
            el('button', { class: 'admin-btn admin-btn-primary', text: '查询', onclick: applyFilter }),
            el('button', {
                class: 'admin-btn', text: '重置', onclick: function () {
                    searchInput.value = ''; bookInput.value = ''; unitInput.value = '';
                    applyFilter();
                }
            }),
            el('button', {
                class: 'admin-btn admin-btn-primary', text: '＋ 新增词条',
                onclick: function () { openEditor(null); }
            }),
            el('button', { class: 'admin-btn', text: '⇪ 批量导入', onclick: openImport })
        ]),
        el('datalist', { id: 'admBooks' }, state.options.books.map(function (b) { return el('option', { value: b }); })),
        el('datalist', { id: 'admUnits' }, state.options.units.map(function (u) { return el('option', { value: u }); }))
    ]));

    // ---- 统计 ----
    var page = Math.floor(state.offset / state.limit) + 1;
    var pages = Math.max(1, Math.ceil(state.total / state.limit));
    root.appendChild(el('div', { class: 'admin-hint', text: '共 ' + state.total + ' 条 · 本页 ' + state.items.length + ' 条 · 第 ' + page + ' / ' + pages + ' 页' }));

    // ---- 空态 ----
    if (state.items.length === 0) {
        root.appendChild(el('div', { class: 'admin-empty' }, [
            el('div', { class: 'admin-empty-title', text: '没有匹配的词条' }),
            el('div', { text: state.total === 0 ? '词库为空，可以用「批量导入」粘贴词表。' : '试试调整搜索或筛选条件。' })
        ]));
        return;
    }

    // ---- 表格 ----
    var table = el('table', { class: 'admin-table' }, [
        el('thead', null, el('tr', null, [
            el('th', { text: 'ID' }),
            el('th', { text: '单词' }),
            el('th', { text: '音标' }),
            el('th', { text: '释义' }),
            el('th', { text: '例句' }),
            el('th', { text: '词书' }),
            el('th', { text: '单元' }),
            el('th', { text: '操作' })
        ])),
        el('tbody', null, state.items.map(renderRow))
    ]);
    root.appendChild(el('div', { class: 'admin-card' }, el('div', { class: 'admin-table-wrap' }, table)));

    // ---- 分页 ----
    root.appendChild(el('div', { class: 'admin-pager' }, [
        el('button', {
            class: 'admin-btn admin-btn-sm', text: '← 上一页',
            disabled: state.offset <= 0 ? 'disabled' : null,
            onclick: function () {
                if (state.offset <= 0) return;
                state.offset = Math.max(0, state.offset - state.limit);
                loadList();
            }
        }),
        el('span', { text: '第 ' + page + ' / ' + pages + ' 页' }),
        el('button', {
            class: 'admin-btn admin-btn-sm', text: '下一页 →',
            disabled: (state.offset + state.limit) >= state.total ? 'disabled' : null,
            onclick: function () {
                if (state.offset + state.limit >= state.total) return;
                state.offset += state.limit;
                loadList();
            }
        })
    ]));
}

function renderRow(item) {
    var el = ctx.el;
    var senses = item.senses || [];

    // 释义列：多条释义时补一个角标，方便一眼看出这个词已经拆过义项
    var meaningCell = el('td', { class: 'col-text' }, [
        el('span', { text: item.meaning || '—' }),
        senses.length > 1 ? el('span', { class: 'admin-badge admin-badge-sm', text: senses.length + ' 条释义' }) : null
    ]);

    // 例句列：第二行显示中文翻译；没填的标出来，方便逐条补齐
    var exampleCell = el('td', { class: 'col-text' }, [
        el('div', { text: item.example || '—' }),
        item.example_translation
            ? el('div', { class: 'col-sub', text: '译：' + item.example_translation })
            : el('div', { class: 'col-sub col-sub-missing', text: '（缺例句翻译）' })
    ]);

    return el('tr', null, [
        el('td', { text: item.id }),
        el('td', { text: item.word }),
        el('td', { text: item.phonetic || '—' }),
        meaningCell,
        exampleCell,
        el('td', { text: item.book || '—' }),
        el('td', { text: item.unit || '—' }),
        el('td', { class: 'col-actions' }, [
            el('button', {
                class: 'admin-btn admin-btn-sm', text: '编辑',
                onclick: function () { openEditor(item); }
            }),
            el('button', {
                class: 'admin-btn admin-btn-sm admin-btn-danger', text: '删除',
                onclick: function () { removeWord(item); }
            })
        ])
    ]);
}

// ===================== 新增 / 编辑 =====================

function openEditor(item) {
    state.view = 'editor';
    if (item) {
        state.editing = {
            id: item.id, word: item.word, phonetic: item.phonetic, meaning: item.meaning,
            example: item.example, example_translation: item.example_translation || '',
            // 深拷贝一份释义数组：编辑时直接改 state，取消编辑不会污染列表数据
            senses: (item.senses || []).map(function (s) {
                return {
                    pos: s.pos || '', meaning: s.meaning || '',
                    example: s.example || '', translation: s.translation || ''
                };
            }),
            book: item.book, unit: item.unit
        };
    } else {
        // 新增时把当前的筛选条件带进去，方便连续录入同一单元的词
        state.editing = {
            id: 0, word: '', phonetic: '', meaning: '', example: '', example_translation: '',
            senses: [],
            book: state.book, unit: state.unit
        };
    }
    renderEditor();
}

// 画「多释义」编辑区：一行一个义项（词性 / 释义 / 例句 / 译文），可增可删
function renderSenseEditor(container) {
    var el = ctx.el;
    var list = state.editing.senses;

    function rerender() {
        container.textContent = '';
        build();
    }

    function build() {
        for (var i = 0; i < list.length; i++) {
            (function (index) {
                var sense = list[index];
                function field(label, key, attrs) {
                    var input = el('input', attrs || { type: 'text' });
                    input.value = sense[key] || '';
                    input.addEventListener('input', function () { sense[key] = input.value; });
                    return el('div', { class: 'admin-field' }, [el('label', { text: label }), input]);
                }
                container.appendChild(el('div', { class: 'admin-sense-row' }, [
                    el('div', { class: 'admin-sense-head' }, [
                        el('span', { text: '释义 ' + (index + 1) }),
                        el('button', {
                            class: 'admin-btn admin-btn-sm admin-btn-danger', text: '删除这条',
                            onclick: function () {
                                list.splice(index, 1);
                                rerender();
                            }
                        })
                    ]),
                    el('div', { class: 'admin-grid-2' }, [
                        field('词性', 'pos', { type: 'text', placeholder: 'n. / v. / adj.' }),
                        field('释义', 'meaning', { type: 'text', placeholder: '好处；益处' })
                    ]),
                    field('例句（可空，空了用上面那条例句）', 'example', { type: 'text', placeholder: 'Exercise has many benefits.' }),
                    field('例句翻译（可空）', 'translation', { type: 'text', placeholder: '锻炼有很多好处。' })
                ]));
            })(i);
        }

        container.appendChild(el('div', { class: 'admin-actions' }, [
            el('button', {
                class: 'admin-btn admin-btn-sm', text: '＋ 添加一条释义',
                onclick: function () {
                    list.push({ pos: '', meaning: '', example: '', translation: '' });
                    rerender();
                }
            })
        ]));
    }

    build();
}

function renderEditor() {
    var el = ctx.el;
    var e = state.editing;
    var isNew = !e.id;
    root.textContent = '';

    // 输入即写回 state，避免切视图丢内容
    function bind(key, attrs) {
        var input = el('input', attrs);
        input.value = e[key] || '';
        input.addEventListener('input', function () { e[key] = input.value; });
        return input;
    }

    var textarea = el('textarea');
    textarea.value = e.example || '';
    textarea.addEventListener('input', function () { e.example = textarea.value; });

    var senseBox = el('div', { class: 'admin-senses' });

    root.appendChild(el('div', { class: 'admin-card' }, [
        el('div', { class: 'admin-card-title', text: isNew ? '新增词条' : ('编辑词条 #' + e.id) }),
        el('div', { class: 'admin-grid-2' }, [
            el('div', { class: 'admin-field' }, [el('label', { text: '单词 *' }), bind('word', { type: 'text', placeholder: 'abandon' })]),
            el('div', { class: 'admin-field' }, [el('label', { text: '音标' }), bind('phonetic', { type: 'text', placeholder: '/əˈbæn.dən/' })])
        ]),
        el('div', { class: 'admin-field' }, [el('label', { text: '释义' }), bind('meaning', { type: 'text', placeholder: 'v. 放弃；抛弃' })]),
        el('div', { class: 'admin-field' }, [el('label', { text: '例句' }), textarea]),
        el('div', { class: 'admin-field' }, [el('label', { text: '例句翻译' }), bind('example_translation', { type: 'text', placeholder: '他放弃了他那辆旧车。' })]),
        el('div', { class: 'admin-card-title', style: 'margin-top:6px', text: '多释义（可选）' }),
        el('div', { class: 'admin-hint', text: '一个词有多个义项时在这里一条条填（名词一块、动词一块），复习时就会分块显示；' +
            '每条还能带自己的例句与译文。留空则按上面「释义」里的词性标签自动分块（例如「n. 好处；益处 v. 有益于」拆成两块）。' }),
        senseBox,
        el('div', { class: 'admin-grid-2' }, [
            el('div', { class: 'admin-field' }, [el('label', { text: '词书' }), bind('book', { type: 'text', placeholder: '如：必修一', list: 'admBooks' })]),
            el('div', { class: 'admin-field' }, [el('label', { text: '单元' }), bind('unit', { type: 'text', placeholder: '如：Unit 1', list: 'admUnits' })])
        ]),
        el('datalist', { id: 'admBooks' }, state.options.books.map(function (b) { return el('option', { value: b }); })),
        el('datalist', { id: 'admUnits' }, state.options.units.map(function (u) { return el('option', { value: u }); })),
        el('div', { class: 'admin-actions' }, [
            el('button', { class: 'admin-btn admin-btn-primary', text: isNew ? '新增' : '保存', onclick: saveWord }),
            el('button', { class: 'admin-btn', text: '取消', onclick: backToList })
        ]),
        el('div', { class: 'admin-hint', text: '提示：词书 / 单元可留空；同名词条已存在时会提示而不是新增。' })
    ]));

    renderSenseEditor(senseBox);
}

// 提交前清洗释义：丢掉「词性和释义都没填」的空行（后端也会再清一遍）
function cleanSenses() {
    return (state.editing.senses || []).map(function (s) {
        return {
            pos: (s.pos || '').trim(),
            meaning: (s.meaning || '').trim(),
            example: (s.example || '').trim(),
            translation: (s.translation || '').trim()
        };
    }).filter(function (s) {
        return s.pos || s.meaning;
    });
}

function saveWord() {
    var e = state.editing;
    var word = (e.word || '').trim();
    if (!word) {
        ctx.toast('单词不能为空', 'error');
        return;
    }
    var body = {
        word: word,
        phonetic: (e.phonetic || '').trim(),
        meaning: (e.meaning || '').trim(),
        example: (e.example || '').trim(),
        example_translation: (e.example_translation || '').trim(),
        senses: cleanSenses(),
        book: (e.book || '').trim(),
        unit: (e.unit || '').trim()
    };
    var isNew = !e.id;

    var request = isNew
        ? ctx.api('/api/words', { method: 'POST', body: { words: [body] } })
        : ctx.api('/api/words/' + e.id, { method: 'PUT', body: body });

    request.then(function (res) {
        if (isNew) {
            var d = (res && res.data) || {};
            if (!d.created) {
                ctx.toast('该单词已存在，未新增', 'error');
                return;
            }
            ctx.toast('已新增「' + word + '」', 'success');
        } else {
            ctx.toast('已更新「' + word + '」', 'success');
        }
        backToList();
    }).catch(function (err) {
        ctx.toast('保存失败：' + err, 'error');
    });
}

function removeWord(item) {
    ctx.confirm(
        '确认删除「' + item.word + '」？\n\n' +
        '注意：会同时删除它的复习状态与复习日志，该词的记忆进度会丢失。\n' +
        '如果只是想改错别字，请用「编辑」，不要删了重建。'
    ).then(function (ok) {
        if (!ok) return;
        return ctx.api('/api/words/' + item.id, { method: 'DELETE' }).then(function (res) {
            var d = (res && res.data) || {};
            ctx.toast('已删除「' + d.deleted_word + '」，连带清理复习状态 ' +
                (d.removed_reviews || 0) + ' 条、日志 ' + (d.removed_logs || 0) + ' 条', 'success');
            return loadList().then(loadOptions);
        }).catch(function (err) {
            ctx.toast('删除失败：' + err, 'error');
        });
    });
}

function backToList() {
    state.view = 'list';
    state.editing = null;
    state.parse = null;
    loadList();
    loadOptions();
}

// ===================== 批量导入 =====================

function openImport() {
    state.view = 'import';
    state.parse = null;
    renderImport();
}

function renderImport() {
    var el = ctx.el;
    root.textContent = '';

    var textarea = el('textarea', {
        style: 'min-height:200px;font-family:Consolas,monospace',
        placeholder: '在此粘贴词表，一行一个词。例如：\n' +
            'abandon\n' +
            'ability\t/əˈbɪl.ə.ti/\tn. 能力；才能\tShe has the ability to lead.\t她有领导团队的能力。\n' +
            'achieve | /əˈtʃiːv/ | v. 实现；达到\n' +
            'adapt, /əˈdæpt/, v. 适应'
    });
    textarea.value = state.pasteText || '';
    textarea.addEventListener('input', function () { state.pasteText = textarea.value; });

    var bookInput = el('input', { type: 'text', placeholder: '如：必修一（可空）', list: 'admBooks' });
    bookInput.value = state.importBook || '';
    bookInput.addEventListener('input', function () { state.importBook = bookInput.value; });

    var unitInput = el('input', { type: 'text', placeholder: '如：Unit 1（可空）', list: 'admUnits' });
    unitInput.value = state.importUnit || '';
    unitInput.addEventListener('input', function () { state.importUnit = unitInput.value; });

    root.appendChild(el('div', { class: 'admin-card' }, [
        el('div', { class: 'admin-card-title', text: '批量导入词条' }),
        el('div', { class: 'admin-hint', html:
            '支持的格式（自动识别，按优先级）：<br>' +
            '① 制表符（从 Excel 直接粘贴最省事）　② 竖线 <code>|</code>　③ 逗号 <code>,</code>　④ 空格<br>' +
            '字段按位置对应：<b>单词 / 音标 / 释义 / 例句 / 例句翻译</b>，多出的字段忽略。<br>' +
            '以 <code>#</code> 或 <code>//</code> 开头的行与空行会被跳过；只有单词一行也能导入。<br>' +
            '导入只填「单条释义」；多释义请在列表里逐个词用「编辑」补（一个词一块词性）。' }),
        el('div', { class: 'admin-field' }, [el('label', { text: '词表内容' }), textarea]),
        el('div', { class: 'admin-grid-2' }, [
            el('div', { class: 'admin-field' }, [el('label', { text: '本次导入的词书（应用到全部）' }), bookInput]),
            el('div', { class: 'admin-field' }, [el('label', { text: '本次导入的单元（应用到全部）' }), unitInput])
        ]),
        el('datalist', { id: 'admBooks' }, state.options.books.map(function (b) { return el('option', { value: b }); })),
        el('datalist', { id: 'admUnits' }, state.options.units.map(function (u) { return el('option', { value: u }); })),
        el('div', { class: 'admin-actions' }, [
            el('button', { class: 'admin-btn admin-btn-primary', text: '解析预览', onclick: doParse }),
            el('button', { class: 'admin-btn', text: '返回列表', onclick: backToList })
        ])
    ]));

    if (state.parse) renderParseResult();
}

// 解析结果面板：统计徽章 + 前 20 行预览 + 被跳过的错误行（前 50）+ 文件内重复词 + 导入按钮
function renderParseResult() {
    var el = ctx.el;
    var p = state.parse;
    var s = p.stats;

    var box = el('div', { class: 'admin-card' }, [
        el('div', { class: 'admin-card-title', text: '解析结果' }),
        el('div', { class: 'admin-actions' }, [
            el('span', { class: 'admin-badge', text: '可导入 ' + s.ok + ' 条' }),
            el('span', { class: 'admin-badge', text: '解析行数 ' + s.total_lines }),
            s.duplicates ? el('span', { class: 'admin-badge', text: '文件内重复 ' + s.duplicates + ' 条' }) : null,
            s.errors ? el('span', { class: 'admin-badge', text: '错误 ' + s.errors + ' 行' }) : null
        ])
    ]);

    var preview = p.rows.slice(0, 20);
    if (preview.length > 0) {
        box.appendChild(el('div', { class: 'admin-table-wrap' }, el('table', { class: 'admin-table' }, [
            el('thead', null, el('tr', null, [
                el('th', { text: '行' }), el('th', { text: '单词' }), el('th', { text: '音标' }),
                el('th', { text: '释义' }), el('th', { text: '例句' }), el('th', { text: '例句翻译' })
            ])),
            el('tbody', null, preview.map(function (r) {
                return el('tr', null, [
                    el('td', { text: r.line }),
                    el('td', { text: r.word }),
                    el('td', { text: r.phonetic || '—' }),
                    el('td', { class: 'col-text', text: r.meaning || '—' }),
                    el('td', { class: 'col-text', text: r.example || '—' }),
                    el('td', { class: 'col-text', text: r.example_translation || '—' })
                ]);
            }))
        ])));
        if (p.rows.length > preview.length) {
            box.appendChild(el('div', { class: 'admin-hint', text: '（仅预览前 ' + preview.length + ' 条，共 ' + p.rows.length + ' 条）' }));
        }
    }

    if (p.errors.length > 0) {
        box.appendChild(el('div', { class: 'admin-card-title', style: 'margin-top:14px', text: '被跳过的错误行' }));
        box.appendChild(el('div', { class: 'admin-table-wrap' }, el('table', { class: 'admin-table' }, [
            el('thead', null, el('tr', null, [el('th', { text: '行' }), el('th', { text: '原文' }), el('th', { text: '原因' })])),
            el('tbody', null, p.errors.slice(0, 50).map(function (e) {
                return el('tr', null, [
                    el('td', { text: e.line }),
                    el('td', { class: 'col-text', text: e.text }),
                    el('td', { text: e.reason })
                ]);
            }))
        ])));
        if (p.errors.length > 50) {
            box.appendChild(el('div', { class: 'admin-hint', text: '（仅显示前 50 条错误）' }));
        }
    }

    if (p.duplicates.length > 0) {
        box.appendChild(el('div', { class: 'admin-hint', text: '文件内重复（已跳过，大小写不同也算重复）：' +
            p.duplicates.slice(0, 20).map(function (d) { return d.word + '（第 ' + d.line + ' 行）'; }).join('、') +
            (p.duplicates.length > 20 ? ' 等' : '') }));
    }

    box.appendChild(el('div', { class: 'admin-actions', style: 'margin-top:14px' }, [
        el('button', {
            class: 'admin-btn admin-btn-primary',
            text: state.importing ? '导入中…' : ('确认导入 ' + s.ok + ' 条'),
            disabled: (state.importing || s.ok === 0) ? 'disabled' : null,
            onclick: doImport
        }),
        el('span', { class: 'admin-hint', id: 'impProgress', text: '' })
    ]));

    root.appendChild(box);
}

function doParse() {
    var text = state.pasteText || '';
    if (!text.trim()) {
        ctx.toast('请先粘贴词表内容', 'error');
        return;
    }
    loadEngine().then(function (mod) {
        // 解析在引擎（Rust）里完成，这里只接收结构化结果
        state.parse = JSON.parse(mod.parse_word_list(text));
        renderImport();
        var s = state.parse.stats;
        ctx.toast('解析完成：可导入 ' + s.ok + ' 条' +
            (s.duplicates ? '，重复 ' + s.duplicates + ' 条' : '') +
            (s.errors ? '，错误 ' + s.errors + ' 行' : ''));
    }).catch(function (err) {
        ctx.toast('引擎加载失败：' + err, 'error');
        console.error('加载词表解析引擎失败:', err);
    });
}

function doImport() {
    if (state.importing || !state.parse || state.parse.rows.length === 0) return;
    state.importing = true;
    renderImport();

    var rows = state.parse.rows;
    var acc = { created: 0, skipped: 0 };
    var progressEl = document.getElementById('impProgress');

    function onProgress(done, total, created, skipped) {
        if (progressEl) {
            progressEl.textContent = '已处理 ' + done + ' / ' + total +
                '（新增 ' + created + '，跳过 ' + skipped + '）';
        }
    }

    ctx.toast('开始导入 ' + rows.length + ' 条…');

    importChunks(rows, 0, acc, onProgress).then(function () {
        state.importing = false;
        state.parse = null;
        state.pasteText = '';
        ctx.toast('导入完成：新增 ' + acc.created + ' 条，跳过 ' + acc.skipped + ' 条', 'success');
        backToList();
    }).catch(function (err) {
        state.importing = false;
        renderImport();
        ctx.toast('导入中断（已成功 ' + acc.created + ' 条）：' + err, 'error');
    });
}

// 分批提交，避免单次请求过大 / 后端处理过久
function importChunks(rows, index, acc, onProgress) {
    if (index >= rows.length) return Promise.resolve(acc);

    var slice = rows.slice(index, index + IMPORT_CHUNK);
    var words = slice.map(function (r) {
        return {
            word: r.word, phonetic: r.phonetic, meaning: r.meaning, example: r.example,
            example_translation: r.example_translation,
            book: state.importBook, unit: state.importUnit
        };
    });

    return ctx.api('/api/words', { method: 'POST', body: { words: words } }).then(function (res) {
        var d = (res && res.data) || {};
        acc.created += d.created || 0;
        acc.skipped += d.skipped || 0;
        onProgress(Math.min(index + IMPORT_CHUNK, rows.length), rows.length, acc.created, acc.skipped);
        return importChunks(rows, index + IMPORT_CHUNK, acc, onProgress);
    });
}

// ===================== 引擎 =====================

// 动态加载引擎（只有用到「解析预览」时才会真正加载 wasm）
function loadEngine() {
    if (engine) return Promise.resolve(engine);
    return import(ENGINE_URL).then(function (mod) {
        return mod.default().then(function () {
            engine = mod;
            return mod;
        });
    });
}

// ===================== 通用小工具 =====================

function renderMessage(text, detail) {
    if (!root || !ctx) return;
    root.textContent = '';
    root.appendChild(ctx.el('div', { class: 'admin-empty' }, [
        ctx.el('div', { class: 'admin-empty-title', text: text }),
        detail ? ctx.el('div', { text: detail }) : null
    ]));
}
