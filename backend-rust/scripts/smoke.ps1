<#
  广学 · 账号系统端到端冒烟测试

  用法（在仓库根目录或任意目录执行）：

      # 1) 先起账号服务（另开一个终端）
      #    cd backend-rust; cargo run --release
      # 2) 跑脚本：默认直连账号服务
      pwsh backend-rust/scripts/smoke.ps1
      # 3) 也可以经 dev-server 代理跑（顺便验证分流）
      pwsh backend-rust/scripts/smoke.ps1 -BaseUrl http://127.0.0.1:8899

  前置条件：
    - 账号服务以 development 启动（`/api/auth/dev/codes` 才存在，验证码从它取）；
    - 管理员账号已由服务启动时自动 seed（AUTH_ADMIN_EMAIL / AUTH_ADMIN_PASSWORD）；
    - PowerShell 7+（用到 -SkipHttpErrorCheck）。

  覆盖内容：发码 → 注册 → 多端登录 → /me → 会话列表 → 刷新轮换 → 重放检测 →
            单端登出 → 管理员发邀请码，以及一组失败分支的错误码断言。
  退出码：0 = 全部通过；1 = 有失败项。
#>
[CmdletBinding()]
param(
    [string]$BaseUrl = 'http://127.0.0.1:8081',
    [string]$AdminEmail = '2262997289@qq.com',
    [string]$AdminPassword = '7289HR_RedSun',
    [string]$UserPassword = 'SmokeTest123'
)

$ErrorActionPreference = 'Stop'
$script:Pass = 0
$script:Fail = 0

# 本次运行用一个独立的「来源 IP」与独立的邮箱前缀：
# 服务的发码限流是「同 IP 20 次/小时 + 同邮箱 1 次/分钟」，
# 不隔离的话连续跑几次脚本就会互相干扰（第二次跑必然 429）。
# 这里用的是 TEST-NET-3 网段（203.0.113.0/24），本来就只用于文档与测试。
$script:ClientIp = "203.0.113.$((Get-Random -Minimum 2 -Maximum 254))"
$script:RunId = [guid]::NewGuid().ToString('N').Substring(0, 8)

function Write-Check([string]$Name, [bool]$Ok, [string]$Detail = '') {
    if ($Ok) {
        $script:Pass++
        Write-Host ("  [OK]   " + $Name) -ForegroundColor Green
    } else {
        $script:Fail++
        Write-Host ("  [FAIL] " + $Name + "   " + $Detail) -ForegroundColor Red
    }
}

function New-Session { New-Object Microsoft.PowerShell.Commands.WebRequestSession }

function Invoke-Api {
    param(
        [string]$Method,
        [string]$Path,
        $Body,
        $Session,
        [hashtable]$Headers
    )
    $params = @{
        Uri                = "$BaseUrl$Path"
        Method             = $Method
        SkipHttpErrorCheck = $true
        WebSession         = $Session
        TimeoutSec         = 20
        Headers            = @{ 'X-Forwarded-For' = $script:ClientIp }
    }
    if ($null -ne $Body) {
        $params.Body = ($Body | ConvertTo-Json -Compress)
        $params.ContentType = 'application/json'
    }
    if ($Headers) {
        foreach ($key in $Headers.Keys) { $params.Headers[$key] = $Headers[$key] }
    }

    $response = Invoke-WebRequest @params
    $json = $null
    try { $json = $response.Content | ConvertFrom-Json } catch { }
    [pscustomobject]@{ Status = [int]$response.StatusCode; Json = $json; Raw = $response.Content }
}

function Get-ErrorCode($Result) {
    if ($null -eq $Result.Json) { return '' }
    return [string]$Result.Json.error
}

function Get-CookieNames($Session) {
    # ⚠️ 直接枚举 $Session.Cookies 在 PowerShell 7 里拿不到内容，必须走 GetAllCookies()
    return (($Session.Cookies.GetAllCookies() | ForEach-Object { $_.Name }) -join ',')
}

