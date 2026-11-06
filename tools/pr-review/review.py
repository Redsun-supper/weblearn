#!/usr/bin/env python3
"""PR 审查器：静态检查 + 测试 + 接口契约 + AI 分析，产出一份可读的 Markdown 报告。

设计原则（重要，别改）：
  1. 只读。本脚本不修改任何源码，只在 pr-review/ 下写报告。
  2. 两层分离。第一层是确定性检查（编译、测试、契约、规则、影响面），没有 AI 也能跑，
     结论可复现；第二层才调大模型，负责「错误解释」和「简化建议」，属于参考意见。
  3. AI 失败不影响结论。没有密钥、网络不通、返回不是 JSON，都只降级为一条提示。
  4. 退出码只由阻断项决定，方便直接当合并卡点用。
  5. 项目无关。语言目录靠探测，不硬编码 gateway/ 或 kernel/。

用法：
    python tools/pr-review/review.py                      # 审 origin/main...HEAD
    python tools/pr-review/review.py --head origin/feat-x # 审别人推到远程的分支
    python tools/pr-review/review.py --no-ai              # 只跑确定性检查（CI 用这个）
    python tools/pr-review/review.py --no-tests           # 跳过测试，快速看规则
    python tools/pr-review/review.py --ai-samples 2       # 同一 diff 问两次，只留都出现的结论
"""

from __future__ import annotations

import argparse
import ast
import datetime
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from html.parser import HTMLParser
from pathlib import Path

# ---------------------------------------------------------------- 基础类型

SEVERITY_LABEL = {"block": "阻断", "warn": "警告", "info": "提示"}
SEVERITY_ORDER = {"block": 0, "warn": 1, "info": 2}

# 这些路径不参与审查：二进制、依赖、构建产物、历史备份、临时目录
SKIP_DIRS = {
    ".git", ".venv", "venv", "node_modules", "target", "dist", "build",
    "__pycache__", "备份", "backups", "plans", "snapshot", ".tmp-test",
    ".workbuddy-ai", ".workbuddy", ".pytest_cache", ".cache", ".idea", ".vscode",
}
SKIP_SUFFIX = {
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".pdf", ".zip",
    ".exe", ".dll", ".so", ".dylib", ".o", ".rlib", ".rmeta", ".lock",
    ".woff", ".woff2", ".ttf", ".mp4", ".db", ".sqlite", ".db-shm", ".db-wal",
}
CODE_SUFFIX = {".go", ".py", ".rs", ".js", ".css", ".html", ".toml", ".yml", ".yaml"}

# 第三方代码不参与引用统计，否则一个符号在 codemirror.min.js 里出现一次就被算成「共享」
VENDOR_PARTS = {"vendor", "node_modules", "dist", "build", "backups", "备份", ".tmp-test"}


@dataclass
class Finding:
    """一条审查结论。source 区分它是机器算出来的还是模型说的。"""

    severity: str
    title: str
    detail: str = ""
    file: str = ""
    line: int = 0
    suggestion: str = ""
    source: str = "static"

    def sort_key(self) -> tuple:
        return (SEVERITY_ORDER.get(self.severity, 9), self.file, self.line)


@dataclass
class Profile:
    """仓库形态。一个仓库可以同时有多种语言，每门语言可以有多个模块。

    全部用列表：monorepo 里常见「Go 主服务 + Rust 账号服务 + 两个前端包」，
    只取一个会漏掉其余模块的编译和测试。
    """

    go_modules: list[Path] = field(default_factory=list)
    rust_crates: list[Path] = field(default_factory=list)
    python_roots: list[Path] = field(default_factory=list)
    js_packages: list[Path] = field(default_factory=list)
    brief: str = ""

    def stack(self) -> str:
        """给模型看的技术栈摘要，例如「Go ×2、Rust、JS」。"""
        parts = []
        for label, items in (
            ("Go", self.go_modules), ("Rust", self.rust_crates),
            ("Python", self.python_roots), ("JS", self.js_packages),
        ):
            if items:
                parts.append(label if len(items) == 1 else f"{label} ×{len(items)}")
        return "、".join(parts) or "未探测到已知语言的模块"


@dataclass
class Config:
    """项目级配置，来自仓库根的 `.pr-audit.json`。没有这个文件就全走默认值。

    存在的意义：项目之间的差异（哪些目录别扫、要不要检查接口契约、
    自己的编码红线）应该写在项目里，而不是每次改脚本。
    """

    ignore: list[str] = field(default_factory=list)      # 额外忽略的路径前缀
    rules: list[dict] = field(default_factory=list)      # 额外规则，见 check_rules
    contract_mode: str = "auto"                          # auto / on / off
    contract_prefix: str = ""                            # 空 = 从路由表自动推断
    client_suffixes: tuple[str, ...] = (".js", ".mjs", ".ts", ".html", ".vue")
    pr_lines: int = 800
    file_lines: int = 900
    function_lines: int = 120
    big_function_lines: int = 150
    run_tests: bool = True
    # 新建文件的署名头 / 许可证头：很多项目要求每个新源码文件带固定几行
    headers_required: list[str] = field(default_factory=list)
    header_suffixes: tuple[str, ...] = ()
    header_skip: list[str] = field(default_factory=list)


def load_config(root: Path) -> Config:
    """读 `.pr-audit.json`。坏文件不报错，退回默认值 —— 审计工具不该因为配置写错就跑不起来。"""
    cfg = Config()
    path = root / ".pr-audit.json"
    if not path.is_file():
        return cfg
    try:
        data = json.loads(path.read_text(encoding="utf-8", errors="replace"))
    except (OSError, json.JSONDecodeError):
        return cfg
    if not isinstance(data, dict):
        return cfg

    if isinstance(data.get("ignore"), list):
        cfg.ignore = [str(item) for item in data["ignore"]]
    if isinstance(data.get("rules"), list):
        cfg.rules = [item for item in data["rules"] if isinstance(item, dict)]

    contract = data.get("contract")
    if isinstance(contract, dict):
        mode = str(contract.get("mode", "auto")).lower()
        if mode in {"auto", "on", "off"}:
            cfg.contract_mode = mode
        if isinstance(contract.get("prefix"), str):
            cfg.contract_prefix = contract["prefix"]
        if isinstance(contract.get("clients"), list):
            cfg.client_suffixes = tuple(
                s if str(s).startswith(".") else "." + str(s)
                for s in contract["clients"]
            )

    limits = data.get("limits")
    if isinstance(limits, dict):
        for key, attr in (
            ("pr_lines", "pr_lines"), ("file_lines", "file_lines"),
            ("function_lines", "function_lines"), ("big_function_lines", "big_function_lines"),
        ):
            value = limits.get(key)
            if isinstance(value, int) and value > 0:
                setattr(cfg, attr, value)

    if isinstance(data.get("tests"), bool):
        cfg.run_tests = data["tests"]

    headers = data.get("headers")
    if isinstance(headers, dict):
        if isinstance(headers.get("required"), list):
            cfg.headers_required = [str(item) for item in headers["required"]]
        if isinstance(headers.get("suffixes"), list):
            cfg.header_suffixes = tuple(
                s if str(s).startswith(".") else "." + str(s)
                for s in headers["suffixes"]
            )
        if isinstance(headers.get("skip"), list):
            cfg.header_skip = [str(item) for item in headers["skip"]]
    return cfg


@dataclass
class Context:
    """一次审查的全部输入，后面各阶段共享。"""

    root: Path
    base: str
    head: str
    base_sha: str = ""
    head_sha: str = ""
    range_desc: str = ""
    files: list[str] = field(default_factory=list)
    added_files: list[str] = field(default_factory=list)
    numstat: dict[str, tuple[int, int]] = field(default_factory=dict)
    patch: str = ""
    commits: int = 0
    # 审计指纹：diff 内容的摘要。PR 一旦有新提交，指纹就变，上一次审计自动作废。
    fingerprint: str = ""
    profile: Profile = field(default_factory=Profile)
    config: Config = field(default_factory=Config)
    sources: list[Path] | None = None
    findings: list[Finding] = field(default_factory=list)
    checks: list[tuple[str, str, str]] = field(default_factory=list)  # (名称, 状态, 说明)
    ai_summary: str = ""
    ai_error: str = ""


# ---------------------------------------------------------------- 子进程

def run(cmd: list[str], cwd: Path, timeout: int = 180) -> tuple[int, str]:
    """跑一条命令，统一 utf-8 解码。Windows 上默认 GBK，不解码会直接抛异常。"""
    try:
        proc = subprocess.run(
            cmd, cwd=str(cwd), capture_output=True, text=True,
            encoding="utf-8", errors="replace", timeout=timeout,
        )
        return proc.returncode, (proc.stdout or "") + (proc.stderr or "")
    except FileNotFoundError:
        return 127, f"找不到命令：{cmd[0]}"
    except subprocess.TimeoutExpired:
        return 124, f"命令超时（{timeout}s）：{' '.join(cmd)}"


def git(ctx_args: list[str], cwd: Path) -> str:
    code, out = run(["git"] + ctx_args, cwd)
    if code != 0:
        raise SystemExit(f"git 执行失败：{' '.join(ctx_args)}\n{out}")
    return out


def tail(text: str, limit: int) -> str:
    """编译和测试的报错都在末尾，截断要留尾部。"""
    text = text.strip()
    if len(text) <= limit:
        return text
    return "…（前略）\n" + text[-limit:]


# ---------------------------------------------------------------- 项目探测

# 语言登记表。加一门语言 = 这里加一行 + 写对应的 build/test 函数。
# 不在这里的语言不会被检查，但也不会报错，只是不出现在「验证覆盖」里。
LANGUAGES: dict[str, dict] = {
    "go":     {"name": "Go",     "suffixes": (".go",),                 "marker": "go.mod"},
    "rust":   {"name": "Rust",   "suffixes": (".rs",),                 "marker": "Cargo.toml"},
    "python": {"name": "Python", "suffixes": (".py",),                 "marker": "pyproject.toml"},
    "js":     {"name": "JS",     "suffixes": (".js", ".mjs", ".cjs"),  "marker": "package.json"},
}

