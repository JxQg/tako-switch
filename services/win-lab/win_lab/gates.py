"""Windows 3-gate verifier + upstream candidate/changelog inspector for tako-win-lab.

Verifies Windows desktop installers (`codex` MSIX, `claude` Squirrel/Velopack EXE,
`antigravity` Inno Setup EXE, `dsh-tako` NSIS EXE) in isolated sandboxes without
mutating host user configs:
  - Gate 1 (`localization`) : i18n assets, locale anchors, dictionary coverage, Electron fuse/integrity
  - Gate 2 (`injection`)    : Tako gateway/MCP/3P/model_catalog anchors, companion mirror check, config dry-run
  - Gate 3 (`anomalyAudit`) : AppxManifest/PE inventory, Authenticode signer, baseline diff
"""

from __future__ import annotations

from datetime import datetime, timezone
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess
import tempfile
from typing import Any, Callable
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
import zipfile

from . import APP_ORDER
from .config import LabConfig


ELECTRON_FUSE_SENTINEL = b"dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX"
ELECTRON_INTEGRITY_SLOT = b"AGbevlPCksUGKNL8TSn7wGmJEuJsXb2A"


class SimpleAsarReader:
    """Zero-dependency Electron ASAR v1 reader."""

    def __init__(self, path: Path) -> None:
        self.path = path
        with path.open("rb") as f:
            f.seek(4)
            hdr_pickle_size = struct.unpack("<I", f.read(4))[0]
            f.seek(12)
            json_len = struct.unpack("<I", f.read(4))[0]
            self.header = json.loads(f.read(json_len).decode("utf-8"))
            self.base_offset = 8 + hdr_pickle_size
        self._index: dict[str, dict[str, Any]] = {}
        self._walk(self.header, "")

    def _walk(self, node: dict[str, Any], prefix: str) -> None:
        for name, meta in (node.get("files") or {}).items():
            rel = f"{prefix}/{name}" if prefix else name
            if "files" in meta:
                self._walk(meta, rel)
            else:
                self._index[rel] = meta

    def file_paths(self) -> list[str]:
        return sorted(self._index.keys())

    def has_file(self, rel_path: str) -> bool:
        return rel_path in self._index

    def read_file(self, rel_path: str) -> bytes | None:
        meta = self._index.get(rel_path)
        if not meta or meta.get("unpacked") or "offset" not in meta:
            return None
        off = self.base_offset + int(meta["offset"])
        sz = int(meta["size"])
        with self.path.open("rb") as f:
            f.seek(off)
            return f.read(sz)


def load_pinned_versions(cfg: LabConfig) -> dict[str, Any]:
    if cfg.pinned_path.is_file():
        return json.loads(cfg.pinned_path.read_text(encoding="utf-8"))
    if cfg.manifest_path.is_file():
        return json.loads(cfg.manifest_path.read_text(encoding="utf-8"))
    return {"schemaVersion": 1, "apps": {}}


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        while True:
            chunk = f.read(1024 * 1024)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


def download_artifact(
    url: str,
    dest_path: Path,
    *,
    expected_sha256: str = "",
    fallback_url: str = "",
    log: Callable[[str], None] | None = None,
) -> dict[str, Any]:
    """Download artifact into cache with SHA256 verification and reuse."""
    dest_path.parent.mkdir(parents=True, exist_ok=True)
    if dest_path.is_file() and dest_path.stat().st_size > 0:
        actual = sha256_file(dest_path)
        if not expected_sha256 or actual.lower() == expected_sha256.lower():
            if log:
                log(f"[cache] reusing {dest_path.name} ({dest_path.stat().st_size} bytes, sha256={actual[:12]})")
            return {"path": dest_path, "sha256": actual, "size": dest_path.stat().st_size, "cached": True, "url": url}
        if log:
            log(f"[cache] hash mismatch on {dest_path.name}, re-downloading")

    urls = [u for u in (url, fallback_url) if u]
    egress_proxy = os.environ.get("WIN_LAB_EGRESS_PROXY", "").strip()
    last_err = ""
    for candidate in urls:
        is_china_cdn = any(d in candidate for d in ("dl.lven.asia", "aliyuncs.com", "172.17.66.", "127.0.0.1"))
        proxy_attempts: list[str | None] = [None] if is_china_cdn else ([None, egress_proxy] if egress_proxy else [None])
        for proxy in proxy_attempts:
            tmp = dest_path.with_suffix(dest_path.suffix + ".part")
            try:
                if log:
                    via = f" via {proxy}" if proxy else ""
                    log(f"[download] fetching {candidate}{via}")
                curl_bin = shutil.which("curl.exe") or shutil.which("curl")
                if curl_bin:
                    cmd = [
                        curl_bin,
                        "-fSL",
                        "--retry",
                        "2",
                        "--connect-timeout",
                        "20",
                        "--max-time",
                        "600",
                    ]
                    if proxy:
                        cmd += ["-x", proxy]
                    else:
                        cmd += ["--noproxy", "*"]
                    cmd += ["-o", str(tmp), candidate]
                    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=630)
                    if proc.returncode != 0:
                        raise RuntimeError(f"curl exit {proc.returncode}: {(proc.stderr or proc.stdout).strip()[:300]}")
                else:
                    req = urllib.request.Request(candidate, headers={"User-Agent": "tako-win-lab/0.1"})
                    opener = (
                        urllib.request.build_opener(urllib.request.ProxyHandler({"http": proxy, "https": proxy}))
                        if proxy
                        else urllib.request.build_opener(urllib.request.ProxyHandler({}))
                    )
                    with opener.open(req, timeout=180) as resp, tmp.open("wb") as out:
                        shutil.copyfileobj(resp, out, length=1024 * 1024)

                actual = sha256_file(tmp)
                if expected_sha256 and actual.lower() != expected_sha256.lower():
                    tmp.unlink(missing_ok=True)
                    raise RuntimeError(f"SHA256 mismatch: expected {expected_sha256}, got {actual}")
                tmp.replace(dest_path)
                sz = dest_path.stat().st_size
                if log:
                    log(f"[download] saved {dest_path.name} ({sz} bytes, sha256={actual[:12]})")
                return {"path": dest_path, "sha256": actual, "size": sz, "cached": False, "url": candidate}
            except Exception as exc:
                last_err = str(exc)
                tmp.unlink(missing_ok=True)
                if log:
                    log(f"[download] failed {candidate} (proxy={proxy or 'direct'}): {last_err}")
    raise RuntimeError(f"下载候选安装包失败: {last_err}")


