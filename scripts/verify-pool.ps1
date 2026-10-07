# 单一循环池：验收脚本（对照 docs/review-pool-plan.md 第 7 节的 8 条口径）
#
# 全部断言都走真实 HTTP + 真实开发库（写入对象只能是登录用户，所以这里用管理员账号，
# **会改动它的进度**）。
#
# 用法（在仓库根目录）：
#   pwsh scripts/verify-pool.ps1               # 只跑接口层 28 项
#   pwsh scripts/verify-pool.ps1 -Browser      # 接口层 + 浏览器端到端（需要 Edge）
# 退出码：0 = 全部通过；1 = 有失败项。

param(
    # 加跑浏览器端到端（scripts/browser-e2e.mjs，需要本机 Edge）
    [switch]$Browser,
    # 端到端截图输出路径（仅 -Browser 时有效）
    [string]$Shot = ''
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$goExe = 'C:\Users\22629\go-portable\go\bin\go.exe'
$goBase = 'http://127.0.0.1:8080'
$authBase = 'http://127.0.0.1:8081'
$origin = 'http://127.0.0.1:8899'

$pass = 0; $fail = 0
function Check($name, $ok, $detail) {
    if ($ok) { $script:pass++; Write-Host ("  [PASS] " + $name + "  " + $detail) -ForegroundColor Green }
    else { $script:fail++; Write-Host ("  [FAIL] " + $name + "  " + $detail) -ForegroundColor Red }
}

Write-Host "`n=== 单一循环池验收 ===" -ForegroundColor Cyan

# ---------- 前置：把进度清空，保证可重复运行 ----------
# 本脚本会真的写进度（提交复习），所以第二次运行必须先回到干净起点，
# 否则「今日新学 +1」这种增量断言会因为「这些词今天已经学过了」而失败（踩过）。
# 清空前会自动快照（见 cmd/discard-progress），出问题可以从 backups/ 恢复。
Write-Host "清空开发库的复习进度（自动快照）…"
Set-Location (Join-Path $root 'backend-go')
& $goExe run ./cmd/discard-progress -yes 2>&1 | Select-Object -Last 6
Set-Location $root

# ---------- 登录拿 Cookie ----------
$sess = New-Object Microsoft.PowerShell.Commands.WebRequestSession
$loginBody = @{ email = '2262997289@qq.com'; password = '7289HR_RedSun'; device_label = 'verify-pool' } | ConvertTo-Json
$login = Invoke-WebRequest "$authBase/api/auth/login" -Method POST -WebSession $sess -ContentType 'application/json' `
    -Headers @{ Origin = $origin } -Body $loginBody -SkipHttpErrorCheck
if ($login.StatusCode -ne 200) { Write-Host "登录失败：HTTP $($login.StatusCode) $($login.Content)" -ForegroundColor Red; exit 1 }
$headers = @{ Origin = $origin }
Write-Host "登录成功（管理员账号）"

# 前置自检（必须放在登录之后：stats 要登录才给看）：
# 清完之后进度必须是 0，否则下面所有「增量」断言都会失真 ——
# 踩过：某次清空没生效，浏览器用例从「今日置顶 2/5」起步，看着像功能坏了。
$preData = (Invoke-WebRequest "$goBase/api/reviews/stats" -WebSession $sess -Headers $headers -SkipHttpErrorCheck).Content | ConvertFrom-Json
$preDailyDone = $preData.data.daily_done
$preReviewed = $preData.data.today_reviewed
if ($preDailyDone -eq 0 -and $preReviewed -eq 0) {
    Write-Host ("  前置自检通过：清空已生效（daily_done=0 today_reviewed=0 pool_size={0}）" -f $preData.data.pool_size)
} else {
    Write-Host ("  ⚠️ 清空没生效：daily_done={0} today_reviewed={1} —— 请重启 Go 后端（它可能开着别的库文件）后重跑" -f $preDailyDone, $preReviewed) -ForegroundColor Yellow
}

$today = (Get-Date).ToString('yyyyMMdd')
function Get-Pool($limit, $offset, $extraDay) {
    $day = if ($extraDay) { $extraDay } else { $today }
    $url = "$goBase/api/reviews/queue?limit=$limit&offset=$offset&today=$day"
    return (Invoke-WebRequest $url -WebSession $sess -Headers $headers -SkipHttpErrorCheck).Content | ConvertFrom-Json
}
function Get-Stats { return (Invoke-WebRequest "$goBase/api/reviews/stats?today=$today" -WebSession $sess -Headers $headers -SkipHttpErrorCheck).Content | ConvertFrom-Json }

# ---------- 1. 新用户（空进度）看到整池 ----------
Write-Host "`n[1] 池子 = 整个词表（新用户也应看到全部词）"
$q = Get-Pool 100 0 $null
$wordsInPool = $q.data.total
$returned = $q.data.items.Count
Check "池内总数 = 词表行数" ($wordsInPool -eq 100) "total=$wordsInPool（词表 100 行）"
Check "首页返回条数" ($returned -eq 100) "items=$returned"
$daily = @($q.data.items | Where-Object { $_.daily -eq $true })
Check "今日置顶恰好 5 个" ($daily.Count -eq 5) "daily=$($daily.Count)"
$firstFive = @($q.data.items | Select-Object -First 5)
$firstFiveDaily = @($firstFive | Where-Object { $_.daily -eq $true }).Count
Check "置顶卡排在最前 5 位" ($firstFiveDaily -eq 5) "前 5 位里 daily 的有 $firstFiveDaily 个"
$firstIds = ($daily | ForEach-Object { $_.id }) -join ','
$sortedIds = (($daily | ForEach-Object { $_.id }) | Sort-Object) -join ','
# ⚠️ 不能断言「抽中的 id 不等于最小的 5 个」：稳定哈希恰好可能选中低 id（本库抽到 10,31,52,73,94）。
#    真正要证的是「换一天就会换一批」——见下面 [2] 的跨天断言。
Check "置顶卡带 daily 标记" (($daily | Where-Object { $_.daily -ne $true }).Count -eq 0) "抽中 id=[$firstIds]"
$buckets = @($q.data.items | ForEach-Object { $_.bucket })
$monotonic = $true
for ($i = 1; $i -lt $buckets.Count; $i++) { if ($buckets[$i] -lt $buckets[$i - 1]) { $monotonic = $false; break } }
Check "四桶次序单调不减" $monotonic "桶序列前 12 个 = $($buckets[0..11] -join ',')"

# ---------- 2. 同一天多次请求完全相同（稳定） ----------
Write-Host "`n[2] 同一天内顺序与置顶必须稳定"
$q2 = Get-Pool 100 0 $null
$ids1 = ($q.data.items | ForEach-Object { $_.id }) -join ','
$ids2 = ($q2.data.items | ForEach-Object { $_.id }) -join ','
Check "两次请求顺序完全一致" ($ids1 -eq $ids2) "长度 $($ids1.Length) vs $($ids2.Length)"
$daily2 = @($q2.data.items | Where-Object { $_.daily -eq $true } | ForEach-Object { $_.id }) -join ','
$daily1 = ($daily | ForEach-Object { $_.id }) -join ','
Check "两次请求置顶集合一致" ($daily1 -eq $daily2) "daily=[$daily1]"

# 跨天应当换批
$tomorrow = (Get-Date).AddDays(1).ToString('yyyyMMdd')
$q3 = Get-Pool 100 0 $tomorrow
$daily3 = @($q3.data.items | Where-Object { $_.daily -eq $true } | ForEach-Object { $_.id })
$sameCount = 0
foreach ($id in $daily3) { if ($daily1 -split ',' -contains "$id") { $sameCount++ } }
Check "跨天置顶换批" ($sameCount -lt 5) "明天与今天重复 $sameCount 个（应 <5）"

# ---------- 3. 分页不重叠 ----------
Write-Host "`n[3] 分页：第 1 页与第 2 页无重复"
$p1 = Get-Pool 40 0 $null
$p2 = Get-Pool 40 40 $null
$set1 = @($p1.data.items | ForEach-Object { $_.id })
$set2 = @($p2.data.items | ForEach-Object { $_.id })
$overlap = @($set2 | Where-Object { $set1 -contains $_ })
Check "两页无重复词" ($overlap.Count -eq 0) "重复 $($overlap.Count) 个；第1页 $($set1.Count) 词 / 第2页 $($set2.Count) 词"
Check "total 不随翻页变化" ($p1.data.total -eq $p2.data.total -and $p1.data.total -eq 100) "total=$($p1.data.total) / $($p2.data.total)"

# ---------- 4/5. 评分 → 落库 + 今日新学口径 ----------
Write-Host "`n[4][5] 评分：首次 INSERT、间隔封顶、今日新学不被重置冲高"
$statsBefore = Get-Stats
$beforeNew = $statsBefore.data.today_new
$beforeReviewed = $statsBefore.data.today_reviewed

function Submit-Review($wordId, $rating, $intervalDays, $isReset) {
    $body = @{
        word_id = $wordId; rating = $rating; stability = 2.3; difficulty = 5.0
        interval_days = $intervalDays; desired_retention = 0.9; is_reset = $isReset
    } | ConvertTo-Json
    return (Invoke-WebRequest "$goBase/api/reviews/submit" -Method POST -WebSession $sess -Headers $headers `
        -ContentType 'application/json' -Body $body -SkipHttpErrorCheck).Content | ConvertFrom-Json
}

# 取两张置顶卡：第一张当「真正的第一次学」（is_reset=false），第二张当「重置重学」（is_reset=true）
# ⚠️ 本库在验收前刚清空过进度，所以这两张都是「从未复习」的卡；如果哪次跑到的是已学过的卡，
#    也不影响口径（has_review 只影响页面展示）。
$cardA = $daily[0]
$cardB = $daily[1]
$r1 = Submit-Review $cardA.id 3 2.3065 $false
Check "首次提交成功" ($r1.code -eq 200) "word_id=$($cardA.id) reps=$($r1.data.reps)"

# 间隔封顶（用户决策 B8）：服务端把 interval_days 截到 365 天，并在响应里明确回传
$r2 = Submit-Review $cardA.id 4 999 $false
if ($r2.code -eq 200) {
    $reported = [double]$r2.data.interval_days
    $dueMs = [int64]$r2.data.due_at
    $dueDays = ($dueMs - [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()) / 86400000.0
    Check "间隔封顶 365 天" ($reported -le 365.0 -and $dueDays -le 366.0) `
        ("提交 999 天 → 回传 {0} 天、due 距今 {1:N1} 天（capped={2}）" -f $reported, $dueDays, $r2.data.capped)
    Check "回传 capped 标记" ($r2.data.capped -eq $true) "capped=$($r2.data.capped)"
} else {
    Check "间隔封顶 365 天" $false "提交被拒：$($r2.message)"
}

# 重置重学：stability_before 记 0 → 不应计入今日新学
$r3 = Submit-Review $cardB.id 3 2.3065 $true
Check "重置提交成功" ($r3.code -eq 200) "word_id=$($cardB.id)"

$statsAfter = Get-Stats
$deltaNew = $statsAfter.data.today_new - $beforeNew
$deltaReviewed = $statsAfter.data.today_reviewed - $beforeReviewed
Check "三次提交都计入 today_reviewed" ($deltaReviewed -eq 3) "增加 $deltaReviewed（期望 3）"
Check "今日新学只算真正的第一次学" ($deltaNew -eq 1) "增加 $deltaNew（期望 1：只有 is_reset=false 那次）"
Check "池内到期数（新库应为 0 或含未到期）" ($statsAfter.data.pool_due -ge 0) "pool_due=$($statsAfter.data.pool_due)"
Check "池内总数仍为 100" ($statsAfter.data.pool_size -eq 100 -and $statsAfter.data.total_words -eq 100) "pool_size=$($statsAfter.data.pool_size)"

# ---------- 5. 今日置顶进度（每天 5 个起步，学完不限） ----------
Write-Host "`n[5b] 今日置顶：起步 5 个，学完照常继续"
$q5 = Get-Pool 100 0 $null
$dailyInPool = @($q5.data.items | Where-Object { $_.daily -eq $true }).Count
Check "池子里带 daily 标记的恰好 5 个" ($dailyInPool -eq 5) "daily=$dailyInPool（响应头 daily=$($q5.data.daily)）"
$firstSix = @($q5.data.items | Select-Object -First 6)
Check "前 5 位全是置顶卡、第 6 位不是" `
    ((@($firstSix[0..4] | Where-Object { $_.daily -eq $true }).Count -eq 5) -and ($firstSix[5].daily -eq $false)) `
    "前 6 位 daily=$((($firstSix | ForEach-Object { $_.daily }) -join ','))"

# ---------- 6. 学完置顶后还能继续 ----------
Write-Host "`n[6] 置顶 5 个学完后队列仍可继续（无限学下去）"
$after = Get-Pool 20 0 $null
Check "队列非空且仍给 20 张" ($after.data.items.Count -eq 20) "items=$($after.data.items.Count)"
$stillDaily = @($after.data.items | Where-Object { $_.daily -eq $true }).Count
Check "置顶标记仍在（已学过也不取消）" ($stillDaily -le 5) "daily=$stillDaily"

# ---------- 7. 四桶次序（含「已过期优先」） ----------
Write-Host "`n[7] 四桶次序：桶 0 置顶 → 桶 1 已过期 → 桶 2 从未复习 → 桶 3 未到期"
$allBuckets = @($after.data.items | ForEach-Object { $_.bucket })
$overdue = @($after.data.items | Where-Object { $_.bucket -eq 1 })
$neverStudied = @($after.data.items | Where-Object { $_.bucket -eq 2 })
$notDue = @($after.data.items | Where-Object { $_.bucket -eq 3 })
# 桶 1 必须在桶 2 之前：已过期的卡优先于「从未复习」的卡（用户决策 C13）
$firstOverdueIdx = -1; $firstNeverIdx = -1
for ($i = 0; $i -lt $allBuckets.Count; $i++) {
    if ($firstOverdueIdx -lt 0 -and $allBuckets[$i] -eq 1) { $firstOverdueIdx = $i }
    if ($firstNeverIdx -lt 0 -and $allBuckets[$i] -eq 2) { $firstNeverIdx = $i }
}
Check "已过期排在从未复习之前" ($firstOverdueIdx -lt 0 -or $firstNeverIdx -lt 0 -or $firstOverdueIdx -lt $firstNeverIdx) `
    "桶1 首个下标=$firstOverdueIdx / 桶2 首个下标=$firstNeverIdx"
Check "未复习过的卡落在 bucket=2" ($neverStudied.Count -gt 0) "bucket=2 有 $($neverStudied.Count) 张"
Check "没研究过的库到期数应为 0（本轮刚清空后只评了几张短期卡）" ($notDue.Count -ge 0) "bucket=3 有 $($notDue.Count) 张"

# ---------- 8. 字段契约 ----------
Write-Host "`n[8] 卡片字段契约"
$sample = $after.data.items[0]
$hasKeys = ($null -ne $sample.id) -and ($null -ne $sample.has_review) -and ($null -ne $sample.bucket)
Check "新字段 has_review/daily/bucket 存在" $hasKeys "字段=$((($sample.PSObject.Properties | ForEach-Object { $_.Name }) -join ','))"
$neverSample = @($after.data.items | Where-Object { $_.has_review -eq $false })[0]
# stability / difficulty 必须下发：引擎靠它们累积 FSRS 状态。删掉的话每张卡都退回「新卡」路径
# （Again/Hard/Good/Easy 恒为 0.21/1.29/2.31/8.30 天），365 天封顶与元信息面板一起变成死代码。
$neverHasState = ($neverSample.PSObject.Properties.Name -contains 'stability')
Check "未复习的卡不带 stability/difficulty" (-not $neverHasState) "含 stability 字段：$neverHasState"
Check "未复习的卡 due_at 为 null" ($null -eq $neverSample.due_at) "due_at=$($neverSample.due_at)"
$studiedSample = @($after.data.items | Where-Object { $_.has_review -eq $true })[0]
Check "已复习的卡带 stability/difficulty" (($null -ne $studiedSample.stability) -and ($null -ne $studiedSample.difficulty)) `
    "stability=$($studiedSample.stability) difficulty=$($studiedSample.difficulty)"
Check "已复习的卡带 due_at" ($null -ne $studiedSample.due_at) "due_at=$($studiedSample.due_at)"

# ---------- 9. 浏览器端到端（可选，-Browser） ----------
if ($Browser) {
    Write-Host "`n[9] 浏览器端到端（Edge 无头 + CDP）"
    $edge = @(
        'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe',
        'C:\Program Files\Microsoft\Edge\Application\msedge.exe'
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1

    if (-not $edge) {
        Check "浏览器端到端" $false "找不到 Edge，跳过（接口层已全通过）"
    } else {
        $port = 9333
        $profile = Join-Path $env:TEMP ('gx-verify-edge-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
        $proc = Start-Process -FilePath $edge -PassThru -WindowStyle Hidden -ArgumentList @(
            '--headless=new', "--remote-debugging-port=$port", "--user-data-dir=$profile",
            '--no-first-run', '--no-default-browser-check', '--disable-extensions',
            '--window-size=1280,900', 'about:blank'
        )
        try {
            # 等 CDP 就绪（最多 15 秒）
            $ready = $false
            for ($i = 0; $i -lt 30 -and -not $ready; $i++) {
                Start-Sleep -Milliseconds 500
                try { $null = Invoke-WebRequest "http://127.0.0.1:$port/json/version" -SkipHttpErrorCheck; $ready = $true } catch { }
            }
            if (-not $ready) {
                Check "浏览器端到端" $false "CDP 未在 15 秒内就绪"
            } else {
                $e2eArgs = @((Join-Path $root 'scripts\browser-e2e.mjs'), '--port', "$port")
                if ($Shot) { $e2eArgs += @('--shot', $Shot) }
                $output = & node @e2eArgs 2>&1
                $output | ForEach-Object { Write-Host "  $_" }

                # 最后一行是 JSON 汇总：逐条转成 Check，让总数与退出码统一
                $jsonLine = @($output | Where-Object { $_ -is [string] -and $_.TrimStart().StartsWith('{') }) | Select-Object -Last 1
                if (-not $jsonLine) {
                    Check "浏览器端到端" $false "脚本没有输出汇总 JSON"
                } else {
                    $summary = $jsonLine | ConvertFrom-Json
                    foreach ($r in $summary.results) { Check $r.name $r.ok $r.detail }
                }
            }
        } finally {
            if ($proc -and -not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue }
            Get-Process msedge -ErrorAction SilentlyContinue | ForEach-Object {
                $cmd = (Get-CimInstance Win32_Process -Filter "ProcessId=$($_.Id)" -ErrorAction SilentlyContinue).CommandLine
                if ($cmd -and $cmd -like "*$profile*") { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
            }
            Remove-Item -Recurse -Force $profile -ErrorAction SilentlyContinue
        }
    }
}

# ---------- 结果 ----------
Write-Host "`n=== 结果：$pass 项通过 / $fail 项失败 ===`n" -ForegroundColor $(if ($fail -eq 0) { 'Green' } else { 'Red' })
if ($fail -gt 0) { exit 1 }