# 往下扫多少层找模块。再深基本是依赖目录、示例或测试夹具，不是主模块。
MAX_SCAN_DEPTH = 4


def find_modules(root: Path, marker: str) -> list[Path]:
    """全仓找含某个标记文件的目录，返回**全部**命中，支持 monorepo。

    旧实现只取「路径最短的一个」，一个仓库里有 Go 主服务 + 两个前端包时，
    另外两个包就永远不被检查。现在全部返回，逐个跑。
    """
    found: set[Path] = set()
    try:
        for path in root.rglob(marker):
            if any(part in SKIP_DIRS for part in path.parts):
                continue
            try:
                rel = path.parent.relative_to(root)
            except ValueError:
                continue
            if len(rel.parts) > MAX_SCAN_DEPTH:
                continue
            found.add(path.parent)
    except OSError:
        return []
    return sorted(found, key=lambda p: (len(p.parts), str(p)))


def read_brief(root: Path) -> str:
    """项目画像：从仓库自带的说明文档里抽，让模型知道自己在审什么项目。

    以前这里写死了「Go 网关 + Python 内核 + Rust 身份服务」，换个仓库就是错的画像，
    模型会按错误的前提给建议。改成读仓库自己的文档。
    """
    for name in ("CLAUDE.md", "AGENTS.md", "README.md"):
        path = root / name
        if not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        picked = [
            line.strip() for line in text.splitlines()
            if line.strip() and not line.strip().startswith(("![", "<img", "[!["))
        ][:30]
        if picked:
            return "\n".join(picked)[:1500]
    return ""


def find_python_roots(root: Path) -> list[Path]:
    """Python 没有统一的模块标记，退一步：有构建文件或任意 .py 就把仓库根当根。"""
    for marker in ("pyproject.toml", "setup.py", "pytest.ini", "tox.ini", "setup.cfg"):
        if (root / marker).is_file():
            return [root]
    try:
        for path in root.rglob("*.py"):
            if any(part in SKIP_DIRS for part in path.parts):
                continue
            return [root]
    except OSError:
        pass
    return []


def detect_profile(root: Path) -> Profile:
    return Profile(
        go_modules=find_modules(root, "go.mod"),
        rust_crates=find_modules(root, "Cargo.toml"),
        python_roots=find_python_roots(root),
        js_packages=find_modules(root, "package.json"),
        brief=read_brief(root),
    )


def changed_languages(ctx: Context) -> list[str]:
    """这次改动涉及哪几门语言，按登记表顺序返回。"""
    suffixes = {Path(f).suffix.lower() for f in ctx.files}
    return [key for key, spec in LANGUAGES.items() if suffixes & set(spec["suffixes"])]


def find_go() -> str | None:
    """先查 PATH，再查环境变量，最后兜底常见的便携安装位置。"""
    if shutil.which("go"):
        return "go"
    env = os.environ.get("GO_BIN")
    if env and Path(env).exists():
        return env
    for guess in (
        Path.home() / "go-portable/go/bin/go.exe",
        Path(r"C:\Program Files\Go\bin\go.exe"),
        Path(r"C:\Program Files (x86)\Go\bin\go.exe"),
    ):
        if guess.exists():
            return str(guess)
    return None


# ---------------------------------------------------------------- 收集改动

def is_interesting(path: str) -> bool:
    parts = Path(path).parts
    if any(p in SKIP_DIRS for p in parts):
        return False
    suffix = Path(path).suffix.lower()
    return suffix not in SKIP_SUFFIX


def matches_ignore(path: str, patterns: list[str]) -> bool:
    """配置里的忽略项按「路径前缀」匹配，比 glob 好写也更好预测。"""
    normalized = path.replace("\\", "/")
    for pattern in patterns:
        clean = pattern.replace("\\", "/").rstrip("/")
        if clean and (normalized == clean or normalized.startswith(clean + "/")):
            return True
    return False


def collect(args) -> Context:
    root = Path(git(["rev-parse", "--show-toplevel"], Path.cwd()).strip())
    config = load_config(root)

    def keep(path: str) -> bool:
        return is_interesting(path) and not matches_ignore(path, config.ignore)

    base_sha = git(["rev-parse", "--verify", args.base], root).strip()
    head_sha = git(["rev-parse", "--verify", args.head], root).strip()
    # 三点比较：以共同祖先为基准，只看这一支带来的改动，别人的提交不算进来
    merge_base = git(["merge-base", base_sha, head_sha], root).strip()
    rng = f"{merge_base}...{head_sha}"

    files = [f for f in git(["diff", "--name-only", rng], root).splitlines() if f.strip()]
    files = [f for f in files if keep(f)]

    # 新增文件单独列一份：署名头 / 许可证头这类要求只针对新建文件，
    # 老文件补头是另一件事，不该混进每个 PR 都报。
    added = [
        f for f in git(["diff", "--diff-filter=A", "--name-only", rng], root).splitlines()
        if f.strip() and keep(f)
    ]

    numstat: dict[str, tuple[int, int]] = {}
    for line in git(["diff", "--numstat", rng], root).splitlines():
        cols = line.split("\t")
        if len(cols) == 3 and keep(cols[2]):
            try:
                numstat[cols[2]] = (int(cols[0]), int(cols[1]))
            except ValueError:
                numstat[cols[2]] = (0, 0)  # 二进制，diff 里是 "-"

    commits = int(git(["rev-list", "--count", f"{merge_base}..{head_sha}"], root).strip() or 0)
    patch = git(["diff", "-U3", "--no-color", rng, "--"] + files, root) if files else ""

    # 指纹只算 diff 内容，不算提交信息：同样的改动换个提交信息重提，指纹不变，审计仍然有效。
    fingerprint = hashlib.sha256(patch.encode("utf-8")).hexdigest()[:12]

    return Context(
        root=root, base=args.base, head=args.head,
        base_sha=base_sha, head_sha=head_sha,
        range_desc=f"{args.base}...{args.head}（{commits} 个提交，共同祖先 {merge_base[:8]}）",
        files=files, numstat=numstat, patch=patch, commits=commits,
        added_files=added,
        fingerprint=fingerprint, profile=detect_profile(root), config=config,
    )


def added_lines(patch: str) -> list[tuple[str, int, str]]:
    """从 patch 里取出所有新增行，返回 (文件, 行号, 内容)。"""
    out: list[tuple[str, int, str]] = []
    current, lineno = "", 0
    for raw in patch.splitlines():
        if raw.startswith("+++ b/"):
            current = raw[6:]
        elif raw.startswith("@@"):
            match = re.search(r"\+(\d+)", raw)
            lineno = int(match.group(1)) if match else 0
        elif raw.startswith("+") and not raw.startswith("+++"):
            out.append((current, lineno, raw[1:]))
            lineno += 1
        elif raw.startswith("-") and not raw.startswith("---"):
            continue
        else:
            lineno += 1
    return out


# 只认顶格声明（第 0 列）。缩进的都是函数内局部变量，改了不会影响别人。
SYMBOL_RE = re.compile(
    r"^(?:export\s+)?(?:async\s+)?"
    r"(?:func\s+(?:\([^)]*\)\s*)?(\w+)|function\s+(\w+)|def\s+(\w+)"
    r"|(?:const|let|var)\s+(\w+)\s*=|class\s+(\w+))"
)

# 这些名字太泛，改了也不代表影响面。留着会天天误报。
GENERIC_SYMBOLS = {
    "list", "data", "value", "values", "result", "items", "options", "index",
    "text", "name", "state", "config", "error", "node", "event", "props", "args",
    "main", "init", "setup", "start", "stop", "run", "load", "save", "parse",
    "render", "update", "create", "build", "reset", "apply", "handle", "emit",
}


def changed_symbols(ctx: Context) -> set[str]:
    """diff 里「改动前就有、改动后还在」的顶格符号 = 被修改的既有符号。"""
    removed, added = set(), set()
    for raw in ctx.patch.splitlines():
        if raw.startswith("---") or raw.startswith("+++"):
            continue
        if raw[:1] not in "+-":
            continue
        match = SYMBOL_RE.match(raw[1:])
        if not match:
            continue
        name = next((g for g in match.groups() if g), None)
        if not name or len(name) < 4 or name in GENERIC_SYMBOLS:
            continue
        (added if raw.startswith("+") else removed).add(name)
    return removed & added


def get_sources(ctx: Context) -> list[Path]:
    """仓库里的源码文件清单。影响面分析和上下文装配共用，算一次就缓存。"""
    if ctx.sources is not None:
        return ctx.sources
    out: list[Path] = []
    try:
        for path in ctx.root.rglob("*"):
            if not path.is_file():
                continue
            if path.suffix not in {".go", ".py", ".js", ".rs"}:
                continue
            if any(part in SKIP_DIRS or part in VENDOR_PARTS for part in path.parts):
                continue
            try:
                if path.stat().st_size > 400_000:
                    continue
            except OSError:
                continue
            out.append(path)
    except OSError:
        pass
    ctx.sources = out
    return out


# ---------------------------------------------------------------- 第一层：确定性检查

def check_compile(ctx: Context) -> None:
    """语法与编译。这一步失败就是硬伤，直接阻断。

    按「这次改动涉及哪几门语言」调度，没涉及的语言不会白跑。
    缺工具链就跳过并在报告里写明原因，不会因为「没装 cargo」就让整份报告看起来全绿。
    """
    for key in changed_languages(ctx):
        checker = BUILD_CHECKS.get(key)
        if checker:
            checker(ctx)


def module_label(root: Path, module: Path, total: int) -> str:
    """多个模块时在检查名后面带上路径，否则报告里会出现四行一模一样的 go test。"""
    if total <= 1:
        return ""
    try:
        rel = module.relative_to(root).as_posix()
    except ValueError:
        return ""
    return "" if rel == "." else f" @{rel}"


def check_python_syntax(ctx: Context) -> None:
    files = [f for f in ctx.files if f.endswith(".py")]
    if not files:
        return
    code, out = run([sys.executable, "-m", "compileall", "-q"] + files, ctx.root)
    ctx.checks.append(("Python 语法", "通过" if code == 0 else "失败", out.strip()[:800]))
    if code != 0:
        ctx.findings.append(Finding("block", "Python 文件存在语法错误", out.strip()[:1200]))