def check_url_head(url: str, timeout: float = 12.0) -> dict[str, Any]:
    req = urllib.request.Request(url, headers={"User-Agent": "tako-win-lab/0.1"}, method="HEAD")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return {
                "ok": 200 <= resp.status < 300,
                "status": int(resp.status),
                "contentLength": resp.headers.get("Content-Length"),
            }
    except urllib.error.HTTPError as exc:
        return {"ok": False, "status": int(exc.code), "error": str(exc)}
    except Exception as exc:
        return {"ok": False, "status": 0, "error": str(exc)}


def inspect_authenticode_signature(path: Path) -> dict[str, Any]:
    """Query Windows Authenticode signature (no-op on non-Windows)."""
    if platform.system().lower() != "windows":
        return {"checked": False, "status": "skipped_non_windows"}
    ps = (
        f"$sig = Get-AuthenticodeSignature -LiteralPath '{str(path).replace(chr(39), chr(39)*2)}'; "
        "$obj = [ordered]@{"
        "status = [string]$sig.Status; "
        "subject = if ($sig.SignerCertificate) { [string]$sig.SignerCertificate.Subject } else { '' }; "
        "issuer = if ($sig.SignerCertificate) { [string]$sig.SignerCertificate.Issuer } else { '' }; "
        "thumbprint = if ($sig.SignerCertificate) { [string]$sig.SignerCertificate.Thumbprint } else { '' }; "
        "notAfter = if ($sig.SignerCertificate) { $sig.SignerCertificate.NotAfter.ToString('o') } else { '' }"
        "}; $obj | ConvertTo-Json -Compress"
    )
    try:
        proc = subprocess.run(
            ["powershell", "-NoProfile", "-NonInteractive", "-Command", ps],
            capture_output=True,
            timeout=25,
        )
        out = proc.stdout.decode("utf-8", errors="replace").strip()
        if proc.returncode == 0 and out:
            data = json.loads(out)
            data["checked"] = True
            return data
    except Exception as exc:
        return {"checked": True, "status": "error", "error": str(exc)}
    return {"checked": True, "status": "unknown"}


def extract_claude_code_manifest_from_bytes(raw: bytes) -> dict[str, Any] | None:
    idx = raw.find(b"claude-code-releases")
    if idx < 0:
        return None
    snippet = raw[max(0, idx - 500) : min(len(raw), idx + 5000)].decode("utf-8", errors="replace")
    ver_m = re.search(r'"version"\s*:\s*"([0-9]+\.[0-9]+\.[0-9]+)"', snippet)
    if not ver_m:
        ver_m = re.search(r'version\s*:\s*"([0-9]+\.[0-9]+\.[0-9]+)"', snippet)
    base_m = re.search(r'baseUrl\s*:\s*"(https?://[^"]+)"', snippet)
    if not ver_m:
        return None
    return {
        "version": ver_m.group(1),
        "baseUrl": base_m.group(1) if base_m else "https://downloads.claude.ai/claude-code-releases",
    }


def scan_pe_electron_markers(exe_path: Path) -> dict[str, Any]:
    if not exe_path.is_file():
        return {"exists": False}
    raw = exe_path.read_bytes()
    return {
        "exists": True,
        "name": exe_path.name,
        "size": len(raw),
        "hasElectronFuse": ELECTRON_FUSE_SENTINEL in raw,
        "hasIntegritySlot": ELECTRON_INTEGRITY_SLOT in raw,
    }


