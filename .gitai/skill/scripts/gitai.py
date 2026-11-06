#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
"""git-ai 项目内诊断：零依赖，随仓库 clone 传递。

设计目标：贡献者 clone 下来、即使完全没有 git-ai 二进制，也能用这份脚本
诊断常见问题，并按结构化 actions 修复。重活（符号级合并、AST 裁决）仍交给
可选的 git-ai 二进制，这里只做纯 git + 文件读取能确定的判断。

    python .gitai/skill/scripts/gitai.py doctor
    python .gitai/skill/scripts/gitai.py check --json
    python .gitai/skill/scripts/gitai.py fix
"""

import argparse
import fnmatch
import json
import os
import re
import shutil
import subprocess
import sys
import time

SECTIONS = ["## 改了什么", "## 为什么", "## 影响面", "## 怎么验证"]


def run(args):
    try:
        p = subprocess.run(
            args, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, encoding="utf-8", errors="replace",
        )
        return p.returncode, p.stdout.strip(), p.stderr.strip()
    except FileNotFoundError:
        return 127, "", "command not found: " + args[0]


def git(*a):
    return run(["git"] + list(a))


def root():
    rc, out, _ = git("rev-parse", "--show-toplevel")
    return out if rc == 0 else os.getcwd()


def abspath(rel):
    return os.path.join(root(), rel.replace("/", os.sep))


def read(rel):
    try:
        with open(abspath(rel), "r", encoding="utf-8") as f:
            return f.read()
    except OSError:
        return ""


