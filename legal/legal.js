// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
// 法律声明页的脚本，两块互不相干：
//   ① 左栏照片位的「点一下、弹一弹、再跳走」（2026-10 用户拍板方案 B）；
//   ② 出入场过场：动画全在 CSS（legal.css 的 is-staging / is-leaving），这里只做
//      CSS 做不了的三件事：独立打开时播完抹掉地址上的参数；浮层模式下退场播完叫
//      父页面关掉浮层（而不是跳页）；bfcache 恢复时复位退场残留。
//      时长与接力点见 legal.css 的「入场 / 退场」一节与根 TODO.md 第 4 条。
//
// 第 ② 块的整体设计（2026-10 用户拍板、第二轮细化 + 第三轮「浮层化」）：
//   动作：「声明从右往左滑，滑到 3/4 时，下一级（剩余部分）从法律声明下面、一样
//   从右往左运动，反之同理」；且转场要「直接覆盖在原有画面上」—— 法律页以 iframe
//   浮层打开，原页面不动地垫在下面，背衬淡入把它盖住，两栏在背衬上滑入。
//   入场：站名 / 「关于作者」被父页面拦下（普通左键）→ 父页面显示 iframe 并给里面
//         加 is-staging → 声明侧栏从右滑入（340ms），滑到 3/4 时左栏从声明下面同样
//         从右往左滑出（255ms 起手、400ms）；浮层模式下背衬同时淡入（560ms）盖住
//         原页面。独立打开（?from=logo / ?from=about，书签 / 无 JS 原生跳转）时
//         is-staging 由 index.html 头的内联脚本加，背衬本来就不透明。
//   退场：点「← 返回首页 / 返回个人中心」→ is-leaving → 声明向右滑出（280ms），滑到
//         3/4 时左栏同步向右收回、缩回声明下面（210ms 起手、300ms）；浮层模式下背衬
//         同时淡出（210ms 起手、300ms）→ 播完（EXIT_TOTAL_MS = 520ms）：浮层模式叫
//         父页面关掉浮层（原页面露出来），独立模式照旧跳页。
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

    // 2026-10 用户拍板的动作：「声明从右往左滑，滑到 3/4 时，下一级（剩余部分）从法律
    // 声明下面、一样从右往左运动，反之同理」。动画本身全在 legal.css（is-staging /
    // is-leaving 两类），这里只做两件 CSS 做不了的事：入场播完抹掉地址上的参数；
    // 退场播完再跳转。两个总数与 CSS 里的时长 / 接力点是一组人工对齐的数（见根
    // TODO.md 第 4 条）：
    //   入场：声明 340ms，左栏 255ms 起手 + 400ms → 655ms，留 5ms 余量
    //   退场：声明 280ms，左栏 210ms 起手 + 300ms → 510ms，留 10ms 余量
    var ENTER_TOTAL_MS = 660;
    var EXIT_TOTAL_MS = 520;

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
    // 3/4 时剩下的部分收回 —— 播完再走。两种模式：
    //   · 独立打开（直接输地址 / 书签 / 无 JS 时的原生跳转）：照旧跳页；
    //   · 浮层打开（两个入口以 iframe 嵌进来，is-overlay）：不跳页，叫父页面关掉
    //     浮层，原页面就在下面露出来。唯独「返回的页面不是父页面当前所在页」时
    //     （在首页开浮层却点「返回个人中心」）关掉浮层后父页面还要再跳一次。
    // ⚠️ 两个返回链接是**真链接**（NOTICE 附加条款第 3 条要求入口必须好使），只拦普通
    //    左键：带修饰键（Ctrl / Cmd / Shift / Alt）、非左键、以及系统开了「减少动态
    //    效果」时一律放行，让浏览器按原生方式跳；JS 挂了就是原来的裸跳。
    var inOverlay = (function () {
        try { return window.self !== window.top; } catch (e) { return false; }
    })();

    // 浮层退场后要不要父页面跟着跳：返回链接指向的正是父页面所在页时就地关掉即可
    function overlayNavTarget(href) {
        var parentPath = '';
        try { parentPath = window.parent.location.pathname; } catch (e) { return href; }
        var onIndex = parentPath === '/' || /index\.html$/.test(parentPath);
        var onAccount = /(^|\/)account(\/|$)/.test(parentPath);
        if (/index\.html$/.test(href) && onIndex) return null;
        if (/account/.test(href) && onAccount) return null;
        return href;
    }

    // 退场收尾：浮层模式叫父页面关掉浮层（原页面露出来），必要时父页面再跳一次；
    // 独立模式（直接打开的书签页 / 无 JS 的原生跳转）照旧跳页。
    function finishExit(href) {
        if (inOverlay) {
            var nav = overlayNavTarget(href);
            try {
                if (window.parent && typeof window.parent.__closeLegalOverlay === 'function') {
                    window.parent.__closeLegalOverlay(nav);
                    return;
                }
            } catch (err) { /* 跨域父页：落到下面的跳转 */ }
            if (nav) window.parent.location.href = nav;
            return;
        }
        window.location.href = href;
    }

    var leaving = false;
    var links = document.querySelectorAll('.legal-back');
    for (var i = 0; i < links.length; i++) {
        links[i].addEventListener('click', function (e) {
            if (leaving || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
            var reduced = prefersReducedMotion();
            // ⚠️ 浮层模式下任何情况都不放行原生跳转：iframe 里原生跳会把它自己变成
            //    目标页（浮层里嵌套显示一页首页 / 个人中心），宿主还被蒙在鼓里 ——
            //    减少动态效果时实测踩过这条。减少动态效果 = 不播退场动画，但浮层照关。
            if (reduced && !inOverlay) return;
            e.preventDefault();
            leaving = true;
            var href = this.getAttribute('href');
            if (reduced) {
                finishExit(href); // 不播退场，直接就关
                return;
            }
            document.documentElement.classList.add('is-leaving');
            window.setTimeout(function () {
                finishExit(href);
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
