---
name: git-ai-project
description: 项目内 Git 协作诊断。遇到 git 报错、合并冲突、推送被拒、合并请求检查失败时使用。
---

# git-ai 项目内诊断

遇到任何 git / 合并 / 推送问题，先运行：

```
python .gitai/skill/scripts/gitai.py check --json
```

按输出的 `actions` 逐项处理。`python .gitai/skill/scripts/gitai.py fix` 只做机械修复
（生成改动记录模板、写 FIX.md），不会碰代码。脚本零依赖、随仓库 clone，
**不需要 git-ai 二进制，也不需要任何 API key**。

## 功能做完了怎么交给维护者

Git 是拉取模型，没有推送事件——push 完维护者不会自动知道。所以「我做好了」要写成
**随分支入库的文件**：

```bash
python .gitai/skill/scripts/gitai.py request      # 没装 git-ai 也能用
# 或装了二进制：git ai request
```

它会在 `.gitai/requests/<分支名>.md` 生成摘要（意图 + 动了哪些模块 + 新增/删除符号），
提交并推送。维护者 `git ai inbox` 看到后用 `git ai auto --requests-only` 自动合并，
**摘要会在合并后自动删除**，不需要人工清理。

不想立刻推送时加 `--no-push`；只想看会生成什么用 `--dry-run`。

## 边界

- 权限由服务端保护分支裁决，客户端不预判谁有权限。
- 模型只提案，程序裁决；不要绕过 `review` / `apply` 直接改合并结果。
- 改动记录四段里只有「为什么」必须人写，其余事实由工具生成。
- 装有 `git-ai` 时可用 `git ai review --json` / `fix` / `propose` 做更强的裁决；
  没装也能完成基础诊断。

## 所有 agent 都适用的硬约束（包括维护者自己）

- 动手前先跑 `check --json`，按 `actions` 处理；出错不要自己猜。
- 只改 `.gitai/AREAS` 标为 open 的区域；动 guarded/frozen 必须在改动记录里写明理由。
- 不在 main/master 直接提交或 push；不 `push --force`、不 `reset --hard`、
  不 `git add -A`（会带走别人未提交的改动）、不加 `--no-verify`。
- 不整文件格式化、不改测试让它「通过」、不把格式化/重构/新功能塞进同一提交。
- 冲突时不要随便选一侧：`git ai propose` → 填提案 → `git ai apply`。
- 功能做完、要交给维护者合并时运行 `git ai request`：请求文件随分支入库，
  维护者 fetch 后即可看到，不需要额外通知渠道。

上面的 `check` 覆盖了其中可静态检测的部分（受保护分支改动、格式噪声、
提交粒度、未收敛的 `__theirs`）；命令类禁止只能靠自觉 + 服务端保护分支。

规则见 `AGENTS.md` 与 `.gitai/AREAS`。
