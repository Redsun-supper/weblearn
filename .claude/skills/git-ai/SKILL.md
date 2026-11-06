---
name: git-ai-project
description: 项目内 Git 协作诊断，遇到报错、冲突或推送被拒时使用
---

运行 `python .gitai/skill/scripts/gitai.py check --json`，按输出的 `actions` 处理。
完整说明与边界见 `.gitai/skill/SKILL.md`。脚本零依赖、随仓库传递，无需密钥。