function Get-CookieValue($Session, [string]$Name) {
    $cookie = $Session.Cookies.GetAllCookies() | Where-Object { $_.Name -eq $Name } | Select-Object -First 1
    if ($null -eq $cookie) { return $null }
    return [string]$cookie.Value
}

Write-Host "广学 · 账号系统冒烟测试" -ForegroundColor Cyan
Write-Host "  目标: $BaseUrl"

# ---------------------------------------------------------------- 0. 健康检查
$health = Invoke-Api -Method GET -Path '/api/auth/health' -Session (New-Session)
if ($health.Status -ne 200) {
    Write-Host "  账号服务不可达（HTTP $($health.Status)）：$($health.Raw)" -ForegroundColor Red
    Write-Host "  请先启动：cd backend-rust; cargo run --release" -ForegroundColor Yellow
    exit 1
}
Write-Check '服务健康检查 200' ($health.Status -eq 200)
if ($health.Json.data.dev_endpoints -ne $true) {
    Write-Host "  ⚠️ 调试接口未开启（APP_ENV 不是 development？），验证码取不到，脚本无法继续" -ForegroundColor Yellow
    exit 1
}

# ---------------------------------------------------------------- 1. 管理员登录 + 发邀请码
$admin = New-Session
$adminLogin = Invoke-Api -Method POST -Path '/api/auth/login' -Body @{ email = $AdminEmail; password = $AdminPassword; device_label = '冒烟脚本' } -Session $admin
Write-Check '管理员登录（role=admin）' ($adminLogin.Status -eq 200 -and $adminLogin.Json.data.user.role -eq 'admin') "HTTP $($adminLogin.Status)"

$inviteResp = Invoke-Api -Method POST -Path '/api/auth/admin/invites' -Body @{ count = 1; max_uses = 1; expires_in_days = 1; note = '冒烟测试' } -Session $admin
$invite = $null
if ($inviteResp.Status -eq 200) { $invite = $inviteResp.Json.data.codes[0] }
Write-Check '管理员可生成邀请码' ($invite -ne $null -and $invite.Length -eq 16) "HTTP $($inviteResp.Status)"

$invite2Resp = Invoke-Api -Method POST -Path '/api/auth/admin/invites' -Body @{ count = 1; max_uses = 5; expires_in_days = 1; note = '冒烟失败分支' } -Session $admin
$invite2 = $invite2Resp.Json.data.codes[0]

# ---------------------------------------------------------------- 2. 注册（邮箱验证码 + 邀请码）
$email = "smoke-$($script:RunId)@example.com"
$deviceA = New-Session

$send = Invoke-Api -Method POST -Path '/api/auth/email-code' -Body @{ email = $email; invite_code = $invite } -Session $deviceA
Write-Check '发送验证码 200' ($send.Status -eq 200) "HTTP $($send.Status) $($send.Raw)"

$devCode = Invoke-Api -Method GET -Path "/api/auth/dev/codes?email=$([uri]::EscapeDataString($email))" -Session $deviceA
$aCode = [string]$devCode.Json.data.code
Write-Check '开发接口可取到验证码' ($devCode.Status -eq 200 -and $aCode.Length -eq 6) "HTTP $($devCode.Status)"

$register = Invoke-Api -Method POST -Path '/api/auth/register' -Body @{ email = $email; email_code = $aCode; invite_code = $invite; password = $UserPassword; username = '冒烟用户' } -Session $deviceA
Write-Check '注册成功（自动登录）' ($register.Status -eq 200 -and $register.Json.data.user.email -eq $email) "HTTP $($register.Status) $($register.Raw)"

$cookieNames = Get-CookieNames $deviceA
Write-Check '拿到 gx_access 与 gx_refresh 两个 Cookie' ($cookieNames -match 'gx_access' -and $cookieNames -match 'gx_refresh') $cookieNames

$me = Invoke-Api -Method GET -Path '/api/auth/me' -Session $deviceA
Write-Check '带会话访问 /me 成功' ($me.Status -eq 200 -and $me.Json.data.user.email -eq $email) "HTTP $($me.Status)"

