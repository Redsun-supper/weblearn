<#
  构建英语复习引擎（Rust → WASM），并自动写入引擎版本号

  背景：引擎的构建原本是 modules/english/engine/README.md 里手敲的三条命令
  （cargo build → wasm-bindgen → 出 pkg/）。但那条链路之后还有一步**只写在文档里**的约定：
  把 modules/english/english.js 顶部的 ENGINE_VERSION 加一。

  为什么那一步必须有：`pkg/guangxue_wasm_bg.wasm` 的文件名不带指纹，而 dev-server 与 Nginx
  都按扩展名分流缓存（`.wasm` 曾经是「强缓存一天」），所以重建 `pkg/` 之后浏览器跑的仍是旧引擎，
  **不报任何错** —— 真踩过：修好的置顶排序在页面上毫无反应，排查了半天。

  人工 +1 的毛病是「总有一天会忘」。本脚本把它变成算出来的：对产出的 wasm 取 SHA-256，
  前 12 位写进 `pkg/engine-version.js`，由 `english.js` 在加载引擎前读它、拼进 `.js` 与
  `_bg.wasm` 两个 URL（两个文件是分开缓存的，只给一个加版本号没用）——
  引擎一变，版本号就变。

  用法：
    pwsh scripts/build-engine.ps1             # 完整构建：cargo build --release + wasm-bindgen + 版本文件
    pwsh scripts/build-engine.ps1 -SkipBuild  # 不重新编译，只按现有 pkg/ 重算版本文件（几秒）
    pwsh scripts/build-engine.ps1 -Check      # 只校验 pkg/ 与版本号是否对得上（verify.ps1 会调它）
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$Check
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$engineDir = Join-Path $root 'modules\english\engine'
$pkgDir = Join-Path $engineDir 'pkg'
$pkgWasm = Join-Path $pkgDir 'guangxue_wasm_bg.wasm'
$versionFile = Join-Path $pkgDir 'engine-version.js'
$buildArtifact = Join-Path $engineDir 'target\wasm32-unknown-unknown\release\guangxue_wasm.wasm'
$outName = 'guangxue_wasm'