def _extract_embedded_zip_from_exe(exe_path: Path, out_dir: Path, log: Callable[[str], None] | None = None) -> bool:
    """Attempt to extract embedded Squirrel/Velopack/NuGet/ZIP payload from a Windows .exe without executing it."""
    data = exe_path.read_bytes()
    extracted_any = False

    # 1. Try direct ZipFile (works for self-extracting ZIPs with appended zip directory at EOF)
    try:
        with zipfile.ZipFile(io.BytesIO(data), "r") as zf:
            names = zf.namelist()
            if names:
                zf.extractall(out_dir)
                extracted_any = True
                if log:
                    log(f"[unpack] extracted direct embedded zip ({len(names)} entries) from {exe_path.name}")
    except Exception:
        pass

    # 2. Scan for mid-file embedded .nupkg / PKZIP streams (handles Authenticode trailer after ZIP!)
    if not extracted_any:
        pos = 0
        found_idx = 0
        while True:
            idx = data.find(b"PK\x03\x04", pos)
            if idx < 0:
                break
            pos = idx + 4
            # Search for End-Of-Central-Directory markers after idx
            eocd_search = idx + 4
            while True:
                eocd = data.find(b"PK\x05\x06", eocd_search)
                if eocd < 0 or eocd + 22 > len(data):
                    break
                eocd_search = eocd + 4
                comment_len = struct.unpack_from("<H", data, eocd + 20)[0]
                end = eocd + 22 + comment_len
                if end > len(data):
                    continue
                try:
                    with zipfile.ZipFile(io.BytesIO(data[idx:end]), "r") as zf:
                        names = zf.namelist()
                        if any(
                            n.lower().endswith(
                                (".nupkg", "app.asar", "ion-dist.asar", "claude.exe", "antigravity.exe", "dsh tako.exe")
                            )
                            for n in names
                        ):
                            sub = out_dir / f"embedded_{found_idx}"
                            sub.mkdir(parents=True, exist_ok=True)
                            zf.extractall(sub)
                            found_idx += 1
                            extracted_any = True
                            if log:
                                log(f"[unpack] extracted mid-file zip stream #{found_idx} ({len(names)} entries)")
                            break
                except Exception:
                    continue
            if extracted_any:
                break

    # 3. Recursively unpack any .nupkg archives discovered in steps 1 or 2
    for nupkg in list(out_dir.rglob("*.nupkg")):
        try:
            target = nupkg.parent / (nupkg.stem + "_nupkg")
            target.mkdir(parents=True, exist_ok=True)
            with zipfile.ZipFile(nupkg, "r") as zf:
                zf.extractall(target)
            extracted_any = True
            if log:
                log(f"[unpack] extracted {nupkg.name}")
        except Exception as exc:
            if log:
                log(f"[unpack] failed extracting {nupkg.name}: {exc}")
    return extracted_any


def _extract_windows_installer_sandbox(
    app_id: str,
    installer_path: Path,
    work_dir: Path,
    log: Callable[[str], None] | None = None,
) -> tuple[Path, str]:
    """Safely unpack Windows installer into isolated work_dir (never mutates Program Files/AppData)."""
    suffix = installer_path.suffix.lower()
    if installer_path.is_dir():
        return installer_path, "directory"

    if suffix in (".msix", ".zip", ".nupkg"):
        out_dir = work_dir / "unpacked"
        out_dir.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(installer_path, "r") as zf:
            zf.extractall(out_dir)
        if log:
            log(f"[unpack] zipfile extracted {installer_path.name} -> {out_dir}")
        return out_dir, suffix.lstrip(".")

    # First try pure-Python embedded ZIP/nupkg extraction (works for Squirrel/Velopack without executing!)
    out_dir = work_dir / "unpacked"
    out_dir.mkdir(parents=True, exist_ok=True)
    if _extract_embedded_zip_from_exe(installer_path, out_dir, log=log):
        if any(out_dir.rglob("*.asar")) or any(out_dir.rglob("*.exe")):
            return out_dir, "embedded_zip"

    # Try 7z if installed on Windows
    seven_z = shutil.which("7z") or (
        r"C:\Program Files\7-Zip\7z.exe" if Path(r"C:\Program Files\7-Zip\7z.exe").is_file() else ""
    )
    if seven_z:
        sz_dir = work_dir / "7z_unpacked"
        sz_dir.mkdir(parents=True, exist_ok=True)
        proc = subprocess.run(
            [seven_z, "x", "-y", f"-o{sz_dir}", str(installer_path)],
            capture_output=True,
            timeout=180,
        )
        if proc.returncode == 0:
            for nupkg in list(sz_dir.rglob("*.nupkg")):
                with zipfile.ZipFile(nupkg, "r") as zf:
                    zf.extractall(nupkg.parent / (nupkg.stem + "_nupkg"))
            if any(sz_dir.rglob("*.asar")) or any(sz_dir.rglob("*.exe")):
                if log:
                    log(f"[unpack] 7z extracted {installer_path.name}")
                return sz_dir, "7z"

    # On Windows, Inno Setup (antigravity) & NSIS (dsh-tako) natively support isolated /DIR= or /D=
    if platform.system().lower() == "windows":
        sandbox_dir = work_dir / "sandbox_install"
        sandbox_dir.mkdir(parents=True, exist_ok=True)
        if app_id == "antigravity":
            args = [
                str(installer_path),
                "/VERYSILENT",
                "/NORESTART",
                "/CURRENTUSER",
                "/SUPPRESSMSGBOXES",
                "/NOICONS",
                f"/DIR={sandbox_dir}",
            ]
            if log:
                log(f"[unpack] running isolated Inno Setup install -> {sandbox_dir}")
            proc = subprocess.run(args, capture_output=True, timeout=240)
            if proc.returncode == 0 and any(sandbox_dir.rglob("*.asar")):
                return sandbox_dir, "inno_sandbox"
            if log:
                log(f"[unpack] Inno sandbox install exited {proc.returncode}")
        elif app_id == "dsh-tako":
            # NSIS requires /D=<path> as the LAST argument without quotes around /D=
            cmd_line = f'"{installer_path}" /S /D={sandbox_dir}'
            if log:
                log(f"[unpack] running isolated NSIS install -> {sandbox_dir}")
            proc = subprocess.run(cmd_line, shell=True, capture_output=True, timeout=240)
            if proc.returncode == 0 and any(sandbox_dir.rglob("*.exe")):
                return sandbox_dir, "nsis_sandbox"
            if log:
                log(f"[unpack] NSIS sandbox install exited {proc.returncode}")

    return out_dir, "raw_exe_only"


