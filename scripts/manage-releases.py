#!/usr/bin/env python3
"""Controlled desktop-client release manager for Tako Switch.

Replaces blind upstream auto-sync with a 4-stage gated pipeline:
  1. `check`           : Fetch upstream latest versions + official changelogs, analyze risks,
                         and emit a review report WITHOUT modifying `manifest/manifest.json`.
  2. `verify-macos`    : Run the 3 mandatory macOS release gates on a candidate `.app` / `.zip` / `.dmg`
                         in an isolated sandbox (never mutates `/Applications` or real user config):
                           Gate 1 (`localization`) : Can be localized (i18n anchors + dict coverage)
                           Gate 2 (`injection`)    : Tako gateway/MCP/config anchors intact
                           Gate 3 (`anomalyAudit`) : Info.plist/helpers/downloaders diff vs baseline
  3. `promote`         : Promote an approved candidate into `manifest/pinned-versions.json`
                         (requires passing gate report unless explicitly overridden) and render
                         `manifest/manifest.json`.
  4. `render-manifest` : Deterministically render `manifest/manifest.json` from
                         `manifest/pinned-versions.json`.

Zero third-party dependencies (Python 3.9+ stdlib only).
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET
import zipfile


REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_PINNED_PATH = REPO_ROOT / "manifest" / "pinned-versions.json"
DEFAULT_MANIFEST_PATH = REPO_ROOT / "manifest" / "manifest.json"
DEFAULT_AGY_CLI_MANIFESTS_DIR = REPO_ROOT / "manifest" / "agy-cli" / "manifests"
DEFAULT_BASELINES_DIR = REPO_ROOT / "manifest" / "baselines"

AGY_CLI_UPSTREAM_BASE = "https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests"
AGY_CLI_PLATFORMS = (
    "darwin_arm64",
    "darwin_amd64",
    "linux_amd64",
    "linux_arm64",
    "windows_amd64",
    "linux_amd64_musl",
    "linux_arm64_musl",
    "windows_arm64",
)
AGY_CLI_REQUIRED_PLATFORMS = (
    "darwin_arm64",
    "darwin_amd64",
    "linux_amd64",
    "linux_arm64",
    "windows_amd64",
)
SHA512_HEX_RE = re.compile(r"^[0-9a-f]{128}$")

INSTALLED_MACOS_PATHS: dict[str, list[Path]] = {
    "claude": [Path("/Applications/Claude.app")],
    "codex": [Path("/Applications/ChatGPT.app"), Path("/Applications/Codex.app")],
    "antigravity": [Path("/Applications/Antigravity.app")],
    "workbuddy": [Path("/Applications/WorkBuddy.app")],
    "dsh-tako": [Path("/Applications/DSH Tako.app")],
}

EXPECTED_BUNDLE_IDS: dict[str, str] = {
    "claude": "com.anthropic.claudefordesktop",
    "codex": "com.openai.codex",
    "antigravity": "com.google.antigravity",
    "workbuddy": "com.tencent.workbuddy.mac",
    "dsh-tako": "tech.shiroha.dsh-tako",
}

RISK_KEYWORDS: dict[str, list[str]] = {
    "breaking": ["breaking", "removed", "incompatible", "migration required", "drop support"],
    "auth_region": ["oauth", "sign-in", "login", "region", "geo", "account required"],
    "packaging_runtime": ["electron", "asar", "msix", "codesign", "integrity", "sandbox", "webview"],
    "config_mcp": ["config.toml", "mcp", "model_provider", "managed-settings", "3p", "gateway", "models.json"],
    "updater_downloader": ["auto-update", "autoupdate", "downloads.claude.ai", "squirrel", "sparkle"],
}

APP_ORDER = ("claude", "codex", "antigravity", "workbuddy", "dsh-tako")


def load_sync_manifest_module():
    path = REPO_ROOT / "scripts" / "sync-manifest.py"
    spec = importlib.util.spec_from_file_location("tako_sync_manifest", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Unable to load {path}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def http_get_text(url: str, timeout: int = 15, headers: dict[str, str] | None = None) -> str:
    req_headers = {"User-Agent": "tako-switch-release-gate/1.0"}
    if headers:
        req_headers.update(headers)
    req = urllib.request.Request(url, headers=req_headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as exc:
        if exc.code == 403 and "oaistatic.com" in url:
            # Cloudflare may block Python's default TLS/UA; fall back to curl if present.
            return subprocess.check_output(
                ["curl", "-fsSL", "-m", str(timeout), url],
                text=True,
                stderr=subprocess.DEVNULL,
            )
        raise


def http_get_json(url: str, timeout: int = 15) -> object:
    headers = {"Accept": "application/vnd.github+json"} if "api.github.com" in url else None
    token = os.environ.get("GITHUB_TOKEN", "").strip()
    if token and "api.github.com" in url:
        headers = {**(headers or {}), "Authorization": f"Bearer {token}"}
    return json.loads(http_get_text(url, timeout=timeout, headers=headers))


# ---------------------------------------------------------------------------
# Minimal ASAR reader (zero external dependencies)
# ---------------------------------------------------------------------------


class SimpleAsarReader:
    def __init__(self, path: Path) -> None:
        self.path = path
        with path.open("rb") as f:
            f.seek(4)
            hdr_pickle_size = struct.unpack("<I", f.read(4))[0]
            f.seek(12)
            json_len = struct.unpack("<I", f.read(4))[0]
            self.header = json.loads(f.read(json_len).decode("utf-8"))
            self.base_offset = 8 + hdr_pickle_size
        self._index: dict[str, dict] = {}
        self._walk(self.header, "")

    def _walk(self, node: dict, prefix: str) -> None:
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


# ---------------------------------------------------------------------------
# Changelog fetching & risk analysis
# ---------------------------------------------------------------------------


def analyze_changelog_risks(text: str) -> list[dict]:
    lower = text.lower()
    hits: list[dict] = []
    for category, kws in RISK_KEYWORDS.items():
        matched = [kw for kw in kws if kw in lower]
        if matched:
            hits.append({"category": category, "keywords": matched})
    return hits


def fetch_changelogs(upstream_apps: dict) -> dict[str, dict]:
    """Fetch official release notes / changelogs best-effort for each desktop app."""
    notes: dict[str, dict] = {}

    # 1. DSH Tako (GitHub Releases)
    dsh_info: dict = {"source": "github:tako-dev/dsh-tako-desktop", "entries": [], "error": None}
    try:
        rels = http_get_json("https://api.github.com/repos/tako-dev/dsh-tako-desktop/releases?per_page=3")
        if isinstance(rels, list):
            for rel in rels[:3]:
                dsh_info["entries"].append(
                    {
                        "version": str(rel.get("tag_name") or "").lstrip("v"),
                        "publishedAt": rel.get("published_at"),
                        "body": str(rel.get("body") or "").strip()[:1200],
                    }
                )
    except Exception as exc:
        dsh_info["error"] = str(exc)
    notes["dsh-tako"] = dsh_info

    # 2. Codex Desktop (Sparkle appcast + openai/codex CLI GitHub Releases)
    codex_info: dict = {"source": "sparkle-appcast + github:openai/codex", "entries": [], "error": None}
    try:
        xml_raw = http_get_text("https://persistent.oaistatic.com/codex-app-prod/appcast.xml")
        root = ET.fromstring(xml_raw)
        ns = {"sparkle": "http://www.andymatuschak.org/xml-namespaces/sparkle"}
        for item in root.findall("./channel/item")[:4]:
            ver = (item.findtext("sparkle:shortVersionString", "", ns) or item.findtext("title") or "").strip()
            pub = (item.findtext("pubDate") or "").strip()
            desc = (item.findtext("description") or "").strip()
            codex_info["entries"].append(
                {
                    "version": ver,
                    "publishedAt": pub,
                    "body": desc[:800] if desc else "(Sparkle appcast 无正文更新日志，请核对 openai/codex CLI 变更与本地包差分)",
                }
            )
    except Exception as exc:
        codex_info["error"] = f"appcast: {exc}"
    try:
        rels = http_get_json("https://api.github.com/repos/openai/codex/releases?per_page=3")
        if isinstance(rels, list):
            for rel in rels[:2]:
                codex_info["entries"].append(
                    {
                        "version": f"codex-cli {str(rel.get('tag_name') or '')}",
                        "publishedAt": rel.get("published_at"),
                        "body": str(rel.get("body") or "").strip()[:1000],
                    }
                )
    except Exception as exc:
        codex_info["error"] = f"{codex_info['error']}; gh: {exc}" if codex_info["error"] else f"gh: {exc}"
    notes["codex"] = codex_info

    # 3. Claude Desktop (RELEASES.json + anthropics/claude-code GitHub Releases)
    claude_info: dict = {"source": "RELEASES.json + github:anthropics/claude-code", "entries": [], "error": None}
    try:
        rel_json = http_get_json("https://downloads.claude.ai/releases/darwin/universal/RELEASES.json")
        if isinstance(rel_json, dict):
            for rel in (rel_json.get("releases") or [])[-3:]:
                upd = rel.get("updateTo") or {}
                claude_info["entries"].append(
                    {
                        "version": str(rel.get("version") or upd.get("version") or ""),
                        "publishedAt": upd.get("pub_date"),
                        "body": str(upd.get("notes") or "").strip()[:1000]
                        or "(Claude Desktop RELEASES.json 未附正文日志，请重点核对内嵌 Claude Code CLI 版本与 app.asar 差分)",
                    }
                )
    except Exception as exc:
        claude_info["error"] = f"releases.json: {exc}"
    try:
        rels = http_get_json("https://api.github.com/repos/anthropics/claude-code/releases?per_page=3")
        if isinstance(rels, list):
            for rel in rels[:2]:
                claude_info["entries"].append(
                    {
                        "version": f"claude-code {str(rel.get('tag_name') or '')}",
                        "publishedAt": rel.get("published_at"),
                        "body": str(rel.get("body") or "").strip()[:1000],
                    }
                )
    except Exception as exc:
        claude_info["error"] = f"{claude_info['error']}; gh: {exc}" if claude_info["error"] else f"gh: {exc}"
    notes["claude"] = claude_info

    # 4. Antigravity (auto-updater yml metadata)
    agy_info: dict = {"source": "antigravity-updater-yml", "entries": [], "error": None}
    try:
        yml_raw = http_get_text(
            "https://antigravity-hub-auto-updater-974169037036.us-central1.run.app/manifest/latest-arm64-mac.yml"
        )
        ver_m = re.search(r"(?m)^version:\s*['\"]?([^\s'\"]+)", yml_raw)
        date_m = re.search(r"(?m)^releaseDate:\s*['\"]?([^\n'\"]+)", yml_raw)
        notes_m = re.search(r"(?ms)^releaseNotes:\s*(.+?)(?:^\w+:|\Z)", yml_raw)
        agy_info["entries"].append(
            {
                "version": ver_m.group(1) if ver_m else str((upstream_apps.get("antigravity") or {}).get("version") or ""),
                "publishedAt": date_m.group(1).strip() if date_m else None,
                "body": notes_m.group(1).strip()[:1000]
                if notes_m
                else "(updater yml 未附 releaseNotes，请通过 verify-macos 三门禁对比 app.asar 变更)",
            }
        )
    except Exception as exc:
        agy_info["error"] = str(exc)
    notes["antigravity"] = agy_info

    # 5. WorkBuddy (official domestic changelog page)
    wb_info: dict = {"source": "https://www.workbuddy.cn/docs/workbuddy/Changelog", "entries": [], "error": None}
    try:
        wb_html = http_get_text("https://www.workbuddy.cn/docs/workbuddy/Changelog")
        main_m = re.search(r"<main[^>]*>(.*?)</main>", wb_html, flags=re.S)
        wb_text = re.sub(r"<[^>]+>", "\n", main_m.group(1) if main_m else wb_html)
        wb_lines = [ln.strip() for ln in wb_text.splitlines() if ln.strip()]
        joined = "\n".join(wb_lines)
        ver_m = re.search(r"(\d+\.\d+\.\d+)\s*版本发布[^（(]*[（(](\d{4}-\d{2}-\d{2})[）)]", joined)
        wb_info["entries"].append(
            {
                "version": ver_m.group(1) if ver_m else str((upstream_apps.get("workbuddy") or {}).get("version") or ""),
                "publishedAt": ver_m.group(2) if ver_m else None,
                "body": joined[:1200],
            }
        )
    except Exception as exc:
        wb_info["error"] = str(exc)
    notes["workbuddy"] = wb_info

    for app_id, info in notes.items():
        combined_text = "\n".join(str(e.get("body") or "") for e in info.get("entries") or [])
        info["riskHints"] = analyze_changelog_risks(combined_text)
    return notes


def fetch_upstream_agy_cli_manifests(
    base_url: str = AGY_CLI_UPSTREAM_BASE,
    platforms: tuple[str, ...] = AGY_CLI_PLATFORMS,
) -> dict:
    """Fetch upstream Antigravity CLI (agy) per-platform manifests (read-only)."""
    clean_base = base_url.rstrip("/")
    plats_out: dict[str, dict] = {}
    errors: dict[str, str] = {}
    versions_seen: list[str] = []

    for plat in platforms:
        url = f"{clean_base}/{plat}.json"
        try:
            payload = http_get_json(url, timeout=15)
            if not isinstance(payload, dict):
                errors[plat] = "manifest payload is not a JSON object"
                continue
            ver = str(payload.get("version") or "").strip()
            dl_url = str(payload.get("url") or "").strip()
            sha512 = str(payload.get("sha512") or "").strip().lower()
            if not ver or not dl_url or not sha512:
                errors[plat] = "manifest missing version/url/sha512"
                continue
            entry: dict[str, object] = {
                "version": ver,
                "url": dl_url,
                "sha512": sha512,
            }
            if isinstance(payload.get("size"), int) and payload["size"] > 0:
                entry["size"] = payload["size"]
            plats_out[plat] = entry
            if ver not in versions_seen:
                versions_seen.append(ver)
        except Exception as exc:
            errors[plat] = str(exc)

    primary_version = versions_seen[0] if versions_seen else ""
    return {
        "displayName": "Antigravity CLI (agy)",
        "sourceBaseUrl": clean_base,
        "version": primary_version,
        "versionsSeen": versions_seen,
        "platforms": plats_out,
        "errors": errors,
    }


def build_check_report(
    pinned: dict,
    upstream_manifest: dict,
    changelogs: dict,
    upstream_cli_tools: dict | None = None,
) -> tuple[dict, str]:
    pinned_apps = pinned.get("apps") or {}
    upstream_apps = upstream_manifest.get("apps") or {}
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

    comparison: dict[str, dict] = {}
    has_any_update = False
    for app_id in APP_ORDER:
        p = pinned_apps.get(app_id) or {}
        u = upstream_apps.get(app_id) or {}
        p_ver = str(p.get("version") or "")
        u_ver = str(u.get("version") or "")
        plat_diffs = {}
        for plat, u_art in (u.get("platforms") or {}).items():
            p_art = (p.get("platforms") or {}).get(plat) or {}
            pv = str(p_art.get("version") or p_ver)
            uv = str(u_art.get("version") or u_ver)
            if pv != uv or (u_art.get("size") and p_art.get("size") and u_art.get("size") != p_art.get("size")):
                plat_diffs[plat] = {
                    "pinnedVersion": pv,
                    "upstreamVersion": uv,
                    "pinnedSize": p_art.get("size"),
                    "upstreamSize": u_art.get("size"),
                }
        changed = (p_ver != u_ver) or bool(plat_diffs)
        if changed:
            has_any_update = True
        comparison[app_id] = {
            "displayName": p.get("displayName") or u.get("displayName") or app_id,
            "pinnedVersion": p_ver,
            "upstreamVersion": u_ver,
            "pinnedStatus": p.get("status") or "unknown",
            "hasUpstreamUpdate": changed,
            "platformDiffs": plat_diffs,
            "changelog": changelogs.get(app_id) or {},
            "upstreamEntry": u,
        }

    cli_comparison: dict[str, dict] = {}
    if upstream_cli_tools:
        pinned_cli = pinned.get("cliTools") or {}
        for cli_id, u_cli in upstream_cli_tools.items():
            p_cli = pinned_cli.get(cli_id) or {}
            p_ver = str(p_cli.get("version") or "")
            u_ver = str((u_cli or {}).get("version") or "")
            plat_diffs = {}
            for plat, u_art in ((u_cli or {}).get("platforms") or {}).items():
                p_art = (p_cli.get("platforms") or {}).get(plat) or {}
                pv = str(p_art.get("version") or p_ver)
                uv = str(u_art.get("version") or u_ver)
                p_sha = str(p_art.get("sha512") or "").lower()
                u_sha = str(u_art.get("sha512") or "").lower()
                if pv != uv or (u_sha and p_sha != u_sha):
                    plat_diffs[plat] = {
                        "pinnedVersion": pv,
                        "upstreamVersion": uv,
                        "pinnedSha512Prefix": p_sha[:12],
                        "upstreamSha512Prefix": u_sha[:12],
                    }
            changed = bool(u_ver) and ((p_ver != u_ver) or bool(plat_diffs))
            if changed:
                has_any_update = True
            cli_comparison[cli_id] = {
                "displayName": p_cli.get("displayName") or (u_cli or {}).get("displayName") or cli_id,
                "pinnedVersion": p_ver,
                "upstreamVersion": u_ver,
                "pinnedStatus": p_cli.get("status") or "unknown",
                "supportedVersions": p_cli.get("supportedVersions") or ([p_ver] if p_ver else []),
                "minSupportedVersion": p_cli.get("minSupportedVersion") or p_ver,
                "hasUpstreamUpdate": changed,
                "platformDiffs": plat_diffs,
                "upstreamEntry": u_cli,
            }

    report_json = {
        "schemaVersion": 1,
        "checkedAt": now,
        "autoFollowUpstream": False,
        "hasAnyUpstreamUpdate": has_any_update,
        "apps": comparison,
    }
    if cli_comparison:
        report_json["cliTools"] = cli_comparison

    lines = [
        "# Tako Switch 上游版本巡检与 Changelog 风险分析报告",
        "",
        f"- 检查时间：`{now}`",
        "- 版本策略：**受控锁版本（禁止自动盲跟上游）**，须先看更新日志并通过对应门禁验收后再 `promote`。",
        "",
        "## 版本对比总览",
        "",
        "| 客户端 | 当前锁定版本 | 上游最新版本 | 状态 | 风险关键词提示 |",
        "|---|---|---|---|---|",
    ]
    for app_id in APP_ORDER:
        c = comparison[app_id]
        status_label = "🆕 发现上游新版（待验收）" if c["hasUpstreamUpdate"] else "✅ 与锁定版本一致"
        hints = [h["category"] for h in (c["changelog"].get("riskHints") or [])]
        hint_str = ", ".join(hints) if hints else "无高危词命中"
        lines.append(
            f"| {c['displayName']} (`{app_id}`) | `{c['pinnedVersion']}` | `{c['upstreamVersion']}` | {status_label} | {hint_str} |"
        )
    for cli_id, c in cli_comparison.items():
        status_label = "🆕 发现上游新版（待 Gate 5 验收）" if c["hasUpstreamUpdate"] else "✅ 与锁定版本一致"
        lines.append(
            f"| {c['displayName']} (`cli:{cli_id}`) | `{c['pinnedVersion']}` | `{c['upstreamVersion'] or '?'}` | {status_label} | 需先扩 agy-gateway 窗口 |"
        )

    lines.append("")
    lines.append("## 各客户端 Changelog 与平台差异详情")
    for app_id in APP_ORDER:
        c = comparison[app_id]
        lines.append("")
        lines.append(f"### {c['displayName']} (`{app_id}`)")
        lines.append(f"- 锁定版本：`{c['pinnedVersion']}`（状态：`{c['pinnedStatus']}`） / 上游版本：`{c['upstreamVersion']}`")
        if c["platformDiffs"]:
            lines.append("- 平台版本/包体差异：")
            for plat, diff in c["platformDiffs"].items():
                lines.append(
                    f"  - `{plat}`: `{diff['pinnedVersion']}` ({diff['pinnedSize']}) -> `{diff['upstreamVersion']}` ({diff['upstreamSize']})"
                )
        cl = c["changelog"]
        if cl.get("error"):
            lines.append(f"- 日志抓取告警：`{cl['error']}`")
        for entry in (cl.get("entries") or [])[:3]:
            ver = entry.get("version") or "unknown"
            pub = entry.get("publishedAt") or "-"
            body = (entry.get("body") or "").strip().splitlines()
            excerpt = " ".join(line.strip() for line in body[:5])[:360]
            lines.append(f"- **{ver}** ({pub}): {excerpt}")

    if cli_comparison:
        lines.append("")
        lines.append("## 受控 CLI 工具差异详情")
        for cli_id, c in cli_comparison.items():
            lines.append("")
            lines.append(f"### {c['displayName']} (`cli:{cli_id}`)")
            lines.append(
                f"- 锁定版本：`{c['pinnedVersion']}`（支持窗口：`{', '.join(c['supportedVersions'])}`） / 上游版本：`{c['upstreamVersion'] or '?'}`"
            )
            if c["platformDiffs"]:
                lines.append("- 平台差异：")
                for plat, diff in c["platformDiffs"].items():
                    lines.append(
                        f"  - `{plat}`: `{diff['pinnedVersion']}` ({diff['pinnedSha512Prefix'] or '-'}) -> `{diff['upstreamVersion']}` ({diff['upstreamSha512Prefix']})"
                    )
            errs = ((c.get("upstreamEntry") or {}).get("errors") or {})
            if errs:
                lines.append(f"- 平台抓取告警：`{', '.join(f'{k}: {v}' for k, v in sorted(errs.items()))}`")

    lines.append("")
    lines.append("## 下一步审批流程")
    lines.append("")
    lines.append("1. **Desktop 客户端**：阅读更新日志 → 跑 `verify-macos` / `verify-windows` → `promote --app <app>` → 手动触发 `mirror-pinned` workflow。")
    lines.append("2. **Antigravity CLI (`agy`) 独立更新**：先在 `agy-gateway` 跑 Gate 5 并扩窗 → `verify-agy-cli` → `promote-agy-cli` → 手动触发 `mirror-agy-cli` workflow（无需发新版 `tako-cli`）。")
    lines.append("")
    return report_json, "\n".join(lines)


# ---------------------------------------------------------------------------
# macOS 3-Gate Verification (Localization / Injection / Anomaly Audit)
# ---------------------------------------------------------------------------


def collect_macos_bundle_profile(bundle_path: Path) -> dict:
    """Inspect a macOS `.app` bundle read-only and extract security/structure metadata."""
    info_plist_path = bundle_path / "Contents" / "Info.plist"
    if not info_plist_path.is_file():
        raise RuntimeError(f"Missing Info.plist in {bundle_path}")
    with info_plist_path.open("rb") as f:
        plist = plistlib.load(f)

    url_schemes: list[str] = []
    for entry in plist.get("CFBundleURLTypes") or []:
        if isinstance(entry, dict):
            url_schemes.extend(str(s) for s in (entry.get("CFBundleURLSchemes") or []))

    usage_descriptions = sorted(
        k for k in plist.keys() if isinstance(k, str) and k.startswith("NS") and k.endswith("UsageDescription")
    )

    macos_dir = bundle_path / "Contents" / "MacOS"
    executables = sorted(p.name for p in macos_dir.iterdir() if p.is_file()) if macos_dir.is_dir() else []

    frameworks_dir = bundle_path / "Contents" / "Frameworks"
    helpers = (
        sorted(p.name for p in frameworks_dir.iterdir() if p.name.endswith(".app"))
        if frameworks_dir.is_dir()
        else []
    )

    login_items: list[str] = []
    for sub in ("Contents/Library/LoginItems", "Contents/Library/LaunchServices", "Contents/Library/LaunchAgents"):
        d = bundle_path / sub
        if d.is_dir():
            login_items.extend(sorted(f"{sub}/{p.name}" for p in d.iterdir()))

    asar_integrity_keys = sorted((plist.get("ElectronAsarIntegrity") or {}).keys())

    return {
        "bundleName": bundle_path.name,
        "bundleId": str(plist.get("CFBundleIdentifier") or ""),
        "shortVersion": str(plist.get("CFBundleShortVersionString") or ""),
        "bundleVersion": str(plist.get("CFBundleVersion") or ""),
        "minSystemVersion": str(plist.get("LSMinimumSystemVersion") or ""),
        "urlSchemes": sorted(set(url_schemes)),
        "usageDescriptions": usage_descriptions,
        "executables": executables,
        "helpers": helpers,
        "loginItems": sorted(login_items),
        "electronAsarIntegrityKeys": asar_integrity_keys,
    }


def extract_claude_code_manifest_from_bytes(js_bytes: bytes) -> dict | None:
    needle = b"https://downloads.claude.ai/claude-code-releases"
    pos = js_bytes.find(needle)
    if pos == -1:
        # Also detect already-patched bundles
        pos = js_bytes.find(b"127.0.0.1:18419/claude-code-releases")
    if pos == -1:
        return None
    start = js_bytes.rfind(b"JSON.parse('", max(0, pos - 8192), pos)
    end = js_bytes.find(b"')}", pos, pos + 1024)
    if start == -1 or end == -1:
        return None
    raw = js_bytes[start + len(b"JSON.parse('") : end].decode("utf-8", errors="replace")
    # If already patched with dynamic 3p expression, restore URL for JSON parsing.
    raw = re.sub(
        r'"baseUrl":"\'\+\(\(\(\)=>.*?\)\(\)\)\+\'"',
        '"baseUrl":"https://downloads.claude.ai/claude-code-releases"',
        raw,
    )
    try:
        parsed = json.loads(raw)
        if isinstance(parsed, dict) and "version" in parsed and "manifest" in parsed:
            return parsed
    except Exception:
        return None
    return None


def compare_with_baseline(app_id: str, profile: dict, extra_audit: dict, baselines_dir: Path) -> tuple[str, list[str]]:
    baseline_path = baselines_dir / f"{app_id}.macos.json"
    findings: list[str] = []
    if not baseline_path.is_file():
        return "warn", [f"尚未建立基线文件 {baseline_path.name}（首轮验收后可用 --update-baseline 建档）"]

    baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
    b_prof = baseline.get("profile") or {}
    b_extra = baseline.get("extraAudit") or {}

    status = "pass"
    if b_prof.get("bundleId") and profile.get("bundleId") != b_prof.get("bundleId"):
        status = "fail"
        findings.append(f"CFBundleIdentifier 变更: {b_prof.get('bundleId')} -> {profile.get('bundleId')}")

    for field, label in (
        ("executables", "新增主目录可执行文件"),
        ("helpers", "新增 Helper App"),
        ("loginItems", "新增后台 LoginItem/LaunchService"),
        ("urlSchemes", "新增 URL Scheme"),
        ("usageDescriptions", "新增系统隐私权限声明"),
    ):
        old_set = set(b_prof.get(field) or [])
        new_set = set(profile.get(field) or [])
        added = sorted(new_set - old_set)
        if added:
            if status == "pass":
                status = "warn"
            findings.append(f"{label}: {', '.join(added)}")

    for k in ("claudeCodeVersion", "codexCliVersion", "masqueProxyOrigins"):
        if k in b_extra and extra_audit.get(k) != b_extra.get(k):
            if status == "pass":
                status = "warn"
            findings.append(f"{k} 变更: {b_extra.get(k)} -> {extra_audit.get(k)}")

    return status, findings


def verify_macos_bundle(
    app_id: str,
    bundle_path: Path,
    pinned: dict | None = None,
    baselines_dir: Path = DEFAULT_BASELINES_DIR,
    update_baseline: bool = False,
) -> dict:
    profile = collect_macos_bundle_profile(bundle_path)
    expected_bid = EXPECTED_BUNDLE_IDS.get(app_id)

    loc_status = "pass"
    loc_details: list[str] = []
    inj_status = "pass"
    inj_details: list[str] = []
    extra_audit: dict = {}

    if expected_bid and profile["bundleId"] != expected_bid:
        inj_status = "fail"
        inj_details.append(f"Bundle ID 不匹配: 期望 {expected_bid}，实际 {profile['bundleId']}")

    resources = bundle_path / "Contents" / "Resources"

    if app_id == "claude":
        app_asar_path = resources / "app.asar"
        ion_asar_path = resources / "ion-dist.asar"
        if not app_asar_path.is_file() or not ion_asar_path.is_file():
            loc_status = "fail"
            inj_status = "fail"
            loc_details.append("缺少 app.asar 或 ion-dist.asar")
        else:
            app_asar = SimpleAsarReader(app_asar_path)
            ion_asar = SimpleAsarReader(ion_asar_path)

            # Gate 1: localization anchors + dictionary coverage
            en_bytes = app_asar.read_file("resources/i18n/en-US.json")
            if not en_bytes:
                loc_status = "fail"
                loc_details.append("app.asar 缺少 resources/i18n/en-US.json")
            else:
                en_map = json.loads(en_bytes.decode("utf-8"))
                dict_path = REPO_ROOT / "src-tauri" / "src" / "adapters" / "claude_zh_dict.json"
                zh_dict = json.loads(dict_path.read_text(encoding="utf-8")) if dict_path.is_file() else {}
                zh_existing = {}
                zh_existing_bytes = app_asar.read_file("resources/i18n/zh-Hans.json")
                if zh_existing_bytes:
                    try:
                        zh_existing = json.loads(zh_existing_bytes.decode("utf-8"))
                    except Exception:
                        pass
                total_keys = len(en_map)
                covered = sum(
                    1
                    for k, v in en_map.items()
                    if isinstance(v, str) and (k in zh_existing or v in zh_dict)
                )
                coverage = (covered / total_keys * 100.0) if total_keys else 0.0
                loc_details.append(f"app.asar i18n 词条覆盖: {covered}/{total_keys} ({coverage:.1f}%)")
                if coverage < 60.0 and loc_status == "pass":
                    loc_status = "warn"

            build_js = [p for p in app_asar.file_paths() if p.startswith(".vite/build/") and p.endswith(".js")]
            has_intl_anchor = False
            has_regex_or_guard = False
            found_inj_anchors: set[str] = set()
            ccd_manifest: dict | None = None
            has_vm_downloader = False
            required_inj = (
                "inferenceGatewayBaseUrl",
                "managedMcpServers",
                "inferenceModels",
                "disableAutoUpdates",
                "disableNonessentialTelemetry",
            )
            for rel in build_js:
                raw = app_asar.read_file(rel)
                if not raw:
                    continue
                if (b"ja-JP" in raw and b"id-ID" in raw) or b"__TAKO_CLAUDE_ZH_PATCH_V1__" in raw:
                    has_intl_anchor = True
                if (
                    b"[a-z]{2}-[A-Z]{2}" in raw
                    or b"[a-z]{2,3}-[A-Za-z0-9]{2,8}" in raw
                    or b"__TAKO_CLAUDE_ZH_PATCH_V1__" in raw
                ):
                    has_regex_or_guard = True
                for anchor in required_inj:
                    if anchor.encode() in raw:
                        found_inj_anchors.add(anchor)
                if ccd_manifest is None and (
                    b"claude-code-releases" in raw
                ):
                    ccd_manifest = extract_claude_code_manifest_from_bytes(raw)
                if b"downloads.claude.ai/vms/" in raw:
                    has_vm_downloader = True

            if not (has_intl_anchor and has_regex_or_guard):
                loc_status = "fail"
                loc_details.append("未在 .vite/build/*.js 找到 Claude 语言配置入口锚点")
            else:
                loc_details.append("DesktopIntl 语言映射与文件名正则锚点匹配")

            ion_js = [p for p in ion_asar.file_paths() if p.endswith(".js")]
            ion_whitelist_ok = False
            for p in ion_js:
                blob = ion_asar.read_file(p) or b""
                if b"en-US" in blob and b"ja-JP" in blob:
                    ion_whitelist_ok = True
                    break
            if not ion_whitelist_ok:
                loc_status = "fail"
                loc_details.append("未在 ion-dist.asar 中找到 locale 白名单锚点")
            else:
                loc_details.append(f"ion-dist.asar 白名单校验通过（共 {len(ion_js)} 个 JS chunk）")

            missing_inj = [a for a in required_inj if a not in found_inj_anchors]
            if missing_inj:
                inj_status = "fail"
                inj_details.append(f"缺少 3P 注入契约字段: {', '.join(missing_inj)}")
            else:
                inj_details.append("3P 网关/MCP/模型/禁自更新全部契约字段均存在")

            if ccd_manifest:
                ccd_ver = str(ccd_manifest.get("version") or "")
                extra_audit["claudeCodeVersion"] = ccd_ver
                extra_audit["claudeCodeBaseUrl"] = ccd_manifest.get("baseUrl")
                inj_details.append(f"检测到内嵌 Claude Code CLI 版本: {ccd_ver}")
                pinned_ccd = (
                    ((pinned or {}).get("apps") or {})
                    .get("claude", {})
                    .get("companionArtifacts", {})
                    .get("claude-code", {})
                    .get("version")
                )
                if pinned_ccd and pinned_ccd != ccd_ver:
                    if inj_status == "pass":
                        inj_status = "warn"
                    inj_details.append(
                        f"内嵌 Claude Code 版本变更 ({pinned_ccd} -> {ccd_ver})，promote 前须同步更新 companionArtifacts 并镜像"
                    )
            else:
                if inj_status == "pass":
                    inj_status = "warn"
                inj_details.append("未提取到内嵌 claude-code-releases 清单，请人工核对 Code 模式下载源")

            extra_audit["hasVmDownloader"] = has_vm_downloader

    elif app_id == "antigravity":
        app_asar_path = resources / "app.asar"
        bin_path = bundle_path / "Contents" / "MacOS" / "Antigravity"
        if not app_asar_path.is_file():
            loc_status = "fail"
            inj_status = "fail"
            loc_details.append("缺少 Contents/Resources/app.asar")
        else:
            app_asar = SimpleAsarReader(app_asar_path)
            all_files = app_asar.file_paths()
            js_files = [p for p in all_files if p.endswith(".js")]
            loc_details.append(f"app.asar 可解析（共 {len(all_files)} 个文件 / {len(js_files)} 个 JS）")
            if not profile["electronAsarIntegrityKeys"]:
                loc_details.append("Info.plist 无 ElectronAsarIntegrity（无需哈希槽位改写）")
            else:
                loc_details.append(f"Info.plist 含 ElectronAsarIntegrity: {', '.join(profile['electronAsarIntegrityKeys'])}")
            if bin_path.is_file():
                bin_bytes = bin_path.read_bytes()
                has_fuse = b"dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX" in bin_bytes
                has_slot = b"AGbevlPCksUGKNL8TSn7wGmJEuJsXb2A" in bin_bytes
                loc_details.append(f"Mach-O Electron fuse={has_fuse}, integrity_slot={has_slot}")
            dict_path = REPO_ROOT / "src-tauri" / "src" / "adapters" / "antigravity_zh_dict.json"
            if dict_path.is_file():
                agy_dict = json.loads(dict_path.read_text(encoding="utf-8"))
                loc_details.append(f"内置反重力汉化词典条目: {len(agy_dict)}")
            inj_details.append("Antigravity 代理与自定义模型槽位不依赖破坏性 bundle 结构")
            extra_audit["asarFileCount"] = len(all_files)

    elif app_id == "codex":
        zh_lproj = resources / "zh_CN.lproj"
        app_asar_path = resources / "app.asar"
        if zh_lproj.is_dir():
            loc_details.append("原生 zh_CN.lproj 语言资源存在")
        else:
            loc_status = "fail"
            loc_details.append("缺少原生 zh_CN.lproj 语言资源")

        if not app_asar_path.is_file():
            inj_status = "fail"
            inj_details.append("缺少 Contents/Resources/app.asar")
        else:
            raw_asar = app_asar_path.read_bytes()
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

        owl_ini = resources / "owl-app.ini"
        if owl_ini.is_file():
            owl_text = owl_ini.read_text(encoding="utf-8", errors="replace").strip()
            extra_audit["owlAppIni"] = owl_text
            if "UserDataDirectoryName=Codex" not in owl_text:
                if inj_status == "pass":
                    inj_status = "warn"
                inj_details.append(f"owl-app.ini 目录名变化: {owl_text}")

        codex_pkg = resources / "codex-cli" / "codex-package.json"
        if codex_pkg.is_file():
            pkg_obj = json.loads(codex_pkg.read_text(encoding="utf-8"))
            extra_audit["codexCliVersion"] = pkg_obj.get("version")
            inj_details.append(f"内嵌 codex-cli 版本: {pkg_obj.get('version')}")

        masque = resources / "masque-proxy-origins.txt"
        if masque.is_file():
            extra_audit["masqueProxyOrigins"] = masque.read_text(encoding="utf-8", errors="replace").strip()

    elif app_id == "workbuddy":
        loc_details.append("WorkBuddy 国内版原生简体中文")
        asar_path = resources / "app.asar"
        if asar_path.is_file():
            asar = SimpleAsarReader(asar_path)
            prod_raw = asar.read_file("cli/product.json")
            if prod_raw:
                prod = json.loads(prod_raw.decode("utf-8", errors="replace"))
                if (prod.get("productFeatures") or {}).get("CustomModelsJSON") is True:
                    inj_details.append("cli/product.json CustomModelsJSON=true（支持 ~/.workbuddy/models.json 热加载）")
                else:
                    inj_status = "fail"
                    inj_details.append("cli/product.json 缺少 CustomModelsJSON=true，无法通过 models.json 接入 Tako 网关")
            else:
                inj_status = "warn"
                inj_details.append("app.asar 中未找到 cli/product.json，请人工核对 models.json 兼容")
        else:
            inj_status = "fail"
            inj_details.append("Resources 下缺少 app.asar")

    elif app_id == "dsh-tako":
        loc_details.append("自研客户端原生简体中文")
        if profile["executables"]:
            inj_details.append(f"可执行文件就绪: {', '.join(profile['executables'])}")
        else:
            inj_status = "fail"
            inj_details.append("Contents/MacOS 下缺少可执行文件")

    # Gate 3: Anomaly audit against saved baseline
    audit_status, audit_findings = compare_with_baseline(app_id, profile, extra_audit, baselines_dir)

    if update_baseline:
        baselines_dir.mkdir(parents=True, exist_ok=True)
        baseline_file = baselines_dir / f"{app_id}.macos.json"
        baseline_payload = {
            "appId": app_id,
            "capturedAt": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "profile": profile,
            "extraAudit": extra_audit,
        }
        baseline_file.write_text(json.dumps(baseline_payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        audit_status = "pass"
        try:
            display_path = str(baseline_file.relative_to(REPO_ROOT))
        except ValueError:
            display_path = str(baseline_file)
        audit_findings = [f"已更新基线档案 {display_path}"]

    statuses = (loc_status, inj_status, audit_status)
    if "fail" in statuses:
        overall = "fail"
    elif "warn" in statuses:
        overall = "warn"
    else:
        overall = "pass"

    return {
        "appId": app_id,
        "bundlePath": str(bundle_path),
        "version": profile["shortVersion"] or profile["bundleVersion"],
        "verifiedAt": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "overall": overall,
        "profile": profile,
        "extraAudit": extra_audit,
        "gates": {
            "localization": {"status": loc_status, "details": loc_details},
            "injection": {"status": inj_status, "details": inj_details},
            "anomalyAudit": {
                "status": audit_status,
                "details": audit_findings or ["与基线对比无异常新增项"],
            },
        },
    }


def extract_bundle_from_archive(archive_path: Path, tmp_dir: Path) -> Path:
    suffix = archive_path.suffix.lower()
    if suffix == ".zip":
        with zipfile.ZipFile(archive_path, "r") as zf:
            zf.extractall(tmp_dir)
        apps = sorted(tmp_dir.rglob("*.app"))
        if not apps:
            raise RuntimeError(f"ZIP 中未找到 .app: {archive_path}")
        return apps[0]
    if suffix == ".dmg":
        mount_dir = tmp_dir / "dmg-mount"
        mount_dir.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            ["hdiutil", "attach", "-nobrowse", "-readonly", "-mountpoint", str(mount_dir), str(archive_path)],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        try:
            apps = sorted(p for p in mount_dir.iterdir() if p.suffix == ".app")
            if not apps:
                raise RuntimeError(f"DMG 中未找到 .app: {archive_path}")
            copied = tmp_dir / apps[0].name
            shutil.copytree(apps[0], copied, symlinks=True)
            return copied
        finally:
            subprocess.run(["hdiutil", "detach", str(mount_dir), "-quiet"], check=False)
    raise RuntimeError(f"不支持的归档格式: {archive_path}")


# ---------------------------------------------------------------------------
# Rendering & Promotion
# ---------------------------------------------------------------------------


def render_manifest_from_pinned(pinned: dict, generated_at: str | None = None) -> dict:
    apps_out: dict[str, dict] = {}
    for app_id, entry in (pinned.get("apps") or {}).items():
        apps_out[app_id] = {
            "displayName": entry["displayName"],
            "version": entry["version"],
            "platforms": entry["platforms"],
        }
    return {
        "schemaVersion": int(pinned.get("schemaVersion", 1)),
        "generatedAt": generated_at or datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "apps": apps_out,
    }


def render_agy_cli_manifests_from_pinned(
    pinned: dict,
    generated_at: str | None = None,
) -> dict[str, dict]:
    """Render deterministic per-platform agy-cli manifest files + index.json from pinned-versions.json.

    Each `<platform>.json` stays 100% compatible with official `agy` updater format (`version`, `url`, `sha512`)
    while attaching Tako managed metadata (`fallbackUrl`, `size`, `minSupportedVersion`, `supportedVersions`,
    `channel`, `approvedAt`) for independent update checks in `tako-cli`.
    """
    agy = ((pinned.get("cliTools") or {}).get("agy")) or {}
    version = str(agy.get("version") or "").strip()
    platforms = agy.get("platforms") or {}
    if not version or not platforms:
        return {}

    now = generated_at or str(agy.get("approvedAt") or "").strip() or datetime.now(timezone.utc).strftime(
        "%Y-%m-%dT%H:%M:%SZ"
    )
    supported_versions = [
        str(v).strip() for v in (agy.get("supportedVersions") or [version]) if str(v).strip()
    ]
    if version not in supported_versions:
        supported_versions.append(version)
    min_supported = str(agy.get("minSupportedVersion") or supported_versions[0]).strip()
    channel = str(agy.get("channel") or "tako-managed").strip()
    approved_at = str(agy.get("approvedAt") or now).strip()

    files: dict[str, dict] = {}
    index_platforms: dict[str, dict] = {}
    for plat in sorted(platforms.keys()):
        art = platforms[plat] or {}
        url = str(art.get("url") or "").strip()
        sha512 = str(art.get("sha512") or "").strip().lower()
        if not url or not sha512:
            continue
        p_ver = str(art.get("version") or version).strip()
        payload: dict[str, object] = {
            "version": p_ver,
            "url": url,
            "sha512": sha512,
        }
        if art.get("fallbackUrl"):
            payload["fallbackUrl"] = str(art["fallbackUrl"]).strip()
        if isinstance(art.get("size"), int) and art["size"] > 0:
            payload["size"] = art["size"]
        payload["minSupportedVersion"] = min_supported
        payload["supportedVersions"] = supported_versions
        payload["channel"] = channel
        payload["approvedAt"] = approved_at
        files[f"{plat}.json"] = payload

        idx_entry: dict[str, object] = {"url": url, "sha512": sha512}
        if art.get("fallbackUrl"):
            idx_entry["fallbackUrl"] = str(art["fallbackUrl"]).strip()
        if isinstance(art.get("size"), int) and art["size"] > 0:
            idx_entry["size"] = art["size"]
        index_platforms[plat] = idx_entry

    files["index.json"] = {
        "schemaVersion": 1,
        "tool": "agy",
        "displayName": str(agy.get("displayName") or "Antigravity CLI (agy)"),
        "version": version,
        "status": str(agy.get("status") or "approved"),
        "channel": channel,
        "approvedAt": approved_at,
        "generatedAt": now,
        "minSupportedVersion": min_supported,
        "supportedVersions": supported_versions,
        "platforms": index_platforms,
    }
    return files


def write_agy_cli_manifests(manifests: dict[str, dict], out_dir: Path) -> list[Path]:
    out_dir.mkdir(parents=True, exist_ok=True)
    written: list[Path] = []
    expected_names = set(manifests.keys())
    for existing in out_dir.glob("*.json"):
        if existing.name not in expected_names:
            existing.unlink()
    for name, payload in sorted(manifests.items()):
        target = out_dir / name
        target.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        written.append(target)
    return written


def verify_agy_cli_candidate(
    candidate_report: dict,
    gateway_lock: dict | None = None,
    diff_report: dict | None = None,
    require_gateway_lock: bool = False,
) -> dict:
    """Verify an Antigravity CLI (`agy`) candidate before independent promotion.

    Gates:
      1. `checksumVerify`: all required platforms present, HTTPS URLs, valid 128-hex SHA-512, consistent version.
      2. `gatewayLock`   : when `gateway_lock` (`agy-gateway/profiles/lock.json`) is supplied, verifies that
                           `families.cli.versions[ver]` is already registered and inside `supported_window`.
      3. `protocolDiff`  : when `diff_report` (`agy-gateway capture-diff --gate`) is supplied, verifies 0 blocking diffs.
    """
    cli_entry = ((candidate_report.get("cliTools") or {}).get("agy")) or {}
    upstream = cli_entry.get("upstreamEntry") if "upstreamEntry" in cli_entry else cli_entry
    if not isinstance(upstream, dict) or not upstream.get("platforms"):
        raise ValueError("Candidate report missing cliTools.agy.upstreamEntry.platforms")

    version = str(upstream.get("version") or "").strip()
    platforms = upstream.get("platforms") or {}

    # Gate 1: checksum & required platform completeness
    ck_status = "pass"
    ck_details: list[str] = []
    missing_plats = [p for p in AGY_CLI_REQUIRED_PLATFORMS if p not in platforms]
    if missing_plats:
        ck_status = "fail"
        ck_details.append(f"缺少必选平台: {', '.join(missing_plats)}")
    if not version:
        ck_status = "fail"
        ck_details.append("缺少候选版本号")
    for plat, art in sorted(platforms.items()):
        p_ver = str((art or {}).get("version") or "").strip()
        p_url = str((art or {}).get("url") or "").strip()
        p_sha = str((art or {}).get("sha512") or "").strip().lower()
        if version and p_ver and p_ver != version:
            ck_status = "fail"
            ck_details.append(f"{plat} 版本不一致: {p_ver} != {version}")
        if not p_url.startswith("https://"):
            ck_status = "fail"
            ck_details.append(f"{plat} URL 非 HTTPS: {p_url}")
        if not SHA512_HEX_RE.match(p_sha):
            ck_status = "fail"
            ck_details.append(f"{plat} sha512 非合法 128 位 hex")
    if ck_status == "pass":
        ck_details.append(f"已校验 {len(platforms)} 个平台（含 5 个必选平台），版本 {version} 与 SHA-512 格式均合法")

    # Gate 2: agy-gateway lock window check (backend-first invariant)
    gw_status = "pass"
    gw_details: list[str] = []
    if gateway_lock is None:
        if require_gateway_lock:
            gw_status = "fail"
            gw_details.append("未提供 --gateway-lock (agy-gateway/profiles/lock.json)")
        else:
            gw_status = "warn"
            gw_details.append("未传入 --gateway-lock；请人工确认 agy-gateway 已扩窗支持该版本")
    else:
        cli_fam = ((gateway_lock.get("families") or {}).get("cli")) or {}
        window = [str(v) for v in (cli_fam.get("supported_window") or [])]
        ver_entry = (cli_fam.get("versions") or {}).get(version)
        if not isinstance(ver_entry, dict):
            gw_status = "fail"
            gw_details.append(f"agy-gateway profiles/lock.json 尚未登记 cli/{version}（须先扩窗再放行客户端）")
        else:
            v_status = str(ver_entry.get("status") or "").lower()
            if v_status not in ("approved", "supported"):
                gw_status = "fail"
                gw_details.append(f"cli/{version} 在网关锁中的状态为 '{v_status}'（须为 approved/supported）")
            if version not in window:
                gw_status = "fail"
                gw_details.append(f"cli/{version} 不在网关 supported_window ({', '.join(window)}) 内")
            if gw_status == "pass":
                gw_details.append(
                    f"agy-gateway 已登记 cli/{version} (status={v_status}, window={','.join(window)})"
                )

    # Gate 3: Gate 5 protocol capture-diff check
    diff_status = "pass"
    diff_details: list[str] = []
    if diff_report is None:
        diff_status = "warn"
        diff_details.append("未传入 --diff-report；建议附带 agy-gateway capture-diff --gate 报告")
    else:
        passed = diff_report.get("passed")
        fail_count = diff_report.get("fail_count")
        blocking = diff_report.get("blocking_diffs")
        if passed is False or (isinstance(fail_count, int) and fail_count > 0) or (
            isinstance(blocking, list) and len(blocking) > 0
        ):
            diff_status = "fail"
            diff_details.append(
                f"Gate 5 协议字段级 diff 未通过 (passed={passed}, fail_count={fail_count})"
            )
        else:
            diff_details.append("Gate 5 协议字段级 diff 通过（无阻断性差异）")

    statuses = (ck_status, gw_status, diff_status)
    overall = "fail" if "fail" in statuses else ("warn" if "warn" in statuses else "pass")
    return {
        "toolId": "agy",
        "version": version,
        "verifiedAt": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "overall": overall,
        "gates": {
            "checksumVerify": {"status": ck_status, "details": ck_details},
            "gatewayLock": {"status": gw_status, "details": gw_details},
            "protocolDiff": {"status": diff_status, "details": diff_details},
        },
    }


def promote_agy_cli_candidate(
    pinned: dict,
    candidate_report: dict,
    gate_report: dict | None,
    force_skip_gate: bool = False,
    reason: str = "",
    supported_versions: list[str] | None = None,
    min_supported_version: str | None = None,
    changelog_summary: str = "",
) -> dict:
    """Independently promote `cliTools.agy` in `pinned-versions.json` without touching Desktop apps."""
    cand_cli = ((candidate_report.get("cliTools") or {}).get("agy")) or {}
    upstream_entry = cand_cli.get("upstreamEntry") or {}
    if not upstream_entry or not upstream_entry.get("version") or not upstream_entry.get("platforms"):
        raise ValueError("Candidate report missing cliTools.agy.upstreamEntry")

    new_version = str(upstream_entry["version"]).strip()

    if not force_skip_gate:
        if not gate_report:
            raise ValueError("Refusing to promote agy-cli without --gate-report (or explicit --force-skip-gate)")
        if gate_report.get("toolId") != "agy":
            raise ValueError("Gate report is not for toolId 'agy'")
        if gate_report.get("version") and str(gate_report["version"]) != new_version:
            raise ValueError(
                f"Gate report version {gate_report['version']} does not match candidate version {new_version}"
            )
        if gate_report.get("overall") == "fail":
            raise ValueError(f"Refusing to promote agy-cli {new_version}: gate verification FAILED")
        gate_summary = {
            k: v.get("status", "unknown") for k, v in (gate_report.get("gates") or {}).items()
        }
    else:
        if not reason.strip():
            raise ValueError("--force-skip-gate requires a non-empty --reason")
        gate_summary = {
            "checksumVerify": "skipped",
            "gatewayLock": "skipped",
            "protocolDiff": "skipped",
            "skipReason": reason.strip(),
        }

    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    existing = ((pinned.get("cliTools") or {}).get("agy")) or {}
    prev_version = str(existing.get("version") or "").strip()

    if supported_versions:
        window = [v.strip() for v in supported_versions if v.strip()]
    else:
        # Roll forward a 2-version compatibility window [previous, new] so existing users
        # stay uninterrupted while new installs / `tako update agy` get the newly approved build.
        window = [v for v in (prev_version, new_version) if v]
        window = list(dict.fromkeys(window))
    if new_version not in window:
        window.append(new_version)
    min_supported = (min_supported_version or "").strip() or window[0]

    clean_platforms: dict[str, dict] = {}
    for plat, art in sorted((upstream_entry.get("platforms") or {}).items()):
        entry: dict[str, object] = {
            "url": str(art["url"]).strip(),
            "sha512": str(art["sha512"]).strip().lower(),
        }
        if isinstance(art.get("size"), int) and art["size"] > 0:
            entry["size"] = art["size"]
        clean_platforms[plat] = entry

    updated_entry = dict(existing)
    updated_entry["displayName"] = str(existing.get("displayName") or "Antigravity CLI (agy)")
    updated_entry["version"] = new_version
    updated_entry["status"] = "approved"
    updated_entry["approvedAt"] = now
    updated_entry["minSupportedVersion"] = min_supported
    updated_entry["supportedVersions"] = window
    updated_entry["channel"] = str(existing.get("channel") or "tako-managed")
    updated_entry["changelogSummary"] = (
        changelog_summary.strip()
        or f"Promoted Antigravity CLI (agy) {new_version} after Gate 5 & checksum verification."
    )
    updated_entry["gates"] = gate_summary
    updated_entry["platforms"] = clean_platforms

    out = json.loads(json.dumps(pinned))
    out["updatedAt"] = now
    out.setdefault("cliTools", {})["agy"] = updated_entry
    return out


def promote_candidate(
    pinned: dict,
    app_id: str,
    candidate_report: dict,
    gate_report: dict | None,
    force_skip_gate: bool = False,
    reason: str = "",
) -> dict:
    cand_app = (candidate_report.get("apps") or {}).get(app_id)
    if not cand_app or not cand_app.get("upstreamEntry"):
        raise ValueError(f"Candidate report missing upstreamEntry for '{app_id}'")

    if not force_skip_gate:
        if not gate_report:
            raise ValueError("Refusing to promote without --gate-report (or explicit --force-skip-gate)")
        target_gate = gate_report
        if "results" in gate_report and isinstance(gate_report["results"], dict):
            target_gate = gate_report["results"].get(app_id)
        if not target_gate or target_gate.get("appId") != app_id:
            raise ValueError(f"Gate report does not contain verification result for '{app_id}'")
        if target_gate.get("overall") == "fail":
            raise ValueError(f"Refusing to promote '{app_id}': macOS 3-gate verification FAILED")
        gate_summary = {
            k: v.get("status", "unknown") for k, v in (target_gate.get("gates") or {}).items()
        }
    else:
        if not reason.strip():
            raise ValueError("--force-skip-gate requires a non-empty --reason")
        gate_summary = {
            "localization": "skipped",
            "injection": "skipped",
            "anomalyAudit": "skipped",
            "skipReason": reason.strip(),
        }

    upstream_entry = cand_app["upstreamEntry"]
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    existing = (pinned.get("apps") or {}).get(app_id) or {}

    changelog_entries = (cand_app.get("changelog") or {}).get("entries") or []
    summary = (
        f"Promoted {upstream_entry['version']} after review."
        if not changelog_entries
        else f"{upstream_entry['version']}: {(changelog_entries[0].get('body') or '')[:240]}"
    )

    updated_entry = dict(existing)
    updated_entry["displayName"] = upstream_entry.get("displayName") or existing.get("displayName") or app_id
    updated_entry["version"] = upstream_entry["version"]
    updated_entry["status"] = "approved"
    updated_entry["approvedAt"] = now
    updated_entry["changelogSummary"] = summary
    updated_entry["gates"] = gate_summary
    updated_entry["platforms"] = upstream_entry["platforms"]

    out = json.loads(json.dumps(pinned))
    out["updatedAt"] = now
    out.setdefault("apps", {})[app_id] = updated_entry
    return out


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def cmd_check(args: argparse.Namespace) -> int:
    pinned = json.loads(Path(args.pinned).read_text(encoding="utf-8"))
    sync_mod = load_sync_manifest_module()
    upstream_manifest = sync_mod.build_manifest(mirror_base=None)
    changelogs = fetch_changelogs(upstream_manifest.get("apps") or {})
    upstream_cli_tools = None
    if not getattr(args, "skip_cli", False):
        upstream_cli_tools = {"agy": fetch_upstream_agy_cli_manifests()}
    report_json, report_md = build_check_report(
        pinned, upstream_manifest, changelogs, upstream_cli_tools=upstream_cli_tools
    )

    if args.out_json:
        p = Path(args.out_json)
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(json.dumps(report_json, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    if args.out_md:
        p = Path(args.out_md)
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(report_md, encoding="utf-8")
    print(report_md)
    return 0


def cmd_verify_macos(args: argparse.Namespace) -> int:
    pinned = json.loads(Path(args.pinned).read_text(encoding="utf-8")) if Path(args.pinned).is_file() else None
    targets = list(APP_ORDER) if args.app == "all" else [args.app]
    if len(targets) > 1 and (args.bundle or args.archive):
        raise SystemExit("--bundle / --archive can only be used with a single --app")

    results: dict[str, dict] = {}
    overall_failed = False

    with tempfile.TemporaryDirectory(prefix="tako-gate-macos-") as tmp:
        tmp_dir = Path(tmp)
        for app_id in targets:
            if args.bundle:
                bundle = Path(args.bundle)
            elif args.archive:
                bundle = extract_bundle_from_archive(Path(args.archive), tmp_dir)
            else:
                found = next((p for p in INSTALLED_MACOS_PATHS.get(app_id, []) if p.is_dir()), None)
                if not found:
                    results[app_id] = {
                        "appId": app_id,
                        "overall": "fail",
                        "error": f"未在本机 /Applications 找到 {app_id}，请传入 --bundle 或 --archive",
                    }
                    overall_failed = True
                    continue
                bundle = found

            res = verify_macos_bundle(
                app_id=app_id,
                bundle_path=bundle,
                pinned=pinned,
                baselines_dir=Path(args.baselines_dir),
                update_baseline=args.update_baseline,
            )
            results[app_id] = res
            if res["overall"] == "fail":
                overall_failed = True

    payload = (
        results[targets[0]]
        if len(targets) == 1
        else {"verifiedAt": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"), "results": results}
    )

    if args.out_json:
        out_p = Path(args.out_json)
        out_p.parent.mkdir(parents=True, exist_ok=True)
        out_p.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    for app_id, res in results.items():
        icon = {"pass": "PASS", "warn": "WARN", "fail": "FAIL"}.get(res.get("overall", ""), "UNKNOWN")
        print(f"[{icon}] {app_id} v{res.get('version', '?')} ({res.get('bundlePath', '')})")
        if res.get("error"):
            print(f"  ERROR: {res['error']}")
            continue
        for gname, gdata in (res.get("gates") or {}).items():
            print(f"  - {gname}: {gdata['status'].upper()} | {'; '.join(gdata.get('details') or [])}")

    return 1 if overall_failed else 0


def cmd_verify_windows(args: argparse.Namespace) -> int:
    """Dispatch Windows 3-gate (and optional Gate-4 live smoke) verification to .6 tako-win-lab."""
    import time

    base = args.lab_url.rstrip("/")
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def _req(method: str, path: str, body: dict | None = None) -> dict:
        data = json.dumps(body, ensure_ascii=False).encode("utf-8") if body is not None else None
        req = urllib.request.Request(
            f"{base}{path}",
            data=data,
            headers={"Content-Type": "application/json", "Accept": "application/json"},
            method=method,
        )
        with opener.open(req, timeout=30) as resp:
            return json.loads(resp.read().decode("utf-8"))

    run_ids: list[str] = []
    if args.smoke:
        if args.app == "all":
            raise SystemExit("--smoke must target a single --app (codex/claude/antigravity/dsh-tako)")
        resp = _req(
            "POST",
            "/api/runs/smoke",
            {
                "app_id": args.app,
                "candidate_url": args.candidate_url,
                "version": args.version,
                "hold_seconds": args.hold_seconds,
                "keep_installed": args.keep_installed,
                "report_hq": not args.no_hq,
                "trigger": "cli",
            },
        )
        run_ids.append(resp["data"]["run"]["id"])
    else:
        resp = _req(
            "POST",
            "/api/runs/gate",
            {
                "app_id": args.app,
                "candidate_url": args.candidate_url,
                "version": args.version,
                "update_baseline": args.update_baseline,
                "report_hq": not args.no_hq,
                "trigger": "cli",
            },
        )
        run_ids.extend(r["id"] for r in resp["data"]["runs"])

    print(f"Dispatched {len(run_ids)} job(s) to {base}: {', '.join(run_ids)}", flush=True)
    deadline = time.monotonic() + max(60, int(args.timeout))
    finished: dict[str, dict] = {}
    while len(finished) < len(run_ids) and time.monotonic() < deadline:
        for rid in run_ids:
            if rid in finished:
                continue
            detail = _req("GET", f"/api/runs/{rid}")["data"]
            if detail.get("status") not in ("queued", "running"):
                finished[rid] = detail
        if len(finished) < len(run_ids):
            time.sleep(3)

    if len(finished) < len(run_ids):
        print(f"ERROR: timed out waiting for Windows lab jobs: {set(run_ids) - set(finished)}", file=sys.stderr)
        return 2

    overall_failed = False
    for rid in run_ids:
        r = finished[rid]
        st = str(r.get("status") or "error")
        if st in ("fail", "error"):
            overall_failed = True
        print(f"[{st.upper()}] {r.get('app_id')} v{r.get('target_version')} ({r.get('kind')}) - {r.get('summary')}")
        for gname, gdata in (r.get("gates") or {}).items():
            if isinstance(gdata, dict):
                print(f"  - {gname}: {str(gdata.get('status', '')).upper()} | {'; '.join(gdata.get('details') or [])}")
        if r.get("hq_ticket_id"):
            print(f"  - HQ Ticket: #{r['hq_ticket_id']}")

    payload = finished[run_ids[0]] if len(run_ids) == 1 else {"runs": [finished[rid] for rid in run_ids]}
    if args.out_json:
        out_p = Path(args.out_json)
        out_p.parent.mkdir(parents=True, exist_ok=True)
        out_p.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    return 1 if overall_failed else 0


def cmd_render_manifest(args: argparse.Namespace) -> int:
    pinned = json.loads(Path(args.pinned).read_text(encoding="utf-8"))
    if not getattr(args, "agy_cli_only", False):
        out_path = Path(args.out)
        prev_generated = None
        if out_path.is_file():
            try:
                prev = json.loads(out_path.read_text(encoding="utf-8"))
                rendered_preview = render_manifest_from_pinned(pinned, generated_at=prev.get("generatedAt"))
                if prev.get("apps") == rendered_preview.get("apps"):
                    prev_generated = prev.get("generatedAt")
            except Exception:
                pass
        manifest = render_manifest_from_pinned(pinned, generated_at=prev_generated)
        out_path.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        print(f"Rendered {out_path} from {args.pinned} ({len(manifest['apps'])} apps)")

    agy_cli_out = getattr(args, "agy_cli_out_dir", "")
    if agy_cli_out:
        agy_files = render_agy_cli_manifests_from_pinned(pinned)
        if agy_files:
            written = write_agy_cli_manifests(agy_files, Path(agy_cli_out))
            print(
                f"Rendered {len(written)} agy-cli manifest file(s) into {agy_cli_out} "
                f"(version={agy_files['index.json']['version']})"
            )
    return 0


def cmd_verify_agy_cli(args: argparse.Namespace) -> int:
    if args.candidate:
        candidate = json.loads(Path(args.candidate).read_text(encoding="utf-8"))
    else:
        upstream = fetch_upstream_agy_cli_manifests(base_url=args.upstream_base)
        candidate = {"cliTools": {"agy": {"upstreamEntry": upstream}}}
    gateway_lock = (
        json.loads(Path(args.gateway_lock).read_text(encoding="utf-8")) if args.gateway_lock else None
    )
    diff_report = (
        json.loads(Path(args.diff_report).read_text(encoding="utf-8")) if args.diff_report else None
    )
    res = verify_agy_cli_candidate(
        candidate_report=candidate,
        gateway_lock=gateway_lock,
        diff_report=diff_report,
        require_gateway_lock=args.require_gateway_lock,
    )
    if args.out_json:
        out_p = Path(args.out_json)
        out_p.parent.mkdir(parents=True, exist_ok=True)
        out_p.write_text(json.dumps(res, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    icon = {"pass": "PASS", "warn": "WARN", "fail": "FAIL"}.get(res["overall"], "UNKNOWN")
    print(f"[{icon}] agy-cli v{res.get('version', '?')}")
    for gname, gdata in (res.get("gates") or {}).items():
        print(f"  - {gname}: {gdata['status'].upper()} | {'; '.join(gdata.get('details') or [])}")
    return 1 if res["overall"] == "fail" else 0


def cmd_promote_agy_cli(args: argparse.Namespace) -> int:
    pinned_path = Path(args.pinned)
    pinned = json.loads(pinned_path.read_text(encoding="utf-8"))
    candidate = json.loads(Path(args.candidate).read_text(encoding="utf-8"))
    gate_report = (
        json.loads(Path(args.gate_report).read_text(encoding="utf-8")) if args.gate_report else None
    )
    supported_list = (
        [v.strip() for v in args.supported_versions.split(",") if v.strip()]
        if args.supported_versions
        else None
    )
    updated_pinned = promote_agy_cli_candidate(
        pinned=pinned,
        candidate_report=candidate,
        gate_report=gate_report,
        force_skip_gate=args.force_skip_gate,
        reason=args.reason,
        supported_versions=supported_list,
        min_supported_version=args.min_supported_version or None,
        changelog_summary=args.summary or "",
    )
    pinned_path.write_text(json.dumps(updated_pinned, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    agy_files = render_agy_cli_manifests_from_pinned(updated_pinned)
    written = write_agy_cli_manifests(agy_files, Path(args.agy_cli_out_dir))
    agy_entry = updated_pinned["cliTools"]["agy"]
    print(
        f"Promoted cliTools.agy -> {agy_entry['version']} "
        f"(window={','.join(agy_entry['supportedVersions'])}) into {pinned_path} "
        f"and rendered {len(written)} manifest file(s) in {args.agy_cli_out_dir}"
    )
    return 0


def cmd_promote(args: argparse.Namespace) -> int:
    pinned_path = Path(args.pinned)
    pinned = json.loads(pinned_path.read_text(encoding="utf-8"))
    candidate = json.loads(Path(args.candidate).read_text(encoding="utf-8"))
    gate_report = (
        json.loads(Path(args.gate_report).read_text(encoding="utf-8")) if args.gate_report else None
    )
    updated_pinned = promote_candidate(
        pinned=pinned,
        app_id=args.app,
        candidate_report=candidate,
        gate_report=gate_report,
        force_skip_gate=args.force_skip_gate,
        reason=args.reason,
    )
    pinned_path.write_text(json.dumps(updated_pinned, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    manifest_path = Path(args.manifest_out)
    manifest = render_manifest_from_pinned(updated_pinned)
    manifest_path.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(
        f"Promoted {args.app} -> {updated_pinned['apps'][args.app]['version']} "
        f"into {pinned_path} and rendered {manifest_path}"
    )
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Tako Switch controlled release manager & macOS 3-gate verifier")
    sub = parser.add_subparsers(dest="cmd", required=True)

    p_check = sub.add_parser("check", help="Check upstream releases & analyze changelogs (read-only)")
    p_check.add_argument("--pinned", default=str(DEFAULT_PINNED_PATH))
    p_check.add_argument("--skip-cli", action="store_true", help="Skip checking upstream agy-cli manifests")
    p_check.add_argument("--out-json", default="")
    p_check.add_argument("--out-md", default="")
    p_check.set_defaults(func=cmd_check)

    p_verify = sub.add_parser("verify-macos", help="Run macOS 3-gate release verification")
    p_verify.add_argument("--app", choices=(*APP_ORDER, "all"), default="all")
    p_verify.add_argument("--bundle", default="", help="Path to candidate .app bundle")
    p_verify.add_argument("--archive", default="", help="Path to candidate .zip or .dmg installer")
    p_verify.add_argument("--pinned", default=str(DEFAULT_PINNED_PATH))
    p_verify.add_argument("--baselines-dir", default=str(DEFAULT_BASELINES_DIR))
    p_verify.add_argument("--update-baseline", action="store_true", help="Record current bundle as approved baseline")
    p_verify.add_argument("--out-json", default="")
    p_verify.set_defaults(func=cmd_verify_macos)

    p_verify_win = sub.add_parser("verify-windows", help="Dispatch Windows 3-gate or Gate-4 smoke test to .6 tako-win-lab")
    p_verify_win.add_argument("--app", choices=(*APP_ORDER, "all"), default="all")
    p_verify_win.add_argument("--lab-url", default=os.environ.get("TAKO_WIN_LAB_URL", "http://172.17.66.6:18795"))
    p_verify_win.add_argument("--candidate-url", default="", help="Optional custom Windows installer URL (.msix/.exe)")
    p_verify_win.add_argument("--version", default="", help="Optional candidate version hint")
    p_verify_win.add_argument("--smoke", action="store_true", help="Run Gate-4 live install/launch/EventLog smoke test")
    p_verify_win.add_argument("--hold-seconds", type=int, default=15)
    p_verify_win.add_argument("--keep-installed", action="store_true")
    p_verify_win.add_argument("--update-baseline", action="store_true")
    p_verify_win.add_argument("--no-hq", action="store_true", help="Skip HQ event reporting for this manual run")
    p_verify_win.add_argument("--timeout", type=int, default=600)
    p_verify_win.add_argument("--out-json", default="")
    p_verify_win.set_defaults(func=cmd_verify_windows)

    p_verify_cli = sub.add_parser(
        "verify-agy-cli",
        help="Verify Antigravity CLI (agy) candidate against checksums, agy-gateway lock, and Gate 5 diff report",
    )
    p_verify_cli.add_argument("--candidate", default="", help="JSON output from `check --out-json` (fetches live upstream if omitted)")
    p_verify_cli.add_argument("--upstream-base", default=AGY_CLI_UPSTREAM_BASE)
    p_verify_cli.add_argument("--gateway-lock", default="", help="Path to agy-gateway profiles/lock.json")
    p_verify_cli.add_argument("--diff-report", default="", help="Path to agy-gateway capture-diff --gate JSON report")
    p_verify_cli.add_argument("--require-gateway-lock", action="store_true")
    p_verify_cli.add_argument("--out-json", default="")
    p_verify_cli.set_defaults(func=cmd_verify_agy_cli)

    p_render = sub.add_parser("render-manifest", help="Render manifest.json & agy-cli manifests from pinned-versions.json")
    p_render.add_argument("--pinned", default=str(DEFAULT_PINNED_PATH))
    p_render.add_argument("--out", default=str(DEFAULT_MANIFEST_PATH))
    p_render.add_argument("--agy-cli-out-dir", default=str(DEFAULT_AGY_CLI_MANIFESTS_DIR))
    p_render.add_argument("--agy-cli-only", action="store_true", help="Only render manifest/agy-cli/manifests/*.json")
    p_render.set_defaults(func=cmd_render_manifest)

    p_promote = sub.add_parser("promote", help="Promote a gate-verified Desktop candidate into pinned-versions.json")
    p_promote.add_argument("--app", choices=APP_ORDER, required=True)
    p_promote.add_argument("--candidate", required=True, help="JSON output from `check --out-json`")
    p_promote.add_argument("--gate-report", default="", help="JSON output from `verify-macos --out-json`")
    p_promote.add_argument("--pinned", default=str(DEFAULT_PINNED_PATH))
    p_promote.add_argument("--manifest-out", default=str(DEFAULT_MANIFEST_PATH))
    p_promote.add_argument("--force-skip-gate", action="store_true")
    p_promote.add_argument("--reason", default="")
    p_promote.set_defaults(func=cmd_promote)

    p_promote_cli = sub.add_parser(
        "promote-agy-cli",
        help="Independently promote a gate-verified Antigravity CLI (agy) candidate without touching Desktop apps",
    )
    p_promote_cli.add_argument("--candidate", required=True, help="JSON output from `check --out-json`")
    p_promote_cli.add_argument("--gate-report", default="", help="JSON output from `verify-agy-cli --out-json`")
    p_promote_cli.add_argument("--pinned", default=str(DEFAULT_PINNED_PATH))
    p_promote_cli.add_argument("--agy-cli-out-dir", default=str(DEFAULT_AGY_CLI_MANIFESTS_DIR))
    p_promote_cli.add_argument("--supported-versions", default="", help="Optional comma-separated compatibility window")
    p_promote_cli.add_argument("--min-supported-version", default="", help="Optional explicit minimum supported version")
    p_promote_cli.add_argument("--summary", default="", help="Optional changelog summary")
    p_promote_cli.add_argument("--force-skip-gate", action="store_true")
    p_promote_cli.add_argument("--reason", default="")
    p_promote_cli.set_defaults(func=cmd_promote_agy_cli)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