def check_js_syntax(ctx: Context) -> None:
    files = [f for f in ctx.files if f.endswith(".js")]
    if not files:
        return
    node = shutil.which("node")
    if not node:
        ctx.checks.append(("JS 语法", "跳过", "未找到 node"))
        return

    failed = 0
    skipped = 0
    for f in files:
        code, out = run([node, "--check", f], ctx.root, timeout=60)
        if code == 0:
            continue
        # node --check 按 CommonJS 解析，遇到 import 会误报，这种不算错
        if "Cannot use import statement" in out or "Unexpected token 'export'" in out:
            skipped += 1
            continue
        failed += 1
        ctx.findings.append(Finding("block", f"{f} 存在语法错误", out.strip()[:800], file=f))
    if failed:
        ctx.checks.append(("JS 语法", "失败", f"{failed} 个文件有语法错误"))
    elif skipped:
        ctx.checks.append(("JS 语法", "跳过", f"{skipped} 个 ESM 文件，node --check 不适用"))
    else:
        ctx.checks.append(("JS 语法", "通过", f"{len(files)} 个文件"))


def check_go_build(ctx: Context) -> None:
    if not any(f.endswith(".go") for f in ctx.files):
        return
    go_bin = find_go()
    if not go_bin:
        ctx.checks.append(("Go 编译", "跳过", "未找到 go 可执行文件"))
        return
    modules = ctx.profile.go_modules
    if not modules:
        ctx.checks.append(("Go 编译", "跳过", "仓库里没找到 go.mod"))
        return

    for module in modules:
        label = module_label(ctx.root, module, len(modules))
        code, out = run([go_bin, "vet", "./..."], module, timeout=300)
        ctx.checks.append((f"go vet{label}", "通过" if code == 0 else "失败", tail(out, 800)))
        if code != 0:
            ctx.findings.append(Finding("block", f"go vet 未通过{label}", tail(out, 1500)))

        code, out = run([go_bin, "build", "./..."], module, timeout=300)
        ctx.checks.append((f"go build{label}", "通过" if code == 0 else "失败", tail(out, 800)))
        if code != 0:
            ctx.findings.append(Finding("block", f"go build 失败{label}", tail(out, 1500)))


# 这些报错是环境问题（没网、缺依赖、文件锁），不是代码问题。
# 判成「失败」会让整份报告不可信，判成「跳过」并写明原因才是诚实的。
ENV_PROBLEM_MARKERS = (
    "failed to download", "failed to get", "network failure", "connection refused",
    "could not resolve host", "blocking waiting for file lock", "no matching package",
    "failed to load source", "registry index", "dial tcp", "temporary failure in name resolution",
    "enotfound", "etimedout", "offline mode", "cannot find package", "no such host",
)


def is_env_problem(out: str) -> bool:
    low = out.lower()
    return any(marker in low for marker in ENV_PROBLEM_MARKERS)


def check_rust_build(ctx: Context) -> None:
    if not any(f.endswith(".rs") for f in ctx.files):
        return
    cargo = shutil.which("cargo")
    if not cargo:
        ctx.checks.append(("Rust 编译", "跳过", "未找到 cargo"))
        return
    crates = ctx.profile.rust_crates
    if not crates:
        ctx.checks.append(("Rust 编译", "跳过", "仓库里没找到 Cargo.toml"))
        return

    for crate in crates:
        label = module_label(ctx.root, crate, len(crates))
        code, out = run(
            [cargo, "check", "--all-targets", "--quiet", "--message-format", "short"],
            crate, timeout=900,
        )
        if code == 0:
            ctx.checks.append((f"cargo check{label}", "通过", "含测试目标"))
        elif is_env_problem(out):
            ctx.checks.append((f"cargo check{label}", "跳过", tail(out, 300)))
        else:
            ctx.checks.append((f"cargo check{label}", "失败", tail(out, 800)))
            ctx.findings.append(Finding("block", f"cargo check 失败{label}", tail(out, 2000)))


# 语言 → 检查函数。加一门语言时在这里登记，check_compile / check_tests 自动带上。
BUILD_CHECKS = {
    "python": check_python_syntax,
    "js": check_js_syntax,
    "go": check_go_build,
    "rust": check_rust_build,
}


TEST_SUGGESTION = (
    "测试是唯一能证明「这个改动没弄坏别的功能」的东西。红了不要合，"
    "也不要让 AI 去改测试来让它变绿。"
)


def check_tests(ctx: Context) -> None:
    """跑测试。这是整套检查里唯一能证明「功能没被弄坏」的一步。

    编译只能证明语法对，测试才能证明行为对。没有测试的自动合并等于赌博。
    按改动涉及的语言调度，有测试就跑，没测试就说清楚「为什么没跑」。
    """
    if not ctx.config.run_tests:
        ctx.checks.append(("测试", "跳过", "配置里关掉了 tests"))
        return
    for key in changed_languages(ctx):
        runner = TEST_CHECKS.get(key)
        if runner:
            runner(ctx)


def test_go(ctx: Context) -> None:
    if not any(f.endswith(".go") for f in ctx.files):
        return
    modules = ctx.profile.go_modules
    if not modules:
        ctx.checks.append(("go test", "跳过", "仓库里没找到 go.mod"))
        return
    go_bin = find_go()
    if not go_bin:
        ctx.checks.append(("go test", "跳过", "未找到 go 可执行文件"))
        return

    for module in modules:
        label = module_label(ctx.root, module, len(modules))
        code, out = run([go_bin, "test", "./..."], module, timeout=600)
        ctx.checks.append((f"go test{label}", "通过" if code == 0 else "失败", tail(out, 1200)))
        if code != 0:
            ctx.findings.append(Finding("block", f"go test 未通过{label}", tail(out, 2500),
                                        suggestion=TEST_SUGGESTION))


def python_test_dirs(root: Path) -> list[Path]:
    """找出直接含测试文件的目录。

    unittest 的 discover 只递归进「包」（有 __init__.py 的目录），
    app/test_x.py 这种没有 __init__.py 的布局从仓库根发现不到任何用例，
    会误报成「测试失败」。所以按目录逐个发现。
    """
    dirs: set[Path] = set()
    try:
        for path in root.rglob("*.py"):
            if any(part in SKIP_DIRS or part in VENDOR_PARTS for part in path.parts):
                continue
            if path.name.startswith("test_") or path.name.endswith("_test.py"):
                dirs.add(path.parent)
    except OSError:
        pass
    return sorted(dirs)


def test_python(ctx: Context) -> None:
    if not any(f.endswith(".py") for f in ctx.files):
        return
    roots = ctx.profile.python_roots
    if not roots:
        ctx.checks.append(("pytest", "跳过", "仓库里没找到 Python 项目"))
        return
    root = roots[0]
    dirs = python_test_dirs(root)
    if not dirs:
        ctx.checks.append(("pytest", "跳过", "没有 test_*.py / *_test.py"))
        return

    has_pytest = run([sys.executable, "-m", "pytest", "--version"], root, timeout=60)[0] == 0
    if has_pytest:
        name = "pytest"
        targets = [d.relative_to(root).as_posix() if d != root else "." for d in dirs]
        code, out = run([sys.executable, "-m", "pytest", "-q"] + targets, root, timeout=600)
        empty = code == 5  # pytest 的「没收集到用例」专用退出码
    else:
        name = "unittest"
        codes: list[int] = []
        outs: list[str] = []
        for target in dirs:
            c, o = run([sys.executable, "-m", "unittest", "discover",
                        "-s", str(target), "-t", str(target), "-q"], root, timeout=600)
            codes.append(c)
            outs.append(o)
        code = 0 if all(c == 0 for c in codes) else 1
        out = "\n".join(outs)
        empty = "ran 0 tests" in out.lower() or "no tests ran" in out.lower()

    if empty:
        ctx.checks.append((name, "跳过", "没收集到测试用例"))
        return

    ctx.checks.append((name, "通过" if code == 0 else "失败", tail(out, 1200)))
    if code != 0:
        ctx.findings.append(Finding("block", f"{name} 未通过", tail(out, 2500),
                                    suggestion=TEST_SUGGESTION))


def test_rust(ctx: Context) -> None:
    if not any(f.endswith(".rs") for f in ctx.files):
        return
    cargo = shutil.which("cargo")
    if not cargo:
        ctx.checks.append(("cargo test", "跳过", "未找到 cargo"))
        return
    crates = ctx.profile.rust_crates
    if not crates:
        ctx.checks.append(("cargo test", "跳过", "仓库里没找到 Cargo.toml"))
        return

    for crate in crates:
        label = module_label(ctx.root, crate, len(crates))
        code, out = run([cargo, "test", "--quiet"], crate, timeout=900)
        if code == 0:
            ctx.checks.append((f"cargo test{label}", "通过", tail(out, 600)))
        elif is_env_problem(out):
            ctx.checks.append((f"cargo test{label}", "跳过", tail(out, 300)))
        else:
            ctx.checks.append((f"cargo test{label}", "失败", tail(out, 800)))
            ctx.findings.append(Finding("block", f"cargo test 未通过{label}", tail(out, 2500),
                                        suggestion=TEST_SUGGESTION))


def test_js(ctx: Context) -> None:
    if not any(f.endswith(".js") for f in ctx.files):
        return
    packages = ctx.profile.js_packages
    if not packages:
        ctx.checks.append(("npm test", "跳过", "仓库里没有 package.json"))
        return

    npm = shutil.which("npm")
    for pkg_dir in packages:
        label = module_label(ctx.root, pkg_dir, len(packages))
        try:
            pkg = json.loads((pkg_dir / "package.json").read_text(encoding="utf-8", errors="replace"))
        except (OSError, json.JSONDecodeError):
            ctx.checks.append((f"npm test{label}", "跳过", "package.json 读不出来或不是合法 JSON"))
            continue
        if not (pkg.get("scripts") or {}).get("test"):
            ctx.checks.append((f"npm test{label}", "跳过", "没有 test 脚本"))
            continue
        if not npm:
            ctx.checks.append((f"npm test{label}", "跳过", "未找到 npm"))
            continue
        if not (pkg_dir / "node_modules").is_dir():
            ctx.checks.append((f"npm test{label}", "跳过", "node_modules 未安装"))
            continue

        # Windows 上 npm 是 npm.cmd，CreateProcess 不能直接执行，要过一层 cmd /c
        cmd = ["npm", "test", "--silent"]
        if os.name == "nt" and npm.lower().endswith((".cmd", ".bat")):
            cmd = ["cmd", "/c", npm] + cmd[1:]

        code, out = run(cmd, pkg_dir, timeout=600)
        ctx.checks.append((f"npm test{label}", "通过" if code == 0 else "失败", tail(out, 1200)))
        if code != 0:
            ctx.findings.append(Finding("block", f"npm test 未通过{label}", tail(out, 2500),
                                        suggestion=TEST_SUGGESTION))