# ---------------------------------------------------------------- 3. 多端同时登录
$deviceB = New-Session
$loginB = Invoke-Api -Method POST -Path '/api/auth/login' -Body @{ email = $email; password = $UserPassword; device_label = '第二端' } -Session $deviceB
Write-Check '同一账号在第二个端登录成功' ($loginB.Status -eq 200) "HTTP $($loginB.Status)"

$sessions = Invoke-Api -Method GET -Path '/api/auth/sessions' -Session $deviceA
$sessionCount = 0
if ($sessions.Status -eq 200) { $sessionCount = @($sessions.Json.data.items).Count }
Write-Check '会话列表显示两个端' ($sessionCount -eq 2) "实际 $sessionCount"

$meB = Invoke-Api -Method GET -Path '/api/auth/me' -Session $deviceB
Write-Check '第二个端也能访问 /me' ($meB.Status -eq 200) "HTTP $($meB.Status)"

# ---------------------------------------------------------------- 4. refresh 轮换 + 重放检测
$oldRefresh = Get-CookieValue $deviceA 'gx_refresh'
$refresh = Invoke-Api -Method POST -Path '/api/auth/refresh' -Body @{} -Session $deviceA
$newRefresh = Get-CookieValue $deviceA 'gx_refresh'
Write-Check '刷新成功且 refresh 令牌已轮换' ($refresh.Status -eq 200 -and $oldRefresh -and $newRefresh -and $oldRefresh -ne $newRefresh) "HTTP $($refresh.Status)"

$replay = New-Session
$replay.Cookies.Add((New-Object System.Net.Cookie('gx_refresh', $oldRefresh, '/api/auth', ([uri]$BaseUrl).Host)))
$replayResp = Invoke-Api -Method POST -Path '/api/auth/refresh' -Body @{} -Session $replay
Write-Check '旧 refresh 重放被拒（401）' ($replayResp.Status -eq 401 -and (Get-ErrorCode $replayResp) -eq 'unauthenticated') "HTTP $($replayResp.Status) $(Get-ErrorCode $replayResp)"

$meAfterReplay = Invoke-Api -Method GET -Path '/api/auth/me' -Session $deviceA
Write-Check '重放后整条轮换链被吊销（A 端 401）' ($meAfterReplay.Status -eq 401) "HTTP $($meAfterReplay.Status)"

$meB2 = Invoke-Api -Method GET -Path '/api/auth/me' -Session $deviceB
Write-Check '另一端的会话不受影响' ($meB2.Status -eq 200) "HTTP $($meB2.Status)"

# ---------------------------------------------------------------- 5. 单端登出
$logoutB = Invoke-Api -Method POST -Path '/api/auth/logout' -Body @{} -Session $deviceB
$meB3 = Invoke-Api -Method GET -Path '/api/auth/me' -Session $deviceB
Write-Check '登出第二个端后其 /me 变 401' ($logoutB.Status -eq 200 -and $meB3.Status -eq 401) "logout=$($logoutB.Status) me=$($meB3.Status)"

# ---------------------------------------------------------------- 6. 失败分支
$anon = New-Session
$anonMe = Invoke-Api -Method GET -Path '/api/auth/me' -Session $anon
Write-Check '未登录访问 /me → 401 unauthenticated' ($anonMe.Status -eq 401 -and (Get-ErrorCode $anonMe) -eq 'unauthenticated') "HTTP $($anonMe.Status)"

$badInvite = Invoke-Api -Method POST -Path '/api/auth/email-code' -Body @{ email = "bad-invite-$($script:RunId)@example.com"; invite_code = 'ZZZZZZZZZZZZZZZZ' } -Session (New-Session)
Write-Check '无效邀请码 → 400 invalid_invite' ($badInvite.Status -eq 400 -and (Get-ErrorCode $badInvite) -eq 'invalid_invite') "HTTP $($badInvite.Status) $(Get-ErrorCode $badInvite)"

