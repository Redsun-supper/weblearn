// 广学 · 后台管理框架（通用骨架）
//
// 这是一个**独立入口**，与学生站互不影响：不走 main.js 的导航，也不使用学科页的
// localStorage 缓存。本文件只做「通用」的事——布局、侧栏导航、hash 路由、
// 学科后台模块的加载与挂载、通用工具（请求 / 提示 / 确认框 / DOM 构建）。
//
// ============================ 如何新增一个学科后台 ============================
// 两步：
//   1. 在 modules/<学科>/admin/ 下写模块，导出：
//        export var meta = { id, name, description };        // 可选，用于侧栏显示
//        export function mount(container, ctx) { ... }       // 必填，把界面渲染进 container
//        export function unmount() { ... }                   // 可选，切换学科时清理
//   2. 在下面的 SUBJECT_ADMINS 注册表里登记一行。
//
// 学科后台通过 ctx 拿到通用能力，不要各自重复实现：
//   ctx.api(path, {method, body})  取数/提交（自动 JSON、自动抛错）
//   ctx.toast(msg, kind)           右下角提示（kind: 'success' | 'error'）
//   ctx.confirm(msg)               确认框，返回 Promise<boolean>
//   ctx.el(tag, attrs, children)   构建 DOM（默认用 textContent，避免 XSS）
//   ctx.escapeHtml(str)            转义
//   ctx.setTitle(str)              改顶部标题
//
// 本文件保持 ES5 写法（var / function），仅使用 export / import 做模块化。

// ===================== 学科后台注册表 =====================
// main 分支上这里**保持为空**（只有通用骨架）；学科后台随各自模块分支加入。
// 示例（英语后台合并进来后会长这样）：
//   { id: 'english', name: '英语', description: '词条管理', module: '../modules/english/admin/english-admin.js' }
var SUBJECT_ADMINS = [];

// 模块路径相对本文件所在目录（/admin/）解析，因此学科后台写 '../modules/<学科>/admin/...'
var state = {
    entry: null,   // 当前挂载的注册表条目
    module: null,  // 当前学科后台模块
    ctx: null      // 传给 mount 的通用能力
};

// ===================== 启动 =====================

document.addEventListener('DOMContentLoaded', function () {
    // 鉴权预留：当前直接放行；接入登录后返回 false 即停止渲染
    if (!checkAuth()) return;

    renderNav();
    checkBackend();
    route();
    window.addEventListener('hashchange', route);
});

// ===================== 登录鉴权（预留，当前未启用） =====================

// 现在没有登录系统，所以直接放行。
// 将来接入登录只需改这一处：有会话（cookie / token）则返回 true，
// 否则显示 #adminAuthGate 并返回 false。
//
// ⚠️ 需要注意：真正安全的后台必须在**服务端**校验（后端目前也没有鉴权中间件）。
// 只靠前端拦截挡不住直接调接口的人，所以登录实现时后端要一起做。
function checkAuth() {
    var authed = true; // TODO(鉴权): 接入登录后改为真实会话校验
    if (!authed) {
        var gate = document.getElementById('adminAuthGate');
        var layout = document.getElementById('adminLayout');
        if (gate) gate.style.display = 'flex';
        if (layout) layout.style.display = 'none';
        return false;
    }
    return true;
}

// ===================== 布局与路由 =====================

// 依据注册表渲染侧栏导航
function renderNav() {
    var nav = document.getElementById('adminNav');
    if (!nav) return;
    nav.innerHTML = '';

    if (SUBJECT_ADMINS.length === 0) {
        nav.appendChild(el('div', { class: 'admin-nav-empty', text: '暂无已注册的学科后台' }));
        return;
    }
    SUBJECT_ADMINS.forEach(function (entry) {
        nav.appendChild(el('a', {
            class: 'admin-nav-item',
            href: '#/' + entry.id,
            'data-subject-id': entry.id
        }, [
            entry.name,
            entry.description ? el('span', { class: 'admin-nav-desc', text: entry.description }) : null
        ]));
    });
}

