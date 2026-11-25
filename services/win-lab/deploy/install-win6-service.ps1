<#
.SYNOPSIS
  Install or update Tako Switch Windows Release Gate & Test Lab (`tako-win-lab`) on .6 (henan-windows-pc).
  Creates a persistent Scheduled Task running in shiroha's interactive session so both
  HTTP API (172.17.66.6:18795) and real GUI cold-launch smoke tests work seamlessly.
#>
param(
  [string]$InstallRoot = "D:\fleet\tako-win-lab",
  [string]$RepoDir = "D:\fleet\tako-win-lab\repo",
  [string]$DataDir = "D:\fleet\tako-win-lab\data",
  [string]$TaskName = "TakoWinLabService",
  [int]$Port = 18795,
  [string]$HqUrl = "http://172.17.66.26:8789",
  [string]$TelemetryUrl = "http://172.17.66.15:18080"
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8

New-Item -ItemType Directory -Force -Path $InstallRoot, $DataDir, "$DataDir\logs" | Out-Null

$PythonExe = (Get-Command python -ErrorAction Stop).Source
$ServerScript = Join-Path $RepoDir "services\win-lab\server.py"
if (-not (Test-Path $ServerScript)) {
  throw "Missing server script: $ServerScript"
}

$RunnerBat = Join-Path $InstallRoot "run-win-lab.cmd"
$LogFile = Join-Path $DataDir "logs\win-lab.log"

$BatContent = @"
@echo off
setlocal
set PYTHONUTF8=1
set PYTHONUNBUFFERED=1
set WIN_LAB_HOST=0.0.0.0
set WIN_LAB_PORT=$Port
set WIN_LAB_REPO_ROOT=$RepoDir
set WIN_LAB_DATA_DIR=$DataDir
set WIN_LAB_HQ_URL=$HqUrl
set WIN_LAB_TELEMETRY_URL=$TelemetryUrl
set WIN_LAB_SCHEDULE_ENABLED=1
set WIN_LAB_AUTO_REPORT_HQ=1
set WIN_LAB_AUTO_REPORT_TELEMETRY=1
set WIN_LAB_ALLOW_LIVE_SMOKE=1
cd /d "$RepoDir\services\win-lab"
"$PythonExe" "$ServerScript" >> "$LogFile" 2>&1
"@

Set-Content -Path $RunnerBat -Value $BatContent -Encoding ASCII

# Stop any running old instance listening on $Port
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

# Register Scheduled Task under current interactive user (runs on logon + boot, auto-restarts on failure)
$action = New-ScheduledTaskAction -Execute "cmd.exe" -Argument "/c `"$RunnerBat`"" -WorkingDirectory "$RepoDir\services\win-lab"
$triggerLogon = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
$settings = New-ScheduledTaskSettingsSet `
  -AllowStartIfOnBatteries `
  -DontStopIfGoingOnBatteries `
  -ExecutionTimeLimit ([TimeSpan]::Zero) `
  -RestartCount 999 `
  -RestartInterval (New-TimeSpan -Minutes 1) `
  -MultipleInstances IgnoreNew

$principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive -RunLevel Highest

Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $triggerLogon -Settings $settings -Principal $principal -Force | Out-Null
Start-ScheduledTask -TaskName $TaskName

# Wait up to 15s for healthz
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
  Write-Output "--- Tail $LogFile ---"
  Get-Content -Path $LogFile -Tail 40 -ErrorAction SilentlyContinue
  throw "tako-win-lab failed health check on port $Port"
}