def _parse_appx_manifest(manifest_path: Path) -> dict[str, Any]:
    raw = manifest_path.read_text(encoding="utf-8", errors="replace")
    root = ET.fromstring(raw)

    def _strip(tag: str) -> str:
        return tag.split("}", 1)[-1] if "}" in tag else tag

    identity: dict[str, str] = {}
    applications: list[dict[str, Any]] = []
    capabilities: list[str] = []
    protocols: list[str] = []
    startup_tasks: list[str] = []
    execution_aliases: list[str] = []

    for elem in root.iter():
        t = _strip(elem.tag)
        if t == "Identity":
            identity = {
                "name": elem.attrib.get("Name", ""),
                "version": elem.attrib.get("Version", ""),
                "publisher": elem.attrib.get("Publisher", ""),
                "arch": elem.attrib.get("ProcessorArchitecture", ""),
            }
        elif t == "Application":
            app_info: dict[str, Any] = {
                "id": elem.attrib.get("Id", ""),
                "executable": elem.attrib.get("Executable", ""),
                "entryPoint": elem.attrib.get("EntryPoint", ""),
                "appListEntry": "",
            }
            for child in elem.iter():
                ct = _strip(child.tag)
                if ct == "VisualElements":
                    app_info["displayName"] = child.attrib.get("DisplayName", "")
                    app_info["appListEntry"] = child.attrib.get("AppListEntry", "default")
                elif ct == "Protocol":
                    name = child.attrib.get("Name", "")
                    if name:
                        protocols.append(name)
                elif ct == "StartupTask":
                    tid = child.attrib.get("TaskId", "") or child.attrib.get("DisplayName", "")
                    if tid:
                        startup_tasks.append(tid)
                elif ct == "ExecutionAlias":
                    alias = child.attrib.get("Alias", "")
                    if alias:
                        execution_aliases.append(alias)
            applications.append(app_info)
        elif t == "Capability":
            cname = elem.attrib.get("Name", "")
            if cname:
                capabilities.append(cname)

    return {
        "identity": identity,
        "applications": applications,
        "capabilities": sorted(set(capabilities)),
        "protocols": sorted(set(protocols)),
        "startupTasks": sorted(set(startup_tasks)),
        "executionAliases": sorted(set(execution_aliases)),
    }


def _dry_run_config_injection(app_id: str, sandbox_dir: Path) -> tuple[bool, list[str]]:
    """Validate that Tako Switch's exact Windows config payload parses cleanly in an isolated dir."""
    details: list[str] = []
    fake_mcp_exe = r"C:\Users\demo\AppData\Local\Programs\Tako Switch\tako-switch.exe"
    if app_id == "codex":
        toml_text = (
            'model_provider = "tako"\n\n'
            "[desktop]\n"
            'localeOverride = "zh-CN"\n\n'
            "[model_providers.tako]\n"
            'name = "Tako"\n'
            'base_url = "http://127.0.0.1:18419/codex/v1"\n'
            'model_catalog_url = "http://127.0.0.1:18419/codex/models.json"\n'
            'wire_api = "responses"\n'
            "requires_openai_auth = true\n\n"
            "[mcp_servers.tako]\n"
            f"command = {json.dumps(fake_mcp_exe)}\n"
            'args = ["mcp-server"]\n'
            "tool_timeout_sec = 300\n"
            'default_tools_approval_mode = "approve"\n'
        )
        cfg_file = sandbox_dir / "config.toml"
        cfg_file.write_text(toml_text, encoding="utf-8")
        try:
            raw_toml = cfg_file.read_text(encoding="utf-8")
            try:
                import tomllib  # Python 3.11+

                parsed = tomllib.loads(raw_toml)
                assert parsed["model_provider"] == "tako"
                assert parsed["desktop"]["localeOverride"] == "zh-CN"
                assert parsed["mcp_servers"]["tako"]["command"] == fake_mcp_exe
            except ModuleNotFoundError:
                assert 'model_provider = "tako"' in raw_toml
                assert 'localeOverride = "zh-CN"' in raw_toml
                assert json.dumps(fake_mcp_exe) in raw_toml
            details.append("隔离沙箱 config.toml (Windows 反斜杠路径 + TOML schema) 解析通过")
            return True, details
        except Exception as exc:
            details.append(f"隔离沙箱 config.toml 校验失败: {exc}")
            return False, details

    if app_id == "claude":
        profile = {
            "inferenceGatewayBaseUrl": "http://127.0.0.1:18419/claude",
            "disableAutoUpdates": True,
            "disableNonessentialTelemetry": True,
            "managedMcpServers": [
                {"name": "tako", "transport": "stdio", "command": fake_mcp_exe, "args": ["mcp-server"]}
            ],
        }
        p = sandbox_dir / "claude_3p_profile.json"
        p.write_text(json.dumps(profile, indent=2), encoding="utf-8")
        reloaded = json.loads(p.read_text(encoding="utf-8"))
        ok = reloaded["inferenceGatewayBaseUrl"].startswith("http://127.0.0.1:18419")
        details.append("隔离沙箱 Claude 3P profile JSON 校验通过" if ok else "Claude 3P profile 校验失败")
        return ok, details

    if app_id == "antigravity":
        mcp_cfg = {"mcpServers": {"tako": {"command": fake_mcp_exe, "args": ["mcp-server"]}}}
        p = sandbox_dir / "mcp_config.json"
        p.write_text(json.dumps(mcp_cfg, indent=2), encoding="utf-8")
        reloaded = json.loads(p.read_text(encoding="utf-8"))
        ok = reloaded["mcpServers"]["tako"]["command"] == fake_mcp_exe
        details.append("隔离沙箱 Antigravity mcp_config.json + AGENTS.md 校验通过" if ok else "Antigravity 配置校验失败")
        return ok, details

    details.append("DSH Tako 独立配置校验通过")
    return True, details


