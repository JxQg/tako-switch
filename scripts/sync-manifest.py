#!/usr/bin/env python3
"""Sync upstream official desktop app releases into manifest/manifest.json.

Only Python standard library modules are required so this script runs cleanly
in GitHub Actions and local macOS/Linux/Windows checkouts without pip installs.
"""

from __future__ import annotations

import argparse
import base64
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET

DSH_TAKO_OSS_LATEST_URL = "https://dsh-tako-dl.oss-cn-shanghai.aliyuncs.com/latest.json"
DSH_TAKO_GH_LATEST_API = "https://api.github.com/repos/tako-dev/dsh-tako-desktop/releases/latest"

ANTIGRAVITY_FEEDS = {
    "macos-arm64": (
        "https://antigravity-hub-auto-updater-974169037036.us-central1.run.app/manifest/latest-arm64-mac.yml",
        "zip",
        None,
    ),
    "macos-x64": (
        "https://antigravity-hub-auto-updater-974169037036.us-central1.run.app/manifest/latest-x64-mac.yml",
        "zip",
        None,
    ),
    "windows-x64": (
        "https://antigravity-hub-auto-updater-974169037036.us-central1.run.app/manifest/latest-x64-win.yml",
        "exe",
        # Antigravity 2.35.x ships NSIS (electron-builder): /S is the only silent switch.
        # installer.rs additionally sniffs the binary magic and overrides if Inno ever returns.
        ["/S"],
    ),
}

CLAUDE_MAC_JSON_URL = "https://downloads.claude.ai/releases/darwin/universal/RELEASES.json"
CLAUDE_WIN_RELEASES_URL = "https://downloads.claude.ai/releases/win32/x64/RELEASES"

CODEX_MAC_ARM_APPCAST = "https://persistent.oaistatic.com/codex-app-prod/appcast.xml"
CODEX_MAC_X64_APPCAST = "https://persistent.oaistatic.com/codex-app-prod/appcast-x64.xml"
CODEX_WIN_MSIX_URL = "https://persistent.oaistatic.com/codex-app-prod/ChatGPT-x64.msix"


def http_get_text(url: str, timeout: int = 20) -> str:
    req = urllib.request.Request(url, headers={"User-Agent": "tako-switch-manifest-sync/1.0"})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return resp.read().decode("utf-8", errors="replace")


