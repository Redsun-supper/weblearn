// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
// 法律声明页的脚本，两块互不相干：
//   ① 左栏照片位的「点一下、弹一弹、再跳走」（2026-10 用户拍板方案 B）；
//   ② 出入场过场：入场动画全在 CSS（legal.css 的 is-staging），这里只负责播完抹掉
//      地址上的参数；退场由两个返回链接触发（legal.js 加 is-leaving），播完再跳。
//      时长与接力点见 legal.css 的「入场 / 退场」一节与根 TODO.md 第 4 条。
//
// 第 ② 块的整体设计（2026-10 用户拍板：「点击两个入口后，声明先从右到左弹出，弹到
// 3/4 时再弹出剩下的部分，反之同理」）：
//   入场：站名 / 「关于作者」的 href 带 ?from=logo / ?from=about → 内联脚本加 is-staging
//         → 声明侧栏从右滑入（340ms），滑到 3/4 时左栏弹出（255ms 起）→ 播完抹参数；
//   退场：点「← 返回首页 / 返回个人中心」→ is-leaving → 声明向右滑出（280ms），滑到
//         3/4 时左栏收回（210ms 起）→ 播完跳转。
//   ⚠️ 过场全部由本页自己承担：两个入口链接保持原生行为（带参而不拦点击，中击 /
//      Ctrl+点击新开标签、无 JS 时照样到达），目标页（首页 / 个人中心）不做任何
//      过场处理 —— 声明滑出后屏幕是法律页自己的白底，与首页同为白系，接缝看不出来。
//   ⚠️ 两个返回链接是**真链接**，只拦普通左键：带修饰键（Ctrl / Cmd / Shift / Alt）、
//      非左键、以及系统开了「减少动态效果」时一律放行；JS 挂了就是原来的裸跳。

// ---- ① 左栏照片位：点一下、弹一弹、再跳走 ----
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

// ---- ② 入场 / 退场：声明先动，剩下的部分后到 ----
(function () {
    'use strict';

    // 2026-10 用户拍板的动作：「点击两个入口后，声明先从右到左弹出，弹到 3/4 时再弹出
    // 剩下的部分，反之同理」。动画本身全在 legal.css（is-staging / is-leaving 两类），
    // 这里只做两件 CSS 做不了的事：入场播完抹掉地址上的参数；退场播完再跳转。
    // 两个总数与 CSS 里的时长 / 接力点是一组人工对齐的数（见根 TODO.md 第 4 条）：
    //   入场：声明 340ms，左栏 255ms 起手 + 260ms → 515ms，留 5ms 余量
    //   退场：声明 280ms，左栏 210ms 起手 + 180ms → 390ms，留 10ms 余量
    var ENTER_TOTAL_MS = 520;
    var EXIT_TOTAL_MS = 400;

    function prefersReducedMotion() {
        return !!(window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches);
    }

    // 入场播完把参数从地址上抹掉：刷新时不会再播一遍（URL 也干净）。
    // is-staging 类本身留着无害 —— 四段动画都是 both 填充，终帧就是自然状态。
    if (/[?&]from=(logo|about)(&|=|$)/.test(location.search)) {
        window.setTimeout(function () {
            if (window.history && window.history.replaceState) {
                window.history.replaceState(null, '', window.location.pathname);
            }
        }, ENTER_TOTAL_MS);
    }

    // 退场：两个返回链接（首页 / 个人中心）先播「同理」反向 —— 声明向右滑出、滑到
    // 3/4 时剩下的部分收回 —— 播完再跳。两处 href 不同，跳转各自走各自的。
    // ⚠️ 两个返回链接是**真链接**（NOTICE 附加条款第 3 条要求入口必须好使），只拦普通
    //    左键：带修饰键（Ctrl / Cmd / Shift / Alt）、非左键、以及系统开了「减少动态
    //    效果」时一律放行，让浏览器按原生方式跳；JS 挂了就是原来的裸跳。
    var leaving = false;
    var links = document.querySelectorAll('.legal-back');
    for (var i = 0; i < links.length; i++) {
        links[i].addEventListener('click', function (e) {
            if (leaving || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
            if (prefersReducedMotion()) return; // 不播退场，让链接按原生方式跳
            e.preventDefault();
            leaving = true;
            document.documentElement.classList.add('is-leaving');
            var href = this.getAttribute('href');
            window.setTimeout(function () {
                window.location.href = href;
            }, EXIT_TOTAL_MS);
        });
    }

    // bfcache 整页恢复（从目标页按浏览器返回键回来）：退场的 is-leaving 还挂在 html 上，
    // 两段动画都已播完（both 填充停在终帧）—— 不摘的话恢复出来的是一页「空壳法律页」
    //（声明滑走了、左栏透明）。leaving 标记也要复位，否则返回后两个链接再也不响应。
    window.addEventListener('pageshow', function (event) {
        if (!event.persisted) return;
        document.documentElement.classList.remove('is-leaving');
        leaving = false;
    });
})();
