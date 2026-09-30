<#
.SYNOPSIS
  跨服务鉴权联调：账号服务（Rust）发 Cookie，Go 后端本地验签。

.DESCRIPTION
  阶段 2 的鉴权是「共享 AUTH_JWT_SECRET 本地验签」：Go 不查库、也不回调账号服务，
  直接用同一把密钥验 `gx_access` Cookie。这套做法的风险在于**两侧任何一处对不上都会静默 401**
  （密钥不同、算法不同、Cookie 名不同、时钟容差不一致），而两边的单元测试各自 mock 自己的
  密钥，恰恰发现不了这种跨服务的不一致。

  本脚本用**临时数据库**同时起两个真实服务来做端到端验证：

    1. Rust 账号服务起在 -AuthPort（临时 auth.db，开发管理员由环境变量种子）；
    2. Go 后端起在 -ApiPort（临时 guangxue.db），读**同一把**密钥与同一个 Origin 白名单；
    3. 用管理员登录拿到的真 Cookie 打 Go：复习接口应 200、词条写接口应放行；
    4. 再注册一个普通用户，用它的真 Cookie 打 Go：复习接口 200、词条写接口应 403；
    5. 不带 Cookie 打复习接口应 401 `unauthenticated`；
      带外站 Origin 的写请求应 403 `forbidden`（CSRF 闸门）。

  全程不碰真实的 `auth.db` / `guangxue.db`，也不影响 8080/8081 上可能在跑的开发服务。

.PARAMETER AuthPort
  临时账号服务端口（默认 18081，故意避开开发用的 8081）。

.PARAMETER ApiPort
  临时 Go 后端端口（默认 18080，故意避开开发用的 8080）。

.PARAMETER KeepArtifacts
  保留临时目录（含两个服务的日志与临时库），便于失败后排查。

.EXAMPLE
  pwsh scripts/verify-auth.ps1
#>
param(
    [int]$AuthPort = 18081,
    [int]$ApiPort = 18080,
    [int]$TimeoutSeconds = 90,
    [switch]$KeepArtifacts
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

function Find-Go {
    $cmd = Get-Command go -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $portable = 'C:\Users\22629\go-portable\go\bin\go.exe'
    if (Test-Path $portable) { return $portable }
    throw '找不到 go（既不在 PATH，也不在便携版路径）：请先安装 Go 1.21+'
}

function Find-Cargo {
    $cmd = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $fallback = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (Test-Path $fallback) { return $fallback }
    throw '找不到 cargo：请先安装 Rust 工具链'
}

$checks = @()
function Add-Check {
    param([string]$Name, [bool]$Ok, [string]$Detail = '')
    $script:checks += [pscustomobject]@{ Name = $Name; Ok = $Ok; Detail = $Detail }
    $tag = if ($Ok) { 'PASS' } else { 'FAIL' }
    $color = if ($Ok) { 'Green' } else { 'Red' }
    $suffix = if ($Detail) { " — $Detail" } else { '' }
    Write-Host ("  [{0}] {1}{2}" -f $tag, $Name, $suffix) -ForegroundColor $color
}

# 读响应体里的 error 机器码（Rust 与 Go 都用 {code,message,error} 这一层）
function Get-ErrorCode($Response) {
    try { return [string](($Response.Content | ConvertFrom-Json).error) } catch { return '' }
}

function Wait-Healthy {
    param([string]$Url, [int]$Seconds, [string]$Label)
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        try {
            $r = Invoke-WebRequest -Uri $Url -SkipHttpErrorCheck -TimeoutSec 3
            if ($r.StatusCode -eq 200) { return $true }
        } catch { }
        Start-Sleep -Milliseconds 300
    }
    Write-Host "  ⚠️ $Label 在 $Seconds 秒内没有就绪：$Url" -ForegroundColor Yellow
    return $false
}

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$work = Join-Path $env:TEMP "gx-auth-e2e-$stamp"
New-Item -ItemType Directory -Path $work -Force | Out-Null

# 两侧共用的一把随机密钥：本脚本要证明的正是「同一把密钥就能互相认」
$chars = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789'
$secret = -join (1..64 | ForEach-Object { $chars[(Get-Random -Maximum $chars.Length)] })

