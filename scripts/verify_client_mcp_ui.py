#!/usr/bin/env python3
"""Blackbox UI, static asset, CSP, and anti-tampering verifier for Tako Switch."""

from __future__ import annotations

import argparse
import json
import re
import struct
import sys
from pathlib import Path
from typing import Dict, List, Tuple

PREF_CHINESE_SUBTITLE = "Codex / Claude 界面与中文回复 · 反重力系统菜单与中文交互"

REQUIRED_MCP_TOOL_SPECS = [
    ("tako_generate_image", "生图"),
    ("tako_generate_video", "生视频"),
    ("tako_generate_audio", "声音"),
    ("tako_web_search", "联网"),
    ("tako_consult", "外脑"),
]

REQUIRED_TESTIDS = [
    "open-mcp-demo",
    "mcp-demo-tab-image",
    "mcp-demo-tab-video",
    "mcp-demo-tab-audio",
    "mcp-demo-tab-search",
    "mcp-demo-tab-consult",
    "mcp-demo-play-audio",
    "consult-preset-gpt-6.2-sol",
    "mcp-demo-copy-prompt",
]

REQUIRED_DEMO_ASSETS = [
    "ceramic-mug.webp",
    "ceramic-mug-edited.webp",
    "voice-sample.wav",
]

FORBIDDEN_ASAR_PATTERNS = [
    (re.compile(r"app\.asar", re.IGNORECASE), "reference to third-party app.asar"),
    (re.compile(r"@electron/asar", re.IGNORECASE), "electron/asar package usage"),
    (re.compile(r"\basar\s*\.\s*(extract|createPackage|extractAll)\b"), "asar unpack/repack call"),
    (re.compile(r"\bnpx\s+asar\b", re.IGNORECASE), "npx asar CLI invocation"),
]


def parse_csp(csp_raw: str) -> Dict[str, List[str]]:
    directives: Dict[str, List[str]] = {}
    for part in csp_raw.split(";"):
        tokens = part.strip().split()
        if not tokens:
            continue
        directives[tokens[0].lower()] = tokens[1:]
    return directives


def validate_webp(data: bytes) -> Tuple[bool, str]:
    if len(data) < 16:
        return False, f"file too small ({len(data)} bytes)"
    if data[0:4] != b"RIFF" or data[8:12] != b"WEBP":
        return False, f"invalid WEBP magic header: {data[:12]!r}"
    riff_size = struct.unpack_from("<I", data, 4)[0]
    if riff_size + 8 > len(data):
        return False, f"truncated WEBP RIFF payload (declared {riff_size + 8}, actual {len(data)})"
    chunk_fourcc = data[12:16]
    if chunk_fourcc not in (b"VP8 ", b"VP8L", b"VP8X"):
        return False, f"unexpected WEBP primary chunk {chunk_fourcc!r}"
    return True, f"valid WebP ({len(data)} bytes, chunk={chunk_fourcc.decode('ascii', 'replace')})"


def validate_wav(data: bytes) -> Tuple[bool, str]:
    if len(data) < 44:
        return False, f"file too small for WAV header ({len(data)} bytes)"
    if data[0:4] != b"RIFF" or data[8:12] != b"WAVE":
        return False, f"invalid WAV RIFF/WAVE header: {data[:12]!r}"
    riff_size = struct.unpack_from("<I", data, 4)[0]
    if riff_size + 8 > len(data):
        return False, f"truncated WAV RIFF payload (declared {riff_size + 8}, actual {len(data)})"

    pos = 12
    found_fmt = False
    data_chunk_size = None
    while pos + 8 <= len(data):
        chunk_id = data[pos : pos + 4]
        chunk_size = struct.unpack_from("<I", data, pos + 4)[0]
        if chunk_id == b"fmt ":
            if chunk_size < 16:
                return False, f"invalid fmt chunk size {chunk_size}"
            found_fmt = True
        elif chunk_id == b"data":
            data_chunk_size = chunk_size
        pos += 8 + chunk_size + (chunk_size % 2)

    if not found_fmt:
        return False, "missing WAV 'fmt ' chunk"
    if data_chunk_size is None or data_chunk_size == 0:
        return False, f"missing or empty WAV 'data' chunk ({data_chunk_size})"
    return True, f"valid WAV ({len(data)} bytes, pcm_data={data_chunk_size} bytes)"


def has_testid(src_files: Dict[Path, str], testid: str) -> bool:
    literal_patterns = [
        f'data-testid="{testid}"',
        f"data-testid='{testid}'",
        f'data-testid={{"{testid}"}}',
        f"data-testid={{'{testid}'}}",
    ]
    for content in src_files.values():
        if any(pat in content for pat in literal_patterns):
            return True

    # Also support idiomatic React template-literal mapping for tab / preset lists.
    if testid.startswith("mcp-demo-tab-"):
        suffix = testid[len("mcp-demo-tab-") :]
        tpl_re = re.compile(r"data-testid=\{\s*`mcp-demo-tab-\$\{[^}]+\}`\s*\}")
        for content in src_files.values():
            if tpl_re.search(content) and re.search(rf"""['"]{re.escape(suffix)}['"]""", content):
                return True

    if testid.startswith("consult-preset-"):
        suffix = testid[len("consult-preset-") :]
        tpl_re = re.compile(r"data-testid=\{\s*`consult-preset-\$\{[^}]+\}`\s*\}")
        for content in src_files.values():
            if tpl_re.search(content) and re.search(rf"""['"]{re.escape(suffix)}['"]""", content):
                return True

    return False