def write(rel, content):
    path = abspath(rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(content)


def git_config(key):
    rc, out, _ = git("config", "--get", key)
    return out.strip() if rc == 0 else ""


def branch():
    rc, out, _ = git("symbolic-ref", "--short", "HEAD")
    return out if rc == 0 else "HEAD"


def record_rel(br):
    return ".gitai/records/" + br + ".md"


def changed_files(base):
    rc, out, _ = git("diff", "--name-only", base + "...HEAD")
    files = [f.strip() for f in out.split("\n") if f.strip()]
    if rc != 0 or not files:
        rc2, out2, _ = git("status", "--porcelain")
        for line in out2.split("\n"):
            line = line.strip()
            if len(line) > 3:
                files.append(line[3:].strip().strip('"'))
    return sorted(set(files))


def dirty_count():
    rc, out, _ = git("status", "--porcelain")
    if rc != 0:
        return 0
    return len([l for l in out.split("\n") if l.strip()])


def parse_areas():
    rules = []
    for line in read(".gitai/AREAS").split("\n"):
        t = line.strip()
        if not t or t.startswith("#") or t.startswith("!"):
            continue
        parts = t.split()
        if len(parts) < 2 or parts[1] not in ("open", "guarded", "frozen"):
            continue
        rules.append((parts[0], parts[1], " ".join(parts[2:])))
    return rules


def area_of(rules, path):
    for pattern, level, note in rules:
        if fnmatch.fnmatch(path, pattern) or (pattern.endswith("/**") and path.startswith(pattern[:-3] + "/")):
            return pattern, level, note
    return None, "open", ""


def check(base="main"):
    checks = []
    br = branch()
    files = changed_files(base)
    record = read(record_rel(br))

    def add(name, status, detail, actions=None):
        checks.append({"name": name, "status": status, "detail": detail,
                       "actions": actions or []})

    # 1. 冲突标记
    rc, out, _ = git("grep", "-n", "-E", r"^(<<<<<<< |=======|>>>>>>> )", "--", ".")
    if rc == 0 and out:
        lines = [l for l in out.split("\n") if l.strip()][:5]
        add("conflict_markers", "fail", "工作区仍有冲突标记：" + "; ".join(lines),
            ["逐个解决冲突文件，确认两侧改动是否都需要保留", "解决后提交并重新运行 check"])
    else:
        add("conflict_markers", "ok", "无冲突标记")

    # 2. 未提交改动（只提示，不阻断）
    n = dirty_count()
    if n:
        add("uncommitted", "warn", "%d 个文件有未提交改动" % n,
            ["确认这些改动属于本次变更，必要时先提交"])
    else:
        add("uncommitted", "ok", "工作区干净")

    # 3. 改动记录
    protected = ["main", "master"]
    for line in read(".gitai/config").split("\n"):
        t = line.strip()
        if t.startswith("protected") and "=" in t:
            protected = [x.strip() for x in t.split("=", 1)[1].split(",") if x.strip()]
    if not read(".gitai/AREAS") and not read(".gitai/config"):
        add("record", "ok", "仓库未启用 .gitai 规则，跳过")
    elif br in protected or br == "HEAD":
        add("record", "ok", "当前在受保护分支或游离 HEAD，记录应在特性分支生成")
    elif not record:
        add("record", "fail", "缺少 " + record_rel(br),
            ["运行 python .gitai/skill/scripts/gitai.py fix 生成记录模板",
             "填写四段：改了什么 / 为什么 / 影响面 / 怎么验证", "只有「为什么」必须人写"])
    elif "<!-- TODO" in record:
        add("record", "fail", "改动记录还有未填写的 TODO 小节",
            ["打开 " + record_rel(br) + "，补全 TODO 后删除该标记"])
    else:
        missing = [s for s in SECTIONS if s not in record]
        if missing:
            add("record", "fail", "缺少小节：" + "、".join(missing),
                ["补全 " + record_rel(br)])
        else:
            add("record", "ok", "四段齐全")

    # 4. 区域越界
    rules = parse_areas()
    violations = []
    for f in files:
        pattern, level, note = area_of(rules, f)
        if level == "open" or not pattern:
            continue
        root_name = pattern[:-3] if pattern.endswith("/**") else pattern
        if root_name and root_name not in record:
            violations.append("%s 属于 %s 区 %s，需在改动记录里说明" % (f, level, pattern))
    if violations:
        add("areas", "fail", "；".join(violations[:4]),
            ["在 " + record_rel(br) + " 的「影响面」写明涉及区域及理由",
             "若不该动这里，把改动撤回自己负责的区域"])
    else:
        add("areas", "ok", "无未声明的区域越界")

    # 5. 契约变动（只呈现，真正的签名比对由 git-ai 完成）
    if read(".gitai/api.txt"):
        touched = [f for f in files if "/api/" in f or f.endswith("api.txt")]
        if touched:
            add("contract", "warn", "对外契约相关文件有改动：" + "、".join(touched[:4]),
                ["在改动记录里点名被影响的符号与迁移方式",
                 "装有 git-ai 时运行 git ai api diff 做签名级比对"])
        else:
            add("contract", "ok", "对外契约未改动")
    else:
        add("contract", "ok", "无契约快照，跳过")

    # 6. FIX.md
    if read(".gitai/local/FIX.md"):
        add("fix_brief", "warn", "本机存在未处理的修复清单",
            ["先运行 python .gitai/skill/scripts/gitai.py fix", "再按剩余清单逐项处理"])
    else:
        add("fix_brief", "ok", "无待修复清单")

    # 7. 受保护分支上的直接改动（多人/多 agent 最常见的越界）
    if br in protected and n > 0:
        add("protected_branch_edit", "fail",
            "当前在受保护分支 %s 上且有 %d 个未提交改动" % (br, n),
            ["把改动移到特性分支：git switch -c feature-xxx",
             "不要用 git add -A；只 add 自己改动的文件"])
    else:
        add("protected_branch_edit", "ok", "未在受保护分支上直接改动")

    # 8. 格式噪声：整文件重排会把逻辑改动埋掉，也让冲突面积暴涨
    # 必须逐提交比：base...HEAD 是净差异，中间的重排会被后面的提交抵消。
    # 但别为每个提交起两次 git 进程——一次 git log --numstat 就能拿全部分支提交，
    # 两次调用（普通 / 忽略空白）代替原来的 2N 次。
    def commit_stats(text):
        stats = {}
        cur = None
        for line in text.split("\n"):
            if line.startswith("__C__ "):
                cur = line[6:].strip()
                stats[cur] = 0
            elif cur and line.strip():
                parts = line.split()
                if len(parts) >= 3 and parts[0].isdigit() and parts[1].isdigit():
                    stats[cur] += int(parts[0]) + int(parts[1])
        return stats
    log_range = base + "..HEAD"
    _, raw_log, _ = git("log", "--max-count=20", "--format=__C__ %H", "--numstat", log_range)
    _, quiet_log, _ = git("log", "--max-count=20", "--format=__C__ %H", "--numstat",
                          "-w", "--ignore-blank-lines", log_range)
    raw_map, quiet_map = commit_stats(raw_log), commit_stats(quiet_log)
    noise = 0
    for sha, t_lines in raw_map.items():
        if t_lines >= 10:
            noise = max(noise, (t_lines - quiet_map.get(sha, 0)) / t_lines)
    if noise > 0.6:
        add("format_noise", "warn", "约 %d%% 是空白/格式噪声，疑似整文件重排" % (noise * 100),
            ["把格式化拆成独立提交，别和新功能混在一起",
             "确认没有把别人的改动一起重排"])
    else:
        add("format_noise", "ok", "格式噪声在合理范围")

    # 9. 提交粒度：一次改太多文件最容易撞车，也最难局部回滚
    if len(files) > 8:
        add("mixed_commit", "warn", "本分支改动 %d 个文件，建议拆分" % len(files),
            ["按功能拆成多个提交，每个提交只做一件事"])
    else:
        add("mixed_commit", "ok", "改动规模可控")

    # 10. 未收敛的平行实现（合并降级留下的 Foo__theirs）
    _, dup_out, _ = git("grep", "-n", "-E",
                        r"^[[:space:]]*(func|def|class|pub fn|function)[[:space:]]+[A-Za-z_][A-Za-z0-9_]*__theirs",
                        "--", "*.go", "*.py", "*.js", "*.ts", "*.rs")
    if dup_out:
        add("duplicate_symbols", "fail", "存在未收敛的平行实现：" + "; ".join(dup_out.split("\n")[:3]),
            ["用 git ai propose 生成提案、填写收敛结果、git ai apply 裁决",
             "不要随手删掉其中一份，可能丢功能"])
    else:
        add("duplicate_symbols", "ok", "无未收敛的平行实现")

    # 11. 可选增强
    has_bin = shutil.which("git-ai") is not None
    add("gitai_binary", "ok" if has_bin else "info",
        "git-ai 可用" if has_bin else "未安装 git-ai（本脚本仍可独立完成基础诊断）",
        [] if has_bin else ["需要更强裁决时，从维护者处取得 git-ai 放入 PATH"])

    actions = []
    for c in checks:
        if c["status"] in ("fail", "warn"):
            actions.extend(c["actions"])
    ok = not any(c["status"] == "fail" for c in checks)
    return {"ok": ok, "branch": br, "base": base, "changed_files": files,
            "checks": checks, "actions": list(dict.fromkeys(actions))}


# ---------------------------------------------------------------- 合并请求

# 只认各语言的顶层声明，够用来生成摘要，不做语法分析。
SYMBOL_RE = re.compile(
    r"^[ \t]*(?:func[ \t]+(?:\([^)]*\)[ \t]*)?([A-Za-z_]\w*)"
    r"|def[ \t]+([A-Za-z_]\w*)"
    r"|class[ \t]+([A-Za-z_]\w*)"
    r"|(?:pub[ \t]+)?fn[ \t]+([A-Za-z_]\w*)"
    r"|type[ \t]+([A-Za-z_]\w*)"
    r"|(?:export[ \t]+)?(?:const|let|var)[ \t]+([A-Za-z_]\w*))",
    re.M,
)

LANG_EXT = (".go", ".py", ".js", ".jsx", ".ts", ".tsx", ".rs")


def symbols_in(text):
    out = set()
    for m in SYMBOL_RE.finditer(text or ""):
        for g in m.groups():
            if g:
                out.add(g)
                break
    return out


def file_at(rev, rel):
    rc, out, _ = git("show", rev + ":" + rel)
    return out if rc == 0 else ""


def module_of(path):
    parts = path.replace("\\", "/").split("/")
    if len(parts) >= 3:
        return parts[0] + "/" + parts[1]
    if len(parts) == 2:
        return parts[0]
    return ""


def request_rel(br):
    return ".gitai/requests/" + br + ".md"


def record_intent(br):
    text = read(record_rel(br)) or ""
    if not text:
        rc, out, _ = git("show", "HEAD:" + record_rel(br))
        text = out if rc == 0 else ""
    lines = text.split("\n")
    out, inside = [], False
    for line in lines:
        t = line.strip()
        if t.startswith("## "):
            inside = t == "## 改了什么"
            continue
        if inside and not t.startswith("<!--"):
            out.append(line)
    return "\n".join(out).strip()


def do_request(base, push, dry):
    """生成随分支入库的合并请求摘要。没有 git-ai 二进制时也能用。"""
    br = branch()
    if not br:
        return {"ok": False, "error": "无法确定当前分支（游离 HEAD？）"}
    files = [f for f in changed_files(base) if f != request_rel(br)]
    added, removed = set(), set()
    for f in files:
        if not f.endswith(LANG_EXT):
            continue
        before = symbols_in(file_at(base, f))
        after = symbols_in(file_at("HEAD", f))
        added |= (after - before)
        removed |= (before - after)

    modules = {}
    for f in files:
        mod = module_of(f)
        if not mod or f.startswith(".gitai/") or f.startswith(".github/"):
            continue
        modules.setdefault(mod, []).append(f)

    intent = record_intent(br)
    meta = {
        "branch": br,
        "author": "%s <%s>" % (git_config("user.name"), git_config("user.email")),
        "base": base,
        "created": time.strftime("%Y-%m-%d %H:%M"),
        "check_ok": check(base).get("ok", False),
    }

    lines = ["# 合并请求：%s" % br, "", "<!-- git-ai:request"]
    for k in ("branch", "author", "base", "created"):
        lines.append("%s: %s" % (k, meta[k]))
    lines.append("check_ok: %s" % ("true" if meta["check_ok"] else "false"))
    lines += ["-->", "",
              "> 这是贡献者显式发出的「我做好了」信号，随分支入库。",
              "> 维护者 `git ai inbox` / `git ai auto --requests-only` 会据此处理。",
              "", "## 意图", "", intent or "（尚未填写改动记录 %s）" % record_rel(br),
              "", "## 机器事实（由 git 计算，不可手写）", "",
              "- 改动文件（%d）：%s" % (len(files), "、".join(files) or "无"),
              "", "### 动了哪些模块", ""]
    if modules:
        for mod in modules:
            lines.append("- `%s` —— %s" % (mod, "、".join(modules[mod])))
    else:
        lines.append("- 仅仓库根目录下的文件")
    lines += ["", "### 新增", ""]
    lines.append("- 符号：" + "、".join(sorted(added)) if added else "- 无新增的顶层符号")
    lines += ["", "### 删除", ""]
    lines.append("- 符号：%s（**下游需检查是否仍被引用**）" % "、".join(sorted(removed))
                 if removed else "- 无删除的顶层符号")
    lines += ["", "> 合并后本文件会被自动删除，不需要人工清理。", ""]

    rel = request_rel(br)
    if dry:
        return {"ok": True, "dry_run": True, "path": rel, "files": files,
                "added": sorted(added), "removed": sorted(removed)}

    os.makedirs(os.path.dirname(abspath(rel)), exist_ok=True)
    with open(abspath(rel), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines))

    git("add", "--", rel)
    rc, _, err = git("commit", "-q", "-m", "docs: 请求合并 " + br)
    committed = rc == 0
    pushed = False
    if push:
        rc2, _, _ = git("push", "-q", "-u", "origin", br)
        pushed = rc2 == 0
    return {"ok": True, "path": rel, "committed": committed, "pushed": pushed,
            "files": files, "added": sorted(added), "removed": sorted(removed)}