$adminEmail = '2262997289@qq.com'
$adminPassword = '7289HR_RedSun'
$userPassword = 'VerifyAuth123'
$allowedOrigin = 'http://127.0.0.1:8899'
$authBase = "http://127.0.0.1:$AuthPort"
$apiBase = "http://127.0.0.1:$ApiPort"

$authProc = $null
$goProc = $null
$started = Get-Date

Write-Host ''
Write-Host '广学 · 跨服务鉴权联调（Rust 发 Cookie → Go 本地验签）' -ForegroundColor Cyan
Write-Host "  临时目录 : $work"
Write-Host "  账号服务 : $authBase（临时 auth.db）"
Write-Host "  Go 后端  : $apiBase（临时 guangxue.db）"
Write-Host ''

try {
    # ---------------------------------------------------------------- 准备二进制
    $goExe = Find-Go
    $cargoExe = Find-Cargo

    Write-Host '构建 Go 后端…' -ForegroundColor Cyan
    $goBinary = Join-Path $work 'guangxue-go.exe'
    Push-Location (Join-Path $root 'backend-go')
    try {
        & $goExe build -o $goBinary . 2>&1 | ForEach-Object { Write-Host "    $_" }
        if ($LASTEXITCODE -ne 0) { throw "go build 失败（exit $LASTEXITCODE）" }
    } finally { Pop-Location }

    Write-Host '构建账号服务（增量，通常几秒）…' -ForegroundColor Cyan
    Push-Location (Join-Path $root 'backend-rust')
    try {
        & $cargoExe build 2>&1 | Select-Object -Last 3 | ForEach-Object { Write-Host "    $_" }
        if ($LASTEXITCODE -ne 0) { throw "cargo build 失败（exit $LASTEXITCODE）" }
    } finally { Pop-Location }
    $authBinary = Join-Path $root 'backend-rust\target\debug\guangxue-auth.exe'
    if (-not (Test-Path $authBinary)) { throw "找不到账号服务可执行文件：$authBinary" }

    # ---------------------------------------------------------------- 起账号服务
    $env:AUTH_DB_PATH = Join-Path $work 'auth.db'
    $env:AUTH_HOST = '127.0.0.1'
    $env:AUTH_PORT = "$AuthPort"
    $env:APP_ENV = 'development'
    $env:AUTH_JWT_SECRET = $secret
    $env:AUTH_ALLOWED_ORIGINS = $allowedOrigin
    $env:AUTH_MAIL_MODE = 'log'
    $env:AUTH_DEV_ENDPOINTS = 'true'
    $env:AUTH_SEED_ADMIN = 'true'
    $env:AUTH_ADMIN_EMAIL = $adminEmail
    $env:AUTH_ADMIN_PASSWORD = $adminPassword

    Write-Host '启动账号服务…' -ForegroundColor Cyan
    $authProc = Start-Process -FilePath $authBinary -WorkingDirectory (Join-Path $root 'backend-rust') `
        -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $work 'auth.out.log') `
        -RedirectStandardError (Join-Path $work 'auth.err.log')

    if (-not (Wait-Healthy -Url "$authBase/api/auth/health" -Seconds $TimeoutSeconds -Label '账号服务')) {
        throw '账号服务没有起来（日志见临时目录的 auth.err.log）'
    }
    Add-Check '账号服务健康检查 200' $true "$authBase/api/auth/health"

    # ---------------------------------------------------------------- 管理员登录
    $adminSession = New-Object Microsoft.PowerShell.Commands.WebRequestSession
    $login = Invoke-WebRequest -Uri "$authBase/api/auth/login" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ email = $adminEmail; password = $adminPassword; device_label = 'verify-auth' } | ConvertTo-Json)
    $role = ''
    try { $role = [string](($login.Content | ConvertFrom-Json).data.user.role) } catch { }
    Add-Check '账号服务：管理员登录成功且 role=admin' ($login.StatusCode -eq 200 -and $role -eq 'admin') "HTTP $($login.StatusCode) role=$role"

    $accessCookie = @($adminSession.Cookies.GetAllCookies() | Where-Object { $_.Name -eq 'gx_access' })
    Add-Check '账号服务：下发 gx_access Cookie' ($accessCookie.Count -eq 1) "Cookie 数=$($accessCookie.Count)"

    # ---------------------------------------------------------------- 起 Go 后端
    $env:DB_PATH = Join-Path $work 'guangxue.db'
    $env:SERVER_HOST = '127.0.0.1'
    $env:SERVER_PORT = "$ApiPort"
    # AUTH_JWT_SECRET / AUTH_ALLOWED_ORIGINS 保持上面那组值不变：这正是「共享密钥」的前提

    Write-Host '启动 Go 后端…' -ForegroundColor Cyan
    $goProc = Start-Process -FilePath $goBinary -WorkingDirectory (Join-Path $root 'backend-go') `
        -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $work 'go.out.log') `
        -RedirectStandardError (Join-Path $work 'go.err.log')

    if (-not (Wait-Healthy -Url "$apiBase/api/health" -Seconds $TimeoutSeconds -Label 'Go 后端')) {
        throw 'Go 后端没有起来（日志见临时目录的 go.err.log）'
    }
    Add-Check 'Go 后端健康检查 200' $true "$apiBase/api/health"

    # ---------------------------------------------------------------- Go 侧鉴权矩阵
    # 1) 不带 Cookie：复习接口必须 401，且 error 字段是前端用来跳登录的那个值
    $r = Invoke-WebRequest -Uri "$apiBase/api/reviews/stats" -SkipHttpErrorCheck
    Add-Check 'Go：匿名访问 /api/reviews/stats → 401 unauthenticated' `
        ($r.StatusCode -eq 401 -and (Get-ErrorCode $r) -eq 'unauthenticated') `
        "HTTP $($r.StatusCode) error=$(Get-ErrorCode $r)"

    # 2) 账号服务发的 Cookie，Go 必须认（这是整个阶段 2 的核心结论）
    $r = Invoke-WebRequest -Uri "$apiBase/api/reviews/stats" -WebSession $adminSession -SkipHttpErrorCheck
    $code = ''
    try { $code = [string](($r.Content | ConvertFrom-Json).code) } catch { }
    Add-Check 'Go：带账号服务发的 Cookie 访问复习统计 → 200（跨服务验签成立）' `
        ($r.StatusCode -eq 200 -and $code -eq '200') "HTTP $($r.StatusCode) code=$code"

    # 3) 公开接口不受影响
    $r = Invoke-WebRequest -Uri "$apiBase/api/words?limit=1" -SkipHttpErrorCheck
    Add-Check 'Go：匿名 GET /api/words → 200（公开接口没被误挡）' ($r.StatusCode -eq 200) "HTTP $($r.StatusCode)"

    # 4) 管理员写词条：只验证「没被 401/403 拦掉」，不关心业务细节（临时库，随便写）
    $r = Invoke-WebRequest -Uri "$apiBase/api/words" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ words = @(@{ word = 'verify-auth-probe'; phonetic = '/prəʊb/'; meaning = '联调用词条' }) } | ConvertTo-Json -Depth 6)
    Add-Check 'Go：管理员写词条 → 放行（非 401/403）' `
        ($r.StatusCode -ne 401 -and $r.StatusCode -ne 403) "HTTP $($r.StatusCode)"

    # 5) 注册一个普通用户：证明 RequireAdmin 真的区分角色（只有 RequireUser 的话这里会是 200）
    $email = "verify-auth-$stamp@example.com"
    $userSession = New-Object Microsoft.PowerShell.Commands.WebRequestSession
    $null = Invoke-WebRequest -Uri "$authBase/api/auth/email-code" -Method POST -WebSession $userSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ email = $email } | ConvertTo-Json)
    $devCodes = Invoke-WebRequest -Uri "$authBase/api/auth/dev/codes?email=$([uri]::EscapeDataString($email))" -SkipHttpErrorCheck
    $mailCode = ''
    try { $mailCode = [string](($devCodes.Content | ConvertFrom-Json).data.code) } catch { }
    Add-Check '账号服务：开发接口取到邮箱验证码（6 位）' ($mailCode.Length -eq 6) "code 长度=$($mailCode.Length)"

    $register = Invoke-WebRequest -Uri "$authBase/api/auth/register" -Method POST -WebSession $userSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ email = $email; email_code = $mailCode; password = $userPassword; username = '联调用户' } | ConvertTo-Json)
    $userRole = ''
    try { $userRole = [string](($register.Content | ConvertFrom-Json).data.user.role) } catch { }
    Add-Check '账号服务：注册普通用户（role=user，未用邀请码）' ($register.StatusCode -eq 200 -and $userRole -eq 'user') `
        "HTTP $($register.StatusCode) role=$userRole"

    $r = Invoke-WebRequest -Uri "$apiBase/api/reviews/stats" -WebSession $userSession -SkipHttpErrorCheck
    Add-Check 'Go：普通用户访问复习统计 → 200（RequireUser 放行）' ($r.StatusCode -eq 200) "HTTP $($r.StatusCode)"

    $r = Invoke-WebRequest -Uri "$apiBase/api/words" -Method POST -WebSession $userSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ words = @(@{ word = 'should-be-rejected' }) } | ConvertTo-Json -Depth 6)
    Add-Check 'Go：普通用户写词条 → 403 forbidden（RequireAdmin 拦住）' `
        ($r.StatusCode -eq 403 -and (Get-ErrorCode $r) -eq 'forbidden') `
        "HTTP $($r.StatusCode) error=$(Get-ErrorCode $r)"

    # 6) CSRF：写请求带外站 Origin 必须被挡在处理器之前
    $r = Invoke-WebRequest -Uri "$apiBase/api/reviews/submit" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = 'http://evil.example.com' } -SkipHttpErrorCheck `
        -Body '{"word_id":1,"rating":3}'
    Add-Check 'Go：外站 Origin 的写请求 → 403 forbidden（CSRF 闸门）' `
        ($r.StatusCode -eq 403 -and (Get-ErrorCode $r) -eq 'forbidden') `
        "HTTP $($r.StatusCode) error=$(Get-ErrorCode $r)"
}
catch {
    Write-Host ''
    Write-Host "联调中断：$($_.Exception.Message)" -ForegroundColor Red
    $script:checks += [pscustomobject]@{ Name = '联调执行完成'; Ok = $false; Detail = $_.Exception.Message }
}
finally {
    foreach ($proc in @($goProc, $authProc)) {
        if ($proc -and -not $proc.HasExited) {
            try { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue } catch { }
        }
    }
    # 清掉本脚本设置的环境变量，避免污染同一个 PowerShell 会话里的后续命令
    foreach ($name in 'AUTH_DB_PATH', 'AUTH_HOST', 'AUTH_PORT', 'APP_ENV', 'AUTH_JWT_SECRET',
        'AUTH_ALLOWED_ORIGINS', 'AUTH_MAIL_MODE', 'AUTH_DEV_ENDPOINTS', 'AUTH_SEED_ADMIN',
        'AUTH_ADMIN_EMAIL', 'AUTH_ADMIN_PASSWORD', 'DB_PATH', 'SERVER_HOST', 'SERVER_PORT') {
        Remove-Item "env:$name" -ErrorAction SilentlyContinue
    }
}

$failed = @($checks | Where-Object { -not $_.Ok })
$seconds = [math]::Round(((Get-Date) - $started).TotalSeconds, 1)

Write-Host ''
Write-Host '================ 汇总 ================' -ForegroundColor Cyan
Write-Host ''
$checks | ForEach-Object {
    $tag = if ($_.Ok) { 'PASS' } else { 'FAIL' }
    $suffix = if ($_.Detail) { " — $($_.Detail)" } else { '' }
    Write-Host ("  [{0}] {1}{2}" -f $tag, $_.Name, $suffix)
}
Write-Host ''
Write-Host ("共 {0} 项检查，{1} 项失败，用时 {2} 秒" -f $checks.Count, $failed.Count, $seconds)

if ($failed.Count -gt 0) {
    Write-Host "临时目录保留在：$work" -ForegroundColor Yellow
    exit 1
}

if ($KeepArtifacts) {
    Write-Host "临时目录保留在：$work" -ForegroundColor Yellow
} else {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
exit 0