def collect_source_texts(base_dir: Path, extensions: Tuple[str, ...]) -> Dict[Path, str]:
    results: Dict[Path, str] = {}
    if not base_dir.exists():
        return results
    for path in sorted(base_dir.rglob("*")):
        if not path.is_file():
            continue
        if any(part in ("node_modules", "dist", "target", ".git") for part in path.parts):
            continue
        if path.suffix.lower() in extensions:
            results[path] = path.read_text(encoding="utf-8", errors="replace")
    return results


def run_checks(repo_root: Path) -> List[Tuple[str, bool, str]]:
    checks: List[Tuple[str, bool, str]] = []

    # 1. Static demo assets
    demo_dir = repo_root / "public" / "mcp-demo"
    webp_blobs: Dict[str, bytes] = {}
    for asset_name in REQUIRED_DEMO_ASSETS:
        asset_path = demo_dir / asset_name
        if not asset_path.is_file():
            checks.append((f"asset:{asset_name}", False, f"missing file {asset_path}"))
            continue
        data = asset_path.read_bytes()
        if asset_name.endswith(".webp"):
            ok, msg = validate_webp(data)
            webp_blobs[asset_name] = data
            checks.append((f"asset:{asset_name}", ok, msg))
        else:
            ok, msg = validate_wav(data)
            checks.append((f"asset:{asset_name}", ok, msg))

    if "ceramic-mug.webp" in webp_blobs and "ceramic-mug-edited.webp" in webp_blobs:
        distinct = webp_blobs["ceramic-mug.webp"] != webp_blobs["ceramic-mug-edited.webp"]
        checks.append(
            (
                "asset:webp-distinct",
                distinct,
                "ceramic-mug.webp and ceramic-mug-edited.webp are distinct"
                if distinct
                else "ceramic-mug.webp and ceramic-mug-edited.webp have identical bytes",
            )
        )

    # 2. Frontend source contracts
    src_files = collect_source_texts(repo_root / "src", (".ts", ".tsx", ".js", ".jsx", ".css"))
    all_src = "\n".join(src_files.values())

    has_subtitle = PREF_CHINESE_SUBTITLE in all_src
    checks.append(
        (
            "ui:pref-chinese-subtitle",
            has_subtitle,
            "found updated Chinese UI subtitle"
            if has_subtitle
            else f"missing exact subtitle: {PREF_CHINESE_SUBTITLE}",
        )
    )

    has_super_mcp_heading = "Tako 超级 MCP" in all_src
    checks.append(
        (
            "ui:super-mcp-heading",
            has_super_mcp_heading,
            "found 'Tako 超级 MCP' heading" if has_super_mcp_heading else "missing 'Tako 超级 MCP' heading",
        )
    )

    for tool_id, capability_kw in REQUIRED_MCP_TOOL_SPECS:
        ok = (tool_id in all_src) and (capability_kw in all_src)
        checks.append(
            (
                f"ui:mcp-tool-copy:{tool_id}",
                ok,
                f"found tool '{tool_id}' and capability keyword '{capability_kw}'"
                if ok
                else f"missing '{tool_id}' or capability keyword '{capability_kw}' in src/",
            )
        )

    for testid in REQUIRED_TESTIDS:
        ok = has_testid(src_files, testid)
        checks.append(
            (
                f"ui:testid:{testid}",
                ok,
                f"found data-testid='{testid}'" if ok else f"missing data-testid='{testid}'",
            )
        )

    for asset_name in REQUIRED_DEMO_ASSETS:
        ok = asset_name in all_src
        checks.append(
            (
                f"ui:asset-ref:{asset_name}",
                ok,
                f"referenced {asset_name} in src/" if ok else f"{asset_name} not referenced in src/",
            )
        )

    # 3. CSP configuration check
    conf_path = repo_root / "src-tauri" / "tauri.conf.json"
    if not conf_path.is_file():
        checks.append(("csp:tauri.conf.json", False, f"missing {conf_path}"))
    else:
        conf = json.loads(conf_path.read_text(encoding="utf-8"))
        csp_raw = conf.get("app", {}).get("security", {}).get("csp")
        if not isinstance(csp_raw, str) or not csp_raw.strip():
            checks.append(("csp:present", False, "app.security.csp missing or empty"))
        else:
            directives = parse_csp(csp_raw)
            default_src = directives.get("default-src", [])
            img_src = directives.get("img-src", [])
            media_src = directives.get("media-src", default_src)
            script_src = directives.get("script-src", default_src)

            no_wildcard = "*" not in default_src and "*" not in script_src and "'unsafe-eval'" not in script_src
            checks.append(
                (
                    "csp:no-wildcard-or-eval",
                    no_wildcard,
                    "default-src/script-src free of '*' and 'unsafe-eval'"
                    if no_wildcard
                    else f"unsafe CSP directives: default-src={default_src}, script-src={script_src}",
                )
            )
            img_ok = "'self'" in img_src and "data:" in img_src
            checks.append(
                (
                    "csp:img-src",
                    img_ok,
                    f"img-src allows 'self' and data: ({img_src})"
                    if img_ok
                    else f"img-src must include 'self' and data:, got {img_src}",
                )
            )

            uses_blob_or_data_media = bool(
                re.search(r"<audio[^>]+src=\{?[^>}]*(blob:|data:)", all_src)
                or re.search(r"createObjectURL\s*\(", all_src)
            )
            if uses_blob_or_data_media:
                media_ok = "'self'" in media_src and ("blob:" in media_src or "data:" in media_src)
                media_msg = (
                    f"media-src allows dynamic audio source ({media_src})"
                    if media_ok
                    else f"frontend uses blob:/data: media but media-src is {media_src}"
                )
            else:
                media_ok = "'self'" in media_src
                media_msg = (
                    f"media-src (or default-src) allows 'self' ({media_src})"
                    if media_ok
                    else f"media-src must allow 'self', got {media_src}"
                )
            checks.append(("csp:media-src", media_ok, media_msg))

    # 4. Forbidden app.asar tampering scan
    self_path = Path(__file__).resolve()
    scan_dirs = [
        (repo_root / "src-tauri" / "src", (".rs",)),
        (repo_root / "src", (".ts", ".tsx", ".js", ".jsx")),
        (repo_root / "scripts", (".py", ".sh", ".js", ".ts")),
    ]
    asar_hits: List[str] = []
    for base, exts in scan_dirs:
        for fpath, content in collect_source_texts(base, exts).items():
            if fpath.resolve() == self_path:
                continue
            for idx, line in enumerate(content.splitlines(), start=1):
                for pat, desc in FORBIDDEN_ASAR_PATTERNS:
                    if pat.search(line):
                        rel = fpath.relative_to(repo_root)
                        asar_hits.append(f"{rel}:{idx} ({desc}): {line.strip()}")
    checks.append(
        (
            "security:no-app-asar-tampering",
            len(asar_hits) == 0,
            "0 forbidden app.asar tampering matches"
            if not asar_hits
            else "found forbidden matches: " + "; ".join(asar_hits),
        )
    )

    return checks


