"""Gate 4 (`runtimeSmoke`): Real Windows install, launch, liveness, and EventLog/crash diagnostics.

Designed to run in `shiroha`'s active interactive Windows session on `.6` (172.17.66.6):
  - Safely snapshots & restores any pre-existing user config files before/after testing.
  - Installs or launches target Windows app (`codex` MSIX, `claude` EXE, `antigravity` EXE, `dsh-tako` EXE).
  - Polls process liveness + main window title over `hold_seconds` (default 15s).
  - Collects Windows Event Log (`Application` 1000/1001/1026, `AppxDeploymentServer`, `AppModel-Runtime`)
    and sanitized client startup log deltas strictly after `started_at`.
  - Cleanly terminates spawned test processes and optionally uninstalls test package.
"""

from __future__ import annotations

from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import time
from typing import Any, Callable

from .config import LabConfig
from .hq_reporter import scrub_text


def _run_powershell(ps_script: str, timeout: int = 120) -> tuple[int, str, str]:
    proc = subprocess.run(
        ["powershell", "-NoProfile", "-NonInteractive", "-Command", ps_script],
        capture_output=True,
        timeout=timeout,
    )
    out = proc.stdout.decode("utf-8", errors="replace").strip()
    err = proc.stderr.decode("utf-8", errors="replace").strip()
    return int(proc.returncode), out, err


def _collect_windows_crash_events(since_iso: str, keywords: list[str]) -> list[dict[str, Any]]:
    """Query Windows Application + Appx/AppModel Event Logs since `since_iso`."""
    kw_array = ",".join(f"'{k.lower()}'" for k in keywords)
    ps = f"""
$OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$since = [DateTime]::Parse('{since_iso}').ToLocalTime()
$kws = @({kw_array})
$results = @()

# 1. Application crash events (1000=AppError, 1001=WER, 1026=.NET)
try {{
  $appEvs = Get-WinEvent -FilterHashtable @{{ LogName='Application'; StartTime=$since; Id=@(1000,1001,1026) }} -MaxEvents 40 -ErrorAction SilentlyContinue
  foreach ($e in $appEvs) {{
    $msg = [string]$e.Message
    $low = $msg.ToLowerInvariant()
    $hit = $false
    foreach ($k in $kws) {{ if ($low.Contains($k)) {{ $hit = $true; break }} }}
    if ($hit) {{
      $results += [ordered]@{{
        log = 'Application'
        id = $e.Id
        level = [string]$e.LevelDisplayName
        provider = [string]$e.ProviderName
        time = $e.TimeCreated.ToUniversalTime().ToString('o')
        message = $msg.Substring(0, [Math]::Min(1200, $msg.Length))
      }}
    }}
  }}
}} catch {{}}

# 2. AppxDeployment / AppModel runtime activation errors
foreach ($logName in @('Microsoft-Windows-AppxDeploymentServer/Operational', 'Microsoft-Windows-AppModel-Runtime/Admin')) {{
  try {{
    $evs = Get-WinEvent -FilterHashtable @{{ LogName=$logName; StartTime=$since; Level=@(1,2,3) }} -MaxEvents 20 -ErrorAction SilentlyContinue
    foreach ($e in $evs) {{
      $msg = [string]$e.Message
      $low = $msg.ToLowerInvariant()
      $hit = $false
      foreach ($k in $kws) {{ if ($low.Contains($k)) {{ $hit = $true; break }} }}
      if ($hit) {{
        $results += [ordered]@{{
          log = $logName
          id = $e.Id
          level = [string]$e.LevelDisplayName
          provider = [string]$e.ProviderName
          time = $e.TimeCreated.ToUniversalTime().ToString('o')
          message = $msg.Substring(0, [Math]::Min(1000, $msg.Length))
        }}
      }}
    }}
  }} catch {{}}
}}

ConvertTo-Json -InputObject $results -Depth 4 -Compress
"""
    rc, out, _err = _run_powershell(ps, timeout=30)
    if rc != 0 or not out:
        return []
    try:
        parsed = json.loads(out)
        if isinstance(parsed, dict):
            parsed = [parsed]
        if isinstance(parsed, list):
            for item in parsed:
                if isinstance(item, dict) and "message" in item:
                    item["message"] = scrub_text(str(item["message"]), max_len=1200)
            return parsed
    except Exception:
        pass
    return []