def record_template():
    return """# 改动记录

<!-- 四个小节都要填写；变更统计由工具计算，不要手写。 -->

## 改了什么

<!-- TODO: 一句话说明行为变化，不要写文件清单 -->

## 为什么

<!-- TODO: 关联的需求、问题编号或动机（必须人写） -->

## 影响面

<!-- TODO: 波及的功能、对外接口、数据结构、配置项 -->

## 怎么验证

<!-- TODO: 复现步骤或测试命令 -->
"""


def do_fix(base, dry):
    result = check(base)
    br = result["branch"]
    rel = record_rel(br)
    done = []

    if not read(rel):
        if not dry:
            write(rel, record_template())
        done.append("生成 " + rel + "（四段仍待填写，只有「为什么」必须人写）")

    after = check(base)
    remaining = [c for c in after["checks"] if c["status"] == "fail"]
    if remaining and not dry:
        lines = ["# 待修复：" + after["base"] + " ← " + br, "",
                 "> 由项目内脚本生成。逐项修复后重跑 check；全部通过时本文件会自动删除。", ""]
        for i, c in enumerate(remaining, 1):
            lines += ["## %d. %s" % (i, c["name"]), "", "**现状**", "", "- " + c["detail"], ""]
            if c["actions"]:
                lines += ["**要做**", ""]
                lines += ["%d. %s" % (j, a) for j, a in enumerate(c["actions"], 1)]
                lines += [""]
        write(".gitai/local/FIX.md", "\n".join(lines))
    elif not remaining and not dry and read(".gitai/local/FIX.md"):
        try:
            os.remove(abspath(".gitai/local/FIX.md"))
            done.append("全部通过，已清除 FIX.md")
        except OSError:
            pass

    return {"ok": after["ok"], "applied": done,
            "remaining": [c["name"] for c in remaining], "dry_run": dry}