TEST_CHECKS = {
    "go": test_go,
    "python": test_python,
    "rust": test_rust,
    "js": test_js,
}


# ---------------------------------------------------------------- 接口契约

GIN_ENGINE_RE = re.compile(r"(\w+)\s*:?=\s*gin\.(?:Default|New)\(\)")
GROUP_RE = re.compile(r"(\w+)\s*:?=\s*(\w+)\.Group\(\s*\"([^\"]*)\"")
ROUTE_RE = re.compile(r"(\w+)\.(GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS)\(\s*\"([^\"]*)\"")
RUST_ROUTE_RE = re.compile(r"\.route\(\s*\"([^\"]+)\"")
RUST_NEST_RE = re.compile(r"\.nest\(\s*\"([^\"]+)\"")
ROUTE_GREP = (
    r"\.(GET|POST|PUT|DELETE|PATCH)\(|\.Group\("
    r"|gin\.(Default|New)\(|\.route\(|\.nest\("
    r"|@[A-Za-z_]+\.(route|get|post|put|delete|patch)\("
    r"|\.(get|post|put|delete|patch)\(\s*[\"']/"
)
GENERIC_DOT_RE = re.compile(
    r"""\.(?:get|post|put|delete|patch|head|options)\(\s*["'](/[^"']*)["']""", re.I)
GENERIC_DECORATOR_RE = re.compile(
    r"""@\w+\.(?:route|get|post|put|delete|patch)\(\s*["'](/[^"']*)["']""", re.I)
CLIENT_PATH_RE = re.compile(r"""["'](/[^"'\s]{2,})["']""")


def normalize_path(path: str) -> str:
    path = re.sub(r"/{2,}", "/", "/" + path.strip())
    if len(path) > 1:
        path = path.rstrip("/") or "/"
    return path


def parse_go_routes(text: str) -> set[str]:
    """把 gin 的 Group 前缀拼回完整路径。

    只扫文本，不做类型分析。gin 的写法很固定，正则够用；解析不出来就跳过，
    宁可少报也不误报。
    """
    groups: dict[str, str] = {name: "" for name in GIN_ENGINE_RE.findall(text)}
    groups.setdefault("r", "")
    pending = GROUP_RE.findall(text)
    for _ in range(6):  # 支持多级嵌套，收敛即停
        before = dict(groups)
        for var, parent, prefix in pending:
            if var in groups or parent not in groups:
                continue
            groups[var] = normalize_path(groups[parent] + "/" + prefix.strip("/"))
        if groups == before:
            break

    out: set[str] = set()
    for var, _method, path in ROUTE_RE.findall(text):
        if var not in groups:
            continue
        out.add(normalize_path(groups[var] + "/" + path.strip("/")))
    return out


def parse_rust_routes(text: str) -> set[str]:
    """axum 的 .route() 只取字面量。nest 前缀只在唯一时套用，避免猜错。"""
    nests = RUST_NEST_RE.findall(text)
    base = nests[0] if len(nests) == 1 else ""
    return {
        normalize_path(base + "/" + path.strip("/"))
        for path in RUST_ROUTE_RE.findall(text)
    }


def parse_generic_routes(text: str) -> set[str]:
    """Express / Nest / Flask / FastAPI 这类写法：`app.get('/x')`、`@app.route('/x')`。

    不做框架识别，只抓「方法名 + 路径字面量」这个共同形态。
    抓多了顶多多几条路由，抓少了才会漏掉真正的破坏性变更。
    """
    found = set(GENERIC_DOT_RE.findall(text)) | set(GENERIC_DECORATOR_RE.findall(text))
    return {normalize_path(path) for path in found if path}


def routes_at(root: Path, rev: str) -> set[str]:
    """用 git grep 一次性取出某个版本里所有路由声明，避免逐个文件 git show。"""
    code, out = run(
        ["git", "grep", "-n", "-E", ROUTE_GREP, rev, "--", "*.go", "*.rs", "*.js", "*.ts", "*.py"],
        root, timeout=120,
    )
    if code not in (0, 1):  # 1 = 没有匹配，不是错误
        return set()

    by_file: dict[str, list[str]] = {}
    for line in out.splitlines():
        parts = line.split(":", 3)
        if len(parts) < 4:
            continue
        _, path, _, content = parts
        by_file.setdefault(path, []).append(content)

    routes: set[str] = set()
    for path, lines in by_file.items():
        text = "\n".join(lines)
        if path.endswith(".go"):
            routes |= parse_go_routes(text)
        elif path.endswith(".rs"):
            routes |= parse_rust_routes(text)
        else:
            routes |= parse_generic_routes(text)
    return routes


def infer_route_prefix(routes: set[str]) -> str:
    """从路由表推断公共前缀（例如 /api）。

    项目之间的前缀各不相同，写死 /api 就只对一类项目有效。
    取出现最多的第一段，且要占到全部路由的一半以上才算「公共前缀」，
    否则返回空字符串表示不设限。
    """
    counter: dict[str, int] = {}
    for route in routes:
        segs = [s for s in route.strip("/").split("/") if s]
        if len(segs) >= 2:
            counter[segs[0]] = counter.get(segs[0], 0) + 1
    if not counter:
        return ""
    name, count = max(counter.items(), key=lambda kv: kv[1])
    return f"/{name}" if count * 2 >= len(routes) else ""


def client_paths(root: Path, suffixes: tuple[str, ...], prefix: str) -> dict[str, list[str]]:
    """调用方代码里写死的路径字面量，用来判断路由删了会不会有人 404。"""
    found: dict[str, list[str]] = {}
    try:
        for path in root.rglob("*"):
            if path.suffix.lower() not in suffixes:
                continue
            if any(part in SKIP_DIRS or part in VENDOR_PARTS for part in path.parts):
                continue
            try:
                if path.stat().st_size > 400_000:
                    continue
                text = path.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            for literal in CLIENT_PATH_RE.findall(text):
                # 去掉查询串：'/api/stats?days=' 与 '/api/stats' 是同一个接口
                clean = literal.split("?")[0].split("#")[0]
                if clean.count("/") < 2:  # '/api'、'/api/auth' 这类是代理前缀，不是调用
                    continue
                if prefix and not (clean == prefix or clean.startswith(prefix + "/")):
                    continue
                found.setdefault(normalize_path(clean), []).append(
                    path.relative_to(root).as_posix())
    except OSError:
        pass
    return found


def path_hit(call: str, route: str) -> bool:
    """前端调用路径是否落在这条路由上。

    两种真实形态都要认：
      - 路由带参数：/api/words/:id，前端写 '/api/words/' + id → 抹平后相等；
      - 前端自己拼路径：'/api/admin/users/' + id + '/logout-all' → 调用是路由的前缀。
    按段比较而不是按字符串前缀，避免 /api/health 命中 /api/healthz 这种误报。
    """
    flat = re.sub(r"/(?::[^/]+|\{[^/}]+\})", "", route)
    if call == flat:
        return True

    call_segs = [s for s in call.strip("/").split("/") if s]
    route_segs = [s for s in flat.strip("/").split("/") if s]
    # 较短的一侧至少要三段（/api/x/y），否则代理前缀会命中整组接口
    if len(call_segs) >= 3 and route_segs[:len(call_segs)] == call_segs:
        return True
    if len(route_segs) >= 3 and call_segs[:len(route_segs)] == route_segs:
        return True
    return False


def check_contract(ctx: Context) -> None:
    """接口契约：这条改动有没有把调用方还在用的路由删掉。

    「一个功能把另一个功能弄坏」最常见的形态就是接口改名或删除，
    而 git 只会告诉你「这里删了一行」，不会告诉你「有三处调用方会 404」。
    这一步是确定性的，不依赖模型。

    只在能解析出路由时才生效（Go gin / Rust axum / Express / Flask / FastAPI），
    解析不出来就跳过，不会对没有 HTTP 接口的项目瞎报。
    """
    if ctx.config.contract_mode == "off":
        return
    if not ctx.base_sha or not ctx.head_sha:
        return

    base_routes = routes_at(ctx.root, ctx.base_sha)
    head_routes = routes_at(ctx.root, ctx.head_sha)
    if not base_routes:
        ctx.checks.append(("接口契约", "跳过", "基线里没解析到路由声明"))
        return

    removed = base_routes - head_routes
    added = head_routes - base_routes
    prefix = ctx.config.contract_prefix or infer_route_prefix(head_routes)
    note = (
        f"基线 {len(base_routes)} 条 → 当前 {len(head_routes)} 条；"
        f"删除 {len(removed)} 条，新增 {len(added)} 条"
    )
    if prefix:
        note += f"；调用方前缀 {prefix}"
    ctx.checks.append(("接口契约", "通过" if not removed else "失败", note))
    if not removed:
        return

    clients = client_paths(ctx.root, ctx.config.client_suffixes, prefix)

    # 按「调用方文件集合」归组：删掉一整组路由时，同一个调用点会同时命中
    # /x 和 /x/:id，逐条报会把一次破坏性变更刷成好几条，信号被稀释。
    groups: dict[tuple[str, ...], list[str]] = {}
    untouched: list[str] = []
    for route in sorted(removed):
        callers = [path for path in clients if path_hit(path, route)]
        if not callers:
            untouched.append(route)
            continue
        files = tuple(sorted({f for path in callers for f in clients[path]}))
        groups.setdefault(files, []).append(route)

    for files, routes in groups.items():
        shown = "、".join(routes[:6])
        if len(routes) > 6:
            shown += f" 等 {len(routes)} 条"
        ctx.findings.append(Finding(
            "block", f"路由被删除，调用方仍在使用：{shown}",
            f"调用方文件：{'、'.join(files[:5])}。合并后这些请求会 404。",
            suggestion="要么保留这些路由，要么在同一个 PR 里把调用方一起改掉。",
        ))

    if untouched:
        shown = "、".join(untouched[:8])
        if len(untouched) > 8:
            shown += f" 等 {len(untouched)} 条"
        ctx.findings.append(Finding(
            "warn", f"路由被删除：{shown}",
            "仓库里没有直接引用，但可能有外部调用方（监控、脚本、第三方客户端）。",
            suggestion="确认确实没人用了再合。",
        ))