// hash 路由：#/english → 挂载对应学科后台
function route() {
    var id = String(location.hash || '').replace(/^#\/?/, '');

    if (!id) {
        // 没有指定学科：有注册就走第一个，没有就显示引导页
        if (SUBJECT_ADMINS.length > 0) {
            location.hash = '#/' + SUBJECT_ADMINS[0].id; // 触发 hashchange 重新进入 route()
            return;
        }
        setTitle('后台管理');
        renderEmptyState();
        return;
    }

    var entry = findEntry(id);
    if (!entry) {
        setTitle('未找到');
        renderMessage('没有这个学科后台：' + id, '请检查 admin/admin.js 的 SUBJECT_ADMINS 注册表。');
        return;
    }
    mountSubject(entry);
}

function findEntry(id) {
    for (var i = 0; i < SUBJECT_ADMINS.length; i++) {
        if (SUBJECT_ADMINS[i].id === id) return SUBJECT_ADMINS[i];
    }
    return null;
}

// 加载并挂载学科后台模块
function mountSubject(entry) {
    var container = document.getElementById('adminContent');
    if (!container) return;

    setTitle(entry.name);
    setActiveNav(entry.id);

    // 先清理上一个学科后台，避免它的定时器/监听残留
    unmountCurrent();

    container.innerHTML = '';
    container.appendChild(el('div', { class: 'admin-empty', text: '正在加载 ' + entry.name + ' 后台…' }));

    // 动态 import：只有点进该学科才会加载它的后台代码
    import(entry.module).then(function (mod) {
        if (typeof mod.mount !== 'function') {
            throw new Error('学科后台模块未导出 mount()：' + entry.module);
        }
        state.entry = entry;
        state.module = mod;
        state.ctx = createContext(entry);
        container.innerHTML = '';
        mod.mount(container, state.ctx);
    }).catch(function (err) {
        state.entry = null;
        state.module = null;
        container.innerHTML = '';
        var box = el('div', { class: 'admin-empty' }, [
            el('div', { class: 'admin-empty-title', text: '学科后台加载失败' }),
            el('div', { text: String(err) }),
            el('div', { class: 'admin-hint', text: '模块路径：' + entry.module })
        ]);
        container.appendChild(box);
        console.error('学科后台加载失败:', err);
    });
}

// 调用上一个模块的 unmount（若它导出了）
function unmountCurrent() {
    if (state.module && typeof state.module.unmount === 'function') {
        try {
            state.module.unmount();
        } catch (e) {
            console.error('学科后台清理失败:', e);
        }
    }
    state.entry = null;
    state.module = null;
    state.ctx = null;
}

function setTitle(text) {
    var node = document.getElementById('adminPageTitle');
    if (node) node.textContent = String(text);
}

function setActiveNav(id) {
    var items = document.querySelectorAll('.admin-nav-item');
    for (var i = 0; i < items.length; i++) {
        if (items[i].getAttribute('data-subject-id') === id) {
            items[i].classList.add('active');
        } else {
            items[i].classList.remove('active');
        }
    }
}

// ===================== 空状态与提示 =====================

// 尚未注册任何学科后台时的引导页（通用骨架的自我说明）
function renderEmptyState() {
    var container = document.getElementById('adminContent');
    if (!container) return;
    container.innerHTML = '';

    var box = el('div', { class: 'admin-empty' }, [
        el('div', { class: 'admin-empty-title', text: '通用后台骨架已就绪，但还没有注册任何学科后台' }),
        el('div', { text: '新增一个学科后台只需要两步：' }),
        el('div', { class: 'admin-hint' }, [
            el('div', { text: '1. 在 modules/<学科>/admin/ 下写模块，导出 mount(container, ctx)' }),
            el('div', { text: '2. 在 admin/admin.js 的 SUBJECT_ADMINS 注册表里登记一行' })
        ]),
        el('div', { class: 'admin-hint', text: '登录鉴权的位置已预留（checkAuth() + #adminAuthGate），当前未启用。' })
    ]);
    container.appendChild(box);
}

function renderMessage(title, detail) {
    var container = document.getElementById('adminContent');
    if (!container) return;
    container.innerHTML = '';
    container.appendChild(el('div', { class: 'admin-empty' }, [
        el('div', { class: 'admin-empty-title', text: title }),
        detail ? el('div', { text: detail }) : null
    ]));
}

// 后端连通性：调用 /api/health
function checkBackend() {
    var node = document.getElementById('backendStatus');
    if (!node) return;
    apiFetch('/api/health').then(function () {
        node.textContent = '后端正常';
        node.className = 'admin-backend-status ok';
    }).catch(function () {
        node.textContent = '后端未连接';
        node.className = 'admin-backend-status bad';
    });
}

// ===================== 通用能力（通过 ctx 交给学科后台） =====================

function createContext(entry) {
    return {
        entry: entry,
        api: apiFetch,
        toast: toast,
        confirm: confirmDialog,
        el: el,
        escapeHtml: escapeHtml,
        setTitle: setTitle
    };
}

// 请求助手：自动处理 JSON、把后端的 message 变成 Error、非 2xx 直接抛错。
// 注意：后端约定响应体形如 {code, message, data}，code===200 表示业务成功。
function apiFetch(path, options) {
    var opts = options || {};
    var init = { method: opts.method || 'GET', headers: {} };
    if (opts.body !== undefined) {
        init.headers['Content-Type'] = 'application/json';
        init.body = JSON.stringify(opts.body);
    }

    return fetch(path, init).then(function (res) {
        return res.text().then(function (text) {
            var data = null;
            try {
                data = text ? JSON.parse(text) : null;
            } catch (e) {
                // 非 JSON 响应（例如 nginx 的 502 页面），保持 data 为 null
            }
            if (!res.ok) {
                throw new Error((data && data.message) ? data.message : ('HTTP ' + res.status + ' ' + path));
            }
            if (data && typeof data.code === 'number' && data.code !== 200) {
                throw new Error(data.message || ('业务错误 code=' + data.code));
            }
            return data;
        });
    });
}

// 右下角提示条
function toast(message, kind) {
    var box = document.getElementById('adminToasts');
    if (!box) return;
    var item = el('div', { class: 'admin-toast' + (kind ? ' ' + kind : ''), text: String(message) });
    box.appendChild(item);
    setTimeout(function () {
        if (item.parentNode) item.parentNode.removeChild(item);
    }, kind === 'error' ? 6000 : 3000);
}

// 确认框（当前用浏览器原生 confirm，返回 Promise 以便统一写法）
function confirmDialog(message) {
    return Promise.resolve(window.confirm(String(message)));
}

// 极简 DOM 构建器：默认写 textContent，避免把数据当 HTML 解析。
// el('div', { class: 'x', text: '内容' }, [子节点或字符串])
// 需要插入可信 HTML 时用 { html: '...' }（仅用于自己拼的固定结构，不要放用户数据）
function el(tag, attrs, children) {
    var node = document.createElement(tag);
    if (attrs) {
        Object.keys(attrs).forEach(function (key) {
            var value = attrs[key];
            if (value === null || value === undefined) return;
            if (key === 'class') {
                node.className = value;
            } else if (key === 'text') {
                node.textContent = String(value);
            } else if (key === 'html') {
                node.innerHTML = value;
            } else if (key.indexOf('on') === 0 && typeof value === 'function') {
                node.addEventListener(key.slice(2), value);
            } else {
                node.setAttribute(key, value);
            }
        });
    }
    if (children) {
        var list = Array.isArray(children) ? children : [children];
        list.forEach(function (child) {
            if (child === null || child === undefined) return;
            node.appendChild(typeof child === 'string' ? document.createTextNode(child) : child);
        });
    }
    return node;
}

// HTML 转义（拼接字符串时使用；用 el() 的 { text } 则不需要）
function escapeHtml(value) {
    return String(value === null || value === undefined ? '' : value)
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&#39;');
}

// 暴露到 window 便于在浏览器控制台调试（不影响正常使用）
window.__guangxueAdmin = {
    admins: SUBJECT_ADMINS,
    api: apiFetch,
    toast: toast,
    escapeHtml: escapeHtml
};