# 版本号取 wasm 的前 12 位十六进制：够短（能塞进 URL）、够长（300KB 的二进制不会撞）
function Get-WasmVersion {
    param([string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLower().Substring(0, 12)
}

# 从构建产物里读回版本号。只认本脚本写的那种一行 export，读不到就当没有（不猜）
function Read-PkgVersion {
    if (-not (Test-Path -LiteralPath $versionFile)) { return $null }
    $m = [regex]::Match((Get-Content -LiteralPath $versionFile -Raw), "ENGINE_VERSION\s*=\s*'([^']+)'")
    if ($m.Success) { return $m.Groups[1].Value }
    return $null
}

function Find-Cargo {
    $cmd = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $fallback = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (Test-Path -LiteralPath $fallback) { return $fallback }
    return $null
}

# wasm-bindgen CLI 不在 PATH 上（装在 ~/.local/bin/wasm-bindgen-<版本>-<target>/ 下）
function Find-WasmBindgen {
    $cmd = Get-Command wasm-bindgen -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $hit = Get-ChildItem (Join-Path $env:USERPROFILE '.local\bin') -Filter 'wasm-bindgen.exe' -Recurse -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($hit) { return $hit.FullName }
    return $null
}

# ---------- -Check：只比对，不改任何文件（verify.ps1 用这个模式） ----------

if ($Check) {
    if (-not (Test-Path -LiteralPath $pkgWasm)) {
        Write-Output 'pkg/guangxue_wasm_bg.wasm 不存在，跳过版本号校验（先跑 pwsh scripts/build-engine.ps1）'
        exit 0
    }
    $actual = Get-WasmVersion -Path $pkgWasm
    $recorded = Read-PkgVersion
    if ($recorded -eq $actual) {
        Write-Output ("版本号一致：{0}" -f $actual)
        exit 0
    }
    Write-Output ("pkg/ 与版本号对不上：二进制算出来是 {0}，engine-version.js 里写的是 {1}" -f $actual, $(if ($recorded) { $recorded } else { '（读不到）' }))
    Write-Output '这多半意味着有人绕过本脚本重建过 pkg/（或手工改过版本文件）：'
    Write-Output '浏览器会继续吃旧 wasm 的缓存，症状是「改了引擎页面上毫无反应且不报错」。'
    Write-Output '修法：重跑 pwsh scripts/build-engine.ps1（或 -SkipBuild 只重算版本号）。'
    exit 1
}

# ---------- 构建 ----------

if (-not $SkipBuild) {
    $cargo = Find-Cargo
    if (-not $cargo) { throw '找不到 cargo（Rust 工具链未安装或不在 PATH 上）' }
    $bindgen = Find-WasmBindgen
    if (-not $bindgen) { throw '找不到 wasm-bindgen CLI（应装在 %USERPROFILE%\.local\bin 下）' }

    # ⚠️ CLI 版本必须与 Cargo.toml 的 wasm-bindgen 对齐，否则产物在浏览器里报版本不匹配
    $toml = Get-Content -LiteralPath (Join-Path $engineDir 'Cargo.toml') -Raw
    $m = [regex]::Match($toml, 'wasm-bindgen\s*=\s*"([^"]+)"')
    $expected = if ($m.Success) { $m.Groups[1].Value } else { $null }
    $reported = (& $bindgen --version) -join ''
    Write-Output ("wasm-bindgen : {0}" -f $reported.Trim())
    if ($expected -and $reported -notmatch [regex]::Escape($expected)) {
        throw ("wasm-bindgen CLI 版本与 Cargo.toml 不一致：Cargo.toml 写的是 {0}，命令行是「{1}」——请换成同版本（否则产物在浏览器里会报版本不匹配）" -f $expected, $reported.Trim())
    }

    Write-Output 'cargo build --target wasm32-unknown-unknown --release ...'
    Push-Location $engineDir
    try {
        & $cargo build --target wasm32-unknown-unknown --release
        if ($LASTEXITCODE -ne 0) { throw ("cargo build 失败（退出码 {0}）" -f $LASTEXITCODE) }
    } finally {
        Pop-Location
    }
    if (-not (Test-Path -LiteralPath $buildArtifact)) { throw ("构建产物不存在：{0}" -f $buildArtifact) }

    Write-Output 'wasm-bindgen --target web --out-dir pkg ...'
    New-Item -ItemType Directory -Force -Path $pkgDir | Out-Null
    Push-Location $engineDir
    try {
        & $bindgen --target web --out-dir pkg --out-name $outName $buildArtifact
        if ($LASTEXITCODE -ne 0) { throw ("wasm-bindgen 失败（退出码 {0}）" -f $LASTEXITCODE) }
    } finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $pkgWasm)) {
    throw ("找不到 {0} —— 去掉 -SkipBuild 跑一次完整构建" -f $pkgWasm)
}

# ---------- 写版本号 ----------

$version = Get-WasmVersion -Path $pkgWasm
$lines = @(
    '// 本文件由 scripts/build-engine.ps1 自动生成，**不要手改**（下次构建会被覆盖）。'
    '// 内容是 pkg/guangxue_wasm_bg.wasm 的 SHA-256 前 12 位：引擎一变，它就变。'
    '// 用途：english.js 把它拼进 .js 与 _bg.wasm 两个 URL，绕开浏览器的强缓存'
    '//（.wasm 文件名不带指纹又被长缓存时，重建 pkg/ 后页面跑的仍是旧引擎，且不报任何错）。'
    ("export var ENGINE_VERSION = '{0}';" -f $version)
)
# 不带 BOM：ES module 虽然能容忍 BOM，但没有必要（BOM 还可能被别的工具当内容）
$utf8 = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText($versionFile, (($lines -join "`n") + "`n"), $utf8)

# ---------- 汇总 ----------

$size = (Get-Item -LiteralPath $pkgWasm).Length
Write-Output ''
Write-Output '================ 引擎构建完成 ================'
Write-Output ("  版本号     : {0}" -f $version)
Write-Output ("  wasm 大小  : {0} B" -f $size)
Write-Output ("  版本文件   : {0}" -f $versionFile)
Write-Output '  提醒：部署时 pkg/ 要随静态文件一起推上去（它被 gitignore，不进版本库）。'
exit 0
