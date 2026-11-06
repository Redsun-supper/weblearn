# 改动记录

<!-- 四个小节都必须填写，推送时会被 git-ai 校验。变更统计不要手写，由工具计算。 -->

## 改了什么

体检脚本给出的文件清单不再丢首字符，冲突标记检查不再把长横线误判成冲突，`check` 的结论重新可用作合并门禁。

## 为什么

接入后第一次跑 `check` 就暴露两处判定错误：`TODO.md` 被报成 `ODO.md`（porcelain 解析按固定列切，而 `run()` 已对整段输出做过 `strip()`，首行的状态位前导空格被吃掉，路径跟着少一个字）；`NOTICE:28/61` 的 `====` 分隔线被当成冲突标记，`ok` 恒为 `false`。两者都会让体检失去参考价值——报错位置对不上，正常仓库永远红。

## 影响面

仅影响 `.gitai/skill/scripts/gitai.py` 的只读诊断输出，不改仓库源码、不参与合并结果。所有 agent 会话都通过这个脚本读体检结论，修好后各处结论一致。

## 怎么验证

```bash
python .gitai/skill/scripts/gitai.py check --json
```

期望：`changed_files` 里出现带点的完整路径（如 `.gitai/skill/scripts/gitai.py`），`conflict_markers` 为 `ok`。
