#!/usr/bin/env python3
"""Blackbox UI, static asset, CSP, and anti-tampering verifier for Tako Switch V2."""

from __future__ import annotations

import argparse
import json
import re
import struct
import sys
import tempfile
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

INTERNAL_MCP_TOOL_NAMES = [
    "tako_generate_image",
    "tako_generate_video",
    "tako_generate_audio",
    "tako_web_search",
    "tako_consult",
    "tako_account_status",
]

INTERNAL_MCP_TOOL_RE = re.compile(
    r"\b(" + "|".join(re.escape(name) for name in INTERNAL_MCP_TOOL_NAMES) + r")\b"
)

REQUIRED_TESTIDS = [
    "open-mcp-demo",
    "mcp-inline-theater",
    "mcp-demo-tab-image",
    "mcp-demo-tab-video",
    "mcp-demo-tab-audio",
    "mcp-demo-tab-search",
    "mcp-demo-tab-consult",
    "mcp-mode-showcase",
    "mcp-mode-live",
    "mcp-image-slider",
    "mcp-demo-video",
    "mcp-video-compare-frame",
    "mcp-demo-play-audio",
    "mcp-audio-subtitles",
    "mcp-demo-copy-prompt",
    "consult-preset-gpt-6.2-sol",
    "mcp-live-run-btn",
    "main-chinese-ui-toggle",
    "open-model-square-btn",
    "agy-ui-patch-btn",
    "agy-ui-restore-btn",
    "claude-ui-patch-btn",
    "ui-patch-permission-modal",
    "plaza-price-mode-detail",
    "plaza-price-mode-avg",
    "plaza-currency-cny",
    "plaza-currency-usd",
    "plaza-unit-m",
    "plaza-unit-k",
    "plaza-quick-all",
    "plaza-quick-promo",
    "plaza-quick-cheap",
    "plaza-quick-code",
    "plaza-quick-long",
    "plaza-quick-image",
    "mcp-toggle-webpage",
]

REQUIRED_EXT_HUB_TESTIDS = [
    "open-ext-hub-mcp",
    "open-ext-hub-skills",
    "open-ext-hub-plugins",
    "open-ext-hub-prompts",
    "ext-hub-tab-mcp",
    "ext-hub-tab-skills",
    "ext-hub-tab-plugins",
    "ext-hub-tab-prompts",
    "ext-mcp-tako-notice",
    "ext-mcp-goto-super",
    "ext-mcp-search",
    "ext-mcp-add-btn",
    "ext-prompts-new-btn",
    "ext-prompts-platform-claude",
    "ext-prompts-platform-codex",
    "ext-prompts-platform-antigravity",
    "ext-prompts-sync-other-btn",
    "ext-prompts-save-btn",
]

AGY_ZH_DICT_REL = Path("src-tauri/src/adapters/antigravity_zh_dict.json")
MIN_AGY_ZH_DICT_ENTRIES = 1200
REQUIRED_AGY_ZH_DICT_KEYS = (
    "New Conversation",
    "Scheduled Tasks",
    "Skills & Customizations",
    "Global Settings",
    "MCP Servers",
)
CJK_CHAR_RE = re.compile(r"[\u3400-\u4dbf\u4e00-\u9fff]")

REQUIRED_WEBP_ASSETS = [
    "ceramic-mug.webp",
    "ceramic-mug-edited.webp",
    "floating-garden.webp",
    "forest-cabin.webp",
    "multi-reference-1.webp",
    "multi-reference-2.webp",
    "multi-reference-comparison.webp",
]

REQUIRED_MP4_ASSET = "floating-garden-hq.mp4"
MIN_MP4_BYTES = 100_000

REQUIRED_MP3_ASSETS = [
    "voice-bingtang-warm.mp3",
    "voice-bingtang-lively.mp3",
    "voice-bingtang-radio.mp3",
]
MIN_MP3_BYTES = 20_000

FORBIDDEN_LEGACY_ASSETS = [
    "voice-sample.wav",
]

FORBIDDEN_LEGACY_FILES = [
    Path("src/components/McpShowcaseModal.tsx"),
]

CONTROLLED_AGY_I18N_REL = Path("src-tauri/src/adapters/antigravity_i18n.rs")
CONTROLLED_CLAUDE_I18N_REL = Path("src-tauri/src/adapters/claude_i18n.rs")
CONTROLLED_ASAR_PATCH_REL = Path("src-tauri/src/adapters/asar_patch.rs")
CONTROLLED_ASAR_RELS = (
    CONTROLLED_AGY_I18N_REL,
    CONTROLLED_CLAUDE_I18N_REL,
    CONTROLLED_ASAR_PATCH_REL,
)

REQUIRED_AGY_I18N_SYMBOLS = [
    "app.asar.orig",
    "original_asar_sha256",
    "Sha256",
    "verify_clean_backup_asar",
    "restore_antigravity_ui_for_paths",
]

FORBIDDEN_EXTERNAL_ASAR_PATTERNS = [
    (re.compile(r"@electron/asar", re.IGNORECASE), "electron/asar package usage"),
    (re.compile(r"\basar\s*\.\s*(extract|createPackage|extractAll)\b"), "asar unpack/repack call"),
    (re.compile(r"\bnpx\s+asar\b", re.IGNORECASE), "npx asar CLI invocation"),
]