# ---------------------------------------------------------------- HTML / CSS

# HTML 里不需要闭合的标签
VOID_TAGS = {
    "area", "base", "br", "col", "embed", "hr", "img", "input",
    "link", "meta", "param", "source", "track", "wbr",
}
# 浏览器容忍不闭合的标签，报出来只是噪音
OPTIONAL_CLOSE = {"p", "li", "td", "th", "tr", "option", "dt", "dd", "thead", "tbody"}


class HtmlAudit(HTMLParser):
    """用标准库走一遍标签栈，找未闭合标签、重复 id、以及指向不存在文件的资源引用。

    不引第三方校验器：这三类问题都能确定性判定，且都会在浏览器里直接表现为
    错位或 404，属于「机器说了算」的部分。
    """

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.stack: list[tuple[str, int]] = []
        self.errors: list[str] = []
        self.ids: list[str] = []
        self.assets: list[tuple[str, str]] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        data = {k: (v or "") for k, v in attrs}
        if data.get("id"):
            self.ids.append(data["id"])
        for key in ("src", "href"):
            value = data.get(key, "").strip()
            if value:
                self.assets.append((value, tag))
        if tag not in VOID_TAGS:
            self.stack.append((tag, self.getpos()[0]))

    def handle_endtag(self, tag: str) -> None:
        if tag in VOID_TAGS:
            return
        line = self.getpos()[0]
        for index in range(len(self.stack) - 1, -1, -1):
            if self.stack[index][0] != tag:
                continue
            for open_tag, open_line in self.stack[index + 1:]:
                if open_tag not in OPTIONAL_CLOSE:
                    self.errors.append(f"第 {open_line} 行的 <{open_tag}> 没有闭合")
            del self.stack[index:]
            return
        self.errors.append(f"第 {line} 行的 </{tag}> 找不到对应的开标签")


def resolve_assets(root: Path, html_file: Path, ref: str) -> list[Path]:
    """把 HTML 里的相对引用解析成候选磁盘路径。

    两种真实形态都要认：
      - 普通多页站点：相对 HTML 文件所在目录解析；
      - 片段式前端（页面被 fetch 进来插到首页）：相对仓库根解析。
    两者都试，只要有一个存在就算有效 —— 误报的代价高于漏报。
    外部链接和锚点返回空列表（不检查）。
    """
    ref = ref.split("?")[0].split("#")[0].strip()
    if not ref:
        return []
    if ref.startswith(("http://", "https://", "//", "data:", "mailto:", "tel:", "javascript:")):
        return []
    if ref.startswith("/"):
        return [root / ref.lstrip("/")]
    return [html_file.parent / ref, root / ref]


def check_markup(ctx: Context) -> None:
    """HTML 结构与资源引用。"""
    html_files = [f for f in ctx.files if f.endswith((".html", ".htm"))]
    if not html_files:
        return

    problems = 0
    for rel in html_files:
        full = ctx.root / rel
        if not full.exists():
            continue
        try:
            text = full.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue

        audit = HtmlAudit()
        try:
            audit.feed(text)
            audit.close()
        except Exception:  # 解析器本身出错不该让审计失败
            ctx.checks.append((f"HTML 结构 {rel}", "跳过", "解析器无法处理该文件"))
            continue

        for open_tag, open_line in audit.stack:
            if open_tag not in OPTIONAL_CLOSE:
                audit.errors.append(f"第 {open_line} 行的 <{open_tag}> 到文件结尾都没闭合")

        for message in audit.errors[:10]:
            problems += 1
            ctx.findings.append(Finding("block", "HTML 标签结构错误", message, file=rel))

        seen: dict[str, int] = {}
        for item in audit.ids:
            seen[item] = seen.get(item, 0) + 1
        for item, count in sorted(seen.items()):
            if count > 1:
                ctx.findings.append(Finding(
                    "warn", f"重复的 id：{item}",
                    f"{rel} 里出现了 {count} 次。getElementById 只会取到第一个，"
                    "标签页、表单联动这类逻辑会静默错位。",
                    file=rel,
                ))

        for asset, tag in audit.assets:
            candidates = resolve_assets(ctx.root, full, asset)
            if not candidates or any(path.exists() for path in candidates):
                continue
            problems += 1
            ctx.findings.append(Finding(
                "block", f"引用了不存在的本地文件：{asset}",
                f"<{tag}> 指向 {asset}，按文件所在目录和仓库根都找不到，上线后必然是 404。",
                file=rel,
            ))

    ctx.checks.append(("HTML 结构", "通过" if problems == 0 else "失败",
                       f"检查 {len(html_files)} 个文件（标签闭合 / 重复 id / 资源引用）"))


def strip_css_noise(text: str) -> str:
    """去掉注释和字符串，避免其中的花括号和 var() 干扰判断。"""
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    text = re.sub(r'"(?:[^"\\\n]|\\.)*"', '""', text)
    text = re.sub(r"'(?:[^'\\\n]|\\.)*'", "''", text)
    return text


def collect_css_vars(root: Path) -> set[str]:
    """全仓收集 CSS 变量定义。变量的定义和用法常常不在同一个文件里。"""
    names: set[str] = set()
    try:
        for path in root.rglob("*.css"):
            if any(part in SKIP_DIRS or part in VENDOR_PARTS for part in path.parts):
                continue
            try:
                if path.stat().st_size > 1_000_000:
                    continue
                text = strip_css_noise(path.read_text(encoding="utf-8", errors="replace"))
            except OSError:
                continue
            names.update(re.findall(r"(--[\w-]+)\s*:", text))
    except OSError:
        pass
    return names


def check_styles(ctx: Context) -> None:
    """CSS：花括号是否配平、有没有用到未定义的变量。

    这两类问题在浏览器里都是静默的 —— 少一个 } 会让后面整段样式失效，
    拼错变量名会让颜色退回默认值，肉眼很难发现。
    """
    css_files = [f for f in ctx.files if f.endswith(".css")]
    if not css_files:
        return

    defined = collect_css_vars(ctx.root)
    problems = 0
    for rel in css_files:
        full = ctx.root / rel
        if not full.exists():
            continue
        try:
            text = full.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        clean = strip_css_noise(text)

        opens, closes = clean.count("{"), clean.count("}")
        if opens != closes:
            problems += 1
            ctx.findings.append(Finding(
                "block", "CSS 花括号不匹配",
                f"{opens} 个 {{ 对 {closes} 个 }}。少一个 }} 会让它之后的整段样式失效。",
                file=rel,
            ))

        used = set(re.findall(r"var\(\s*(--[\w-]+)", clean))
        for name in sorted(used - defined)[:10]:
            ctx.findings.append(Finding(
                "warn", f"用了未定义的 CSS 变量：{name}",
                f"{rel} 引用了 {name}，但全仓没有任何地方定义它，浏览器会退回默认值。",
                file=rel,
            ))

    ctx.checks.append(("CSS 结构", "通过" if problems == 0 else "失败",
                       f"检查 {len(css_files)} 个文件（括号配平 / 变量定义）"))


def check_headers(ctx: Context) -> None:
    """新建文件的署名头 / 许可证头。

    只查**新增文件**：给老文件补头是另一件事，混进来会让每个 PR 都报一遍。
    在前 4 行里找即可，给 shebang 留出位置。
    """
    required = ctx.config.headers_required
    if not required:
        return

    suffixes = ctx.config.header_suffixes or tuple(CODE_SUFFIX)
    targets = [
        rel for rel in ctx.added_files
        if Path(rel).suffix.lower() in suffixes
        and not matches_ignore(rel, ctx.config.header_skip)
    ]
    if not targets:
        return

    missing = 0
    for rel in targets:
        full = ctx.root / rel
        if not full.exists():
            continue
        try:
            head = "\n".join(
                full.read_text(encoding="utf-8", errors="replace").splitlines()[:4]
            )
        except OSError:
            continue
        absent = [item for item in required if item not in head]
        if not absent:
            continue
        missing += 1
        ctx.findings.append(Finding(
            "block", f"新建文件缺少署名头：{rel}",
            "缺少：" + "、".join(f"`{item}`" for item in absent),
            file=rel, line=1,
            suggestion="按项目约定补在文件开头；带 shebang 的脚本把 shebang 留在第一行，"
                       "署名头紧随其后。",
        ))

    ctx.checks.append(("署名头", "通过" if missing == 0 else "失败",
                       f"检查 {len(targets)} 个新增文件"))