def doctor():
    rc, gv, _ = git("--version")
    return {"python": sys.version.split()[0], "git": gv if rc == 0 else "未找到",
            "repo": root(), "branch": branch(),
            "gitai_binary": shutil.which("git-ai") or "未安装（可选）",
            "areas": bool(read(".gitai/AREAS")),
            "config": bool(read(".gitai/config")),
            "record": bool(read(record_rel(branch()))),
            "project_skill": bool(read(".gitai/skill/SKILL.md"))}


def main():
    ap = argparse.ArgumentParser(description="git-ai 项目内诊断（零依赖）")
    sub = ap.add_subparsers(dest="cmd")
    sub.add_parser("doctor", help="查看环境与规则是否就绪")
    c = sub.add_parser("check", help="诊断当前分支")
    c.add_argument("--base", default="main")
    c.add_argument("--json", action="store_true")
    f = sub.add_parser("fix", help="自动做机械修复")
    f.add_argument("--base", default="main")
    f.add_argument("--dry-run", action="store_true")
    r = sub.add_parser("request", help="生成合并请求摘要并提交（可替代 git-ai request）")
    r.add_argument("--base", default="main")
    r.add_argument("--no-push", action="store_true")
    r.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()

    if a.cmd == "doctor":
        out = doctor()
    elif a.cmd == "fix":
        out = do_fix(a.base, a.dry_run)
    elif a.cmd == "request":
        out = do_request(a.base, not a.no_push, a.dry_run)
    else:
        out = check(getattr(a, "base", "main"))

    if getattr(a, "json", False) or a.cmd in ("doctor", "fix", "request"):
        print(json.dumps(out, ensure_ascii=False, indent=2))
    else:
        print("诊断 %s ← %s：%s" % (out["base"], out["branch"], "通过" if out["ok"] else "未通过"))
        for c in out["checks"]:
            mark = {"ok": "✓", "warn": "!", "fail": "✗", "info": "-"}.get(c["status"], "?")
            print("  %s %-16s %s" % (mark, c["name"], c["detail"]))
        if out["actions"]:
            print("\n要做：")
            for i, x in enumerate(out["actions"], 1):
                print("  %d. %s" % (i, x))
    return 0 if out.get("ok", True) else 1


if __name__ == "__main__":
    sys.exit(main())