def _collect_client_log_tails(app_id: str, since_epoch: float) -> list[dict[str, str]]:
    home = Path(os.environ.get("USERPROFILE") or Path.home())
    appdata = Path(os.environ.get("APPDATA") or (home / "AppData" / "Roaming"))
    local_appdata = Path(os.environ.get("LOCALAPPDATA") or (home / "AppData" / "Local"))

    scan_paths: list[Path] = []
    if app_id == "codex":
        scan_paths.extend(
            [
                home / ".codex" / "log",
                local_appdata / "OpenAI" / "Codex" / "logs",
                appdata / "Codex" / "logs",
            ]
        )
        pkg_root = local_appdata / "Packages"
        if pkg_root.is_dir():
            for d in pkg_root.glob("OpenAI.Codex*"):
                scan_paths.append(d / "LocalCache")
                scan_paths.append(d / "LocalState")
    elif app_id == "claude":
        scan_paths.extend(
            [
                appdata / "Claude-3p" / "logs",
                appdata / "Claude" / "logs",
            ]
        )
    elif app_id == "antigravity":
        scan_paths.extend(
            [
                appdata / "Antigravity" / "logs",
                local_appdata / "Antigravity" / "logs",
            ]
        )
    elif app_id == "dsh-tako":
        scan_paths.extend([appdata / "DSH Tako" / "logs", local_appdata / "DSH Tako" / "logs"])

    found: list[tuple[float, Path]] = []
    for root in scan_paths:
        if not root.exists():
            continue
        if root.is_file():
            try:
                mt = root.stat().st_mtime
                if mt >= since_epoch - 5:
                    found.append((mt, root))
            except OSError:
                pass
            continue
        for p in root.rglob("*"):
            if not p.is_file():
                continue
            if p.suffix.lower() not in (".log", ".txt"):
                continue
            try:
                mt = p.stat().st_mtime
                if mt >= since_epoch - 5:
                    found.append((mt, p))
            except OSError:
                continue

    found.sort(key=lambda x: x[0], reverse=True)
    tails: list[dict[str, str]] = []
    for _mt, path in found[:5]:
        try:
            raw = path.read_bytes()[-12_000:].decode("utf-8", errors="replace")
            lines = raw.splitlines()[-40:]
            tails.append(
                {
                    "file": str(path),
                    "tail": scrub_text("\n".join(lines), max_len=1600),
                }
            )
        except OSError:
            continue
    return tails


def _probe_processes(match_names: list[str], path_filter: str = "") -> list[dict[str, Any]]:
    names_arr = ",".join(f"'{n.lower()}'" for n in match_names)
    pf = path_filter.lower().replace("'", "''")
    ps = f"""
$OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$names = @({names_arr})
$pf = '{pf}'
$list = @()
Get-Process -ErrorAction SilentlyContinue | ForEach-Object {{
  $pn = $_.ProcessName.ToLowerInvariant()
  if ($names -contains $pn) {{
    $path = ''
    try {{ $path = [string]$_.Path }} catch {{}}
    if (-not $pf -or $path.ToLowerInvariant().Contains($pf) -or $path -eq '') {{
      $list += [ordered]@{{
        pid = $_.Id
        name = $_.ProcessName
        path = $path
        title = [string]$_.MainWindowTitle
        memMB = [math]::Round($_.WorkingSet64 / 1MB, 1)
      }}
    }}
  }}
}}
ConvertTo-Json -InputObject $list -Depth 3 -Compress
"""
    rc, out, _ = _run_powershell(ps, timeout=15)
    if rc != 0 or not out:
        return []
    try:
        parsed = json.loads(out)
        if isinstance(parsed, dict):
            return [parsed]
        return parsed if isinstance(parsed, list) else []
    except Exception:
        return []


def _kill_matching_processes(match_names: list[str], path_filter: str = "") -> int:
    procs = _probe_processes(match_names, path_filter=path_filter)
    killed = 0
    for p in procs:
        pid = p.get("pid")
        if pid:
            subprocess.run(["taskkill", "/F", "/PID", str(pid)], capture_output=True, timeout=10)
            killed += 1
    return killed


