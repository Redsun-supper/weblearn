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
      带外站 Origin 的写请求应 403 `forbidden`（CSRF 闸门）；
    6. P0-1 隔离：管理员与普通用户复习**同一个词**之后，各自的统计里都只有自己那 1 条记录
       （改造前两人共用一行，谁的记录都会被算到对方头上）。

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
$inviteProc = $null
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
    # P0-5 起**启动期种子账号是 super_admin**（否则全新部署没人能发码 / 调权限）。
    # 这里刻意保持宽松：admin 或 super_admin 都算「后台角色」，
    # 免得以后调整种子角色时这条检查又假红一次（真正的契约由两侧的常量测试钉住）。
    Add-Check '账号服务：管理员登录成功且是后台角色（admin / super_admin）' `
        ($login.StatusCode -eq 200 -and ($role -eq 'admin' -or $role -eq 'super_admin')) `
        "HTTP $($login.StatusCode) role=$role"

    $accessCookie = @($adminSession.Cookies.GetAllCookies() | Where-Object { $_.Name -eq 'gx_access' })
    Add-Check '账号服务：下发 gx_access Cookie' ($accessCookie.Count -eq 1) "Cookie 数=$($accessCookie.Count)"

    # ---------------------------------------------------------------- 起 Go 后端
    $env:DB_PATH = Join-Path $work 'guangxue.db'
    $env:SERVER_HOST = '127.0.0.1'
    $env:SERVER_PORT = "$ApiPort"
    # AUTH_JWT_SECRET / AUTH_ALLOWED_ORIGINS 保持上面那组值不变：这正是「共享密钥」的前提
    # AUTH_DB_PATH 也保持上面那组值（临时 auth.db）：P1 的管理看板要**只读**读账号库拿账号侧数字，
    # 线上由 AUTH_DB_PATH 指到账号库（systemd 单元里的 ReadOnlyPaths 只是显式的只读保证，
    # 不是读权限的来源），这里则天然可读。
    # ⚠️ 它必须与账号服务指向**同一个**库，否则看板的账号侧全是 0（而不是报错）。

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

    # 5b) P0-5：三级角色的权限边界（这是「管理员 = 内容/运营角色，碰不到权限」的现场验证）
    #     超管能发码；普通用户不能看码、不能进用户列表。
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invites" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ count = 1; grant_role = 'admin'; note = 'verify-auth 联调' } | ConvertTo-Json)
    $issuedCode = ''
    $issuedRole = ''
    try {
        $issuedCode = [string](($r.Content | ConvertFrom-Json).data.items[0].code)
        $issuedRole = [string](($r.Content | ConvertFrom-Json).data.items[0].grant_role)
    } catch { }
    Add-Check 'P0-5：超管发管理员码 → 200 且带 ADMIN- 前缀' `
        ($r.StatusCode -eq 200 -and $issuedRole -eq 'admin' -and $issuedCode.StartsWith('ADMIN-')) `
        "HTTP $($r.StatusCode) grant_role=$issuedRole code=$issuedCode"

    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invites" -WebSession $userSession -SkipHttpErrorCheck
    Add-Check 'P0-5：普通用户看邀请码列表 → 403 forbidden' `
        ($r.StatusCode -eq 403 -and (Get-ErrorCode $r) -eq 'forbidden') `
        "HTTP $($r.StatusCode) error=$(Get-ErrorCode $r)"

    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/users" -WebSession $userSession -SkipHttpErrorCheck
    Add-Check 'P0-5：普通用户看用户列表 → 403 forbidden' `
        ($r.StatusCode -eq 403 -and (Get-ErrorCode $r) -eq 'forbidden') `
        "HTTP $($r.StatusCode) error=$(Get-ErrorCode $r)"

    # 自锁保护：临时库里唯一的超管就是种子账号，降级/封禁都必须被拒
    $superID = 0
    try {
        $usersResp = Invoke-WebRequest -Uri "$authBase/api/auth/admin/users?size=50" -WebSession $adminSession -SkipHttpErrorCheck
        $superID = [int](($usersResp.Content | ConvertFrom-Json).data.items | Where-Object { $_.role -eq 'super_admin' } | Select-Object -First 1).id
    } catch { }
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/users/$superID/role" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ role = 'admin' } | ConvertTo-Json)
    Add-Check 'P0-5：降级最后一个超管 → 400（自锁保护）' `
        ($r.StatusCode -eq 400) "HTTP $($r.StatusCode) message=$((($r.Content | ConvertFrom-Json).message))"

    # 6) P0-1：进度按人隔离 —— 管理员与普通用户复习**同一个词**，各自只该看到自己那一条。
    #    改造前 word_reviews 的唯一键是 word_id（一个词全局一行），两人共用一行、
    #    统计还会把对方的记录算进来（total_reviews 会是 2）——下面的断言正盯着这一点。
    $wordID = 0
    try {
        $wordsResp = Invoke-WebRequest -Uri "$apiBase/api/words?limit=1" -SkipHttpErrorCheck
        $wordID = [int](($wordsResp.Content | ConvertFrom-Json).data.items[0].id)
    } catch { }
    Add-Check 'Go：临时库里有词条可用于隔离检查' ($wordID -gt 0) "word_id=$wordID"

    $submitAsAdmin = Invoke-WebRequest -Uri "$apiBase/api/reviews/submit" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ word_id = $wordID; rating = 3; stability = 5; difficulty = 5; interval_days = 5; desired_retention = 0.9 } | ConvertTo-Json)
    Add-Check 'Go：管理员提交一次复习 → 200' ($submitAsAdmin.StatusCode -eq 200) "HTTP $($submitAsAdmin.StatusCode)"

    $submitAsUser = Invoke-WebRequest -Uri "$apiBase/api/reviews/submit" -Method POST -WebSession $userSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ word_id = $wordID; rating = 1; stability = 1; difficulty = 8; interval_days = 0; desired_retention = 0.9 } | ConvertTo-Json)
    Add-Check 'Go：普通用户提交同一个词 → 200（不撞 UNIQUE 约束）' ($submitAsUser.StatusCode -eq 200) "HTTP $($submitAsUser.StatusCode)"

    foreach ($pair in @(
            @{ Session = $adminSession; Who = '管理员' },
            @{ Session = $userSession; Who = '普通用户' })) {
        $stats = $null
        try {
            $statsResp = Invoke-WebRequest -Uri "$apiBase/api/reviews/stats" -WebSession $pair.Session -SkipHttpErrorCheck
            $stats = ($statsResp.Content | ConvertFrom-Json).data
        } catch { }
        $isolated = $null -ne $stats -and [int]$stats.total_reviews -eq 1 -and
                    [int]$stats.today_new -eq 1 -and [int]$stats.reviewed_words -eq 1
        Add-Check "Go：$($pair.Who)只看到自己的 1 条复习记录（P0-1 进度隔离）" $isolated `
            "total_reviews=$($stats.total_reviews) today_new=$($stats.today_new) reviewed_words=$($stats.reviewed_words)"
    }

    # 7) CSRF：写请求带外站 Origin 必须被挡在处理器之前
    $r = Invoke-WebRequest -Uri "$apiBase/api/reviews/submit" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = 'http://evil.example.com' } -SkipHttpErrorCheck `
        -Body '{"word_id":1,"rating":3}'
    Add-Check 'Go：外站 Origin 的写请求 → 403 forbidden（CSRF 闸门）' `
        ($r.StatusCode -eq 403 -and (Get-ErrorCode $r) -eq 'forbidden') `
        "HTTP $($r.StatusCode) error=$(Get-ErrorCode $r)"

    # 8) P0-2：强制邀请码注册。
    #    上面的流程用的是「开放注册」（AUTH_REQUIRE_INVITE 没开），这里**另起一个实例**
    #    把开关打开 —— 因为它只认「环境变量 → 配置」这一条链路，而 Rust 集成测试是直接
    #    改 `cfg.require_invite` 字段的，恰好测不到「环境变量有没有接上」。
    #
    #    ⚠️ 别把开关直接加在上面那个实例上：第 5) 段要注册普通用户，强制邀请码会让它 400。
    $invitePort = $AuthPort + 1
    $inviteBase = "http://127.0.0.1:$invitePort"
    $strictEmail = 'strict-no-invite@example.com'

    $env:AUTH_PORT = "$invitePort"
    $env:AUTH_DB_PATH = Join-Path $work 'invite-auth.db'
    $env:AUTH_REQUIRE_INVITE = 'true'
    Write-Host '启动账号服务（强制邀请码实例）…' -ForegroundColor Cyan
    $inviteProc = Start-Process -FilePath $authBinary -WorkingDirectory (Join-Path $root 'backend-rust') `
        -RedirectStandardOutput (Join-Path $work 'auth-invite.out.log') `
        -RedirectStandardError (Join-Path $work 'auth-invite.err.log') -PassThru
    Remove-Item env:AUTH_REQUIRE_INVITE -ErrorAction SilentlyContinue

    Add-Check '账号服务（强制邀请码实例）健康检查 200' `
        (Wait-Healthy -Url "$inviteBase/api/auth/health" -Seconds 45 -Label '强制邀请码实例') $inviteBase

    # 关键验收：不带邀请码发码 → 400 invalid_invite
    $strictNoInvite = Invoke-WebRequest -Uri "$inviteBase/api/auth/email-code" -Method POST `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ email = $strictEmail } | ConvertTo-Json)
    Add-Check 'P0-2：不带邀请码发码 → 400 invalid_invite' `
        ($strictNoInvite.StatusCode -eq 400 -and (Get-ErrorCode $strictNoInvite) -eq 'invalid_invite') `
        "HTTP $($strictNoInvite.StatusCode) error=$(Get-ErrorCode $strictNoInvite)"

    # 直接查库：这次拒绝**一条 email_codes 都不能留**（「先发码后校验」等于没拦）
    $strictRows = -1
    try {
        $rowCheck = Invoke-WebRequest -Uri "$inviteBase/api/auth/dev/codes?email=$([uri]::EscapeDataString($strictEmail))" `
            -SkipHttpErrorCheck
        $body = $rowCheck.Content | ConvertFrom-Json
        # 开发接口查不到就是「没写过」；查得到说明码已经落库了
        if ($null -ne $body.data -and $null -ne $body.data.code) { $strictRows = 1 } else { $strictRows = 0 }
    } catch { }
    Add-Check 'P0-2：被拒的发码请求没有留下任何验证码（没写库）' ($strictRows -eq 0) `
        "库里该邮箱的验证码条数=$strictRows"

    # 同一个实例上，没码注册也必须被拦（两道门都要有）
    $strictRegister = Invoke-WebRequest -Uri "$inviteBase/api/auth/register" -Method POST `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ email = $strictEmail; email_code = '000000'; password = $userPassword } | ConvertTo-Json)
    Add-Check 'P0-2：不带邀请码注册 → 被拒（不是 invalid_code 也要是 400）' `
        ($strictRegister.StatusCode -eq 400) `
        "HTTP $($strictRegister.StatusCode) error=$(Get-ErrorCode $strictRegister)"
    # 9) P1 管理面板的后端（看板 / 邀请码 / 用户 / 审计）。
    #    ⚠️ 一律用**显式地址**打两个实例：第 8) 段把 $env:AUTH_PORT / AUTH_DB_PATH 改成了
    #       「强制邀请码」那个实例、且没有还原，靠环境变量拼地址会打到那边去。
    #    ⚠️ 这一段必须放在最后：f) 项会把普通用户的会话全部吊销，
    #       之后再拿 $userSession 打任何需要登录的接口都会 401。
    Write-Host ''
    Write-Host '9) P1 管理面板后端（看板 / 邀请码 / 用户 / 审计）' -ForegroundColor Cyan

    # a) 公开开关：匿名可调，只回一个布尔 —— 注册表单靠它决定「邀请码是不是必填」
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/config" -SkipHttpErrorCheck
    $requireInvite = $null
    try { $requireInvite = ($r.Content | ConvertFrom-Json).data.require_invite } catch { }
    Add-Check 'P1：GET /api/auth/config 匿名可调且 require_invite=false（本实例是开放注册）' `
        ($r.StatusCode -eq 200 -and $requireInvite -eq $false) `
        "HTTP $($r.StatusCode) require_invite=$requireInvite"

    # b) 审计：匿名与普通用户都进不去；超管能看，而且**动作清单跟着列表一起回**
    #    （前端的下拉直接吃这个数组，不硬编码、也不多发一个请求）
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/audit" -SkipHttpErrorCheck
    Add-Check 'P1：匿名读审计 → 401 unauthenticated' `
        ($r.StatusCode -eq 401 -and (Get-ErrorCode $r) -eq 'unauthenticated') "HTTP $($r.StatusCode)"

    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/audit" -WebSession $userSession -SkipHttpErrorCheck
    Add-Check 'P1：普通用户读审计 → 403 forbidden' `
        ($r.StatusCode -eq 403 -and (Get-ErrorCode $r) -eq 'forbidden') "HTTP $($r.StatusCode)"

    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/audit?page=1&size=20" -WebSession $adminSession -SkipHttpErrorCheck
    $auditActions = @()
    $auditTotal = 0
    try {
        $auditData = ($r.Content | ConvertFrom-Json).data
        $auditActions = @($auditData.actions)
        $auditTotal = [int]$auditData.total
    } catch { }
    Add-Check 'P1：超管读审计 → 200，actions 里有这次联调留下的 login_ok' `
        ($r.StatusCode -eq 200 -and $auditTotal -ge 1 -and $auditActions -contains 'login_ok') `
        "HTTP $($r.StatusCode) total=$auditTotal actions=$($auditActions -join ',')"

    # 时间筛选：纯日期当「当天 00:00 UTC」，所以今天这一次联调必然落在范围内
    $utcToday = (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd')
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/audit?from=$utcToday&size=5" -WebSession $adminSession -SkipHttpErrorCheck
    $todayTotal = -1
    try { $todayTotal = [int](($r.Content | ConvertFrom-Json).data.total) } catch { }
    Add-Check 'P1：审计支持 from=YYYY-MM-DD（纯日期按 UTC 当天零点算）' `
        ($r.StatusCode -eq 200 -and $todayTotal -ge 1) "HTTP $($r.StatusCode) from=$utcToday total=$todayTotal"

    # c) 邀请码面板：按批停用（幂等）
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invites" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ count = 2; max_uses = 1; expires_in_days = 7; note = 'P1 联调：整批停用'; grant_role = 'user' } | ConvertTo-Json)
    $batchID = ''
    $batchIDs = @()
    try {
        $batchItems = @(($r.Content | ConvertFrom-Json).data.items)
        $batchID = [string]$batchItems[0].batch_id
        $batchIDs = @($batchItems | ForEach-Object { [int]$_.id })
    } catch { }
    Add-Check 'P1：一次生成 2 张码 → 200 且同属一个批次（batch_id 非空）' `
        ($r.StatusCode -eq 200 -and $batchIDs.Count -eq 2 -and $batchID -ne '') `
        "HTTP $($r.StatusCode) batch=$batchID ids=$($batchIDs -join ',')"

    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invite-batches/$batchID/disable" -Method POST `
        -WebSession $adminSession -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } `
        -SkipHttpErrorCheck -Body '{}'
    $disabled = -1
    try { $disabled = [int](($r.Content | ConvertFrom-Json).data.disabled) } catch { }
    Add-Check 'P1：按批停用 → 一次停掉 2 张' ($r.StatusCode -eq 200 -and $disabled -eq 2) `
        "HTTP $($r.StatusCode) disabled=$disabled"

    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invite-batches/$batchID/disable" -Method POST `
        -WebSession $adminSession -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } `
        -SkipHttpErrorCheck -Body '{}'
    $disabledAgain = -1
    try { $disabledAgain = [int](($r.Content | ConvertFrom-Json).data.disabled) } catch { }
    Add-Check 'P1：同一批再停一次 → 0（幂等，且不报错）' `
        ($r.StatusCode -eq 200 -and $disabledAgain -eq 0) "HTTP $($r.StatusCode) disabled=$disabledAgain"

    # d) 整批发邮件：选中的码按顺序与邮箱一对一配对。⚠️ 用**没被停用**的码 ——
    #    上面那一批刚停掉，所以这里重新发一批（停用的码服务端会逐条报错，不该拿它做正例）。
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invites" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ count = 2; note = 'P1 联调：整批发邮件'; grant_role = 'user' } | ConvertTo-Json)
    $mailIDs = @()
    try { $mailIDs = @(($r.Content | ConvertFrom-Json).data.items | ForEach-Object { [int]$_.id }) } catch { }
    $pairs = @(
        @{ invite_id = $mailIDs[0]; email = "p1-mail-1-$stamp@example.com" },
        @{ invite_id = $mailIDs[1]; email = "p1-mail-2-$stamp@example.com" }
    )
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invite-mail" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ pairs = $pairs } | ConvertTo-Json -Depth 5)
    $sent = -1
    $failed = -1
    $mailMode = ''
    try {
        $mailData = ($r.Content | ConvertFrom-Json).data
        $sent = [int]$mailData.sent
        $failed = [int]$mailData.failed
        $mailMode = [string]$mailData.mail_mode
    } catch { }
    Add-Check 'P1：整批发邮件（AUTH_MAIL_MODE=log）→ sent=2 failed=0' `
        ($r.StatusCode -eq 200 -and $sent -eq 2 -and $failed -eq 0 -and $mailMode -eq 'log') `
        "HTTP $($r.StatusCode) sent=$sent failed=$failed mail_mode=$mailMode"

    # e) 用户管理：keyword 搜索 + 完整邮箱 + 最后登录时间；进度来自**另一个库**（Go 聚合）
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/users?keyword=$([uri]::EscapeDataString($email))" `
        -WebSession $adminSession -SkipHttpErrorCheck
    $userID = 0
    $masked = $null
    $hasLastLoginField = $false
    try {
        $listed = ($r.Content | ConvertFrom-Json).data
        $masked = $listed.email_masked
        $firstUser = @($listed.items)[0]
        $userID = [int]$firstUser.id
        # ⚠️ 刚注册完的用户 last_login_at 是**空的**：注册时的自动登录走的是 register 那条路，
        #    不更新 last_login_at。所以这里只断言「字段在」，它真的会有值由下面管理员那一行证明。
        $hasLastLoginField = $firstUser.PSObject.Properties.Name -contains 'last_login_at'
    } catch { }
    Add-Check 'P1：用户列表支持 keyword 搜索；超管拿到完整邮箱（email_masked=false）' `
        ($r.StatusCode -eq 200 -and $userID -gt 0 -and $masked -eq $false -and $hasLastLoginField) `
        "HTTP $($r.StatusCode) user_id=$userID email_masked=$masked last_login_at 字段在=$hasLastLoginField"

    # 管理员自己刚登录过 → 他那行必须有 last_login_at（证明这个字段真会被写，而不是永远空着）
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/users?keyword=$([uri]::EscapeDataString($adminEmail))" `
        -WebSession $adminSession -SkipHttpErrorCheck
    $adminLastLogin = ''
    try { $adminLastLogin = [string](@((($r.Content | ConvertFrom-Json).data.items))[0].last_login_at) } catch { }
    Add-Check 'P1：登录过的管理员那行 last_login_at 有值' `
        ($r.StatusCode -eq 200 -and $adminLastLogin -ne '') `
        "HTTP $($r.StatusCode) last_login_at=$adminLastLogin"

    $r = Invoke-WebRequest -Uri "$apiBase/api/admin/users/progress?ids=$userID" -WebSession $adminSession -SkipHttpErrorCheck
    $progCount = 0
    try { $progCount = @((($r.Content | ConvertFrom-Json).data.items)).Count } catch { }
    Add-Check 'P1：Go 批量进度 /api/admin/users/progress → 200 且带得回这个用户' `
        ($r.StatusCode -eq 200 -and $progCount -ge 1) "HTTP $($r.StatusCode) items=$progCount"

    $r = Invoke-WebRequest -Uri "$apiBase/api/admin/users/$userID/progress" -WebSession $adminSession -SkipHttpErrorCheck
    $last7 = 0
    try { $last7 = @((($r.Content | ConvertFrom-Json).data.last7)).Count } catch { }
    Add-Check 'P1：Go 单人进度 /api/admin/users/:id/progress → 200 且 last7 正好 7 天' `
        ($r.StatusCode -eq 200 -and $last7 -eq 7) "HTTP $($r.StatusCode) last7=$last7"

    # f) 看板：一个请求出全部数字（账号侧读 auth.db，复习侧读 guangxue.db）
    $r = Invoke-WebRequest -Uri "$apiBase/api/admin/stats/overview" -SkipHttpErrorCheck
    Add-Check 'P1：匿名读看板 → 401 unauthenticated' `
        ($r.StatusCode -eq 401 -and (Get-ErrorCode $r) -eq 'unauthenticated') "HTTP $($r.StatusCode)"

    $r = Invoke-WebRequest -Uri "$apiBase/api/admin/stats/overview" -WebSession $adminSession -SkipHttpErrorCheck
    $authAvail = $null
    $totUsers = -1
    $todayReviews = -1
    try {
        $ov = ($r.Content | ConvertFrom-Json).data
        $authAvail = $ov.auth_db.available
        $totUsers = [int]$ov.totals.users
        $todayReviews = [int]$ov.today.reviews
    } catch { }
    # 这个实例的 AUTH_DB_PATH 与账号服务指向**同一个**临时库（见上面起 Go 之前那一段），
    # 所以账号侧必须数得出那 2 个用户（种子超管 + 注册用户）；复习侧今天有管理员与用户各 1 条。
    Add-Check 'P1：看板聚合端点 → 200，账号侧读得到（2 个用户）、今天有 2 条复习' `
        ($r.StatusCode -eq 200 -and $authAvail -eq $true -and $totUsers -eq 2 -and $todayReviews -ge 2) `
        "HTTP $($r.StatusCode) auth_db.available=$authAvail users=$totUsers today.reviews=$todayReviews"

    $r = Invoke-WebRequest -Uri "$apiBase/api/admin/stats/trend?days=7" -WebSession $adminSession -SkipHttpErrorCheck
    $points = 0
    $lastDate = ''
    try {
        $tr = ($r.Content | ConvertFrom-Json).data
        $points = @($tr.points).Count
        $lastDate = [string]$tr.points[-1].date
    } catch { }
    # 末点必须是「北京时间的今天」：它与 UTC 今天在 16:00 之后同一天，这里用 UTC 今天比对
    # 会有 8 小时的窗口差 —— 临时实例刚建，两端都在同一天内，够用了。
    Add-Check 'P1：趋势端点 days=7 → 7 个点，末点是今天（缺失的天补 0）' `
        ($r.StatusCode -eq 200 -and $points -eq 7 -and $lastDate -in @($utcToday, (Get-Date).ToString('yyyy-MM-dd'))) `
        "HTTP $($r.StatusCode) points=$points 末点=$lastDate"

    # g) 强制下线：超管吊销某人的全部会话，那台设备的 Cookie 立刻失效（跨服务也认这一判定）
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/users/$userID/logout-all" -Method POST `
        -WebSession $adminSession -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } `
        -SkipHttpErrorCheck -Body '{}'
    $revoked = -1
    try { $revoked = [int](($r.Content | ConvertFrom-Json).data.revoked) } catch { }
    Add-Check 'P1：超管强制下线 → 200 且至少吊销 1 个会话' `
        ($r.StatusCode -eq 200 -and $revoked -ge 1) "HTTP $($r.StatusCode) revoked=$revoked"

    $r = Invoke-WebRequest -Uri "$authBase/api/auth/me" -WebSession $userSession -SkipHttpErrorCheck
    Add-Check 'P1：被强制下线的用户再打账号服务 → 401（会话真的没了）' `
        ($r.StatusCode -eq 401 -and (Get-ErrorCode $r) -eq 'unauthenticated') `
        "HTTP $($r.StatusCode) error=$(Get-ErrorCode $r)"

    # ⚠️ 但同一个 Cookie 打 **Go** 仍然 200：Go 是本地验签（共享密钥 + HS256），**不查库**，
    #    所以它感知不到会话被吊销 —— access 令牌到期前（默认 900 秒）还会被接受。
    #    这是阶段 2 就写进文档的已知限制（会话撤销要等 roadmap 的阶段 3），
    #    这里把它**显式测出来**，免得以后有人以为「强制下线 = 立刻全站失效」。
    $r = Invoke-WebRequest -Uri "$apiBase/api/reviews/stats" -WebSession $userSession -SkipHttpErrorCheck
    Add-Check 'P1：同一 Cookie 打 Go 仍 200（已知限制：Go 本地验签不查会话撤销，令牌到期前有效）' `
        ($r.StatusCode -eq 200) "HTTP $($r.StatusCode)"

    # h) 「重新启用已用过的码」（面板上的「重新启用」+ 风险确认）：
    #    先把一张单人码**真用掉**（注册一个人），再清零 —— 它必须又变成可兑换的。
    #    ⚠️ 放在这一段末尾：它会多注册两个人，而上面看板那几条在数「账号侧有几个用户」。
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invites" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ count = 1; max_uses = 1; expires_in_days = 7; note = 'P1 联调：重新启用'; grant_role = 'user' } | ConvertTo-Json)
    $resetID = 0
    $resetCode = ''
    try {
        $resetItem = @(($r.Content | ConvertFrom-Json).data.items)[0]
        $resetID = [int]$resetItem.id
        $resetCode = [string]$resetItem.code
    } catch { }

    # 取一个邮箱验证码的小工具（这一步在脚本里出现了三次，写法保持一致）
    function Get-DevCode($mail, $session, $invite) {
        $body = @{ email = $mail }
        if ($invite) { $body.invite_code = $invite }
        $null = Invoke-WebRequest -Uri "$authBase/api/auth/email-code" -Method POST -WebSession $session `
            -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
            -Body ($body | ConvertTo-Json)
        $resp = Invoke-WebRequest -Uri "$authBase/api/auth/dev/codes?email=$([uri]::EscapeDataString($mail))" -SkipHttpErrorCheck
        try { return [string](($resp.Content | ConvertFrom-Json).data.code) } catch { return '' }
    }

    $resetEmail1 = "p1-reset-1-$stamp@example.com"
    $s1 = New-Object Microsoft.PowerShell.Commands.WebRequestSession
    $c1 = Get-DevCode $resetEmail1 $s1 $resetCode
    $reg1 = Invoke-WebRequest -Uri "$authBase/api/auth/register" -Method POST -WebSession $s1 `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ email = $resetEmail1; email_code = $c1; password = $userPassword; invite_code = $resetCode } | ConvertTo-Json)
    Add-Check 'P1：用这张单人码注册第一个人 → 200（码随即用尽）' `
        ($reg1.StatusCode -eq 200) "HTTP $($reg1.StatusCode) error=$(Get-ErrorCode $reg1)"

    $resetEmail2 = "p1-reset-2-$stamp@example.com"
    $s2 = New-Object Microsoft.PowerShell.Commands.WebRequestSession
    $c2 = Get-DevCode $resetEmail2 $s2 ''
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/register" -Method POST -WebSession $s2 `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ email = $resetEmail2; email_code = $c2; password = $userPassword; invite_code = $resetCode } | ConvertTo-Json)
    Add-Check 'P1：码用尽后第二个人被拒 → 400 invite_exhausted' `
        ($r.StatusCode -eq 400 -and (Get-ErrorCode $r) -eq 'invite_exhausted') `
        "HTTP $($r.StatusCode) error=$(Get-ErrorCode $r)"

    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invites/$resetID/reset" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck -Body '{}'
    $cleared = -1
    try { $cleared = [int](($r.Content | ConvertFrom-Json).data.cleared) } catch { }
    Add-Check 'P1：重新启用已用过的码 → 200 且 cleared=1（只清零，兑换记录不删）' `
        ($r.StatusCode -eq 200 -and $cleared -eq 1) "HTTP $($r.StatusCode) cleared=$cleared"

    # ⚠️ 幂等这一条必须紧跟在刚清零之后：此刻 `used_count` 还是 0，再重置一次才应当是
    # cleared=0。**不能**把它放到下面那次注册之后 —— 那时码又被用掉了一次（used_count=1），
    # 再重置会老老实实清掉那一次并回 cleared=1（那是「有效」而不是「不幂等」）。
    # `cleared` 的口径是「本次清掉了几次使用记录」，不是「这张码是不是第一次被重置」。
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invites/$resetID/reset" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck -Body '{}'
    $clearedAgain = -1
    try { $clearedAgain = [int](($r.Content | ConvertFrom-Json).data.cleared) } catch { }
    Add-Check 'P1：刚清零后再重置一次 → cleared=0（幂等，不报错）' `
        ($r.StatusCode -eq 200 -and $clearedAgain -eq 0) "HTTP $($r.StatusCode) cleared=$clearedAgain"

    # 清零之后第 2 个人必须能注册进去（重新拿一次验证码：上一次那枚可能已被消费）
    $c2b = Get-DevCode $resetEmail2 $s2 ''
    $reg2 = Invoke-WebRequest -Uri "$authBase/api/auth/register" -Method POST -WebSession $s2 `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ email = $resetEmail2; email_code = $c2b; password = $userPassword; invite_code = $resetCode } | ConvertTo-Json)
    Add-Check 'P1：清零后同一个人再注册 → 200（码真的又能用了）' `
        ($reg2.StatusCode -eq 200) "HTTP $($reg2.StatusCode) error=$(Get-ErrorCode $reg2)"

    # 补一条：被别人重新用掉之后，重置应当**再次**生效（cleared=1）——
    # 这正是「一码多用」场景下超管要反复用的路径，与上面那条幂等用例互为对照
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invites/$resetID/reset" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck -Body '{}'
    $clearedReuse = -1
    try { $clearedReuse = [int](($r.Content | ConvertFrom-Json).data.cleared) } catch { }
    Add-Check 'P1：被别人重新用掉后再重置 → cleared=1（重置可反复用，不是一次性）' `
        ($r.StatusCode -eq 200 -and $clearedReuse -eq 1) "HTTP $($r.StatusCode) cleared=$clearedReuse"

    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invites/999999/reset" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck -Body '{}'
    Add-Check 'P1：重置不存在的码 → 404' ($r.StatusCode -eq 404) "HTTP $($r.StatusCode)"

    # i) 自定义邀请码（超管自己指定一串码）+ 永不过期：
    #    2026-10-05 定的用法就是「自己想一个 16 位的码，不过期但只能用一次」。
    #    这一段走完全程：格式挡 → 建 → 真注册 → 撞码 409 → 确认沿用 → 再用掉。
    #    ⚠️ 拼码时脚本自己也要守「正好 16 位」这条规则（8 + 7 + 1 = 16）
    $customCode = 'P1CUSTOM' + (Get-Random -Minimum 1000000 -Maximum 9999999) + 'X'
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invites" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ custom_code = 'SHORT'; max_uses = 1; expires_in_days = 0 } | ConvertTo-Json)
    $shortMsg = ''
    try { $shortMsg = [string]($r.Content | ConvertFrom-Json).message } catch { }
    Add-Check 'P1：自定义码位数不对 → 400 invalid_params，且文案说清要几位' `
        ($r.StatusCode -eq 400 -and (Get-ErrorCode $r) -eq 'invalid_params' -and $shortMsg -match '16') `
        "HTTP $($r.StatusCode) error=$(Get-ErrorCode $r) msg=$shortMsg"

    # 小写 + 手写分组一起进：验证「转大写 + 抹掉 -」（这条规则最容易被前后端写出分歧）
    $grouped = ($customCode.Substring(0, 4) + '-' + $customCode.Substring(4, 4) + '-' + $customCode.Substring(8, 4) + '-' + $customCode.Substring(12)).ToLower()
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invites" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ custom_code = $grouped; max_uses = 1; expires_in_days = 0; note = 'P1 联调：自定义码' } | ConvertTo-Json)
    $customID = 0
    $customBack = ''
    $customNever = 'x'
    try {
        $customItem = @(($r.Content | ConvertFrom-Json).data.items)[0]
        $customID = [int]$customItem.id
        $customBack = [string]$customItem.code
        $customNever = if ($null -eq $customItem.expires_at) { 'null' } else { [string]$customItem.expires_at }
    } catch { }
    Add-Check 'P1：自定义码建得出来、转大写并抹掉了手写的 -、且 0 天 = 永不过期' `
        ($r.StatusCode -eq 200 -and $customBack -eq $customCode -and $customNever -eq 'null') `
        "HTTP $($r.StatusCode) 回执=$customBack 期望=$customCode expires_at=$customNever"

    $customEmail = "p1-custom-$stamp@example.com"
    $s3 = New-Object Microsoft.PowerShell.Commands.WebRequestSession
    $c3 = Get-DevCode $customEmail $s3 $customCode
    $reg3 = Invoke-WebRequest -Uri "$authBase/api/auth/register" -Method POST -WebSession $s3 `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ email = $customEmail; email_code = $c3; password = $userPassword; invite_code = $customCode } | ConvertTo-Json)
    Add-Check 'P1：用这张自定义码真注册一个人 → 200（永不过期不等于不能用）' `
        ($reg3.StatusCode -eq 200) "HTTP $($reg3.StatusCode) error=$(Get-ErrorCode $reg3)"

    # 已经被用掉的码再拿同一个串来建：必须 409，并把那张码的现状带回来（面板要拿它弹确认框）
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invites" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ custom_code = $customCode; max_uses = 1; expires_in_days = 0 } | ConvertTo-Json)
    $takenStatus = ''
    $takenWho = ''
    try {
        $taken = ($r.Content | ConvertFrom-Json)
        $takenStatus = [string]$taken.data.existing.status
        $takenWho = [string]@($taken.data.existing.uses)[0].email
    } catch { }
    Add-Check 'P1：同一个自定义码再建 → 409 invite_code_taken，并带回那张码的状态与谁用过' `
        ($r.StatusCode -eq 409 -and (Get-ErrorCode $r) -eq 'invite_code_taken' -and $takenStatus -eq 'used' -and $takenWho -eq $customEmail) `
        "HTTP $($r.StatusCode) error=$(Get-ErrorCode $r) status=$takenStatus 用过的人=$takenWho"

    # 确认沿用：额度改 2、仍然不过期；**不动**已用次数与兑换记录
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/admin/invites" -Method POST -WebSession $adminSession `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ custom_code = $customCode; max_uses = 2; expires_in_days = 0; allow_existing = $true } | ConvertTo-Json)
    $reusedFlag = $false
    $reusedUsed = -1
    $reusedMax = -1
    try {
        $reusedJson = ($r.Content | ConvertFrom-Json)
        $reusedFlag = [bool]$reusedJson.data.reused
        $reusedUsed = [int]$reusedJson.data.items[0].used_count
        $reusedMax = [int]$reusedJson.data.items[0].max_uses
    } catch { }
    Add-Check 'P1：确认沿用它 → 200 reused=true，额度改成 2，已用次数（1）与兑换记录都还在' `
        ($r.StatusCode -eq 200 -and $reusedFlag -and $reusedUsed -eq 1 -and $reusedMax -eq 2) `
        "HTTP $($r.StatusCode) reused=$reusedFlag used=$reusedUsed max=$reusedMax"

    $customEmail2 = "p1-custom2-$stamp@example.com"
    $s4 = New-Object Microsoft.PowerShell.Commands.WebRequestSession
    $c4 = Get-DevCode $customEmail2 $s4 $customCode
    $reg4 = Invoke-WebRequest -Uri "$authBase/api/auth/register" -Method POST -WebSession $s4 `
        -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
        -Body (@{ email = $customEmail2; email_code = $c4; password = $userPassword; invite_code = $customCode } | ConvertTo-Json)
    Add-Check 'P1：沿用之后额度过来了 → 第二个人用同一张码注册 200' `
        ($reg4.StatusCode -eq 200) "HTTP $($reg4.StatusCode) error=$(Get-ErrorCode $reg4)"

    # j) 三份样式表都要声明「只支持浅色」（`color-scheme: light`）：
    #    手机深色模式 / 浏览器「自动深色」会把没声明的浅色页面**自动反色**，中文一压就发虚
    #    ——2026-10-05 用户报的「这里字有问题」就是这个（见 docs/boundaries.md）。
    #    ⚠️ 直接读仓库里的文件而不是打 HTTP：本脚本不该依赖 dev-server（8899）有没有在跑。
    foreach ($pair in @(
            @{ File = 'main.css'; Who = '学生站' },
            @{ File = 'account/account.css'; Who = '个人中心' },
            @{ File = 'admin/admin.css'; Who = '后台' })) {
        $path = Join-Path $root ($pair.File -replace '/', '\')
        $text = if (Test-Path $path) { Get-Content $path -Raw } else { '' }
        $light = ($text -match 'color-scheme:\s*light')
        Add-Check "样式：$($pair.Who) 的样式表声明了 color-scheme: light（挡住浏览器自动深色反色）" `
            $light "$($pair.File) 含 color-scheme: light=$light"
    }

    # ------------------------------------------------------------------------
    # 10) P2：深度健康检查 + 限流真的可配 + 监控命令
    #
    # 放在最后：这一段会另起两个实例、跑监控命令，还会把「登录」额度用掉
    # （AUTH_RL_LOGIN_IP 实例上故意打满），前面那些用例都还要正常登录。
    # ------------------------------------------------------------------------

    # a) 深度健康：浅检只证明「进程活着」，深检要真查一次库 —— 不健康时回 503
    $r = Invoke-WebRequest -Uri "$authBase/api/auth/health?deep=1" -SkipHttpErrorCheck
    $deepAuth = $null
    try { $deepAuth = ($r.Content | ConvertFrom-Json).data } catch { }
    $authDbState = if ($deepAuth) { [string]$deepAuth.deep.database } else { '' }
    $authMig = if ($deepAuth) { [int]$deepAuth.deep.migration_version } else { -1 }
    $authUp = if ($deepAuth) { [long]$deepAuth.deep.uptime_seconds } else { -1 }
    Add-Check 'P2：账号服务深度健康 → 200 且 database=ok、有迁移版本与运行时长' `
        ($r.StatusCode -eq 200 -and $authDbState -eq 'ok' -and $authMig -ge 1 -and $authUp -ge 0) `
        "HTTP $($r.StatusCode) database=$authDbState migration_version=$authMig uptime=${authUp}s"

    $r = Invoke-WebRequest -Uri "$apiBase/api/health?deep=1" -SkipHttpErrorCheck
    $deepGo = $null
    try { $deepGo = ($r.Content | ConvertFrom-Json).deep } catch { }
    $goDbState = if ($deepGo) { [string]$deepGo.database } else { '' }
    $goAuthDb = if ($deepGo) { [bool]$deepGo.auth_db.available } else { $false }
    Add-Check 'P2：Go 深度健康 → 200 且 database=ok、只读账号库可用' `
        ($r.StatusCode -eq 200 -and $goDbState -eq 'ok' -and $goAuthDb) `
        "HTTP $($r.StatusCode) database=$goDbState auth_db.available=$goAuthDb"

    # b) 浅检的形状不能变：Nginx / dev-server 的探活还在用它（浅检不该带 deep 字段）
    $r = Invoke-WebRequest -Uri "$apiBase/api/health" -SkipHttpErrorCheck
    $shallow = $null
    try { $shallow = $r.Content | ConvertFrom-Json } catch { }
    Add-Check 'P2：浅检形状不变（status=ok，且没有 deep 字段）' `
        ($r.StatusCode -eq 200 -and $shallow.status -eq 'ok' -and $null -eq $shallow.deep) `
        "HTTP $($r.StatusCode) status=$($shallow.status) 有 deep 字段=$($null -ne $shallow.deep)"

    # c) AUTH_RL_* 真的接上了配置：另起一个实例把登录限流压到 2/60
    #    ⚠️ 为什么非得起新实例：Rust 的集成测试直接改 cfg 字段，恰好测不到
    #    「环境变量有没有接上」这一环（P2 之前 AUTH_RL_* 根本没人解析，就是这么漏掉的）。
    $rlPort = $AuthPort + 2
    $rlBase = "http://127.0.0.1:$rlPort"
    $env:AUTH_PORT = "$rlPort"
    $env:AUTH_DB_PATH = Join-Path $work 'rl-auth.db'
    $env:AUTH_RL_LOGIN_IP = '2/60'
    Write-Host '启动账号服务（限流实例：AUTH_RL_LOGIN_IP=2/60）…' -ForegroundColor Cyan
    $rlProc = Start-Process -FilePath $authBinary -WorkingDirectory (Join-Path $root 'backend-rust') `
        -RedirectStandardOutput (Join-Path $work 'auth-rl.out.log') `
        -RedirectStandardError (Join-Path $work 'auth-rl.err.log') -PassThru
    Remove-Item env:AUTH_RL_LOGIN_IP -ErrorAction SilentlyContinue
    Add-Check 'P2：限流实例健康检查 200' `
        (Wait-Healthy -Url "$rlBase/api/auth/health" -Seconds 45 -Label '限流实例') $rlBase

    $rlStatus = @()
    for ($i = 1; $i -le 3; $i++) {
        $r = Invoke-WebRequest -Uri "$rlBase/api/auth/login" -Method POST `
            -ContentType 'application/json' -Headers @{ Origin = $allowedOrigin } -SkipHttpErrorCheck `
            -Body (@{ email = 'nobody@example.com'; password = 'definitely-wrong' } | ConvertTo-Json)
        $rlStatus += [int]$r.StatusCode
    }
    Add-Check 'P2：AUTH_RL_LOGIN_IP=2/60 生效 —— 前两次 401、第三次 429' `
        ($rlStatus.Count -eq 3 -and $rlStatus[0] -eq 401 -and $rlStatus[1] -eq 401 -and $rlStatus[2] -eq 429) `
        "三次状态码=$($rlStatus -join ',')"

    # d) 格式写错必须**拒绝启动**并点名变量：静默退回默认值的症状是
    #    「明明放宽了却还被 429」，那种问题最难查
    $badLog = Join-Path $work 'auth-bad-rl.log'
    $env:AUTH_RL_LOGIN_IP = '两百/60'
    $badProc = Start-Process -FilePath $authBinary -WorkingDirectory (Join-Path $root 'backend-rust') `
        -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $badLog -RedirectStandardError "$badLog.err"
    $badProc.WaitForExit(15000) | Out-Null
    Remove-Item env:AUTH_RL_LOGIN_IP -ErrorAction SilentlyContinue
    $badText = ''
    foreach ($f in @($badLog, "$badLog.err")) {
        if (Test-Path $f) { $badText += (Get-Content $f -Raw) }
    }
    $badExited = $badProc.HasExited
    $badCode = if ($badExited) { $badProc.ExitCode } else { -1 }
    if (-not $badExited) { Stop-Process -Id $badProc.Id -Force -ErrorAction SilentlyContinue }
    Add-Check 'P2：AUTH_RL_* 格式写错 → 拒绝启动，且报错点名变量' `
        ($badExited -and $badCode -ne 0 -and $badText -match 'AUTH_RL_LOGIN_IP') `
        "已退出=$badExited exit=$badCode 日志里出现变量名=$([bool]($badText -match 'AUTH_RL_LOGIN_IP'))"

    # e) 监控命令（guangxue-monitor）：健康 → 0；坏掉 → 1 且记下「开始故障」；
    #    持续坏不重复提醒；恢复 → 0 且报告「已恢复」。
    #    ⚠️ MONITOR_ALERT_TO 留空：这一段只验「判定 + 状态机 + 退出码」，
    #    真发信要走真 SMTP（见 runbook 的告警演练），不在这个只读脚本里做。
    $monitorBin = Join-Path $root 'backend-rust\target\debug\guangxue-monitor.exe'
    if (-not (Test-Path $monitorBin)) {
        Add-Check 'P2：监控命令已构建（cargo build 会一起产出）' $false $monitorBin
    } else {
        $monitorState = Join-Path $work 'monitor-state.json'
        $env:MONITOR_GO_URL = "$apiBase/api/health?deep=1"
        $env:MONITOR_AUTH_URL = "$authBase/api/auth/health?deep=1"
        $env:MONITOR_STATE_FILE = $monitorState
        $env:MONITOR_ALERT_TO = ''
        $env:MONITOR_TIMEOUT_SECONDS = '3'

        $out = (& $monitorBin 2>&1 | Out-String)
        $code = $LASTEXITCODE
        Add-Check 'P2：监控命令在全部健康时退出码 0' `
            ($code -eq 0 -and $out -match '✓.*Go 主后端' -and $out -match '✓.*账号服务') `
            "exit=$code"

        $env:MONITOR_AUTH_URL = 'http://127.0.0.1:1/api/auth/health?deep=1'
        $out = (& $monitorBin 2>&1 | Out-String)
        $code = $LASTEXITCODE
        $stateText = if (Test-Path $monitorState) { Get-Content $monitorState -Raw } else { '' }
        Add-Check 'P2：服务坏掉 → 退出码 1，且报告「开始故障」+ 状态文件记下故障' `
            ($code -eq 1 -and $out -match '开始故障' -and $stateText -match '"failing":\s*true') `
            "exit=$code"

        $out = (& $monitorBin 2>&1 | Out-String)
        Add-Check 'P2：持续故障不重复提醒（只在状态变化时发信）' `
            ($LASTEXITCODE -eq 1 -and $out -notmatch '持续') "exit=$LASTEXITCODE"

        $env:MONITOR_AUTH_URL = "$authBase/api/auth/health?deep=1"
        $out = (& $monitorBin 2>&1 | Out-String)
        Add-Check 'P2：恢复正常 → 退出码 0，且报告「已恢复」' `
            ($LASTEXITCODE -eq 0 -and $out -match '已恢复') "exit=$LASTEXITCODE"
    }
}
catch {
    Write-Host ''
    Write-Host "联调中断：$($_.Exception.Message)" -ForegroundColor Red
    $script:checks += [pscustomobject]@{ Name = '联调执行完成'; Ok = $false; Detail = $_.Exception.Message }
}
finally {
    foreach ($proc in @($goProc, $authProc, $inviteProc, $rlProc, $badProc)) {
        if ($proc -and -not $proc.HasExited) {
            try { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue } catch { }
        }
    }
    # 清掉本脚本设置的环境变量，避免污染同一个 PowerShell 会话里的后续命令
    foreach ($name in 'AUTH_DB_PATH', 'AUTH_HOST', 'AUTH_PORT', 'APP_ENV', 'AUTH_JWT_SECRET',
        'AUTH_ALLOWED_ORIGINS', 'AUTH_MAIL_MODE', 'AUTH_DEV_ENDPOINTS', 'AUTH_SEED_ADMIN',
        'AUTH_ADMIN_EMAIL', 'AUTH_ADMIN_PASSWORD', 'AUTH_REQUIRE_INVITE', 'AUTH_RL_LOGIN_IP',
        'MONITOR_GO_URL', 'MONITOR_AUTH_URL', 'MONITOR_STATE_FILE', 'MONITOR_ALERT_TO',
        'MONITOR_TIMEOUT_SECONDS',
        'DB_PATH', 'SERVER_HOST', 'SERVER_PORT') {
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