def run_self_tests() -> None:
    # Verify WebP validator
    good_webp = b"RIFF" + struct.pack("<I", 12) + b"WEBP" + b"VP8 " + b"\x00\x00\x00\x00"
    assert validate_webp(good_webp)[0]
    assert not validate_webp(b"RIFF\x04\x00\x00\x00PNG ")[0]

    # Verify WAV validator
    good_wav = (
        b"RIFF"
        + struct.pack("<I", 40)
        + b"WAVE"
        + b"fmt "
        + struct.pack("<I", 16)
        + b"\x00" * 16
        + b"data"
        + struct.pack("<I", 4)
        + b"\x00\x01\x02\x03"
    )
    assert validate_wav(good_wav)[0]
    empty_wav = (
        b"RIFF"
        + struct.pack("<I", 36)
        + b"WAVE"
        + b"fmt "
        + struct.pack("<I", 16)
        + b"\x00" * 16
        + b"data"
        + struct.pack("<I", 0)
    )
    assert not validate_wav(empty_wav)[0]

    # Verify template-literal testid matching
    mock_files = {
        Path("Demo.tsx"): """
const TABS = ["image", "video", "audio", "search", "consult"];
const PRESETS = ["gpt-6.2-sol", "swe-2"];
export const Demo = () => (
  <div>
    <button data-testid="open-mcp-demo" />
    {TABS.map((t) => <button data-testid={`mcp-demo-tab-${t}`} />)}
    {PRESETS.map((p) => <button data-testid={`consult-preset-${p}`} />)}
  </div>
);
"""
    }
    assert has_testid(mock_files, "open-mcp-demo")
    assert has_testid(mock_files, "mcp-demo-tab-consult")
    assert has_testid(mock_files, "consult-preset-gpt-6.2-sol")
    assert not has_testid(mock_files, "mcp-demo-play-audio")
    print("Self-tests passed.")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="Path to repository root (defaults to parent of scripts/)",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="Run internal validator unit tests and exit",
    )
    args = parser.parse_args()

    if args.self_test:
        run_self_tests()
        return 0

    checks = run_checks(args.repo_root.resolve())
    passed = sum(1 for _, ok, _ in checks if ok)
    total = len(checks)
    for name, ok, msg in checks:
        status = "PASS" if ok else "FAIL"
        print(f"[{status}] {name}: {msg}")

    print(f"\nSummary: {passed}/{total} checks passed.")
    return 0 if passed == total else 1


if __name__ == "__main__":
    sys.exit(main())