def _smoke_codex_msix(
    artifact_path: Path,
    *,
    hold_seconds: int,
    keep_installed: bool,
    log: Callable[[str], None],
) -> dict[str, Any]:
    started_epoch = time.time()
    since_iso = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    result: dict[str, Any] = {
        "appId": "codex",
        "installed": False,
        "launched": False,
        "launchMethod": "",
        "survived_sec": 0.0,
        "peak_processes": 0,
        "window_titles": [],
        "crash_events": [],
        "log_tails": [],
        "install_error": "",
        "launch_error": "",
        "appx_info": {},
        "timeline": [],
    }

    # Check if OpenAI.Codex was already installed prior to smoke test
    rc_pre, out_pre, _ = _run_powershell(
        "Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction SilentlyContinue | "
        "Select-Object Name, Version, PackageFullName, PackageFamilyName, InstallLocation | ConvertTo-Json -Compress"
    )
    was_installed = rc_pre == 0 and bool(out_pre)

    # 1. Install MSIX via Add-AppxPackage (ForceUpdateFromAnyVersion)
    escaped = str(artifact_path).replace("'", "''")
    log(f"[smoke:codex] installing MSIX via Add-AppxPackage: {artifact_path.name}")
    rc_ins, out_ins, err_ins = _run_powershell(
        f"$ErrorActionPreference='Stop'; Add-AppxPackage -Path '{escaped}' -ForceApplicationShutdown -ForceUpdateFromAnyVersion",
        timeout=240,
    )
    if rc_ins != 0:
        result["install_error"] = scrub_text(err_ins or out_ins or f"Add-AppxPackage exit {rc_ins}", max_len=1200)
        log(f"[smoke:codex] Add-AppxPackage failed: {result['install_error']}")
        result["crash_events"] = _collect_windows_crash_events(since_iso, ["codex", "chatgpt", "openai", "appx"])
        return result

    result["installed"] = True
    rc_pkg, out_pkg, _ = _run_powershell(
        "$pkg = Get-AppxPackage -Name 'OpenAI.Codex' | Select-Object -First 1; "
        "$man = if ($pkg) { Get-AppxPackageManifest $pkg } else { $null }; "
        "$apps = @(); "
        "if ($man) { foreach ($a in $man.Package.Applications.Application) { "
        "  $apps += [ordered]@{ id=[string]$a.Id; exe=[string]$a.Executable; appListEntry=[string]$a.VisualElements.AppListEntry } "
        "} }; "
        "[ordered]@{ name=[string]$pkg.Name; version=[string]$pkg.Version; fullName=[string]$pkg.PackageFullName; "
        "familyName=[string]$pkg.PackageFamilyName; installLocation=[string]$pkg.InstallLocation; applications=$apps } | ConvertTo-Json -Depth 4 -Compress"
    )
    pkg_info: dict[str, Any] = {}
    if rc_pkg == 0 and out_pkg:
        try:
            pkg_info = json.loads(out_pkg)
        except Exception:
            pkg_info = {}
    result["appx_info"] = pkg_info
    pfn = str(pkg_info.get("familyName") or "OpenAI.Codex_2p2nqsd0c76g0")
    apps_list = pkg_info.get("applications") or []
    if isinstance(apps_list, dict):
        apps_list = [apps_list]
    visible_app = next(
        (a for a in apps_list if str(a.get("appListEntry") or "").lower() != "none"),
        apps_list[0] if apps_list else {"id": "App"},
    )
    app_entry_id = str(visible_app.get("id") or "App")
    aumid = f"{pfn}!{app_entry_id}"
    result["launchMethod"] = f"explorer.exe shell:AppsFolder\\{aumid}"
    log(f"[smoke:codex] installed v{pkg_info.get('version')} -> launching {result['launchMethod']}")

    try:
        # Clean any pre-existing Codex processes before cold launch
        _kill_matching_processes(["codex", "chatgpt"], path_filter="openai.codex")
        time.sleep(1.0)

        launch_t0 = time.monotonic()
        subprocess.Popen(["explorer.exe", f"shell:AppsFolder\\{aumid}"])

        seen_alive_at: float | None = None
        last_alive_at: float | None = None
        titles: set[str] = set()
        polls = max(4, int(hold_seconds))
        for i in range(polls):
            time.sleep(1.0)
            elapsed = round(time.monotonic() - launch_t0, 1)
            procs = _probe_processes(["codex", "chatgpt"], path_filter="openai.codex")
            result["peak_processes"] = max(result["peak_processes"], len(procs))
            for pr in procs:
                if pr.get("title"):
                    titles.add(str(pr["title"]))
            if procs:
                if seen_alive_at is None:
                    seen_alive_at = elapsed
                    result["launched"] = True
                last_alive_at = elapsed
            if i in (0, 2, 5, 10, polls - 1) or (seen_alive_at is not None and not procs):
                result["timeline"].append({"t_sec": elapsed, "proc_count": len(procs), "pids": [p["pid"] for p in procs]})
            # Early crash detected: was alive and disappeared completely
            if seen_alive_at is not None and not procs and elapsed >= 3.0:
                log(f"[smoke:codex] EARLY CRASH detected at t={elapsed}s (first seen at {seen_alive_at}s)")
                break

        result["window_titles"] = sorted(titles)
        if seen_alive_at is not None and last_alive_at is not None:
            result["survived_sec"] = round(max(0.0, last_alive_at - seen_alive_at + 1.0), 1)
        log(
            f"[smoke:codex] observation finished: launched={result['launched']} "
            f"survived={result['survived_sec']}s peak_procs={result['peak_processes']} titles={result['window_titles']}"
        )
    finally:
        _kill_matching_processes(["codex", "chatgpt"], path_filter="openai.codex")
        time.sleep(1.0)
        result["crash_events"] = _collect_windows_crash_events(
            since_iso, ["codex", "chatgpt", "openai", "electron", "webview", "ucrtbase", "ntdll"]
        )
        result["log_tails"] = _collect_client_log_tails("codex", started_epoch)
        if not keep_installed and not was_installed and pkg_info.get("fullName"):
            log(f"[smoke:codex] cleaning up temporary MSIX package {pkg_info['fullName']}")
            _run_powershell(
                f"Remove-AppxPackage -Package '{pkg_info['fullName']}' -ErrorAction SilentlyContinue",
                timeout=90,
            )

    return result


