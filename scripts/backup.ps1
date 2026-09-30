<#
  数据库安全快照（SQLite VACUUM INTO）

  为什么不能直接复制 .db 文件：两个库都跑在 WAL 模式下，最近的写入可能还留在
  -wal 文件里，主库文件本身可能是旧的、甚至几乎是空的 —— 例如 backend-rust/auth.db
  只有 4 KB，而旁边的 auth.db-wal 有 3.7 MB。复制 .db 会丢掉这部分数据。

  本脚本调用 backend-go/cmd/backup（VACUUM INTO），在一次读事务里把整个库
  压实写入新文件，**不修改源库**，所以服务正在跑的时候也能安全执行。

  用法：
    pwsh scripts/backup.ps1                     # 快照到 <仓库>/backups/<时间戳>/
    pwsh scripts/backup.ps1 -Out D:\gx-backup   # 指定输出根目录
    pwsh scripts/backup.ps1 -Keep 10            # 顺手清理，只留最近 10 份
    pwsh scripts/backup.ps1 -Out D:\gx-backup -Keep 10

  输出目录已加进 .gitignore（快照含本地数据，不入库）。
#>
[CmdletBinding()]
param(
    [string]$Out,
    [int]$Keep = 0
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $Out) { $Out = Join-Path $root 'backups' }

# 安全闸：与 clean-build-cache.ps1 同一套约定 —— 只读的「备份」目录永不可写
if ([System.IO.Path]::GetFullPath($Out) -match '备份') {
    Write-Output ("拒绝：输出目录不能落在只读的「备份」目录里 -> {0}" -f $Out)
    exit 1
}

# 找 Go：优先 PATH，其次项目固定的便携版（见 CLAUDE.md 红线 8）
function Find-Go {
    $cmd = Get-Command go -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $portable = 'C:\Users\22629\go-portable\go\bin\go.exe'
    if (Test-Path -LiteralPath $portable) { return $portable }
    return $null
}

$go = Find-Go
if (-not $go) {
    Write-Output '找不到 go：把便携版放回 C:\Users\22629\go-portable\go\bin\go.exe，或把它加进 PATH。'
    exit 1
}

$args = @('run', './cmd/backup', '-out', $Out,
          '-db', (Join-Path $root 'backend-go\guangxue.db'),
          '-db', (Join-Path $root 'backend-rust\auth.db'))
if ($Keep -gt 0) { $args += @('-keep', "$Keep") }

Push-Location (Join-Path $root 'backend-go')
try {
    & $go @args
    $code = $LASTEXITCODE
} finally {
    Pop-Location
}
exit $code