# 每条规则：正则、级别、标题、说明、分组。只看新增行，避免老代码的老毛病一直报。
# 分组用于去重：同一行同时命中「硬编码密钥」和「sk- 明文」时只报一次，别让一条问题刷两条。
RULES: list[tuple[re.Pattern, str, str, str, str]] = [
    (
        re.compile(r"""(?i)\b(api[_-]?key|secret|access[_-]?key|token|password|passwd)\b\s*[:=]\s*["'][^"'\s]{8,}["']"""),
        "block", "疑似硬编码密钥",
        "密钥必须从环境变量读。写进代码会随 git 历史永久泄露，删掉也还在。", "secret",
    ),
    (re.compile(r"\bsk-[A-Za-z0-9]{16,}"), "block", "疑似 API Key 明文", "同上，改成读环境变量。", "secret"),
    (re.compile(r"\b(eval|exec)\s*\("), "block", "使用了 eval/exec", "动态执行字符串等于把注入面完全打开，需要明确理由。", ""),
    (re.compile(r"^\s*debugger\s*;?\s*$"), "block", "残留 debugger", "提交前删掉。", ""),
    (re.compile(r"\bbreakpoint\(\)"), "block", "残留 breakpoint()", "提交前删掉。", ""),
    (re.compile(r"InsecureSkipVerify\s*:\s*true"), "warn", "关闭了 TLS 证书校验", "这会让中间人攻击失效，只能用于本地调试。", "tls"),
    (re.compile(r"verify\s*=\s*False"), "warn", "关闭了 TLS 证书校验", "httpx/requests 的 verify=False 同上。", "tls"),
    (re.compile(r"shell\s*=\s*True"), "warn", "使用了 shell=True", "命令拼接会引入注入面，尽量用参数列表。", ""),
    (re.compile(r"except\s*:"), "warn", "裸 except", "会连 KeyboardInterrupt 一起吞掉，至少写成 except Exception as e。", ""),
    (re.compile(r"\.innerHTML\s*="), "warn", "给 innerHTML 赋值", "CSP 拦不住这个，用户输入进去就是 XSS。优先用 textContent 或 createElement。", ""),
    (re.compile(r"\bconsole\.log\("), "info", "console.log 残留", "调试输出上线会污染控制台，也会泄露内部状态。", ""),
    (re.compile(r"\bprint\("), "info", "print 残留", "服务端建议用 logging，print 无法分级也无法关闭。", ""),
    (re.compile(r"\b(TODO|FIXME|XXX)\b"), "info", "新增待办标记", "确认不是未完成的功能被当成完成品提交。", ""),
]


def build_rules(ctx: Context):
    """内置规则 + 项目配置里的规则。

    配置里可以给规则限定后缀，例如「这条只对 .js 生效」。
    这样项目自己的红线（ES5 不许用 let/const、必须带许可证头）写在项目里，
    不用改脚本 —— 技能对所有项目共用同一份引擎。
    """
    rules: list[tuple[re.Pattern, str, str, str, str, tuple[str, ...]]] = [
        (*item, ()) for item in RULES
    ]
    errors: list[str] = []
    for index, item in enumerate(ctx.config.rules, 1):
        raw = str(item.get("pattern", "")).strip()
        if not raw:
            errors.append(f"第 {index} 条缺 pattern 字段")
            continue
        try:
            compiled = re.compile(raw)
        except re.error as exc:
            errors.append(f"第 {index} 条正则不合法：{exc}")
            continue
        severity = str(item.get("severity", "warn")).lower()
        if severity not in SEVERITY_ORDER:
            severity = "warn"
        suffixes = tuple(
            s if str(s).startswith(".") else "." + str(s)
            for s in (item.get("suffixes") or [])
        )
        rules.append((
            compiled, severity,
            str(item.get("title", "项目规则命中")),
            str(item.get("detail", "")), "",
            suffixes,
        ))
    return rules, errors


def check_rules(ctx: Context) -> None:
    rules, errors = build_rules(ctx)
    for message in errors:
        ctx.findings.append(Finding("warn", "配置里的规则没生效", message))

    hits: dict[tuple[str, str, str, str], list[int]] = {}
    seen: set[tuple[str, int, str]] = set()

    for path, lineno, text in added_lines(ctx.patch):
        if not path.endswith(tuple(CODE_SUFFIX)):
            continue
        for pattern, severity, title, detail, group, suffixes in rules:
            if suffixes and not path.endswith(suffixes):
                continue
            if not pattern.search(text):
                continue
            if group:
                if (path, lineno, group) in seen:
                    continue
                seen.add((path, lineno, group))
            hits.setdefault((severity, title, detail, path), []).append(lineno)

    for (severity, title, detail, path), lines in hits.items():
        shown = "、".join(str(n) for n in lines[:6])
        more = f" 等 {len(lines)} 处" if len(lines) > 6 else ""
        ctx.findings.append(
            Finding(severity, title, detail, file=path, line=lines[0],
                    suggestion=f"位置：第 {shown} 行{more}")
        )


def check_size(ctx: Context) -> None:
    """规模指标。改后的文件超过阈值就提醒，这是后续维护成本的先行指标。

    注意：只在这次 PR 给该文件加了 80 行以上时才报。否则 app.js 这种既有 5595 行的文件
    会在每个 PR 里重复刷屏，报告一旦变成狼来了就没人看了。
    阈值来自 `.pr-audit.json` 的 limits，不同项目可以不一样。
    """
    limit = ctx.config.pr_lines
    total_add = sum(a for a, _ in ctx.numstat.values())
    total_del = sum(d for _, d in ctx.numstat.values())
    if total_add + total_del > limit:
        ctx.findings.append(Finding(
            "warn", "PR 规模偏大", f"共 +{total_add} / -{total_del} 行。",
            suggestion=f"超过 {limit} 行的 PR 评审质量会明显下降，"
                       "建议拆成多个只做一件事的 PR。",
        ))

    file_limit = ctx.config.file_lines
    for path in ctx.files:
        full = ctx.root / path
        add = ctx.numstat.get(path, (0, 0))[0]
        if not full.exists() or add < 80:
            continue
        if full.suffix not in {".go", ".py", ".js", ".ts", ".css", ".rs"}:
            continue
        try:
            lines = full.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError:
            continue
        if len(lines) > file_limit:
            ctx.findings.append(Finding(
                "warn", f"文件已超过 {file_limit} 行",
                f"当前 {len(lines)} 行，这次又加了 {add} 行。",
                file=path,
                suggestion="单文件过大后每次改动都会和别人撞车，建议按职责拆分。",
            ))


def check_function_length(ctx: Context) -> None:
    """函数长度。Python 用 ast 精确算，Go/JS 用「行首闭合括号」这个可靠近似。

    只报**这次 PR 碰过的函数**。既有超长函数不是本次改动的责任，报出来只会稀释信号。
    """
    changed: dict[str, set[int]] = {}
    for path, lineno, _ in added_lines(ctx.patch):
        changed.setdefault(path, set()).add(lineno)

    py_limit = ctx.config.function_lines
    js_limit = ctx.config.big_function_lines

    def touched(path: str, start: int, end: int) -> bool:
        lines = changed.get(path)
        if not lines:
            return False
        return any(start <= n <= end for n in lines)

    for path in ctx.files:
        full = ctx.root / path
        if not full.exists():
            continue
        try:
            text = full.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue

        if path.endswith(".py"):
            try:
                tree = ast.parse(text)
            except SyntaxError:
                continue
            for node in ast.walk(tree):
                if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    continue
                start, end = node.lineno, node.end_lineno or node.lineno
                length = end - start + 1
                if length > py_limit and touched(path, start, end):
                    ctx.findings.append(Finding(
                        "warn", f"函数 {node.name} 有 {length} 行", f"超过 {py_limit} 行。",
                        file=path, line=start,
                        suggestion="拆成几个只做一件事的小函数，单测才好写。",
                    ))
        elif path.endswith((".go", ".js")):
            pattern = re.compile(r"^(?:func\s+(?:\([^)]*\)\s*)?(\w+)|function\s+(\w+))\s*\(")
            lines = text.splitlines()
            for idx, line in enumerate(lines):
                match = pattern.match(line)
                if not match:
                    continue
                name = match.group(1) or match.group(2)
                end = idx
                for j in range(idx + 1, len(lines)):
                    if lines[j] == "}":
                        end = j
                        break
                length = end - idx + 1
                if length > js_limit and touched(path, idx + 1, end + 1):
                    ctx.findings.append(Finding(
                        "warn", f"函数 {name} 有 {length} 行", f"超过 {js_limit} 行。",
                        file=path, line=idx + 1,
                        suggestion="拆成几个只做一件事的小函数。",
                    ))


def check_shared_symbols(ctx: Context) -> None:
    """影响面分析：找出「被修改的已有符号」，统计它在仓库里被引用多少次。

    这是整套检查里最能防「一个功能弄坏另一个功能」的一条 —— 语义冲突在合并时是静默的，
    但「你改了被 12 处调用的 sortModels」这件事可以被算出来。
    """
    modified = changed_symbols(ctx)
    if not modified:
        return

    sources = get_sources(ctx)
    counts: dict[str, list[str]] = {}
    for symbol in sorted(modified):
        word = re.compile(rf"\b{re.escape(symbol)}\b")
        hits = []
        for src in sources:
            try:
                if word.search(src.read_text(encoding="utf-8", errors="replace")):
                    hits.append(src.relative_to(ctx.root).as_posix())
            except OSError:
                continue
        if len(hits) >= 3:
            counts[symbol] = hits

    for symbol, hits in sorted(counts.items(), key=lambda kv: -len(kv[1])):
        shown = "、".join(hits[:6]) + ("…" if len(hits) > 6 else "")
        ctx.findings.append(Finding(
            "warn", f"修改了共享符号 {symbol}",
            f"该符号被 {len(hits)} 个文件引用：{shown}",
            suggestion="合并前逐个确认调用方是否还能正常工作；"
                       "如果只是自己功能要用，优先在自己的文件里加一层，不要改公共实现。",
        ))


# ---------------------------------------------------------------- 第二层：AI 分析

AI_SYSTEM_TMPL = """你是一名严格的代码评审员，为一次合并请求做合并前审计。

## 项目技术栈（自动探测，可能有遗漏）
{stack}

## 项目背景（摘自仓库自带的说明文档）
{brief}

## 你的职责
找出这次改动里**真实的错误**和**可以简化的地方**，不是夸奖代码。

判断标准：
- 错误：会导致功能失效、数据丢失、安全缺口、并发问题、错误处理缺失。必须指出具体触发条件。
- 简化：重复逻辑、可以合并的分支、多余的抽象、过度防御、命名与职责不符。
- 不确定的不要编。宁可少说，也不要说「可能有问题」这种没有信息量的话。

## 关于上下文
除 diff 之外，你还会收到「相关上下文」：被改动符号的定义与调用方片段。
判断「这个改动会不会弄坏调用方」时以这些片段为准，不要凭空推测没给你的代码。
如果上下文不足以判断，就在 detail 里说明缺什么，不要猜。

只输出一个 JSON 对象，不要任何解释性文字，不要 markdown 代码围栏。格式：
{{
  "summary": "两到三句话，说清这次改动做了什么、最大的风险在哪",
  "findings": [
    {{"severity": "block|warn|info", "file": "相对路径", "line": 12,
     "title": "一句话结论", "detail": "为什么会出问题、什么条件下触发",
     "suggestion": "具体怎么改"}}
  ],
  "simplifications": [
    {{"file": "相对路径", "line": 12, "before": "现在的写法", "after": "简化后的写法",
     "why": "为什么更简单，且行为不变"}}
  ]
}}
simplifications 最多 5 条，只给真正值得改的。没有就给空数组。"""

