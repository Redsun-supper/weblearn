# 改动记录

<!-- 四个小节都必须填写，推送时会被 git-ai 校验。变更统计不要手写，由工具计算。 -->

## 改了什么

法律声明页的过场动作整体换了一版：入场从「背衬先淡、两栏再先后滑入」改成**整页一张纸滑入**（420ms，过冲约 5px），退场从「声明先走、左栏跟随」改成**先左半、后声明栏**的两段接力（300 + 300ms）。同时修掉压力测试暴露的三个真缺陷：入场中途点返回会让背衬与布局分离、双击返回漏 `preventDefault`、`leaving` 标记跨浮层会话不复位（第二次打开后返回链接永久失效，两个宿主页现在会调 `__legalReset`）。另外补了内容加载失败的自救错误态、导航「悬停 / 选中」的视觉区分、移动端 `100dvh` 视口修正，并把 PR 审查工具及其 CI 一并入库。

## 为什么

前三轮的转场是「分层动作」：背衬淡入是一件事、两栏滑入是另一件事，观感上是组件在动，不是一页纸在动。第四轮定稿要的是整页覆盖的原页面感——背衬就是页面的一部分，跟着内容一起从右往左滑，原页面在下面一点点被盖住；退场反过来分批露出垫在下面的宿主页。曲线也重调了：上一版 `cubic-bezier(0.3, 1.25, 0.55, 1)` 过冲 29px，整页文档越过落位点再撞回来，不符合「一页纸滑入」的常理。压力测试那三个缺陷都是真实可复现的（浮层模式下会把 iframe 自己变成目标页、或者浮层只能靠外部关），不是理论风险。

## 影响面

- 入口行为：首页与个人中心的 `.legal-frame` 浮层、法律页独立打开的降级路径都走同一套动画；`legal/legal.js` 新增 `window.__legalReset`，由两个宿主的 `openLegalOverlay()` 在每次显示前调用
- 时序常量：`legal/legal.js` 的 `ENTER_TOTAL_MS` = 440、`EXIT_TOTAL_MS` = 620，必须与 `legal/legal.css` 的动画时长成组改（`TODO.md` 的时长表已同步）
- 视觉：导航悬停压到 `#f2f4f7`、选中保留 `#e0e0e0` 加品牌蓝下划线；`::selection` 换成品牌蓝低饱和版；内容加载失败改成带「重新加载」按钮的空状态
- 移动端：`main.css`、`account/account.css`、`admin/admin.css`、`modules/english/english.css` 的 `vh` 全部补了 `dvh` 覆盖，桌面端不受影响
- 新入库：`tools/pr-review/review.py`、`.github/workflows/pr-review.yml`、`.pr-audit.json`；`.gitai/config` 的 `verify` 指向 `python tools/pr-review/review.py --no-ai`
- 文档：`docs/frontend.md` 与 `TODO.md` 已按第四轮口径重写，旧的两段滑入描述作废

## 怎么验证

先跑确定性检查：

```bash
python .gitai/skill/scripts/gitai.py check --json
git ai review feature/legal-transition-v4
```

转场本身要手动看：

1. 首页点页脚「法律声明」→ 整页从右滑入盖住首页，收尾只轻微一顿；按返回 → 左半先滑走、露出首页，声明栏随后退出
2. 浮层里连开两次，第二次点返回仍有效（`leaving` 复位）
3. 入场动画播到一半就点返回，背衬与内容不应分离
4. 手机（或 DevTools 设备模式）上下滚动地址栏，页面底部不应多出空白