$usedInvite = Invoke-Api -Method POST -Path '/api/auth/email-code' -Body @{ email = "used-invite-$($script:RunId)@example.com"; invite_code = $invite } -Session (New-Session)
Write-Check '用尽的邀请码 → 400 invite_exhausted' ($usedInvite.Status -eq 400 -and (Get-ErrorCode $usedInvite) -eq 'invite_exhausted') "HTTP $($usedInvite.Status) $(Get-ErrorCode $usedInvite)"

$dupEmail = Invoke-Api -Method POST -Path '/api/auth/email-code' -Body @{ email = $email; invite_code = $invite2 } -Session (New-Session)
if ($dupEmail.Status -eq 429 -and (Get-ErrorCode $dupEmail) -eq 'rate_limited') {
    # 同一邮箱 60 秒内只能发一次码，而「发码限流」刻意排在「邮箱已注册」之前，
    # 所以脚本紧接着重发同一个邮箱会先撞下限流。这条分支的确定性覆盖在
    # tests/auth_flow.rs::duplicate_email_is_rejected（那里不受 60 秒窗口影响）。
    Write-Host "  [SKIP] 重复邮箱 → 409 email_taken（被同邮箱 60 秒发码限流先挡住，集成测试已覆盖）" -ForegroundColor Yellow
} else {
    Write-Check '重复邮箱 → 409 email_taken' ($dupEmail.Status -eq 409 -and (Get-ErrorCode $dupEmail) -eq 'email_taken') "HTTP $($dupEmail.Status) $(Get-ErrorCode $dupEmail)"
}

$wrongCode = Invoke-Api -Method POST -Path '/api/auth/register' -Body @{ email = "no-code-$($script:RunId)@example.com"; email_code = '000000'; invite_code = $invite2; password = $UserPassword } -Session (New-Session)
Write-Check '没有验证码 → 400 invalid_code' ($wrongCode.Status -eq 400 -and (Get-ErrorCode $wrongCode) -eq 'invalid_code') "HTTP $($wrongCode.Status) $(Get-ErrorCode $wrongCode)"

$wrongPwd = Invoke-Api -Method POST -Path '/api/auth/login' -Body @{ email = $email; password = 'WrongPass123' } -Session (New-Session)
Write-Check '密码错误 → 401 bad_credentials' ($wrongPwd.Status -eq 401 -and (Get-ErrorCode $wrongPwd) -eq 'bad_credentials') "HTTP $($wrongPwd.Status) $(Get-ErrorCode $wrongPwd)"

$unknown = Invoke-Api -Method POST -Path '/api/auth/login' -Body @{ email = "not-registered-$($script:RunId)@example.com"; password = $UserPassword } -Session (New-Session)
Write-Check '未注册邮箱 → 同样的 401 文案' ($unknown.Status -eq 401 -and $unknown.Json.message -eq $wrongPwd.Json.message) "HTTP $($unknown.Status)"

$crossOrigin = Invoke-Api -Method POST -Path '/api/auth/login' -Body @{ email = $email; password = $UserPassword } -Session (New-Session) -Headers @{ Origin = 'http://evil.example' }
Write-Check '跨站 Origin → 403 forbidden（CSRF 防护）' ($crossOrigin.Status -eq 403 -and (Get-ErrorCode $crossOrigin) -eq 'forbidden') "HTTP $($crossOrigin.Status)"

$badToken = New-Session
$badToken.Cookies.Add((New-Object System.Net.Cookie('gx_access', 'not-a-real-token', '/', ([uri]$BaseUrl).Host)))
$badTokenResp = Invoke-Api -Method GET -Path '/api/auth/me' -Session $badToken
Write-Check '伪造 access 令牌 → 401' ($badTokenResp.Status -eq 401) "HTTP $($badTokenResp.Status)"

# ---------------------------------------------------------------- 汇总
Write-Host ""
Write-Host ("通过 {0} 项，失败 {1} 项" -f $script:Pass, $script:Fail) -ForegroundColor ($(if ($script:Fail -eq 0) { 'Green' } else { 'Red' }))
if ($script:Fail -gt 0) { exit 1 }
exit 0