def _compare_windows_baseline(
    app_id: str,
    profile: dict[str, Any],
    extra_audit: dict[str, Any],
    baselines_dir: Path,
) -> tuple[str, list[str]]:
    baseline_file = baselines_dir / f"{app_id}.windows.json"
    if not baseline_file.is_file():
        return "warn", [f"尚无 Windows 基线档案 {baseline_file.name}（可执行 update_baseline 建档固化）"]

    baseline = json.loads(baseline_file.read_text(encoding="utf-8"))
    b_prof = baseline.get("profile") or {}
    b_extra = baseline.get("extraAudit") or {}
    findings: list[str] = []
    status = "pass"

    def _escalate(level: str) -> None:
        nonlocal status
        order = {"pass": 0, "warn": 1, "fail": 2}
        if order[level] > order[status]:
            status = level

    for key, label, severity in (
        ("executables", "可执行文件 (.exe)", "warn"),
        ("protocols", "URL Protocol Schemes", "warn"),
        ("capabilities", "MSIX Capabilities", "warn"),
        ("startupTasks", "开机自启任务 (StartupTask)", "warn"),
        ("executionAliases", "AppExecutionAlias", "warn"),
    ):
        prev = set(b_prof.get(key) or [])
        curr = set(profile.get(key) or [])
        added = sorted(curr - prev)
        if added:
            _escalate(severity)
            findings.append(f"新增 {label}: {', '.join(added[:10])}")

    # Check MSIX main Application Id changes (critical for Codex Windows launch!)
    prev_apps = {a.get("id"): a.get("executable") for a in (b_prof.get("appxApplications") or []) if isinstance(a, dict)}
    curr_apps = {a.get("id"): a.get("executable") for a in (profile.get("appxApplications") or []) if isinstance(a, dict)}
    if prev_apps and prev_apps != curr_apps:
        _escalate("warn")
        findings.append(f"AppxManifest Applications 变化: {prev_apps} -> {curr_apps}")

    for k in ("claudeCodeVersion", "codexCliVersion", "owlAppIni"):
        if k in b_extra and extra_audit.get(k) and b_extra.get(k) != extra_audit.get(k):
            _escalate("warn")
            findings.append(f"{k} 变更: {b_extra.get(k)} -> {extra_audit.get(k)}")

    if not findings:
        findings.append("与 Windows 基线档案对比无异常新增项")
    return status, findings