# 单次请求的 diff 上限。超过就按文件切块，避免「截断后模型只看到半个 PR」。
AI_CHUNK_LIMIT = 40_000


def load_llm_config(ctx: Context) -> dict[str, str]:
    """模型配置。环境变量优先，其次仓库里的 .env。

    以前写死读 kernel/.env，换个仓库就永远找不到配置、AI 层静默降级。
    现在按「环境变量 > 根 .env > .env.local > kernel/.env」的顺序找。
    """
    cfg = {"base_url": "", "api_key": "", "model": ""}
    mapping = {"LLM_BASE_URL": "base_url", "LLM_API_KEY": "api_key", "LLM_MODEL": "model"}

    for env_key, slot in mapping.items():
        value = os.environ.get(env_key, "").strip()
        if value:
            cfg[slot] = value

    for env_file in (ctx.root / ".env", ctx.root / ".env.local", ctx.root / "kernel" / ".env"):
        if not env_file.is_file():
            continue
        try:
            lines = env_file.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError:
            continue
        for line in lines:
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            key, _, value = line.partition("=")
            key = key.strip()
            if key in mapping and not cfg[mapping[key]]:
                cfg[mapping[key]] = value.strip().strip('"').strip("'")
    return cfg


def decl_line(text: str, symbol: str) -> int:
    """找符号的声明行。找不到就返回 0。"""
    for idx, line in enumerate(text.splitlines(), 1):
        match = SYMBOL_RE.match(line)
        if match and symbol in [g for g in match.groups() if g]:
            return idx
    return 0


def first_hit(text: str, symbol: str) -> int:
    word = re.compile(rf"\b{re.escape(symbol)}\b")
    for idx, line in enumerate(text.splitlines(), 1):
        if word.search(line):
            return idx
    return 1


def slice_around(text: str, line_no: int, pad: int) -> str:
    lines = text.splitlines()
    start = max(0, line_no - 1 - pad)
    end = min(len(lines), line_no + pad)
    return "\n".join(lines[start:end])


def build_context_snippets(ctx: Context, max_symbols: int = 5, max_chars: int = 16_000) -> str:
    """给模型补上它看不到的东西：被改符号的定义和调用方。

    这是提升 AI 结论质量最有效的一步。只喂 diff 时，模型看不到调用方，
    判断「改了会不会弄坏别人」只能靠猜，猜出来的结论没法用。
    """
    symbols = changed_symbols(ctx)
    if not symbols:
        return ""

    sources = get_sources(ctx)
    ranked: list[tuple[int, str, list[tuple[Path, str]]]] = []
    for symbol in symbols:
        word = re.compile(rf"\b{re.escape(symbol)}\b")
        hits: list[tuple[Path, str]] = []
        for src in sources:
            try:
                text = src.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            if word.search(text):
                hits.append((src, text))
        if len(hits) >= 2:
            ranked.append((len(hits), symbol, hits))

    if not ranked:
        return ""
    ranked.sort(key=lambda item: -item[0])

    blocks: list[str] = []
    budget = max_chars
    for count, symbol, hits in ranked[:max_symbols]:
        block = [f"### 符号 `{symbol}`，被 {count} 个文件引用"]

        decl = None
        for src, text in hits:
            line_no = decl_line(text, symbol)
            if line_no:
                decl = (src, text, line_no)
                break
        if decl:
            src, text, line_no = decl
            block.append(f"定义处 `{src.relative_to(ctx.root).as_posix()}:{line_no}`：")
            block.append("```")
            block.append(slice_around(text, line_no, 10))
            block.append("```")

        shown = 0
        for src, text in hits:
            if decl and src == decl[0]:
                continue
            line_no = first_hit(text, symbol)
            block.append(f"调用方 `{src.relative_to(ctx.root).as_posix()}:{line_no}`：")
            block.append("```")
            block.append(slice_around(text, line_no, 6))
            block.append("```")
            shown += 1
            if shown >= 2:
                break

        rendered = "\n".join(block)
        if len(rendered) > budget:
            break
        blocks.append(rendered)
        budget -= len(rendered)

    return "\n\n".join(blocks)


def split_patch(patch: str, limit: int = AI_CHUNK_LIMIT) -> list[str]:
    """按文件把 diff 切块。整段截断会让大 PR 的后面一半等于没审。"""
    if len(patch) <= limit:
        return [patch]

    blocks: list[str] = []
    buf: list[str] = []
    for line in patch.splitlines(keepends=True):
        if line.startswith("diff --git ") and buf:
            blocks.append("".join(buf))
            buf = [line]
        else:
            buf.append(line)
    if buf:
        blocks.append("".join(buf))

    chunks: list[str] = []
    cur: list[str] = []
    size = 0
    for block in blocks:
        if cur and size + len(block) > limit:
            chunks.append("".join(cur))
            cur, size = [], 0
        cur.append(block)
        size += len(block)
    if cur:
        chunks.append("".join(cur))
    return chunks or [patch]


def build_ai_prompt(ctx: Context, chunk: str, snippets: str, index: int, total: int) -> str:
    static_lines = [
        f"- [{SEVERITY_LABEL[f.severity]}] {f.title} {('@' + f.file + ':' + str(f.line)) if f.file else ''}"
        for f in sorted(ctx.findings, key=Finding.sort_key)
    ][:40]

    part = f"（第 {index + 1}/{total} 块）" if total > 1 else ""
    sections = [
        f"## 改动范围{part}\n{ctx.range_desc}",
        "## 改动文件\n" + "\n".join(
            f"- {f} (+{ctx.numstat.get(f, (0, 0))[0]}/-{ctx.numstat.get(f, (0, 0))[1]})"
            for f in ctx.files[:40]
        ),
        "## 机器已经查出的问题（不用重复，可以补充原因）\n" + ("\n".join(static_lines) or "（无）"),
    ]
    if snippets:
        sections.append("## 相关上下文（定义与调用方）\n" + snippets)
    sections.append(f"## 本块 diff\n```diff\n{chunk}\n```")
    return "\n\n".join(sections)


