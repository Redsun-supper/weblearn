// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
// 法律声明页的脚本，两块互不相干：
//   ① 左栏照片位的「点一下、弹一弹、再跳走」（2026-10 用户拍板方案 B）；
//   ② 出入场过场：动画全在 CSS（legal.css 的 is-staging / is-leaving），这里只做
//      CSS 做不了的三件事：独立打开时播完抹掉地址上的参数；浮层模式下退场播完叫
//      父页面关掉浮层（而不是跳页）；bfcache 恢复时复位退场残留。
//      时长与接力点见 legal.css 的「入场 / 退场」一节与根 TODO.md 第 4 条。
//
// 第 ② 块的整体设计（2026-10 用户拍板、第四轮定稿）：
//   动作：「声明页面中两部分整体从右往左出现，而不是先用背景盖起来然后组件再从右往左
//   走」；退场：「左半边组件先从左往右露出下面将要跳转到的页面，然后声明栏再从左往右
//   退出，然后跳转完成」。浮层模式下原页面垫在下面，入场时被整页盖住、退场时先随左半边
//   滑走而露出来。
//   入场：站名 / 「关于作者」被父页面拦下（普通左键）→ 父页面显示 iframe 并给里面加
//         is-staging → 法律页**整体**（背衬 + 两栏同一个动画）从右往左滑入落位，420ms。
//         独立打开（?from=logo / ?from=about，书签 / 无 JS 原生跳转）时 is-staging 由
//         index.html 头的内联脚本加，同一套动画。
//   退场：点「← 返回首页 / 返回个人中心」→ is-leaving → 第一段左半边（背衬 + 左栏同一
//         像素距离）从左往右滑走、露出下面垫着的页面（340ms），第二段声明栏再从左往右
//         退出（340ms 起手、300ms）→ 播完（EXIT_TOTAL_MS = 660ms）：浮层模式叫父页面
//         关掉浮层（原页面已露出来），独立模式照旧跳页。
//   ⚠️ 两个返回链接是**真链接**，只拦普通左键：带修饰键（Ctrl / Cmd / Shift / Alt）、
//      非左键一律放行；浮层模式下任何情况都不放行原生跳转（iframe 原生跳会把它自己
//      变成目标页）；减少动态效果时不播动画但浮层照关。

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

    // 2026-10 用户拍板的动作（第四轮 + 压力测试修订）：「声明页面中两部分整体从右往左
    // 出现，而不是先用背景盖起来然后组件再从右往左走」；退场：「左半边组件先从左往右
    // 露出下面将要跳转到的页面，然后声明栏再从左往右退出，然后跳转完成」。动画全在
    // legal.css（is-staging / is-leaving 两类），这里只做两件 CSS 做不了的事：入场播完
    // 抹掉地址上的参数；退场播完再跳转。两个总数与 CSS 里的时长是一组人工对齐的数
    // （见根 TODO.md 第 4 条）：
    //   入场：整页（背衬 + 两栏同一个动画）420ms → 留 20ms 余量
    //   退场：第一段左半边 300ms + 第二段声明栏 300ms（CSS 里用 animation-delay 300ms
    //         排好接力）→ 600ms，留 20ms 余量
    var ENTER_TOTAL_MS = 440;
    var EXIT_TOTAL_MS = 620;

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
            if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
            var reduced = prefersReducedMotion();
            // ⚠️ 浮层模式下任何情况都不放行原生跳转：iframe 里原生跳会把它自己变成
            //    目标页（浮层里嵌套显示一页首页 / 个人中心），宿主还被蒙在鼓里 ——
            //    减少动态效果时实测踩过这条。减少动态效果 = 不播退场动画，但浮层照关。
            if (reduced && !inOverlay) return;
            // ⚠️ preventDefault 必须在 leaving 检查**之前**：双击返回链接时第二次点击
            //    leaving 已为 true，若在检查里直接 return，浏览器会放行原生跳转 ——
            //    浮层模式下等于把 iframe 自己变成目标页（压力测试 S2 踩过），独立模式
            //    下也会让退场动画刚起播就跳走、过场被跳过。
            e.preventDefault();
            if (leaving) return; // 防重入：退场进行中再点，忽略
            leaving = true;
            var href = this.getAttribute('href');
            if (reduced) {
                finishExit(href); // 不播退场，直接就关
                return;
            }
            // ⚠️ 入场还没播完就点返回（压力测试 S1 踩过）：is-staging 的 pageSlideIn 还
            //    停在中间帧（both 填充），此刻加 is-leaving，背衬的 animation 简写被
            //    层叠成 backdropHalfOut（源码序后者胜）→ pageSlideIn 取消、元素瞬移回
            //    translateX(0) —— 宿主页被「瞬间盖回去」再开始退场，观感是画面跳一下。
            //    修法：先把入场动画收尾到终帧（finish 后停在落位 0），退场从 0 起手，
            //    两段无缝衔接。
            var entering = document.getAnimations().filter(function (a) {
                return a.animationName === 'pageSlideIn' && a.playState === 'running';
            });
            for (var j = 0; j < entering.length; j++) {
                try { entering[j].finish(); } catch (err) { /* 已结束的不管 */ }
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

    // 供宿主复位：浮层关掉时本文件不会重跑（iframe 的 src 不变、不重新加载），
    // leaving 标记会一直留着 —— 第二次打开浮层后两个返回链接就永久失效
    //（压力测试 S3 实测踩过：连开第二轮后点返回毫无反应、浮层只能靠外部关）。
    // 两个宿主的 openLegalOverlay() 在显示浮层前调用本函数。
    window.__legalReset = function () {
        leaving = false;
        document.documentElement.classList.remove('is-leaving', 'is-staging');
    };
})();
