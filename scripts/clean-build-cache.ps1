<#
  清理 Rust 编译缓存（可再生产物）

  背景：两个 crate（backend-rust 账号服务、modules/english/engine wasm 引擎）
  各有一套 target/，含调试符号与增量编译缓存，加起来能到 4.5 GB —— 约为仓库
  源码体积的 300 倍。它们全部被 .gitignore 排除（不会进版本库、不会传云端），
  但会持续占着本地磁盘。

  默认只删 debug 产物（cargo test 产生的那部分，占比最大）：
    - backend-rust/target/debug
    - modules/english/engine/target/debug
  release 二进制（正在运行的 guangxue-auth.exe 就是它）与 pkg/（浏览器加载的
  wasm 胶水）都会保留，所以删完站点和账号服务照常可用。

  用法：
    pwsh scripts/clean-build-cache.ps1            # 只删 debug（推荐，约释放 3.8 GB）
    pwsh scripts/clean-build-cache.ps1 -All       # 连 release 一起删（需先停账号服务）
    pwsh scripts/clean-build-cache.ps1 -DryRun    # 只报告会删什么，不真删
#>
[CmdletBinding()]
param(
    [switch]$All,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

function Get-DirMB([string]$p) {
    if (-not (Test-Path -LiteralPath $p)) { return 0 }
    $f = Get-ChildItem -Recurse -File -Force -LiteralPath $p -ErrorAction SilentlyContinue
    return [math]::Round((($f | Measure-Object -Sum Length).Sum / 1MB), 1)
}

# 要清理的目标：默认只动 debug；-All 时连整个 target 一起删（pkg/ 始终保留）
$targets = @()
foreach ($crate in 'backend-rust', 'modules\english\engine') {
    $t = Join-Path $root (Join-Path $crate 'target')
    if (-not (Test-Path -LiteralPath $t)) { continue }
    if ($All) { $targets += $t }
    else { $targets += (Join-Path $t 'debug') }
}

if ($targets.Count -eq 0) {
    Write-Output '没有找到可清理的编译缓存（target/ 不存在）'
    exit 0
}

# 安全闸：只允许删仓库内的 target/ 或 target/debug，别的一律拒绝
$safe = $true
foreach ($t in $targets) {
    $full = [System.IO.Path]::GetFullPath($t)
    $isInside = $full.StartsWith([System.IO.Path]::GetFullPath($root), [StringComparison]::OrdinalIgnoreCase)
    $isCacheDir = ($full -match '\\target(\\debug)?$')
    if (-not $isInside -or -not $isCacheDir -or $full -match '备份') {
        Write-Output ("拒绝：路径不在白名单内 -> {0}" -f $full)
        $safe = $false
    }
}
if (-not $safe) { exit 1 }

$freed = 0
foreach ($t in $targets) {
    $mb = Get-DirMB $t
    $rel = $t.Replace($root, '').TrimStart('\')
    if ($DryRun) {
        Write-Output ("[DryRun] 将删除 {0}  ({1:N0} MB)" -f $rel, $mb)
        $freed += $mb
        continue
    }
    try {
        Remove-Item -Recurse -Force -LiteralPath $t
        Write-Output ("已删除 {0}  ({1:N0} MB)" -f $rel, $mb)
        $freed += $mb
    } catch {
        # 最常见的原因是 exe 正在运行、文件被占用
        Write-Output ("跳过 {0}：{1}" -f $rel, $_.Exception.Message)
        Write-Output '        （若提示文件被占用，先停掉账号服务再重试；-All 才会碰到正在运行的 exe）'
    }
}

if ($DryRun) {
    Write-Output ("[DryRun] 预计释放 {0:N0} MB（{1:N2} GB）" -f $freed, ($freed / 1024))
} else {
    Write-Output ("合计释放 {0:N0} MB（{1:N2} GB）" -f $freed, ($freed / 1024))
    Write-Output '提示：下次 cargo test / build 会重新生成这些缓存，属正常现象。'
}