def call_llm(cfg: dict[str, str], system: str, prompt: str, timeout: int = 240) -> str:
    url = cfg["base_url"].rstrip("/") + "/chat/completions"
    body = json.dumps({
        "model": cfg["model"],
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": prompt},
        ],
        "temperature": 0.2,
        "stream": False,
    }).encode("utf-8")

    request = urllib.request.Request(
        url, data=body,
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {cfg['api_key']}"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=timeout) as resp:
        payload = json.loads(resp.read().decode("utf-8"))
    return payload["choices"][0]["message"]["content"]


def parse_ai(raw: str) -> dict:
    """解析模型输出。不是 JSON 就抛 ValueError，由调用方降级。"""
    text = raw.strip()
    if text.startswith("```"):
        text = re.sub(r"^```[a-zA-Z]*\n?", "", text)
        text = re.sub(r"\n?```$", "", text)
    start, end = text.find("{"), text.rfind("}")
    if start < 0 or end < 0:
        raise ValueError("返回的不是 JSON")
    data = json.loads(text[start:end + 1])
    if not isinstance(data, dict):
        raise ValueError("JSON 顶层不是对象")
    return data


def finding_key(item: dict) -> tuple[str, str]:
    return (str(item.get("file", "")), str(item.get("title", ""))[:60])


def intersect_runs(runs: list[dict]) -> dict:
    """多次采样取交集：只保留每次都出现的结论，压掉模型的随机发挥。"""
    if len(runs) == 1:
        return runs[0]
    counts: dict[tuple[str, str], int] = {}
    first: dict[tuple[str, str], dict] = {}
    for run in runs:
        for item in run.get("findings", []) or []:
            key = finding_key(item)
            counts[key] = counts.get(key, 0) + 1
            first.setdefault(key, item)
    stable = [first[k] for k, c in counts.items() if c == len(runs)]
    return {
        "summary": runs[0].get("summary", ""),
        "findings": stable,
        "simplifications": runs[0].get("simplifications", []) or [],
    }


def run_ai(ctx: Context, enabled: bool, samples: int = 1) -> None:
    if not enabled:
        ctx.ai_error = "已用 --no-ai 跳过"
        return

    cfg = load_llm_config(ctx)
    missing = [k for k in ("base_url", "api_key", "model") if not cfg[k]]
    if missing:
        ctx.ai_error = (
            f"没找到模型配置（缺 {', '.join(missing)}），跳过 AI 分析。"
            "用环境变量 LLM_BASE_URL / LLM_API_KEY / LLM_MODEL，或仓库根目录的 .env 提供。"
        )
        return

    system = AI_SYSTEM_TMPL.format(
        stack=ctx.profile.stack(),
        brief=ctx.profile.brief or "（仓库没有可读的项目说明文档）",
    )
    chunks = split_patch(ctx.patch)
    snippets = build_context_snippets(ctx)

    summaries: list[str] = []
    findings: list[dict] = []
    simplifications: list[dict] = []
    errors: list[str] = []

    for index, chunk in enumerate(chunks):
        prompt = build_ai_prompt(ctx, chunk, snippets, index, len(chunks))
        runs: list[dict] = []
        for _ in range(max(1, samples)):
            try:
                runs.append(parse_ai(call_llm(cfg, system, prompt)))
            except urllib.error.HTTPError as exc:
                errors.append(f"第 {index + 1} 块：接口返回 {exc.code}")
            except Exception as exc:  # 网络、超时、证书、JSON，都不该让整个审查失败
                errors.append(f"第 {index + 1} 块：{type(exc).__name__}: {exc}")
        if not runs:
            continue
        merged = intersect_runs(runs)
        summaries.append(str(merged.get("summary", "")).strip())
        findings.extend(merged.get("findings", []) or [])
        simplifications.extend(merged.get("simplifications", []) or [])

    if errors and not findings:
        ctx.ai_error = "；".join(errors[:3])
        return
    if errors:
        ctx.ai_error = "部分分块失败：" + "；".join(errors[:3])

    ctx.ai_summary = " ".join(s for s in summaries if s).strip()

    seen: set[tuple[str, int, str]] = set()
    for item in findings:
        key = (str(item.get("file", "")), int(item.get("line") or 0), str(item.get("title", "")))
        if key in seen:
            continue
        seen.add(key)
        severity = str(item.get("severity", "info")).lower()
        if severity not in SEVERITY_ORDER:
            severity = "info"
        ctx.findings.append(Finding(
            severity, str(item.get("title", "")).strip(),
            str(item.get("detail", "")).strip(),
            file=str(item.get("file", "")), line=int(item.get("line") or 0),
            suggestion=str(item.get("suggestion", "")).strip(), source="ai",
        ))

    for item in simplifications[:5]:
        ctx.findings.append(Finding(
            "info", f"可简化：{str(item.get('why', '')).strip() or '重复或冗余'}",
            f"当前写法：\n```\n{str(item.get('before', '')).strip()[:600]}\n```",
            file=str(item.get("file", "")), line=int(item.get("line") or 0),
            suggestion=f"建议改成：\n```\n{str(item.get('after', '')).strip()[:600]}\n```",
            source="ai-simplify",
        ))


# ---------------------------------------------------------------- 渲染

def coverage_line(ctx: Context) -> str:
    """一行说清「这次到底验了什么、什么没验」。全自动的前提是把跳过的地方摆出来。"""
    passed = [n for n, s, _ in ctx.checks if s == "通过"]
    failed = [n for n, s, _ in ctx.checks if s == "失败"]
    skipped = [n for n, s, _ in ctx.checks if s == "跳过"]
    parts = []
    if passed:
        parts.append("通过 " + "、".join(passed))
    if failed:
        parts.append("失败 " + "、".join(failed))
    if skipped:
        parts.append("跳过 " + "、".join(skipped))
    return " ｜ ".join(parts) if parts else "没有可执行的检查"


def render(ctx: Context) -> str:
    findings = sorted(ctx.findings, key=Finding.sort_key)
    blocks = [f for f in findings if f.severity == "block"]
    warns = [f for f in findings if f.severity == "warn"]
    infos = [f for f in findings if f.severity == "info" and f.source != "ai-simplify"]
    simple = [f for f in findings if f.source == "ai-simplify"]

    total_add = sum(a for a, _ in ctx.numstat.values())
    total_del = sum(d for _, d in ctx.numstat.values())

    if blocks:
        verdict = f"不建议合并：有 {len(blocks)} 项阻断问题"
    elif warns:
        verdict = f"可以合并，但先看 {len(warns)} 项警告"
    else:
        verdict = "没有发现阻断性问题"

    out: list[str] = []
    out.append("# PR 审查报告")
    out.append("")
    out.append(f"- **结论**：{verdict}")
    out.append(f"- **范围**：`{ctx.range_desc}`")
    out.append(f"- **规模**：{len(ctx.files)} 个文件，+{total_add} / -{total_del}")
    out.append(f"- **技术栈**：{ctx.profile.stack()}")
    out.append(f"- **审计指纹**：`{ctx.fingerprint}`（PR 有新提交后此报告作废，需重跑）")
    out.append(f"- **验证覆盖**：{coverage_line(ctx)}")
    out.append(f"- **统计**：阻断 {len(blocks)} · 警告 {len(warns)} · 提示 {len(infos)} · 简化建议 {len(simple)}")
    out.append("")

    if ctx.ai_summary:
        out.append("## 改动概述（AI）")
        out.append("")
        out.append(ctx.ai_summary)
        out.append("")
    if ctx.ai_error:
        out.append(f"> AI 分析未完全执行：{ctx.ai_error}")
        out.append("")

    def section(title: str, items: list[Finding]) -> None:
        if not items:
            return
        out.append(f"## {title}")
        out.append("")
        for item in items:
            where = ""
            if item.file:
                loc = f"{item.file}:{item.line}" if item.line else item.file
                where = f" `{loc}`"
            tag = "机器" if item.source == "static" else "AI"
            out.append(f"### [{SEVERITY_LABEL[item.severity]}] {item.title}{where}  <sub>{tag}</sub>")
            if item.detail:
                out.append("")
                out.append(item.detail)
            if item.suggestion:
                out.append("")
                out.append(item.suggestion)
            out.append("")

    section("阻断项", blocks)
    section("警告", warns)
    section("提示", infos)
    section("简化建议", simple)

    if ctx.checks:
        out.append("## 检查明细")
        out.append("")
        out.append("| 检查 | 结果 | 说明 |")
        out.append("|---|---|---|")
        for name, status, note in ctx.checks:
            note = (note or "").replace("\n", " ").replace("|", "\\|")[:160]
            out.append(f"| {name} | {status} | {note} |")
        out.append("")

    out.append("## 改动文件")
    out.append("")
    for path in ctx.files:
        add, delete = ctx.numstat.get(path, (0, 0))
        out.append(f"- `{path}` (+{add}/-{delete})")
    out.append("")

    out.append("---")
    out.append("")
    out.append("本报告只读生成，未修改任何源码。AI 结论属于参考意见，合并决定由人做出。")
    return "\n".join(out)


# ---------------------------------------------------------------- 入口

def _loc(item: Finding) -> str:
    """评论里的定位前缀。没有行号就只给文件，别显示 `:0`。"""
    if not item.file:
        return ""
    loc = f"{item.file}:{item.line}" if item.line else item.file
    return f"`{loc}` "


def render_comment(ctx: Context) -> str:
    """生成一段可以直接贴到 PR 评论里的精简结论。

    为什么单独做一份：完整报告几十上百行，贴到 PR 里没人读。
    评论只需要「能不能合 + 卡在哪」，细节让人自己点开报告看。
    """
    findings = sorted(ctx.findings, key=Finding.sort_key)
    blocks = [f for f in findings if f.severity == "block"]
    warns = [f for f in findings if f.severity == "warn"]

    if blocks:
        verdict = f"**不建议合并**：{len(blocks)} 项阻断"
    elif warns:
        verdict = f"**可以合并**：{len(warns)} 项警告待确认"
    else:
        verdict = "**通过**：没有发现阻断性问题"

    out = ["## AI 审计结论", "", verdict, ""]

    if blocks:
        out.append("### 阻断（改完必须重审）")
        out.append("")
        for item in blocks[:10]:
            out.append(f"- {_loc(item)}{item.title}")
        out.append("")

    if warns:
        out.append("### 警告（合并前请确认）")
        out.append("")
        for item in warns[:10]:
            out.append(f"- {_loc(item)}{item.title}")
            if item.detail:
                out.append(f"  {item.detail.splitlines()[0][:160]}")
        if len(warns) > 10:
            out.append(f"- …另有 {len(warns) - 10} 项，见完整报告")
        out.append("")

    out.append(f"> 审计指纹 `{ctx.fingerprint}`，范围 `{ctx.range_desc}`。")
    out.append("> PR 再推新提交后指纹会变，这次审计自动作废，需要重跑。")
    out.append("> 完整报告见 `pr-review/`。AI 结论仅作参考，合并决定由人做出。")
    return "\n".join(out)


def main() -> int:
    parser = argparse.ArgumentParser(description="PR 审查：静态检查 + 测试 + 契约 + AI 分析")
    parser.add_argument("--base", default="origin/main", help="基线分支（默认 origin/main）")
    parser.add_argument("--head", default="HEAD", help="待审分支（默认 HEAD）")
    parser.add_argument("--no-ai", action="store_true", help="只跑确定性检查")
    parser.add_argument("--no-tests", action="store_true", help="跳过测试执行")
    parser.add_argument("--no-contract", action="store_true", help="跳过接口契约检查")
    parser.add_argument("--ai-samples", type=int, default=1,
                        help="AI 采样次数，>1 时只保留每次都出现的结论（默认 1）")
    parser.add_argument("--out", default="pr-review", help="报告输出目录")
    parser.add_argument("--stdout", action="store_true", help="同时把报告打到标准输出")
    parser.add_argument("--strict", action="store_true", help="警告也算失败（退出码 1），用于硬卡点")
    parser.add_argument("--comment", action="store_true", help="额外输出一段可直接贴到 PR 的精简结论")
    args = parser.parse_args()

    ctx = collect(args)
    if not ctx.files:
        print(f"没有需要审查的改动（{ctx.range_desc}）")
        return 0

    check_compile(ctx)
    if not args.no_tests:
        check_tests(ctx)
    if not args.no_contract:
        check_contract(ctx)
    check_markup(ctx)
    check_styles(ctx)
    check_headers(ctx)
    check_rules(ctx)
    check_size(ctx)
    check_function_length(ctx)
    check_shared_symbols(ctx)
    run_ai(ctx, not args.no_ai, max(1, args.ai_samples))

    report = render(ctx)

    out_dir = ctx.root / args.out
    out_dir.mkdir(exist_ok=True)
    sha = git(["rev-parse", "--short", args.head], ctx.root).strip()
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M")
    target = out_dir / f"{stamp}-{sha}.md"
    target.write_text(report, encoding="utf-8")

    if args.stdout:
        print(report)

    blocks = sum(1 for f in ctx.findings if f.severity == "block")
    warns = sum(1 for f in ctx.findings if f.severity == "warn")
    failed = blocks > 0 or (args.strict and warns > 0)

    print(f"报告已写入：{target.relative_to(ctx.root)}")
    print(f"审计指纹：{ctx.fingerprint}")
    print(f"结论：阻断 {blocks} · 警告 {warns}"
          + ("（--strict：警告也算失败）" if args.strict and blocks == 0 and warns else ""))

    if args.comment:
        comment = render_comment(ctx)
        (out_dir / f"{stamp}-{sha}-comment.md").write_text(comment, encoding="utf-8")
        print()
        print(comment)

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