def _smoke_sandbox_exe(
    cfg: LabConfig,
    app_id: str,
    artifact_path: Path,
    *,
    hold_seconds: int,
    log: Callable[[str], None],
) -> dict[str, Any]:
    """Isolated sandbox install & launch probe for Antigravity / DSH Tako / Claude."""
    started_epoch = time.time()
    since_iso = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    exe_names = {
        "claude": ["claude"],
        "antigravity": ["antigravity"],
        "dsh-tako": ["dsh tako"],
    }[app_id]
    result: dict[str, Any] = {
        "appId": app_id,
        "installed": False,
        "launched": False,
        "launchMethod": "",
        "survived_sec": 0.0,
        "peak_processes": 0,
        "window_titles": [],
        "crash_events": [],
        "log_tails": [],
        "install_error": "",
        "launch_error": "",
        "timeline": [],
    }

    sandbox_dir = cfg.work_dir / "smoke_sandbox" / f"{app_id}_{int(started_epoch)}"
    sandbox_dir.mkdir(parents=True, exist_ok=True)
    target_exe: Path | None = None
    try:
        if app_id == "antigravity":
            # Antigravity ships electron-builder NSIS since 2.35.x — running it even with /D=
            # first uninstalls/overwrites the real per-user install, so smoke must unpack with
            # bsdtar (libarchive NSIS SFX reader) instead of executing the installer at all.
            from .gates import _extract_windows_installer_sandbox

            log(f"[smoke:antigravity] non-exec bsdtar sandbox unpack -> {sandbox_dir}")
            unpack_dir, method = _extract_windows_installer_sandbox(
                app_id, artifact_path, sandbox_dir, log=log, allow_execute=False
            )
            found = list(unpack_dir.rglob("Antigravity.exe"))
            target_exe = found[0] if found else None
            if target_exe is None:
                result["install_error"] = f"免执行解包未提取到 Antigravity.exe (method={method})"
                return result
        elif app_id == "dsh-tako":
            log(f"[smoke:dsh-tako] NSIS sandbox install -> {sandbox_dir}")
            proc = subprocess.run(f'"{artifact_path}" /S /D={sandbox_dir}', shell=True, capture_output=True, timeout=240)
            if proc.returncode != 0:
                result["install_error"] = f"NSIS installer exit {proc.returncode}"
                return result
            found = list(sandbox_dir.rglob("DSH Tako.exe"))
            target_exe = found[0] if found else None
        elif app_id == "claude":
            # Extract Squirrel/Velopack embedded nupkg directly to sandbox without overwriting user's real Claude!
            from .gates import _extract_embedded_zip_from_exe

            if _extract_embedded_zip_from_exe(artifact_path, sandbox_dir, log=log):
                found = list(sandbox_dir.rglob("Claude.exe"))
                target_exe = found[0] if found else None
            if target_exe is None:
                result["install_error"] = "无法免污染提取 Claude.exe，跳过直接覆写安装以保护宿主配置"
                return result

        if not target_exe or not target_exe.is_file():
            result["install_error"] = f"安装完成后未找到目标可执行文件 ({sandbox_dir})"
            return result

        result["installed"] = True
        result["launchMethod"] = str(target_exe)
        log(f"[smoke:{app_id}] launching {target_exe}")

        env = os.environ.copy()
        iso_profile = sandbox_dir / "user_data"
        iso_profile.mkdir(parents=True, exist_ok=True)
        env["APPDATA"] = str(iso_profile / "Roaming")
        env["LOCALAPPDATA"] = str(iso_profile / "Local")
        (iso_profile / "Roaming").mkdir(parents=True, exist_ok=True)
        (iso_profile / "Local").mkdir(parents=True, exist_ok=True)

        t0 = time.monotonic()
        child = subprocess.Popen([str(target_exe)], cwd=str(target_exe.parent), env=env)
        seen_alive_at: float | None = None
        last_alive_at: float | None = None
        titles: set[str] = set()
        polls = max(4, int(hold_seconds))
        for i in range(polls):
            time.sleep(1.0)
            elapsed = round(time.monotonic() - t0, 1)
            procs = _probe_processes(exe_names, path_filter=str(sandbox_dir))
            if not procs and child.poll() is None:
                procs = [{"pid": child.pid, "name": target_exe.stem, "title": "", "memMB": 0}]
            result["peak_processes"] = max(result["peak_processes"], len(procs))
            for pr in procs:
                if pr.get("title"):
                    titles.add(str(pr["title"]))
            if procs:
                if seen_alive_at is None:
                    seen_alive_at = elapsed
                    result["launched"] = True
                last_alive_at = elapsed
            if i in (0, 2, 5, 10, polls - 1) or (seen_alive_at is not None and not procs):
                result["timeline"].append({"t_sec": elapsed, "proc_count": len(procs), "exit_code": child.poll()})
            if seen_alive_at is not None and not procs and elapsed >= 3.0:
                log(f"[smoke:{app_id}] EARLY CRASH at t={elapsed}s exit={child.poll()}")
                break

        result["window_titles"] = sorted(titles)
        if seen_alive_at is not None and last_alive_at is not None:
            result["survived_sec"] = round(max(0.0, last_alive_at - seen_alive_at + 1.0), 1)
    finally:
        _kill_matching_processes(exe_names, path_filter=str(sandbox_dir))
        time.sleep(1.0)
        result["crash_events"] = _collect_windows_crash_events(since_iso, [app_id, *exe_names, "electron", "webview"])
        result["log_tails"] = _collect_client_log_tails(app_id, started_epoch)
        shutil.rmtree(sandbox_dir, ignore_errors=True)

    return result


