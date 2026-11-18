<#
.SYNOPSIS
  以沉浸模式（带远程调试端口）重新打开一个 Electron 应用，默认是 reclaude 客户端。

.DESCRIPTION
  沉浸模式需要目标应用带 --remote-debugging-port 启动。reclaude 启动客户端时会带一组自己的参数
  （代理、用户数据目录等），所以本脚本不自己拼参数，而是：
    1. 读取正在运行的客户端主进程的完整命令行，保存到 ~/.translate-anything/launch-<名称>.txt；
    2. 等你从托盘菜单把客户端完全退出；
    3. 用同样的命令行 + --remote-debugging-port=<端口> 重新打开。
  客户端没在运行时，直接用上次保存的命令行启动。
  软件本体不做任何修改；想回到普通模式，退出后照常用 reclaude 打开即可。

  注意：调试端口只监听本机 127.0.0.1，但本机任何程序都能通过它操控该应用，用完建议照常重启。

.EXAMPLE
  pwsh -File scripts/immersive-launch.ps1
  pwsh -File scripts/immersive-launch.ps1 -Exe "C:\path\to\App.exe" -Name myapp -Port 39223
#>
param(
  [string]$Exe = (Join-Path $env:USERPROFILE '.rec\claude-app\app\claude.exe'),
  [string]$Name = 'reclaude',
  [int]$Port = 39222,
  [int]$WaitSeconds = 300
)

$ErrorActionPreference = 'Stop'
$saveDir = Join-Path $env:USERPROFILE '.translate-anything'
# 改名前的旧数据目录：新目录还没有时整个迁移过来（和主程序启动时的迁移逻辑一致），
# 否则先建了新目录，主程序就不会再迁移旧目录里的配置和译文库
$legacyDir = Join-Path $env:USERPROFILE '.translate-filter'
if (-not (Test-Path $saveDir) -and (Test-Path $legacyDir -PathType Container)) {
  try { Rename-Item -Path $legacyDir -NewName '.translate-anything' }
  catch { $saveDir = $legacyDir }
}
$saveFile = Join-Path $saveDir "launch-$Name.txt"

function Get-MainProcess {
  # Electron 主进程的命令行里没有 --type=（子进程都带 --type=renderer/gpu-process 等）
  Get-CimInstance Win32_Process -Filter "Name='$([IO.Path]::GetFileName($Exe))'" |
    Where-Object { $_.ExecutablePath -eq $Exe -and $_.CommandLine -notmatch '--type=' } |
    Select-Object -First 1
}

function Test-Port {
  try {
    Invoke-WebRequest -NoProxy "http://127.0.0.1:$Port/json/version" -TimeoutSec 2 | Out-Null
    return $true
  } catch { return $false }
}

if (-not (Test-Path $Exe)) { throw "找不到程序：$Exe" }

if (Test-Port) {
  Write-Host "调试端口 $Port 已开启，应用已处于沉浸模式，无需重启。"
  exit 0
}

$main = Get-MainProcess
if ($main) {
  New-Item -ItemType Directory -Force $saveDir | Out-Null
  Set-Content -Path $saveFile -Value $main.CommandLine -Encoding utf8
  Write-Host "已记录当前启动参数：$saveFile"
  Write-Host "请从托盘菜单把应用【完全退出】（只关窗口不算），脚本会在它退出后自动以沉浸模式重新打开……"
  $deadline = (Get-Date).AddSeconds($WaitSeconds)
  while (Get-MainProcess) {
    if ((Get-Date) -gt $deadline) { throw "等待超时：应用仍在运行。" }
    Start-Sleep -Milliseconds 500
  }
  # 等子进程收尾，避免单实例锁还没释放
  Start-Sleep -Seconds 2
} elseif (-not (Test-Path $saveFile)) {
  throw "应用没有在运行，也没有保存过启动参数。请先用平常的方式（如 reclaude app）打开一次，再运行本脚本。"
}

# 拆出参数部分：去掉开头的程序路径（可能带引号），并去掉旧的调试端口参数
$cmd = (Get-Content $saveFile -Raw -Encoding utf8).Trim()
if ($cmd.StartsWith('"')) { $args0 = $cmd.Substring($cmd.IndexOf('"', 1) + 1) }
elseif ($cmd.Contains(' ')) { $args0 = $cmd.Substring($cmd.IndexOf(' ') + 1) }
else { $args0 = '' }
$argLine = ($args0 -replace '--remote-debugging-port=\d+', '').Trim()
$argLine = "$argLine --remote-debugging-port=$Port"

Start-Process -FilePath $Exe -ArgumentList $argLine | Out-Null
Write-Host "已以沉浸模式启动（调试端口 $Port）。随译运行中会自动接管它的界面。"
