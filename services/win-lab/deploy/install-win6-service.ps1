<#
.SYNOPSIS
  Install or update Tako Switch Windows Release Gate & Test Lab (`tako-win-lab`) on .6 (henan-windows-pc).
  Uses the real Python binary from sys.base_prefix (avoiding Microsoft Store AppExecutionAlias stubs)
  and registers a persistent Scheduled Task running in shiroha's interactive session.
#>
param(
  [string]$InstallRoot = "D:\fleet\tako-win-lab",
  [string]$RepoDir = "D:\fleet\tako-win-lab\repo",
  [string]$DataDir = "D:\fleet\tako-win-lab\data",
  [string]$TaskName = "TakoWinLabService",
  [int]$Port = 18795,
  [string]$HqUrl = "http://172.17.66.26:8789",
  [string]$TelemetryUrl = "http://172.17.66.15:18080",
  [string]$EgressProxy = "http://172.17.66.4:3128"
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8

New-Item -ItemType Directory -Force -Path $InstallRoot, $DataDir, "$DataDir\logs" | Out-Null

# Use the AppExecutionAlias or standard Python path from PATH (Windows blocks direct execution inside C:\Program Files\WindowsApps\)
$PythonExe = (Get-Command python -ErrorAction Stop).Source
$ServerScript = Join-Path $RepoDir "services\win-lab\server.py"
if (-not (Test-Path $ServerScript)) {
  throw "Missing server script: $ServerScript"
}

$RunnerPs1 = Join-Path $InstallRoot "run-win-lab.ps1"
$LogFile = Join-Path $DataDir "logs\win-lab.log"
$WorkDir = Join-Path $RepoDir "services\win-lab"

$Lines = @(
  '$env:PYTHONUTF8 = "1"',
  '$env:PYTHONUNBUFFERED = "1"',
  '$env:WIN_LAB_HOST = "0.0.0.0"',
  "`$env:WIN_LAB_PORT = `"$Port`"",
  "`$env:WIN_LAB_REPO_ROOT = `"$RepoDir`"",
  "`$env:WIN_LAB_DATA_DIR = `"$DataDir`"",
  "`$env:WIN_LAB_HQ_URL = `"$HqUrl`"",
  "`$env:WIN_LAB_TELEMETRY_URL = `"$TelemetryUrl`"",
  "`$env:WIN_LAB_EGRESS_PROXY = `"$EgressProxy`"",
  '$env:WIN_LAB_OPS_CONSOLE_URL = "http://172.17.66.26:8767/win-lab"',
  '$env:WIN_LAB_SCHEDULE_ENABLED = "1"',
  '$env:WIN_LAB_AUTO_REPORT_HQ = "1"',
  '$env:WIN_LAB_AUTO_REPORT_TELEMETRY = "1"',
  '$env:WIN_LAB_ALLOW_LIVE_SMOKE = "1"',
  "Set-Location `"$WorkDir`"",
  "& `"$PythonExe`" `"$ServerScript`""
)
Set-Content -Path $RunnerPs1 -Value ($Lines -join "`r`n") -Encoding UTF8

# Stop any running old task or process listening on $Port
Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
$existingConns = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue
foreach ($c in $existingConns) {
  if ($c.OwningProcess -gt 4) {
    Write-Output "Stopping existing process PID=$($c.OwningProcess) on port $Port"
    Stop-Process -Id $c.OwningProcess -Force -ErrorAction SilentlyContinue
  }
}

# Add Windows Firewall allow rule scoped to EasyTier Mesh (172.17.66.0/24) + loopback
$ruleName = "TakoWinLab-Mesh-$Port"
if (-not (Get-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue)) {
  New-NetFirewallRule -DisplayName $ruleName -Direction Inbound -Action Allow `
    -Protocol TCP -LocalPort $Port -RemoteAddress 172.17.66.0/24,127.0.0.1 | Out-Null
}

# Register Scheduled Task under full DOMAIN\User in Interactive session (supports GUI Gate-4 smoke tests)
$currentUser = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
$action = New-ScheduledTaskAction `
  -Execute "C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe" `
  -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$RunnerPs1`"" `
  -WorkingDirectory $WorkDir
$triggerLogon = New-ScheduledTaskTrigger -AtLogOn -User $currentUser
$settings = New-ScheduledTaskSettingsSet `
  -AllowStartIfOnBatteries `
  -DontStopIfGoingOnBatteries `
  -ExecutionTimeLimit ([TimeSpan]::Zero) `
  -RestartCount 999 `
  -RestartInterval (New-TimeSpan -Minutes 1) `
  -MultipleInstances IgnoreNew
$principal = New-ScheduledTaskPrincipal -UserId $currentUser -LogonType Interactive -RunLevel Highest

Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $triggerLogon -Settings $settings -Principal $principal -Force | Out-Null
Start-ScheduledTask -TaskName $TaskName

$healthy = $false
for ($i = 0; $i -lt 15; $i++) {
  Start-Sleep -Seconds 1
  try {
    $resp = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/healthz" -TimeoutSec 3 -UseBasicParsing
    if ($resp.StatusCode -eq 200) {
      Write-Output "HEALTHY: $($resp.Content)"
      $healthy = $true
      break
    }
  } catch {}
}

if (-not $healthy) {
  Write-Output "--- Task Info ---"
  Get-ScheduledTaskInfo -TaskName $TaskName | Format-List LastRunTime, LastTaskResult
  Write-Output "--- Tail $LogFile ---"
  Get-Content -Path $LogFile -Tail 40 -ErrorAction SilentlyContinue
  throw "tako-win-lab failed health check on port $Port"
}