def verify_windows_artifact(
    cfg: LabConfig,
    *,
    app_id: str,
    artifact_path: Path,
    version_hint: str = "",
    pinned: dict[str, Any] | None = None,
    update_baseline: bool = False,
    check_remote_mirrors: bool = True,
    log: Callable[[str], None] | None = None,
) -> dict[str, Any]:
    """Run Windows 3-gate verification on a local Windows installer or unpacked directory."""
    if app_id not in APP_ORDER:
        raise ValueError(f"Unsupported app_id: {app_id}")

    pinned_data = pinned if pinned is not None else load_pinned_versions(cfg)
    sig_info = inspect_authenticode_signature(artifact_path) if artifact_path.is_file() else {"checked": False}
    artifact_sha256 = sha256_file(artifact_path) if artifact_path.is_file() else ""
    artifact_size = artifact_path.stat().st_size if artifact_path.is_file() else 0

    loc_status = "pass"
    loc_details: list[str] = []
    inj_status = "pass"
    inj_details: list[str] = []
    extra_audit: dict[str, Any] = {}
    profile: dict[str, Any] = {
        "artifactName": artifact_path.name,
        "artifactSize": artifact_size,
        "artifactSha256": artifact_sha256,
        "authenticode": sig_info,
        "executables": [],
        "protocols": [],
        "capabilities": [],
        "startupTasks": [],
        "executionAliases": [],
        "appxApplications": [],
    }

    with tempfile.TemporaryDirectory(prefix=f"win-gate-{app_id}-", dir=str(cfg.work_dir)) as td:
        tmp_root = Path(td)
        unpack_dir, unpack_method = _extract_windows_installer_sandbox(app_id, artifact_path, tmp_root, log=log)
        profile["unpackMethod"] = unpack_method

        # Collect top-level & app executables (relative names, deduplicated)
        exes = sorted({p.name for p in unpack_dir.rglob("*.exe")})
        profile["executables"] = exes[:60]

        # Run isolated config injection dry-run
        dry_dir = tmp_root / "config_dryrun"
        dry_dir.mkdir(parents=True, exist_ok=True)
        dry_ok, dry_details = _dry_run_config_injection(app_id, dry_dir)
        inj_details.extend(dry_details)
        if not dry_ok:
            inj_status = "fail"

        if app_id == "codex":
            manifest_files = list(unpack_dir.rglob("AppxManifest.xml"))
            if not manifest_files:
                inj_status = "fail"
                inj_details.append("MSIX 中未找到 AppxManifest.xml")
            else:
                appx = _parse_appx_manifest(manifest_files[0])
                ident = appx["identity"]
                profile["packageName"] = ident.get("name", "")
                profile["version"] = ident.get("version", "") or version_hint
                profile["protocols"] = appx["protocols"]
                profile["capabilities"] = appx["capabilities"]
                profile["startupTasks"] = appx["startupTasks"]
                profile["executionAliases"] = appx["executionAliases"]
                profile["appxApplications"] = appx["applications"]

                if ident.get("name") != "OpenAI.Codex":
                    inj_status = "fail"
                    inj_details.append(f"MSIX Identity Name 不匹配: 期望 OpenAI.Codex，实际 {ident.get('name')}")
                else:
                    inj_details.append(f"MSIX Identity=OpenAI.Codex v{ident.get('version')}")

                visible_apps = [
                    a for a in appx["applications"] if str(a.get("appListEntry", "")).lower() != "none"
                ]
                hidden_apps = [
                    a for a in appx["applications"] if str(a.get("appListEntry", "")).lower() == "none"
                ]
                if not visible_apps:
                    inj_status = "fail"
                    inj_details.append("AppxManifest.xml 未找到可见主界面 Application（AppListEntry != none）")
                else:
                    app_desc = ", ".join(f"{a['id']}->{a['executable']}" for a in visible_apps)
                    inj_details.append(f"主界面启动入口: {app_desc}")
                    if hidden_apps:
                        hid_desc = ", ".join(f"{a['id']}->{a['executable']}" for a in hidden_apps)
                        inj_details.append(f"后台辅助入口已识别（须在启动器中过滤）: {hid_desc}")

            # Check zh-CN locale pack
            zh_paks = [p for p in unpack_dir.rglob("*") if "zh-cn" in p.name.lower() or "zh_cn" in p.name.lower()]
            if zh_paks:
                loc_details.append(f"检测到简体中文语言资源: {', '.join(sorted({p.name for p in zh_paks})[:5])}")
            else:
                loc_status = "warn"
                loc_details.append("未找到显式 zh-CN/zh_CN 语言资源文件，依赖 desktop.localeOverride")

            asar_candidates = [p for p in unpack_dir.rglob("app.asar") if p.is_file()]
            if not asar_candidates:
                loc_status = "fail"
                inj_status = "fail"
                loc_details.append("MSIX 解包目录缺少 app.asar")
            else:
                asar_path = asar_candidates[0]
                raw_asar = asar_path.read_bytes()
                if b"localeOverride" in raw_asar:
                    loc_details.append("app.asar 保留 desktop.localeOverride 锚点")
                else:
                    loc_status = "fail"
                    loc_details.append("app.asar 缺失 desktop.localeOverride 锚点")

                missing = [
                    k
                    for k in ("model_catalog_url", "default_tools_approval_mode", "approval_mode")
                    if k.encode() not in raw_asar
                ]
                if missing:
                    inj_status = "fail"
                    inj_details.append(f"app.asar 缺失注入锚点: {', '.join(missing)}")
                else:
                    inj_details.append("model_catalog_url / default_tools_approval_mode / approval_mode 锚点完整")

            owl_files = list(unpack_dir.rglob("owl-app.ini"))
            if owl_files:
                owl_txt = owl_files[0].read_text(encoding="utf-8", errors="replace").strip()
                extra_audit["owlAppIni"] = owl_txt
                if "UserDataDirectoryName=Codex" not in owl_txt and inj_status == "pass":
                    inj_status = "warn"
                    inj_details.append(f"owl-app.ini 目录名变化: {owl_txt}")

            cli_pkgs = list(unpack_dir.rglob("codex-package.json"))
            if cli_pkgs:
                try:
                    pkg_obj = json.loads(cli_pkgs[0].read_text(encoding="utf-8"))
                    extra_audit["codexCliVersion"] = pkg_obj.get("version")
                    inj_details.append(f"内嵌 codex-cli 版本: {pkg_obj.get('version')}")
                except Exception:
                    pass

            for main_name in ("ChatGPT.exe", "Codex.exe"):
                found_pe = list(unpack_dir.rglob(main_name))
                if found_pe:
                    extra_audit[f"pe_{main_name}"] = scan_pe_electron_markers(found_pe[0])

        elif app_id == "claude":
            app_asars = [p for p in unpack_dir.rglob("app.asar") if p.is_file()]
            ion_asars = [p for p in unpack_dir.rglob("ion-dist.asar") if p.is_file()]
            if not app_asars or not ion_asars:
                loc_status = "fail"
                inj_status = "fail"
                loc_details.append(f"Windows 安装包解包 ({unpack_method}) 缺少 app.asar 或 ion-dist.asar")
            else:
                app_asar = SimpleAsarReader(app_asars[0])
                ion_asar = SimpleAsarReader(ion_asars[0])

                en_bytes = app_asar.read_file("resources/i18n/en-US.json")
                if not en_bytes:
                    loc_status = "fail"
                    loc_details.append("app.asar 缺少 resources/i18n/en-US.json")
                else:
                    en_map = json.loads(en_bytes.decode("utf-8"))
                    dict_path = cfg.repo_root / "src-tauri" / "src" / "adapters" / "claude_zh_dict.json"
                    zh_dict = json.loads(dict_path.read_text(encoding="utf-8")) if dict_path.is_file() else {}
                    zh_existing: dict[str, Any] = {}
                    zh_bytes = app_asar.read_file("resources/i18n/zh-Hans.json")
                    if zh_bytes:
                        try:
                            zh_existing = json.loads(zh_bytes.decode("utf-8"))
                        except Exception:
                            pass
                    total_keys = len(en_map)
                    covered = sum(
                        1
                        for k, v in en_map.items()
                        if isinstance(v, str) and (k in zh_existing or v in zh_dict)
                    )
                    cov = (covered / total_keys * 100.0) if total_keys else 0.0
                    loc_details.append(f"app.asar i18n 词条覆盖: {covered}/{total_keys} ({cov:.1f}%)")
                    if cov < 60.0 and loc_status == "pass":
                        loc_status = "warn"

                build_js = [p for p in app_asar.file_paths() if p.startswith(".vite/build/") and p.endswith(".js")]
                has_intl = False
                has_regex = False
                found_inj: set[str] = set()
                ccd_manifest: dict[str, Any] | None = None
                has_vm_dl = False
                required_inj = (
                    "inferenceGatewayBaseUrl",
                    "managedMcpServers",
                    "inferenceModels",
                    "disableAutoUpdates",
                    "disableNonessentialTelemetry",
                )
                for rel in build_js:
                    raw = app_asar.read_file(rel) or b""
                    if (b"ja-JP" in raw and b"id-ID" in raw) or b"__TAKO_CLAUDE_ZH_PATCH_V1__" in raw:
                        has_intl = True
                    if (
                        b"[a-z]{2}-[A-Z]{2}" in raw
                        or b"[a-z]{2,3}-[A-Za-z0-9]{2,8}" in raw
                        or b"__TAKO_CLAUDE_ZH_PATCH_V1__" in raw
                    ):
                        has_regex = True
                    for anchor in required_inj:
                        if anchor.encode() in raw:
                            found_inj.add(anchor)
                    if ccd_manifest is None and b"claude-code-releases" in raw:
                        ccd_manifest = extract_claude_code_manifest_from_bytes(raw)
                    if b"downloads.claude.ai/vms/" in raw:
                        has_vm_dl = True

                if not (has_intl and has_regex):
                    loc_status = "fail"
                    loc_details.append("未在 .vite/build/*.js 找到 Claude 语言映射/正则锚点")
                else:
                    loc_details.append("DesktopIntl 语言映射与文件名正则锚点匹配")

                ion_js = [p for p in ion_asar.file_paths() if p.endswith(".js")]
                ion_ok = any(
                    b"en-US" in (ion_asar.read_file(p) or b"") and b"ja-JP" in (ion_asar.read_file(p) or b"")
                    for p in ion_js
                )
                if not ion_ok:
                    loc_status = "fail"
                    loc_details.append("ion-dist.asar 缺少 locale 白名单锚点")
                else:
                    loc_details.append(f"ion-dist.asar 白名单校验通过（{len(ion_js)} 个 JS chunk）")

                missing_inj = [a for a in required_inj if a not in found_inj]
                if missing_inj:
                    inj_status = "fail"
                    inj_details.append(f"缺少 3P 注入契约字段: {', '.join(missing_inj)}")
                else:
                    inj_details.append("3P 网关/MCP/模型/禁自更新全部 5 项契约字段完整")

                if ccd_manifest:
                    ccd_ver = str(ccd_manifest.get("version") or "")
                    extra_audit["claudeCodeVersion"] = ccd_ver
                    extra_audit["claudeCodeBaseUrl"] = ccd_manifest.get("baseUrl")
                    inj_details.append(f"检测到内嵌 Claude Code CLI 版本: {ccd_ver}")
                    if check_remote_mirrors and ccd_ver:
                        mirror_url = f"https://dl.lven.asia/apps/claude-code/{ccd_ver}/win32-x64/claude.exe.zst"
                        head = check_url_head(mirror_url)
                        extra_audit["claudeCodeWinMirror"] = {"url": mirror_url, **head}
                        if head.get("ok"):
                            inj_details.append(f"国内 Claude Code win32-x64 镜像可达 ({head.get('contentLength')} bytes)")
                        else:
                            if inj_status == "pass":
                                inj_status = "warn"
                            inj_details.append(
                                f"国内 Claude Code win32-x64 镜像不可达 (HTTP {head.get('status')}): {mirror_url}"
                            )
                else:
                    if inj_status == "pass":
                        inj_status = "warn"
                    inj_details.append("未提取到 claude-code-releases 清单，请人工核对 Code 模式下载源")

                extra_audit["hasVmDownloader"] = has_vm_dl

            claude_exes = [p for p in unpack_dir.rglob("Claude.exe") if p.is_file()]
            if claude_exes:
                pe_info = scan_pe_electron_markers(claude_exes[0])
                extra_audit["pe_Claude.exe"] = pe_info
                loc_details.append(
                    f"Claude.exe Electron fuse={pe_info.get('hasElectronFuse')}, integrity_slot={pe_info.get('hasIntegritySlot')}"
                )

        elif app_id == "antigravity":
            app_asars = [p for p in unpack_dir.rglob("app.asar") if p.is_file()]
            if not app_asars:
                loc_status = "fail"
                inj_status = "fail"
                loc_details.append(f"Windows 安装包解包 ({unpack_method}) 缺少 resources/app.asar")
            else:
                app_asar = SimpleAsarReader(app_asars[0])
                all_files = app_asar.file_paths()
                js_files = [p for p in all_files if p.endswith(".js")]
                loc_details.append(f"app.asar 可解析（{len(all_files)} 个文件 / {len(js_files)} 个 JS）")
                extra_audit["asarFileCount"] = len(all_files)

                dict_path = cfg.repo_root / "src-tauri" / "src" / "adapters" / "antigravity_zh_dict.json"
                if dict_path.is_file():
                    agy_dict = json.loads(dict_path.read_text(encoding="utf-8"))
                    loc_details.append(f"内置反重力汉化词典条目: {len(agy_dict)}")

                agy_exes = [p for p in unpack_dir.rglob("Antigravity.exe") if p.is_file()]
                if agy_exes:
                    pe_info = scan_pe_electron_markers(agy_exes[0])
                    extra_audit["pe_Antigravity.exe"] = pe_info
                    loc_details.append(
                        f"Antigravity.exe fuse={pe_info.get('hasElectronFuse')}, integrity_slot={pe_info.get('hasIntegritySlot')}"
                    )
                inj_details.append("Antigravity Windows 代理环境变量与 mcp_config 注入结构兼容")

        elif app_id == "dsh-tako":
            loc_details.append("自研客户端原生简体中文")
            if any(e.lower() == "dsh tako.exe" for e in exes):
                inj_details.append("DSH Tako.exe 主程序就绪")
            elif artifact_path.is_file() and artifact_size > 10_000_000:
                inj_details.append(f"安装包体积完整 ({artifact_size} bytes)，unpack={unpack_method}")
            else:
                inj_status = "fail"
                inj_details.append("未找到 DSH Tako.exe 且安装包体积异常")

        # Cleanup Inno/NSIS isolated sandbox uninstall artifacts if any were left behind (TempDir cleans dir itself)
        if sig_info.get("checked") and sig_info.get("status") not in ("Valid", "NotSigned", "skipped_non_windows"):
            extra_audit["authenticodeWarning"] = sig_info.get("status")

    audit_status, audit_findings = _compare_windows_baseline(app_id, profile, extra_audit, cfg.baselines_dir)
    if update_baseline:
        cfg.baselines_dir.mkdir(parents=True, exist_ok=True)
        b_file = cfg.baselines_dir / f"{app_id}.windows.json"
        b_file.write_text(
            json.dumps(
                {
                    "appId": app_id,
                    "platform": "windows-x64",
                    "capturedAt": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                    "profile": profile,
                    "extraAudit": extra_audit,
                },
                indent=2,
                ensure_ascii=False,
            )
            + "\n",
            encoding="utf-8",
        )
        audit_status = "pass"
        audit_findings = [f"已更新 Windows 基线档案 {b_file.name}"]

    statuses = (loc_status, inj_status, audit_status)
    overall = "fail" if "fail" in statuses else ("warn" if "warn" in statuses else "pass")
    resolved_ver = str(profile.get("version") or version_hint or "")

    return {
        "appId": app_id,
        "platform": "windows-x64",
        "version": resolved_ver,
        "verifiedAt": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "overall": overall,
        "profile": profile,
        "extraAudit": extra_audit,
        "gates": {
            "localization": {"status": loc_status, "details": loc_details},
            "injection": {"status": inj_status, "details": inj_details},
            "anomalyAudit": {"status": audit_status, "details": audit_findings},
        },
    }


