<#
  一键验证：Go 主后端 + Rust 账号服务 + wasm 复习引擎

  背景：验证路径原本散在四份 README 里，而且工具链有两处「不在 PATH 上」的坑
  （Go 是便携版、wasm-bindgen 装在 ~/.local/bin 下）。本脚本把它们收敛成一条命令，
  每步结果汇总成 PASS / FAIL 表，末尾以退出码收尾（0 = 全绿）。

  覆盖的是「构建 + 单元/集成测试」。**不含**端到端冒烟 —— 那需要真起两个服务，
  用 -IncludeSmoke 追加（见 backend-rust/scripts/smoke.ps1）。

  用法：
    pwsh scripts/verify.ps1                 # 全跑：Go → 账号服务 → 引擎 → wasm 目标
    pwsh scripts/verify.ps1 -Only go         # 只跑一段（go / rust / engine）
    pwsh scripts/verify.ps1 -SkipWasmTarget  # 跳过 wasm32 目标检查（省一次编译）
    pwsh scripts/verify.ps1 -IncludeSmoke    # 末尾追加账号服务冒烟（需服务已在跑）
#>
[CmdletBinding()]
param(
    [ValidateSet('all', 'go', 'rust', 'engine')]
    [string]$Only = 'all',
    [switch]$SkipWasmTarget,
    [switch]$IncludeSmoke,
    [string]$SmokeBaseUrl = 'http://127.0.0.1:8081'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$results = New-Object System.Collections.Generic.List[object]

# ---------- 工具链定位 ----------

function Find-Go {
    $cmd = Get-Command go -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $portable = 'C:\Users\22629\go-portable\go\bin\go.exe'
    if (Test-Path -LiteralPath $portable) { return $portable }
    return $null
}

function Find-Cargo {
    $cmd = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $fallback = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (Test-Path -LiteralPath $fallback) { return $fallback }
    return $null
}

# wasm-bindgen 不参与编译，只有出 pkg/ 时才用；这里只报告它是否就位、版本是否对齐
function Find-WasmBindgen {
    $cmd = Get-Command wasm-bindgen -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $hit = Get-ChildItem (Join-Path $env:USERPROFILE '.local\bin') -Filter 'wasm-bindgen.exe' -Recurse -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($hit) { return $hit.FullName }
    return $null
}

$go = Find-Go
$cargo = Find-Cargo
$wasmBindgen = Find-WasmBindgen

# ---------- 步骤执行与汇总 ----------

function Invoke-Step {
    param(
        [string]$Name,
        [string]$WorkDir,
        [scriptblock]$Block
    )
    Write-Output ''
    Write-Output ("=== {0} ===" -f $Name)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $ok = $false
    $detail = ''
    Push-Location $WorkDir
    try {
        $output = & $Block 2>&1
        $code = $LASTEXITCODE
        if ($null -eq $code) { $code = 0 }
        $ok = ($code -eq 0)
        $output | Select-Object -Last 40 | ForEach-Object { Write-Output $_ }
        if (-not $ok) {
            # 失败时把完整输出留下来，不然只能靠 40 行尾巴猜
            $log = Join-Path $env:TEMP ('gx-verify-{0}.log' -f ($Name -replace '[^\w]', '_'))
            $output | Set-Content -LiteralPath $log -Encoding UTF8
            $detail = "退出码 $code，完整输出：$log"
        }
    } catch {
        $detail = $_.Exception.Message
        Write-Output $detail
    } finally {
        Pop-Location
    }
    $sw.Stop()
    $results.Add([pscustomobject]@{
        Step    = $Name
        Status  = if ($ok) { 'PASS' } else { 'FAIL' }
        Seconds = [math]::Round($sw.Elapsed.TotalSeconds, 1)
        Detail  = $detail
    }) | Out-Null
    return $ok
}

$goDir = Join-Path $root 'backend-go'
$rustDir = Join-Path $root 'backend-rust'
$engineDir = Join-Path $root 'modules\english\engine'

Write-Output '工具链：'
Write-Output ("  go           : {0}" -f $(if ($go) { $go } else { '未找到（Go 段会被跳过）' }))
Write-Output ("  cargo        : {0}" -f $(if ($cargo) { $cargo } else { '未找到（Rust 段会被跳过）' }))
Write-Output ("  wasm-bindgen : {0}" -f $(if ($wasmBindgen) { $wasmBindgen } else { '未找到（只有出 pkg/ 时才需要）' }))
if ($go) { Write-Output ("  go version   : {0}" -f ((& $go version) -join '')) }
if ($cargo) { Write-Output ("  cargo version: {0}" -f ((& $cargo --version) -join '')) }

# ---------- Go ----------

if ($Only -in 'all', 'go') {
    if (-not $go) {
        $results.Add([pscustomobject]@{ Step = 'Go'; Status = 'SKIP'; Seconds = 0; Detail = '本机没有 go' }) | Out-Null
    } else {
        Invoke-Step -Name 'Go 构建' -WorkDir $goDir -Block { & $go build ./... } | Out-Null
        Invoke-Step -Name 'Go vet' -WorkDir $goDir -Block { & $go vet ./... } | Out-Null
        # 目前 backend-go 还没有任何 _test.go，"no test files" 是正常结果，不是失败
        Invoke-Step -Name 'Go 测试' -WorkDir $goDir -Block { & $go test ./... } | Out-Null
    }
}

# ---------- Rust 账号服务 ----------

if ($Only -in 'all', 'rust') {
    if (-not $cargo) {
        $results.Add([pscustomobject]@{ Step = '账号服务'; Status = 'SKIP'; Seconds = 0; Detail = '本机没有 cargo' }) | Out-Null
    } else {
        Invoke-Step -Name '账号服务测试' -WorkDir $rustDir -Block { & $cargo test } | Out-Null
    }
}

# ---------- wasm 引擎 ----------

if ($Only -in 'all', 'engine') {
    if (-not $cargo) {
        $results.Add([pscustomobject]@{ Step = '引擎'; Status = 'SKIP'; Seconds = 0; Detail = '本机没有 cargo' }) | Out-Null
    } else {
        Invoke-Step -Name '引擎宿主测试' -WorkDir $engineDir -Block { & $cargo test } | Out-Null
        if (-not $SkipWasmTarget) {
            Invoke-Step -Name '引擎 wasm32 检查' -WorkDir $engineDir -Block {
                & $cargo check --target wasm32-unknown-unknown
            } | Out-Null
        } else {
            $results.Add([pscustomobject]@{ Step = '引擎 wasm32 检查'; Status = 'SKIP'; Seconds = 0; Detail = '-SkipWasmTarget' }) | Out-Null
        }
    }
}

# ---------- 冒烟（可选，需服务已在跑）----------

if ($IncludeSmoke) {
    $smoke = Join-Path $rustDir 'scripts\smoke.ps1'
    Invoke-Step -Name '账号服务冒烟' -WorkDir $root -Block {
        & $smoke -BaseUrl $SmokeBaseUrl 2>&1 | Select-Object -Last 20
    } | Out-Null
}

# ---------- 汇总 ----------

Write-Output ''
Write-Output '================ 汇总 ================'
$results | Format-Table -AutoSize | Out-String | Write-Output
$failed = @($results | Where-Object { $_.Status -eq 'FAIL' })
$total = [math]::Round((($results | Measure-Object -Property Seconds -Sum).Sum), 1)
Write-Output ("共 {0} 步，{1} 步失败，用时 {2} 秒" -f $results.Count, $failed.Count, $total)

if ($failed.Count -gt 0) { exit 1 }
exit 0