FORBIDDEN_ASAR_PATTERNS = [
    (re.compile(r"app\.asar", re.IGNORECASE), "unauthorized reference to third-party app.asar"),
    *FORBIDDEN_EXTERNAL_ASAR_PATTERNS,
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
    if riff_size < 12 or riff_size + 8 > len(data):
        return False, f"invalid or truncated WEBP RIFF payload (declared {riff_size + 8}, actual {len(data)})"
    chunk_fourcc = data[12:16]
    if chunk_fourcc not in (b"VP8 ", b"VP8L", b"VP8X"):
        return False, f"unexpected WEBP primary chunk {chunk_fourcc!r}"
    return True, f"valid WebP ({len(data)} bytes, chunk={chunk_fourcc.decode('ascii', 'replace')})"


def validate_mp4(data: bytes, min_bytes: int = MIN_MP4_BYTES) -> Tuple[bool, str]:
    if len(data) <= min_bytes:
        return False, f"MP4 size {len(data)} bytes is not > {min_bytes} bytes"
    if len(data) < 16 or data[4:8] != b"ftyp":
        return False, f"missing MP4 'ftyp' header box: {data[:16]!r}"

    pos = 0
    seen_boxes: List[str] = []
    while pos + 8 <= len(data):
        box_size = struct.unpack_from(">I", data, pos)[0]
        box_type = data[pos + 4 : pos + 8]
        header_len = 8
        if box_size == 1:
            if pos + 16 > len(data):
                return False, f"truncated 64-bit MP4 box at offset {pos}"
            box_size = struct.unpack_from(">Q", data, pos + 8)[0]
            header_len = 16
        elif box_size == 0:
            box_size = len(data) - pos

        if box_size < header_len or pos + box_size > len(data):
            return False, f"corrupted MP4 box {box_type!r} at offset {pos} (size={box_size}, total={len(data)})"

        seen_boxes.append(box_type.decode("ascii", "replace"))
        pos += box_size

    if "ftyp" not in seen_boxes:
        return False, "missing top-level ftyp box"
    if "moov" not in seen_boxes:
        return False, f"missing MP4 'moov' box (boxes={seen_boxes})"
    if b"hvc1" in data:
        return False, "found Apple-only bare 'hvc1' codec atom; must use cross-platform H.264 'avc1'"
    if b"avc1" not in data:
        return False, "missing H.264 'avc1' codec atom in MP4 container"

    return True, f"valid H.264 MP4 ({len(data)} bytes, boxes={seen_boxes}, codec=avc1)"


def is_valid_mpeg_frame_header(header: bytes) -> bool:
    if len(header) < 4:
        return False
    b0, b1, b2, _b3 = header[0], header[1], header[2], header[3]
    if b0 != 0xFF or (b1 & 0xE0) != 0xE0:
        return False
    version_bits = (b1 >> 3) & 0x03
    layer_bits = (b1 >> 1) & 0x03
    bitrate_idx = (b2 >> 4) & 0x0F
    sample_rate_idx = (b2 >> 2) & 0x03
    if version_bits == 0x01 or layer_bits == 0x00:
        return False
    if bitrate_idx in (0x00, 0x0F) or sample_rate_idx == 0x03:
        return False
    return True


def validate_mp3(data: bytes, min_bytes: int = MIN_MP3_BYTES) -> Tuple[bool, str]:
    if len(data) <= min_bytes:
        return False, f"MP3 size {len(data)} bytes is not > {min_bytes} bytes"

    if data[:3] == b"ID3":
        if len(data) < 14:
            return False, f"truncated ID3v2 header ({len(data)} bytes)"
        if data[3] == 0xFF or data[4] == 0xFF or any(b >= 0x80 for b in data[6:10]):
            return False, f"invalid ID3v2 version/synchsafe header: {data[:10]!r}"
        tag_size = (
            ((data[6] & 0x7F) << 21)
            | ((data[7] & 0x7F) << 14)
            | ((data[8] & 0x7F) << 7)
            | (data[9] & 0x7F)
        )
        audio_start = 10 + tag_size
        if audio_start + 4 > len(data):
            return False, f"ID3v2 tag extends past EOF (audio_start={audio_start}, len={len(data)})"
        # Scan a small window after ID3 tag in case of ID3 padding bytes
        scan_end = min(len(data) - 4, audio_start + 2048)
        found_sync = any(
            is_valid_mpeg_frame_header(data[i : i + 4]) for i in range(audio_start, scan_end + 1)
        )
        if not found_sync:
            return False, f"ID3v2 tag present (size={tag_size}) but no MPEG frame sync found after tag"
        return True, f"valid MP3 ({len(data)} bytes, container=ID3v2.{data[3]}+MPEG)"

    if is_valid_mpeg_frame_header(data[:4]):
        return True, f"valid MP3 ({len(data)} bytes, container=raw-MPEG-sync)"

    return False, f"invalid MP3 header (expected ID3 or MPEG frame sync, got {data[:8]!r})"


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

    # Support idiomatic React template-literal mapping for tab / preset lists.
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

    if testid.startswith("plaza-quick-"):
        suffix = testid[len("plaza-quick-") :]
        tpl_re = re.compile(r"data-testid=\{\s*`plaza-quick-\$\{[^}]+\}`\s*\}")
        for content in src_files.values():
            if tpl_re.search(content) and re.search(rf"""['"]{re.escape(suffix)}['"]""", content):
                return True

    return False


def has_mcp_tool_attr(src_files: Dict[Path, str], tool_id: str) -> bool:
    literal_patterns = [
        f'data-mcp-tool="{tool_id}"',
        f"data-mcp-tool='{tool_id}'",
        f'data-mcp-tool={{"{tool_id}"}}',
        f"data-mcp-tool={{'{tool_id}'}}",
    ]
    for content in src_files.values():
        if any(pat in content for pat in literal_patterns):
            return True

    dynamic_attr_re = re.compile(r"data-mcp-tool\s*=\s*\{[^}]+\}")
    tool_str_re = re.compile(rf"""['"]{re.escape(tool_id)}['"]""")
    has_dynamic_attr = any(dynamic_attr_re.search(c) for c in src_files.values())
    has_tool_const = any(tool_str_re.search(c) for c in src_files.values())
    return has_dynamic_attr and has_tool_const


def has_extension_testid(src_files: Dict[Path, str], testid: str) -> bool:
    if has_testid(src_files, testid):
        return True
    for prefix in ("open-ext-hub-", "ext-hub-tab-", "ext-prompts-platform-"):
        if testid.startswith(prefix):
            suffix = re.escape(testid[len(prefix) :])
            dynamic = re.compile(r"data-testid=\{\s*`" + re.escape(prefix) + r"\$\{[^}]+\}`\s*\}")
            return any(dynamic.search(content) for content in src_files.values()) and any(
                re.search(rf'''["']{suffix}["']''', content) for content in src_files.values()
            )
    all_source = "\n".join(src_files.values())
    return (
        bool(re.search(rf'''testId:\s*["']{re.escape(testid)}["']''', all_source))
        and "data-testid={actions.primary.testId}" in all_source
    ) or (
        bool(re.search(rf'''testId=["']{re.escape(testid)}["']''', all_source))
        and "data-testid={testId}" in all_source
    )


def extension_contract_checks(repo_root: Path, src_files: Dict[Path, str]) -> List[Tuple[str, bool, str]]:
    checks: List[Tuple[str, bool, str]] = []
    overview = src_files.get(repo_root / "src/components/OverviewPanel.tsx", "")
    mcp_center = src_files.get(repo_root / "src/components/McpCenterPanel.tsx", "")
    for testid in REQUIRED_EXT_HUB_TESTIDS:
        checks.append((f"extensions:testid:{testid}", has_extension_testid(src_files, testid), testid))
    clean_home = (
        "Tako 模式下自动注入，开关即时生效" in overview
        and "本项目特有王牌模组" not in overview
        and "open-multi-sync" not in overview
        and "grid-cols-2 min-[760px]:grid-cols-4" in overview
    )
    checks.append(("extensions:home-contract", clean_home, "home subtitle, independent entries and 720px 2-column grid"))
    checks.append(("extensions:super-mcp-isolated", "MultiPlatformSyncPanel" not in mcp_center and "activeSection" not in mcp_center, "super MCP no longer hosts the generic sync UI"))
    extension_files = {
        path: content for path, content in src_files.items()
        if path == repo_root / "src/components/ExtensionsHubPanel.tsx"
        or path.parent == repo_root / "src/components/extensions"
    }
    hits = [
        f"{path.relative_to(repo_root)}:{index}"
        for path, content in extension_files.items()
        for index, line in enumerate(content.splitlines(), 1)
        if re.search(r"#[0-9a-fA-F]{3,8}\b|tracking-\[", line)
    ]
    checks.append(("extensions:theme-tokens", bool(extension_files) and not hits, "CSS variables only" if not hits else ", ".join(hits)))
    return checks


def _strip_comments_preserve_lines(text: str) -> str:
    out: List[str] = []
    i = 0
    n = len(text)
    in_quote: str | None = None
    while i < n:
        ch = text[i]
        if in_quote is not None:
            out.append(ch)
            if ch == "\\" and i + 1 < n:
                out.append(text[i + 1])
                i += 2
                continue
            if ch == in_quote:
                in_quote = None
            i += 1
            continue

        if ch in ("'", '"', "`"):
            in_quote = ch
            out.append(ch)
            i += 1
            continue

        if ch == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                out.append(" ")
                i += 1
            continue

        if ch == "/" and i + 1 < n and text[i + 1] == "*":
            out.append("  ")
            i += 2
            while i < n and not (text[i] == "*" and i + 1 < n and text[i + 1] == "/"):
                out.append("\n" if text[i] == "\n" else " ")
                i += 1
            if i + 1 < n:
                out.append("  ")
                i += 2
            continue

        out.append(ch)
        i += 1
    return "".join(out)


def find_exposed_mcp_tool_names(
    src_files: Dict[Path, str], repo_root: Path | None = None
) -> List[str]:
    """Detect internal tako_* function names rendered in JSX text nodes.

    Allowed:
      - inside `data-mcp-tool="..."` or `data-mcp-tool={...}` attributes
      - inside TS/JS constant mappings / object literals outside JSX element text
    Forbidden:
      - raw JSX element inner text (e.g. `<code>tako_generate_image</code>`)
      - literal JSX string expression children (e.g. `<span>{"tako_consult"}</span>`)
    """
    hits: List[str] = []

    jsx_raw_text_re = re.compile(r"(?<![=])>([^<>{}]*)<")
    jsx_expr_literal_re = re.compile(
        r"(?<![=])>\s*\{\s*(['\"`])([^'\"`]*?)\1\s*\}\s*<"
    )
    quoted_span_re = re.compile(r"""(['"`])(?:\\.|(?!\1).)*?\1""")

    for fpath, raw_content in src_files.items():
        if fpath.suffix.lower() not in (".tsx", ".jsx"):
            continue
        cleaned = _strip_comments_preserve_lines(raw_content)
        rel = str(fpath.relative_to(repo_root)) if repo_root else str(fpath)

        for m in jsx_raw_text_re.finditer(cleaned):
            segment = m.group(1)
            # Ignore matches inside complete JS string literals (e.g. `a > b ? "tako_..." : ... <div`)
            segment_no_quotes = quoted_span_re.sub(" ", segment)
            tool_m = INTERNAL_MCP_TOOL_RE.search(segment_no_quotes)
            if tool_m:
                line_no = cleaned[: m.start()].count("\n") + 1
                hits.append(f"{rel}:{line_no} raw JSX text exposes '{tool_m.group(1)}'")

        for m in jsx_expr_literal_re.finditer(cleaned):
            segment = m.group(2)
            tool_m = INTERNAL_MCP_TOOL_RE.search(segment)
            if tool_m:
                line_no = cleaned[: m.start()].count("\n") + 1
                hits.append(
                    f"{rel}:{line_no} JSX string child exposes '{tool_m.group(1)}'"
                )

    return hits


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

    # 1. Static demo assets (WebP x7, H.264 MP4 x1, MP3 x3, legacy WAV removed)
    demo_dir = repo_root / "public" / "mcp-demo"
    webp_blobs: Dict[str, bytes] = {}
    for asset_name in REQUIRED_WEBP_ASSETS:
        asset_path = demo_dir / asset_name
        if not asset_path.is_file():
            checks.append((f"asset:{asset_name}", False, f"missing file {asset_path}"))
            continue
        data = asset_path.read_bytes()
        ok, msg = validate_webp(data)
        webp_blobs[asset_name] = data
        checks.append((f"asset:{asset_name}", ok, msg))

    if len(webp_blobs) == len(REQUIRED_WEBP_ASSETS):
        unique_count = len(set(webp_blobs.values()))
        all_distinct = unique_count == len(REQUIRED_WEBP_ASSETS)
        checks.append(
            (
                "asset:webp-all-distinct",
                all_distinct,
                f"all {len(REQUIRED_WEBP_ASSETS)} WebP assets have distinct byte payloads"
                if all_distinct
                else f"only {unique_count}/{len(REQUIRED_WEBP_ASSETS)} WebP assets are byte-distinct",
            )
        )

    mp4_path = demo_dir / REQUIRED_MP4_ASSET
    if not mp4_path.is_file():
        checks.append((f"asset:{REQUIRED_MP4_ASSET}", False, f"missing file {mp4_path}"))
    else:
        ok, msg = validate_mp4(mp4_path.read_bytes())
        checks.append((f"asset:{REQUIRED_MP4_ASSET}", ok, msg))

    mp3_blobs: Dict[str, bytes] = {}
    for asset_name in REQUIRED_MP3_ASSETS:
        asset_path = demo_dir / asset_name
        if not asset_path.is_file():
            checks.append((f"asset:{asset_name}", False, f"missing file {asset_path}"))
            continue
        data = asset_path.read_bytes()
        ok, msg = validate_mp3(data)
        mp3_blobs[asset_name] = data
        checks.append((f"asset:{asset_name}", ok, msg))

    if len(mp3_blobs) == len(REQUIRED_MP3_ASSETS):
        unique_mp3 = len(set(mp3_blobs.values()))
        mp3_distinct = unique_mp3 == len(REQUIRED_MP3_ASSETS)
        checks.append(
            (
                "asset:mp3-all-distinct",
                mp3_distinct,
                f"all {len(REQUIRED_MP3_ASSETS)} Bingtang MP3 clips are distinct"
                if mp3_distinct
                else f"only {unique_mp3}/{len(REQUIRED_MP3_ASSETS)} MP3 clips are byte-distinct",
            )
        )

    for legacy_asset in FORBIDDEN_LEGACY_ASSETS:
        legacy_path = demo_dir / legacy_asset
        removed = not legacy_path.exists()
        checks.append(
            (
                f"asset:removed:{legacy_asset}",
                removed,
                f"legacy mechanical sample {legacy_asset} removed"
                if removed
                else f"legacy sample {legacy_path} must be removed",
            )
        )

    # 2. Frontend source contracts
    src_files = collect_source_texts(repo_root / "src", (".ts", ".tsx", ".js", ".jsx", ".css"))
    all_src = "\n".join(src_files.values())

    for rel_legacy in FORBIDDEN_LEGACY_FILES:
        exists = (repo_root / rel_legacy).exists()
        checks.append(
            (
                f"ui:removed:{rel_legacy.as_posix()}",
                not exists,
                f"{rel_legacy.as_posix()} removed (replaced by inline theater)"
                if not exists
                else f"obsolete modal {rel_legacy.as_posix()} still exists",
            )
        )

    no_modal_ref = "McpShowcaseModal" not in all_src
    checks.append(
        (
            "ui:no-modal-reference",
            no_modal_ref,
            "no McpShowcaseModal references remain in src/"
            if no_modal_ref
            else "found stale McpShowcaseModal reference in src/",
        )
    )

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

    all_src_lower = all_src.lower()
    has_webpage_brand = "tako web page" in all_src_lower and "edgeone" in all_src_lower
    checks.append(
        (
            "ui:edgeone-webpage-entry",
            has_webpage_brand,
            "found Tako Web Page / EdgeOne entry in src/"
            if has_webpage_brand
            else "missing Tako Web Page / EdgeOne entry in src/",
        )
    )

    for tool_id, capability_kw in REQUIRED_MCP_TOOL_SPECS:
        has_attr = has_mcp_tool_attr(src_files, tool_id)
        has_cn = capability_kw in all_src
        ok = has_attr and has_cn
        checks.append(
            (
                f"ui:mcp-tool-binding:{tool_id}",
                ok,
                f"bound data-mcp-tool='{tool_id}' with Chinese label '{capability_kw}'"
                if ok
                else f"missing data-mcp-tool='{tool_id}' ({has_attr}) or Chinese label '{capability_kw}' ({has_cn})",
            )
        )

    exposed_hits = find_exposed_mcp_tool_names(src_files, repo_root=repo_root)
    checks.append(
        (
            "ui:no-exposed-internal-tool-names",
            len(exposed_hits) == 0,
            "0 internal tako_* function names exposed in JSX text nodes"
            if not exposed_hits
            else "; ".join(exposed_hits),
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

    checks.extend(extension_contract_checks(repo_root, src_files))

    all_required_assets = REQUIRED_WEBP_ASSETS + [REQUIRED_MP4_ASSET] + REQUIRED_MP3_ASSETS
    for asset_name in all_required_assets:
        ok = asset_name in all_src
        checks.append(
            (
                f"ui:asset-ref:{asset_name}",
                ok,
                f"referenced {asset_name} in src/" if ok else f"{asset_name} not referenced in src/",
            )
        )

    for legacy_asset in FORBIDDEN_LEGACY_ASSETS:
        no_ref = legacy_asset not in all_src
        checks.append(
            (
                f"ui:no-legacy-ref:{legacy_asset}",
                no_ref,
                f"no stale reference to {legacy_asset} in src/"
                if no_ref
                else f"found stale reference to {legacy_asset} in src/",
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

            no_wildcard = (
                "*" not in default_src
                and "*" not in script_src
                and "*" not in img_src
                and "*" not in media_src
                and "'unsafe-eval'" not in script_src
                and "'unsafe-eval'" not in default_src
            )
            checks.append(
                (
                    "csp:no-wildcard-or-eval",
                    no_wildcard,
                    "CSP directives free of '*' and 'unsafe-eval'"
                    if no_wildcard
                    else f"unsafe CSP directives: default={default_src}, script={script_src}, img={img_src}, media={media_src}",
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

            media_ok = "'self'" in media_src and "data:" in media_src
            checks.append(
                (
                    "csp:media-src",
                    media_ok,
                    f"media-src allows 'self' and data: ({media_src})"
                    if media_ok
                    else f"media-src must allow 'self' and data:, got {media_src}",
                )
            )

    # 4. Controlled Antigravity/Claude i18n backup & forbidden app.asar tampering scan
    self_path = Path(__file__).resolve()
    controlled_i18n_path = (repo_root / CONTROLLED_AGY_I18N_REL).resolve()
    controlled_asar_paths = {(repo_root / rel).resolve() for rel in CONTROLLED_ASAR_RELS}
    scan_dirs = [
        (repo_root / "src-tauri" / "src", (".rs",)),
        (repo_root / "src", (".ts", ".tsx", ".js", ".jsx")),
        (repo_root / "scripts", (".py", ".sh", ".js", ".ts")),
    ]
    asar_hits: List[str] = []
    for base, exts in scan_dirs:
        for fpath, content in collect_source_texts(base, exts).items():
            resolved = fpath.resolve()
            if resolved == self_path:
                continue
            patterns = (
                FORBIDDEN_EXTERNAL_ASAR_PATTERNS
                if resolved in controlled_asar_paths
                else FORBIDDEN_ASAR_PATTERNS
            )
            for idx, line in enumerate(content.splitlines(), start=1):
                for pat, desc in patterns:
                    if pat.search(line):
                        rel = fpath.relative_to(repo_root)
                        asar_hits.append(f"{rel}:{idx} ({desc}): {line.strip()}")
    checks.append(
        (
            "security:no-app-asar-tampering",
            len(asar_hits) == 0,
            "0 unauthorized app.asar or external asar CLI matches"
            if not asar_hits
            else "found forbidden matches: " + "; ".join(asar_hits),
        )
    )

    if not controlled_i18n_path.is_file():
        checks.append(
            (
                "security:controlled-agy-i18n-backup",
                False,
                f"missing {CONTROLLED_AGY_I18N_REL}",
            )
        )
    else:
        i18n_text = controlled_i18n_path.read_text(encoding="utf-8")
        missing_syms = [sym for sym in REQUIRED_AGY_I18N_SYMBOLS if sym not in i18n_text]
        checks.append(
            (
                "security:controlled-agy-i18n-backup",
                len(missing_syms) == 0,
                "antigravity_i18n.rs enforces .orig backup, SHA256 check, and restore"
                if not missing_syms
                else f"antigravity_i18n.rs missing required safety symbols: {missing_syms}",
            )
        )

    # 5. Standalone Antigravity Chinese dictionary check (v2.2.0)
    dict_ok, dict_msg = validate_antigravity_zh_dict(repo_root / AGY_ZH_DICT_REL)
    checks.append(("i18n:antigravity-zh-dict", dict_ok, dict_msg))

    return checks


def validate_antigravity_zh_dict(dict_path: Path) -> Tuple[bool, str]:
    if not dict_path.is_file():
        return False, f"missing {AGY_ZH_DICT_REL.as_posix()}"
    try:
        parsed = json.loads(dict_path.read_text(encoding="utf-8"))
    except Exception as exc:
        return False, f"invalid JSON in {AGY_ZH_DICT_REL.as_posix()}: {exc}"
    if not isinstance(parsed, dict):
        return False, f"{AGY_ZH_DICT_REL.as_posix()} root must be a JSON object"

    empty_keys = [k for k in parsed.keys() if not isinstance(k, str) or not k.strip()]
    if empty_keys:
        return False, f"found {len(empty_keys)} empty/non-string dictionary keys"

    bad_vals = [
        k for k, v in parsed.items() if not isinstance(v, str) or not v.strip()
    ]
    if bad_vals:
        return False, f"found {len(bad_vals)} empty/non-string dictionary values (e.g. {bad_vals[:3]})"

    total = len(parsed)
    if total < MIN_AGY_ZH_DICT_ENTRIES:
        return (
            False,
            f"expected >= {MIN_AGY_ZH_DICT_ENTRIES} entries in {AGY_ZH_DICT_REL.as_posix()}, got {total}",
        )

    missing_core = [k for k in REQUIRED_AGY_ZH_DICT_KEYS if k not in parsed]
    if missing_core:
        return False, f"missing required core dictionary keys: {missing_core}"

    non_zh_core = [
        k for k in REQUIRED_AGY_ZH_DICT_KEYS if not CJK_CHAR_RE.search(parsed[k])
    ]
    if non_zh_core:
        return False, f"core dictionary keys missing Chinese translation: {non_zh_core}"

    return True, f"valid Antigravity zh dict ({total} entries >= {MIN_AGY_ZH_DICT_ENTRIES})"


def _build_synthetic_mp4(size: int, include_avc1: bool = True, include_hvc1: bool = False) -> bytes:
    ftyp = struct.pack(">I", 20) + b"ftyp" + b"isom\x00\x00\x02\x00isom"
    moov_payload = b"trak" + (b"avc1" if include_avc1 else b"mp4v") + (b"hvc1" if include_hvc1 else b"mp4a")
    moov = struct.pack(">I", 8 + len(moov_payload)) + b"moov" + moov_payload
    mdat_len = max(16, size - len(ftyp) - len(moov))
    mdat = struct.pack(">I", mdat_len) + b"mdat" + (b"\x00" * (mdat_len - 8))
    return ftyp + moov + mdat


def _build_synthetic_mp3(size: int, with_id3: bool = True) -> bytes:
    frame = b"\xff\xfb\x90\x00" + (b"\x55" * 128)
    if with_id3:
        tag_body = b"TAKO"
        tag_size = len(tag_body)
        synchsafe = bytes(
            [
                (tag_size >> 21) & 0x7F,
                (tag_size >> 14) & 0x7F,
                (tag_size >> 7) & 0x7F,
                tag_size & 0x7F,
            ]
        )
        prefix = b"ID3\x03\x00\x00" + synchsafe + tag_body
    else:
        prefix = b""
    pad_len = max(0, size - len(prefix) - len(frame))
    return prefix + frame + (b"\x00" * pad_len)


def run_self_tests() -> None:
    # 1. WebP validator
    good_webp = b"RIFF" + struct.pack("<I", 12) + b"WEBP" + b"VP8 " + b"\x00\x00\x00\x00"
    assert validate_webp(good_webp)[0]
    assert not validate_webp(b"RIFF\x04\x00\x00\x00PNG ")[0]
    assert not validate_webp(b"RIFF\x99\x00\x00\x00WEBPVP8 \x00\x00\x00\x00")[0]

    # 2. MP4 validator (size > 100KB, ftyp, moov, avc1, no hvc1)
    good_mp4 = _build_synthetic_mp4(100_500, include_avc1=True, include_hvc1=False)
    assert validate_mp4(good_mp4)[0], validate_mp4(good_mp4)[1]
    assert not validate_mp4(_build_synthetic_mp4(50_000, include_avc1=True))[0]
    assert not validate_mp4(_build_synthetic_mp4(100_500, include_avc1=True, include_hvc1=True))[0]
    assert not validate_mp4(_build_synthetic_mp4(100_500, include_avc1=False))[0]
    assert not validate_mp4(b"NOTMP4" * 20_000)[0]

    # 3. MP3 validator (ID3 + MPEG sync, raw MPEG sync, size > 20KB, reject WAV)
    good_id3_mp3 = _build_synthetic_mp3(20_500, with_id3=True)
    good_raw_mp3 = _build_synthetic_mp3(20_500, with_id3=False)
    assert validate_mp3(good_id3_mp3)[0], validate_mp3(good_id3_mp3)[1]
    assert validate_mp3(good_raw_mp3)[0], validate_mp3(good_raw_mp3)[1]
    assert not validate_mp3(_build_synthetic_mp3(15_000, with_id3=True))[0]
    assert not validate_mp3(b"RIFF" + b"\x00" * 25_000)[0]

    # 4. Template-literal testid, data-mcp-tool attr, and JSX raw text leak detection
    clean_files = {
        Path("src/components/McpInlineTheater.tsx"): """
const TABS = [
  { key: "image", label: "生图", mcpTool: "tako_generate_image" },
  { key: "video", label: "生视频", mcpTool: "tako_generate_video" },
  { key: "audio", label: "声音", mcpTool: "tako_generate_audio" },
  { key: "search", label: "联网", mcpTool: "tako_web_search" },
  { key: "consult", label: "外脑", mcpTool: "tako_consult" },
];
const PRESETS = ["gpt-6.2-sol", "swe-2"];
function pickTool(n: number) {
  return n > 1 ? "tako_generate_image" : "tako_consult";
}
export const McpInlineTheater = () => (
  <section data-testid="mcp-inline-theater">
    <button data-testid="open-mcp-demo" />
    {TABS.map((t) => (
      <button key={t.key} data-testid={`mcp-demo-tab-${t.key}`} data-mcp-tool={t.mcpTool}>
        <span>{t.label}</span>
      </button>
    ))}
    {PRESETS.map((p) => <button key={p} data-testid={`consult-preset-${p}`} />)}
  </section>
);
"""
    }
    assert has_testid(clean_files, "open-mcp-demo")
    assert has_testid(clean_files, "mcp-inline-theater")
    assert has_testid(clean_files, "mcp-demo-tab-consult")
    assert has_testid(clean_files, "consult-preset-gpt-6.2-sol")
    assert not has_testid(clean_files, "mcp-live-run-btn")
    for tool_id, _ in REQUIRED_MCP_TOOL_SPECS:
        assert has_mcp_tool_attr(clean_files, tool_id)
    assert find_exposed_mcp_tool_names(clean_files) == []

    extension_files = {
        Path("src/components/OverviewPanel.tsx"): '''
const IDS = ["mcp", "skills", "plugins", "prompts"];
<button data-testid={`open-ext-hub-${id}`} />
''',
        Path("src/components/ExtensionsHubPanel.tsx"): '''<button data-testid={actions.primary.testId} />''',
        Path("src/components/extensions/McpHubView.tsx"): '''primary: { testId: "ext-mcp-add-btn" }''',
    }
    assert has_extension_testid(extension_files, "open-ext-hub-prompts")
    assert has_extension_testid(extension_files, "ext-mcp-add-btn")
    assert not has_extension_testid(extension_files, "ext-prompts-save-btn")

    leaky_files = {
        Path("src/App.tsx"): """
export const Leaky = () => (
  <div>
    <code>tako_generate_image</code>
    <span>{"tako_consult"}</span>
  </div>
);
"""
    }
    leaks = find_exposed_mcp_tool_names(leaky_files)
    assert len(leaks) == 2, f"expected 2 JSX text leaks, got {leaks}"

    # 5. End-to-end synthetic V2 repo verification
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "public" / "mcp-demo").mkdir(parents=True)
        (root / "src" / "components").mkdir(parents=True)
        (root / "src" / "components" / "extensions").mkdir()
        (root / "src-tauri" / "src" / "adapters").mkdir(parents=True)
        (root / "scripts").mkdir(parents=True)

        (root / CONTROLLED_AGY_I18N_REL).write_text(
            """
use sha2::Sha256;
// Controlled app.asar backup & restore
const ORIG: &str = "app.asar.orig";
struct Meta { original_asar_sha256: String }
fn verify_clean_backup_asar() {}
pub fn restore_antigravity_ui_for_paths() {}
""",
            encoding="utf-8",
        )

        # Synthetic valid standalone Antigravity Chinese dictionary (>= 1200 entries)
        synth_dict = {
            "New Conversation": "新建对话",
            "Scheduled Tasks": "定时任务",
            "Skills & Customizations": "技能与自定义",
            "Global Settings": "全局设置",
            "MCP Servers": "MCP 服务器",
        }
        for i in range(MIN_AGY_ZH_DICT_ENTRIES):
            synth_dict[f"Synthetic Label {i}"] = f"合成词条 {i}"
        (root / AGY_ZH_DICT_REL).write_text(
            json.dumps(synth_dict, ensure_ascii=False), encoding="utf-8"
        )
        assert validate_antigravity_zh_dict(root / AGY_ZH_DICT_REL)[0]

        for idx, name in enumerate(REQUIRED_WEBP_ASSETS):
            blob = b"RIFF" + struct.pack("<I", 12) + b"WEBP" + b"VP8 " + bytes([idx, 1, 2, 3])
            (root / "public" / "mcp-demo" / name).write_bytes(blob)
        (root / "public" / "mcp-demo" / REQUIRED_MP4_ASSET).write_bytes(
            _build_synthetic_mp4(100_500, include_avc1=True)
        )
        for idx, name in enumerate(REQUIRED_MP3_ASSETS):
            mp3_blob = _build_synthetic_mp3(20_500, with_id3=(idx % 2 == 0)) + bytes([idx])
            (root / "public" / "mcp-demo" / name).write_bytes(mp3_blob)

        asset_refs = "\n".join(
            f'const A_{i} = "/mcp-demo/{n}";'
            for i, n in enumerate(REQUIRED_WEBP_ASSETS + [REQUIRED_MP4_ASSET] + REQUIRED_MP3_ASSETS)
        )
        (root / "src" / "components" / "McpInlineTheater.tsx").write_text(
            f"""
const TITLE = "Tako 超级 MCP";
const WEBPAGE_TITLE = "网站上线 · Tako Web Page (EdgeOne)";
const SUB = "{PREF_CHINESE_SUBTITLE}";
{asset_refs}
const TOOLS = [
  {{ key: "image", label: "生图", mcpTool: "tako_generate_image" }},
  {{ key: "video", label: "生视频", mcpTool: "tako_generate_video" }},
  {{ key: "audio", label: "声音", mcpTool: "tako_generate_audio" }},
  {{ key: "search", label: "联网", mcpTool: "tako_web_search" }},
  {{ key: "consult", label: "外脑", mcpTool: "tako_consult" }},
];
const QUICKS = ["all", "promo", "cheap", "code", "long", "image"];
export const McpInlineTheater = () => (
  <div data-testid="mcp-inline-theater">
    <button data-testid="mcp-toggle-webpage" />
    <button data-testid="open-mcp-demo" />
    {{TOOLS.map((t) => <button data-testid={{`mcp-demo-tab-${{t.key}}`}} data-mcp-tool={{t.mcpTool}}>{{t.label}}</button>)}}
    {{QUICKS.map((q) => <button data-testid={{`plaza-quick-${{q}}`}}>{{q}}</button>)}}
    <button data-testid="mcp-mode-showcase" />
    <button data-testid="mcp-mode-live" />
    <div data-testid="mcp-image-slider" />
    <video data-testid="mcp-demo-video" />
    <div data-testid="mcp-video-compare-frame" />
    <button data-testid="mcp-demo-play-audio" />
    <div data-testid="mcp-audio-subtitles" />
    <button data-testid="mcp-demo-copy-prompt" />
    <button data-testid="consult-preset-gpt-6.2-sol" />
    <button data-testid="mcp-live-run-btn" />
    <button data-testid="main-chinese-ui-toggle" />
    <button data-testid="open-model-square-btn" />
    <button data-testid="agy-ui-patch-btn" />
    <button data-testid="agy-ui-restore-btn" />
    <button data-testid="claude-ui-patch-btn" />
    <div data-testid="ui-patch-permission-modal" />
    <button data-testid="plaza-price-mode-detail" />
    <button data-testid="plaza-price-mode-avg" />
    <button data-testid="plaza-currency-cny" />
    <button data-testid="plaza-currency-usd" />
    <button data-testid="plaza-unit-m" />
    <button data-testid="plaza-unit-k" />
  </div>
);
""",
            encoding="utf-8",
        )
        (root / "src/components/OverviewPanel.tsx").write_text(
            'const subtitle = "Tako 模式下自动注入，开关即时生效";\n'
            '<div className="grid-cols-2 min-[760px]:grid-cols-4">\n'
            + "\n".join(f'<button data-testid="{testid}" />' for testid in REQUIRED_EXT_HUB_TESTIDS)
            + "\n</div>",
            encoding="utf-8",
        )
        (root / "src/components/ExtensionsHubPanel.tsx").write_text(
            '<section className="bg-[var(--bg-card)]" />', encoding="utf-8"
        )
        (root / "src-tauri" / "tauri.conf.json").write_text(
            json.dumps(
                {
                    "app": {
                        "security": {
                            "csp": "default-src 'self'; img-src 'self' data:; media-src 'self' data:; script-src 'self'"
                        }
                    }
                }
            ),
            encoding="utf-8",
        )
        results = run_checks(root)
        failed = [(n, m) for n, ok, m in results if not ok]
        assert not failed, f"synthetic V2 repo should pass all checks, failed: {failed}"

        # Negative checks for v2.2.0 antigravity_zh_dict.json validator
        bad_dict_path = root / "bad_dict.json"
        assert not validate_antigravity_zh_dict(bad_dict_path)[0]
        bad_dict_path.write_text("{not-json", encoding="utf-8")
        assert not validate_antigravity_zh_dict(bad_dict_path)[0]
        bad_dict_path.write_text("[]", encoding="utf-8")
        assert not validate_antigravity_zh_dict(bad_dict_path)[0]
        bad_dict_path.write_text(json.dumps({"New Conversation": "新建对话"}), encoding="utf-8")
        assert not validate_antigravity_zh_dict(bad_dict_path)[0]
        empty_val_dict = dict(synth_dict)
        empty_val_dict["Empty Entry"] = "   "
        bad_dict_path.write_text(json.dumps(empty_val_dict, ensure_ascii=False), encoding="utf-8")
        assert not validate_antigravity_zh_dict(bad_dict_path)[0]
        non_zh_dict = dict(synth_dict)
        non_zh_dict["New Conversation"] = "New Conversation"
        bad_dict_path.write_text(json.dumps(non_zh_dict, ensure_ascii=False), encoding="utf-8")
        assert not validate_antigravity_zh_dict(bad_dict_path)[0]
        bad_dict_path.unlink()

        # Too-small antigravity_zh_dict.json in repo must fail i18n check
        (root / AGY_ZH_DICT_REL).write_text(
            json.dumps({"New Conversation": "新建对话"}, ensure_ascii=False),
            encoding="utf-8",
        )
        small_dict_res = dict((n, ok) for n, ok, _ in run_checks(root))
        assert not small_dict_res["i18n:antigravity-zh-dict"]
        (root / AGY_ZH_DICT_REL).write_text(
            json.dumps(synth_dict, ensure_ascii=False), encoding="utf-8"
        )

        # Unauthorized app.asar outside antigravity_i18n.rs must fail
        (root / "src" / "rogue.ts").write_text('const p = "Resources/app.asar";\n', encoding="utf-8")
        rogue_res = dict((n, ok) for n, ok, _ in run_checks(root))
        assert not rogue_res["security:no-app-asar-tampering"]
        (root / "src" / "rogue.ts").unlink()

        # npx asar inside antigravity_i18n.rs must still fail
        (root / CONTROLLED_AGY_I18N_REL).write_text('const BAD: &str = "npx asar pack";\n', encoding="utf-8")
        bad_cli_res = dict((n, ok) for n, ok, _ in run_checks(root))
        assert not bad_cli_res["security:no-app-asar-tampering"]
        assert not bad_cli_res["security:controlled-agy-i18n-backup"]

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