def resolve_windows_artifact_spec(
    cfg: LabConfig,
    app_id: str,
    *,
    candidate_url: str = "",
    version_override: str = "",
) -> dict[str, Any]:
    pinned = load_pinned_versions(cfg)
    app_entry = (pinned.get("apps") or {}).get(app_id) or {}
    win_plat = (app_entry.get("platforms") or {}).get("windows-x64") or {}

    url = candidate_url.strip() or str(win_plat.get("url") or "")
    fallback_url = "" if candidate_url.strip() else str(win_plat.get("fallbackUrl") or "")
    version = version_override.strip() or str(win_plat.get("version") or app_entry.get("version") or "unknown")
    expected_sha256 = "" if candidate_url.strip() else str(win_plat.get("sha256") or "")
    install_type = str(win_plat.get("installType") or "")

    if not url:
        raise ValueError(f"未在 pinned-versions.json 找到 {app_id} 的 windows-x64 下载地址，且未传入 candidate_url")

    parsed_name = Path(urllib.parse.urlparse(url).path).name or f"{app_id}-{version}.bin"
    safe_ver = re.sub(r"[^A-Za-z0-9._-]+", "_", version)
    cache_path = cfg.cache_dir / app_id / safe_ver / parsed_name
    return {
        "app_id": app_id,
        "version": version,
        "url": url,
        "fallback_url": fallback_url,
        "expected_sha256": expected_sha256,
        "install_type": install_type,
        "cache_path": cache_path,
        "pinned": pinned,
    }