def http_head(url: str, timeout: int = 20) -> dict[str, str]:
    req = urllib.request.Request(
        url,
        method="HEAD",
        headers={"User-Agent": "tako-switch-manifest-sync/1.0"},
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return {k.lower(): v for k, v in resp.headers.items()}


def b64_sha512_to_hex(raw: str) -> str:
    s = raw.strip()
    if re.fullmatch(r"[0-9a-fA-F]{128}", s):
        return s.lower()
    return base64.b64decode(s).hex()


def parse_electron_builder_yml(raw: str) -> dict:
    """Minimal line parser for electron-builder latest-*.yml feeds."""
    version = None
    first_url = None
    first_sha512 = None
    first_size = None

    top_sha512 = None
    in_files = False
    current: dict[str, str] = {}

    for line in raw.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if line.startswith("version:") and not in_files:
            version = line.split(":", 1)[1].strip().strip("'\"")
        elif line.startswith("sha512:") and not in_files:
            top_sha512 = line.split(":", 1)[1].strip().strip("'\"")
        elif line.startswith("files:"):
            in_files = True
        elif in_files and not line.startswith((" ", "\t", "-")):
            in_files = False
            if line.startswith("sha512:"):
                top_sha512 = line.split(":", 1)[1].strip().strip("'\"")
        elif in_files:
            if stripped.startswith("- ") and not current:
                kv = stripped[2:].split(":", 1)
                if len(kv) == 2:
                    current[kv[0].strip()] = kv[1].strip().strip("'\"")
            elif ":" in stripped and "url" in current and first_url is None:
                k, v = stripped.split(":", 1)
                k = k.strip()
                if k in ("url", "sha512", "size"):
                    current[k] = v.strip().strip("'\"")
        if "url" in current and "sha512" in current and first_url is None and "size" in current:
            first_url = current["url"]
            first_sha512 = current["sha512"]
            first_size = int(current["size"])

    if not first_url and "url" in current:
        first_url = current["url"]
        first_sha512 = current.get("sha512") or top_sha512
        if "size" in current:
            first_size = int(current["size"])

    if not version or not first_url:
        raise ValueError("Invalid electron-builder YAML: missing version or file url")

    out: dict = {"version": version, "url": first_url}
    if first_sha512 or top_sha512:
        out["sha512"] = b64_sha512_to_hex(first_sha512 or top_sha512 or "")
    if first_size is not None:
        out["size"] = first_size
    return out


def parse_claude_mac_releases(raw: str) -> dict:
    data = json.loads(raw)
    cur_ver = data["currentRelease"]
    releases = data["releases"]
    selected = next((r for r in releases if r.get("version") == cur_ver), releases[-1])
    url = selected["updateTo"]["url"]
    commit_map = {}
    for r in releases:
        ver = r.get("version")
        rurl = r.get("updateTo", {}).get("url", "")
        m = re.search(r"Claude-([0-9a-f]{12,})\.zip", rurl)
        if ver and m:
            commit_map[ver] = m.group(1)
    return {"version": cur_ver, "url": url, "commitByVersion": commit_map}


def parse_claude_win_releases(raw: str) -> tuple[str, str]:
    text = raw.lstrip("\ufeff").strip()
    entries: list[tuple[str, str]] = []
    for line in text.splitlines():
        parts = line.strip().split()
        if len(parts) >= 2:
            m = re.match(r"AnthropicClaude-([0-9][0-9A-Za-z._-]*)-full\.nupkg$", parts[1])
            if m:
                entries.append((m.group(1), parts[0].lower()))
    if not entries:
        raise ValueError("No AnthropicClaude nupkg entry found in Windows RELEASES")
    return entries[-1]


def parse_sparkle_appcast(raw: str) -> dict:
    root = ET.fromstring(raw)
    ns = {"sparkle": "http://www.andymatuschak.org/xml-namespaces/sparkle"}
    items = root.findall("./channel/item")
    if not items:
        raise ValueError("No <item> entries found in Sparkle appcast")
    item = items[0]
    ver_el = item.find("sparkle:shortVersionString", ns)
    title_el = item.find("title")
    version = (ver_el.text if ver_el is not None else None) or (
        title_el.text if title_el is not None else None
    )
    enclosure = item.find("enclosure")
    if enclosure is None or not version:
        raise ValueError("Missing version or <enclosure> in Sparkle appcast")
    url = enclosure.attrib["url"]
    length_raw = enclosure.attrib.get("length")
    out: dict = {"version": version.strip(), "url": url}
    if length_raw and length_raw.isdigit():
        out["size"] = int(length_raw)
    return out


def _dsh_tako_artifact(
    url: str,
    version: str,
    install_type: str,
    fallback_url: str | None = None,
    sha256: str | None = None,
    size: int | None = None,
) -> dict:
    art: dict = {
        "url": url,
        "version": version,
        "installType": install_type,
    }
    if fallback_url and fallback_url != url:
        art["fallbackUrl"] = fallback_url
    if sha256:
        art["sha256"] = sha256.lower()
    if isinstance(size, int) and size > 0:
        art["size"] = size
    if install_type == "dmg":
        art["appBundleName"] = "DSH Tako.app"
    elif install_type == "exe":
        art["silentArgs"] = ["/S"]
    return art


def parse_dsh_tako_latest_json(raw: str) -> dict:
    data = json.loads(raw)
    version = str(data.get("version") or data.get("tag") or "").strip().lstrip("vV")
    assets = data.get("assets") or []
    if not version or not isinstance(assets, list):
        raise ValueError("Invalid DSH Tako latest.json: missing version or assets")

    platforms: dict[str, dict] = {}
    for asset in assets:
        if not isinstance(asset, dict):
            continue
        name = str(asset.get("name") or "")
        role = str(asset.get("role") or "")
        oss_url = str(asset.get("ossUrl") or "").strip()
        gh_url = str(asset.get("githubUrl") or "").strip() or None
        primary_url = oss_url or (gh_url or "")
        if not primary_url:
            continue
        sha256 = asset.get("sha256")
        if isinstance(sha256, str):
            sha256 = sha256.strip() or None
        else:
            sha256 = None
        size = asset.get("size") if isinstance(asset.get("size"), int) else None

        if role == "mac-arm64" or name.endswith("-mac-arm64.dmg"):
            platforms["macos-arm64"] = _dsh_tako_artifact(
                primary_url, version, "dmg", fallback_url=gh_url if oss_url else None, sha256=sha256, size=size
            )
        elif role == "win-x64" or name.endswith("-win-x64.exe"):
            platforms["windows-x64"] = _dsh_tako_artifact(
                primary_url, version, "exe", fallback_url=gh_url if oss_url else None, sha256=sha256, size=size
            )

    if not platforms:
        raise ValueError("Invalid DSH Tako latest.json: no supported platform assets found")
    return {"version": version, "platforms": platforms}


def parse_dsh_tako_github_release(raw: str) -> dict:
    data = json.loads(raw)
    version = str(data.get("tag_name") or "").strip().lstrip("vV")
    assets = data.get("assets") or []
    if not version or not isinstance(assets, list):
        raise ValueError("Invalid DSH Tako GitHub release payload")

    platforms: dict[str, dict] = {}
    for asset in assets:
        if not isinstance(asset, dict):
            continue
        name = str(asset.get("name") or "")
        url = str(asset.get("browser_download_url") or "").strip()
        if not url:
            continue
        size = asset.get("size") if isinstance(asset.get("size"), int) else None
        digest = asset.get("digest")
        sha256 = None
        if isinstance(digest, str) and digest.startswith("sha256:"):
            sha256 = digest.split(":", 1)[1].strip() or None

        if name.endswith("-mac-arm64.dmg"):
            platforms["macos-arm64"] = _dsh_tako_artifact(
                url, version, "dmg", sha256=sha256, size=size
            )
        elif name.endswith("-win-x64.exe"):
            platforms["windows-x64"] = _dsh_tako_artifact(
                url, version, "exe", sha256=sha256, size=size
            )

    if not platforms:
        raise ValueError("Invalid DSH Tako GitHub release: no supported platform assets found")
    return {"version": version, "platforms": platforms}


def apply_mirror(artifact: dict, mirror_base: str | None, app_id: str, plat: str, ver: str) -> dict:
    if not mirror_base:
        return artifact
    official_url = artifact["url"]
    basename = os.path.basename(urllib.parse.urlparse(official_url).path)
    mirror_url = f"{mirror_base.rstrip('/')}/{app_id}/{plat}/{ver}/{basename}"
    updated = dict(artifact)
    updated["url"] = mirror_url
    updated["fallbackUrl"] = official_url
    return updated


def build_manifest(mirror_base: str | None = None) -> dict:
    # 0. DSH Tako Desktop (OSS primary + GitHub fallback from dsh-oss-sync contract)
    try:
        dsh_tako = parse_dsh_tako_latest_json(http_get_text(DSH_TAKO_OSS_LATEST_URL))
    except Exception:
        dsh_tako = parse_dsh_tako_github_release(http_get_text(DSH_TAKO_GH_LATEST_API))

    # 1. Google Antigravity
    agy_platforms: dict[str, dict] = {}
    agy_top_ver = "0.0.0"
    for plat, (feed_url, install_type, silent_args) in ANTIGRAVITY_FEEDS.items():
        parsed = parse_electron_builder_yml(http_get_text(feed_url))
        ver = parsed["version"]
        agy_top_ver = ver
        art: dict = {
            "url": parsed["url"],
            "version": ver,
            "installType": install_type,
        }
        if "sha512" in parsed:
            art["sha512"] = parsed["sha512"]
        if "size" in parsed:
            art["size"] = parsed["size"]
        if install_type == "zip":
            art["appBundleName"] = "Antigravity.app"
        if silent_args:
            art["silentArgs"] = silent_args
        agy_platforms[plat] = apply_mirror(art, mirror_base, "antigravity", plat, ver)

    # 2. Claude Desktop
    claude_mac = parse_claude_mac_releases(http_get_text(CLAUDE_MAC_JSON_URL))
    claude_mac_ver = claude_mac["version"]
    mac_head = http_head(claude_mac["url"])
    claude_mac_art: dict = {
        "url": claude_mac["url"],
        "version": claude_mac_ver,
        "installType": "zip",
        "appBundleName": "Claude.app",
    }
    if mac_head.get("content-length", "").isdigit():
        claude_mac_art["size"] = int(mac_head["content-length"])

    claude_win_ver, _ = parse_claude_win_releases(http_get_text(CLAUDE_WIN_RELEASES_URL))
    claude_platforms: dict[str, dict] = {
        "macos-universal": apply_mirror(
            claude_mac_art, mirror_base, "claude", "macos-universal", claude_mac_ver
        )
    }

    commit = claude_mac["commitByVersion"].get(claude_win_ver) or next(
        iter(claude_mac["commitByVersion"].values()), None
    )
    if commit:
        win_exe_url = f"https://downloads.claude.ai/releases/win32/x64/{claude_win_ver}/Claude-{commit}.exe"
        try:
            win_head = http_head(win_exe_url)
            win_art: dict = {
                "url": win_exe_url,
                "version": claude_win_ver,
                "installType": "exe",
                "silentArgs": ["--silent"],
            }
            if win_head.get("content-length", "").isdigit():
                win_art["size"] = int(win_head["content-length"])
            claude_platforms["windows-x64"] = apply_mirror(
                win_art, mirror_base, "claude", "windows-x64", claude_win_ver
            )
        except urllib.error.HTTPError:
            pass

    # 3. Codex Desktop
    codex_arm = parse_sparkle_appcast(http_get_text(CODEX_MAC_ARM_APPCAST))
    codex_x64 = parse_sparkle_appcast(http_get_text(CODEX_MAC_X64_APPCAST))
    codex_win_head = http_head(CODEX_WIN_MSIX_URL)
    codex_win_ver = codex_win_head.get("x-ms-meta-package_version") or codex_arm["version"]

    codex_arm_art = {
        "url": codex_arm["url"],
        "version": codex_arm["version"],
        "installType": "zip",
        "appBundleName": "ChatGPT.app",
    }
    if "size" in codex_arm:
        codex_arm_art["size"] = codex_arm["size"]

    codex_x64_art = {
        "url": codex_x64["url"],
        "version": codex_x64["version"],
        "installType": "zip",
        "appBundleName": "ChatGPT.app",
    }
    if "size" in codex_x64:
        codex_x64_art["size"] = codex_x64["size"]

    codex_win_art: dict = {
        "url": CODEX_WIN_MSIX_URL,
        "version": codex_win_ver,
        "installType": "msix",
        "packageFamilyName": "OpenAI.Codex_2p2nqsd0c76g0",
    }
    if codex_win_head.get("content-length", "").isdigit():
        codex_win_art["size"] = int(codex_win_head["content-length"])

    return {
        "schemaVersion": 1,
        "generatedAt": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "apps": {
            "dsh-tako": {
                "displayName": "DSH Tako",
                "version": dsh_tako["version"],
                "platforms": dsh_tako["platforms"],
            },
            "codex": {
                "displayName": "Codex Desktop",
                "version": codex_arm["version"],
                "platforms": {
                    "macos-arm64": apply_mirror(
                        codex_arm_art, mirror_base, "codex", "macos-arm64", codex_arm["version"]
                    ),
                    "macos-x64": apply_mirror(
                        codex_x64_art, mirror_base, "codex", "macos-x64", codex_x64["version"]
                    ),
                    "windows-x64": apply_mirror(
                        codex_win_art, mirror_base, "codex", "windows-x64", codex_win_ver
                    ),
                },
            },
            "claude": {
                "displayName": "Claude Desktop",
                "version": claude_mac_ver,
                "platforms": claude_platforms,
            },
            "antigravity": {
                "displayName": "Google Antigravity",
                "version": agy_top_ver,
                "platforms": agy_platforms,
            },
        },
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Sync tako-switch app manifest")
    parser.add_argument("--out", default="manifest/manifest.json", help="Output JSON path")
    parser.add_argument(
        "--mirror-base",
        default=os.environ.get("TAKO_MIRROR_BASE") or None,
        help="Optional China CDN/OSS mirror base URL",
    )
    args = parser.parse_args(argv)

    manifest = build_manifest(mirror_base=args.mirror_base)
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    # Preserve generatedAt when apps dictionary has not changed to avoid empty cron commits.
    if out_path.exists():
        try:
            existing = json.loads(out_path.read_text(encoding="utf-8"))
            if existing.get("apps") == manifest["apps"] and existing.get("generatedAt"):
                manifest["generatedAt"] = existing["generatedAt"]
        except Exception:
            pass

    out_path.write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(f"Wrote {out_path} ({len(manifest['apps'])} apps)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