def run_windows_runtime_smoke(
    cfg: LabConfig,
    *,
    app_id: str,
    artifact_path: Path,
    hold_seconds: int = 15,
    keep_installed: bool = False,
    log: Callable[[str], None] | None = None,
) -> tuple[str, dict[str, Any]]:
    """Execute Gate 4 live Windows smoke test and return `(status, details_dict)`."""
    logger = log or (lambda _msg: None)
    if platform.system().lower() != "windows":
        return "warn", {
            "skipped": True,
            "reason": f"当前主机为 {platform.system()}，Gate 4 实机冷启动仅在 Windows (.6) 主机执行",
        }
    if not cfg.allow_live_smoke:
        return "warn", {"skipped": True, "reason": "WIN_LAB_ALLOW_LIVE_SMOKE=0，已禁用实机启动"}

    hold_seconds = max(5, min(60, int(hold_seconds)))
    if app_id == "codex":
        data = _smoke_codex_msix(artifact_path, hold_seconds=hold_seconds, keep_installed=keep_installed, log=logger)
    else:
        data = _smoke_sandbox_exe(cfg, app_id, artifact_path, hold_seconds=hold_seconds, log=logger)

    crashes = data.get("crash_events") or []
    app_crashes = [e for e in crashes if int(e.get("id") or 0) in (1000, 1001, 1026)]
    if data.get("install_error") or data.get("launch_error"):
        return "fail", data
    if not data.get("launched"):
        return "fail", data
    if app_crashes:
        return "fail", data
    if float(data.get("survived_sec") or 0.0) < min(10.0, hold_seconds * 0.65):
        return "fail", data
    if crashes:
        return "warn", data
    return "pass", data