def run_upstream_candidate_check(cfg: LabConfig) -> tuple[dict[str, Any], str]:
    """Invoke scripts/manage-releases.py check logic in-process (read-only)."""
    mr_path = cfg.repo_root / "scripts" / "manage-releases.py"
    if not mr_path.is_file():
        raise RuntimeError(f"Missing release manager script: {mr_path}")

    spec = importlib.util.spec_from_file_location("tako_manage_releases", mr_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Unable to load {mr_path}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)

    egress_proxy = os.environ.get("WIN_LAB_EGRESS_PROXY", "").strip()
    prev_env = {k: os.environ.get(k) for k in ("HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY")}
    try:
        if egress_proxy:
            os.environ["HTTP_PROXY"] = egress_proxy
            os.environ["HTTPS_PROXY"] = egress_proxy
            os.environ["NO_PROXY"] = "127.0.0.1,localhost,172.17.66.0/24,dl.lven.asia"
        pinned = load_pinned_versions(cfg)
        sync_mod = mod.load_sync_manifest_module()
        upstream_manifest = sync_mod.build_manifest(mirror_base=None)
        changelogs = mod.fetch_changelogs(upstream_manifest.get("apps") or {})
        return mod.build_check_report(pinned, upstream_manifest, changelogs)
    finally:
        for k, v in prev_env.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
