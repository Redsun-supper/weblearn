// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
// 管理面板共用的小工具（P1）
//
// 四个面板都要「把 UTC 文本按本地时间显示」「复制到剪贴板」「导出 CSV」「角色/状态的中文名」，
// 所以放一份在这里，不各写一遍。风格与 modules/english/admin/english-admin.js 一致：
// 保持 ES5 写法（var / function），只用 `export` 做模块导出。

// 服务端给的时间都是 **UTC 文本**（形如 2025-10-09T08:53:20Z）。
// ⚠️ 必须按**本地时间**显示：库里的 23:30Z 在国内是第二天早上 07:30，
// 直接截字符串会让人对不上「我到底是什么时候操作的」。
export function fmtTime(raw) {
    if (!raw) return '—';
    var date = new Date(raw);
    if (isNaN(date.getTime())) return String(raw);
    function pad(n) {
        return n < 10 ? '0' + n : String(n);
    }
    return (
        date.getFullYear() +
        '-' +
        pad(date.getMonth() + 1) +
        '-' +
        pad(date.getDate()) +
        ' ' +
        pad(date.getHours()) +
        ':' +
        pad(date.getMinutes())
    );
}

// 只要日期部分（本地时区）
export function fmtDate(raw) {
    var text = fmtTime(raw);
    return text.length >= 10 ? text.slice(0, 10) : text;
}

export function fmtNum(value) {
    if (value === null || value === undefined || value === '') return '0';
    return String(value);
}

// 0.1234 → 12.3%
export function pctText(ratio) {
    var n = Number(ratio);
    if (!isFinite(n)) return '—';
    return (n * 100).toFixed(1) + '%';
}

export function roleLabel(role) {
    if (role === 'super_admin') return '超级管理员';
    if (role === 'admin') return '管理员';
    if (role === 'user') return '普通用户';
    return role || '—';
}

// 邀请码状态 → { text, kind }（kind 直接当 pn-badge 的修饰类用）
export function inviteStatusLabel(status) {
    var table = {
        unused: { text: '未使用', kind: 'is-ok' },
        used: { text: '已用完', kind: 'is-info' },
        expired: { text: '已过期', kind: 'is-warn' },
        disabled: { text: '已停用', kind: 'is-muted' }
    };
    return table[status] || { text: status || '—', kind: 'is-muted' };
}

export function userStatusLabel(status) {
    if (status === 'active') return { text: '正常', kind: 'is-ok' };
    if (status === 'disabled') return { text: '已封禁', kind: 'is-bad' };
    return { text: status || '—', kind: 'is-muted' };
}

// 复制到剪贴板：优先用异步 API（https / localhost 才有），否则退回隐藏 textarea。
// 返回 Promise<boolean>，调用方据此提示「已复制 / 复制失败，请手动选中」。
export function copyText(text) {
    var value = String(text === null || text === undefined ? '' : text);
    if (navigator.clipboard && navigator.clipboard.writeText) {
        return navigator.clipboard.writeText(value).then(
            function () {
                return true;
            },
            function () {
                return fallbackCopy(value);
            }
        );
    }
    return Promise.resolve(fallbackCopy(value));
}

function fallbackCopy(value) {
    var area = document.createElement('textarea');
    area.value = value;
    area.setAttribute('readonly', 'readonly');
    area.style.position = 'fixed';
    area.style.left = '-9999px';
    document.body.appendChild(area);
    area.select();
    var ok = false;
    try {
        ok = document.execCommand('copy');
    } catch (e) {
        ok = false;
    }
    document.body.removeChild(area);
    return ok;
}

// 导出 CSV：多一列就多一列，别让调用方去操心引号与逗号
export function downloadCsv(filename, rows) {
    var lines = rows.map(function (row) {
        return row
            .map(function (cell) {
                var text = cell === null || cell === undefined ? '' : String(cell);
                return '"' + text.replace(/"/g, '""') + '"';
            })
            .join(',');
    });
    // \ufeff = BOM：不加它，Excel 打开中文会乱码
    var blob = new Blob(['\ufeff' + lines.join('\r\n')], { type: 'text/csv;charset=utf-8' });
    var url = URL.createObjectURL(blob);
    var link = document.createElement('a');
    link.href = url;
    link.download = filename;
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
    // 立刻回收：文件已经在下载队列里了
    setTimeout(function () {
        URL.revokeObjectURL(url);
    }, 0);
}

// 把一组 id 拼成 `?ids=1,2,3`（批量接口用；空数组返回空串）
export function idsQuery(ids) {
    if (!ids || !ids.length) return '';
    return ids.join(',');
}
