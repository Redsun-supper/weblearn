// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
// 法律声明页唯一的脚本：左栏照片位的「点一下、弹一弹、再跳走」。
//
// 2026-10 用户拍板**方案 B**（在「按下缩放 + 悬停抬起」之上再叠一段点击弹跳）：
//   hover / active 两段是纯 CSS（见 legal.css 的 .legal-photo），这里只负责点击那一段 ——
//   挂上 .is-bouncing 放大到 1.03，BOUNCE_MS 之后摘掉（回落到 1.0）再跳转。
// ⚠️ BOUNCE_MS 与 legal.css 里 .is-bouncing / .legal-photo 的 transition 时长是一组，
//    改一个要顺带看另一个（这也是「转场时长人工对齐」的老问题，见根 TODO.md 第 4 条）。
//
// ⚠️ 跳转刻意放在 setTimeout 里（用户接受这 260ms 的延迟），于是**必须**处理被弹窗拦截的情况：
//   用户手势的「瞬时激活」通常有几秒，正常不会被拦；但真被拦了就得退回当前页跳转，
//   否则用户点完没反应、控制台也不报错，看起来就像「点击坏了」。
(function () {
    'use strict';

    var BOUNCE_MS = 260;

    var photo = document.getElementById('legalPhoto');
    if (!photo) {
        return;
    }

    var busy = false;

    photo.addEventListener('click', function (e) {
        // 带修饰键的点击（Ctrl / Cmd / Shift / Alt + 点）交回浏览器 —— 那是用户自己要新开标签、
        // 新开窗口或另存，我们不该抢过来自己处理
        if (busy || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) {
            return;
        }

        e.preventDefault(); // 先别跳，让动画播完
        busy = true;
        photo.classList.add('is-bouncing');

        window.setTimeout(function () {
            photo.classList.remove('is-bouncing');
            busy = false;

            // HTML 上已经写了 rel="noopener"，这里再用 opener = null 兜一层
            // （老浏览器不认 rel="noopener"；不能用 window.open 的 'noopener' 特性串 ——
            //   带它时返回值恒为 null，会把「被拦截」的兜底误判成真的被拦了）
            var opened = window.open(photo.href, '_blank');
            if (opened) {
                opened.opener = null;
            } else {
                window.location.href = photo.href;
            }
        }, BOUNCE_MS);
    });
})();
