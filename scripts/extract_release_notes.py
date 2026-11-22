#!/usr/bin/env python3
"""Extract release notes for a specific version from CHANGELOG.md.

Used by `.github/workflows/release.yml` so that `latest.json` (Tauri updater +
website download page) and GitHub Release bodies stay in sync with a single
source of truth in `CHANGELOG.md`.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Optional

HEADING_RE = re.compile(
    r"^##\s+\[?v?([0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?)\]?(?:\s|$)"
)


def normalize_version(raw: str) -> str:
    return raw.strip().lstrip("vV")


def extract_section_from_markdown(markdown: str, version: str) -> Optional[str]:
    """Return the body lines under `## v<version> ...`, or None if missing."""
    target = normalize_version(version)
    if not target:
        return None

    lines = markdown.splitlines()
    collecting = False
    collected: list[str] = []

    for line in lines:
        match = HEADING_RE.match(line.strip())
        if match:
            heading_ver = normalize_version(match.group(1))
            if collecting:
                break
            if heading_ver == target:
                collecting = True
            continue
        if collecting:
            if line.startswith("## "):
                break
            collected.append(line.rstrip())

    if not collecting:
        return None

    # Trim leading and trailing blank lines.
    while collected and not collected[0].strip():
        collected.pop(0)
    while collected and not collected[-1].strip():
        collected.pop()

    text = "\n".join(collected).strip()
    return text or None


def fallback_git_notes(repo_dir: Path, tag: str) -> Optional[str]:
    """Build bullet list from commits since the previous v* tag."""
    try:
        tags_raw = subprocess.check_output(
            ["git", "tag", "--sort=-v:refname", "v*"],
            cwd=repo_dir,
            stderr=subprocess.DEVNULL,
            text=True,
        )
    except (subprocess.SubprocessError, OSError):
        return None

    norm_tag = "v" + normalize_version(tag)
    prev_tag: Optional[str] = None
    for candidate in (t.strip() for t in tags_raw.splitlines() if t.strip()):
        if candidate != norm_tag and candidate != tag.strip():
            prev_tag = candidate
            break

    rev_range = f"{prev_tag}..HEAD" if prev_tag else "HEAD~5..HEAD"
    try:
        log_raw = subprocess.check_output(
            ["git", "log", rev_range, "--no-merges", "--pretty=format:%s"],
            cwd=repo_dir,
            stderr=subprocess.DEVNULL,
            text=True,
        )
    except (subprocess.SubprocessError, OSError):
        return None

    subjects = [
        line.strip()
        for line in log_raw.splitlines()
        if line.strip() and "[skip ci]" not in line
    ]
    if not subjects:
        return None
    return "\n".join(f"- {s}" for s in subjects[:12])


def resolve_release_notes(
    changelog_path: Path,
    tag: str,
    repo_dir: Optional[Path] = None,
    allow_fallback: bool = True,
) -> str:
    version = normalize_version(tag)
    if changelog_path.is_file():
        markdown = changelog_path.read_text(encoding="utf-8")
        section = extract_section_from_markdown(markdown, version)
        if section:
            return section

    if not allow_fallback:
        raise ValueError(
            f"CHANGELOG.md ({changelog_path}) is missing a non-empty section for v{version}"
        )

    if repo_dir is not None:
        git_notes = fallback_git_notes(repo_dir, tag)
        if git_notes:
            return git_notes

    return f"Tako Switch v{version}"


def main() -> int:
    parser = argparse.ArgumentParser(description="Extract Tako Switch release notes")
    parser.add_argument(
        "--changelog",
        default="CHANGELOG.md",
        help="Path to CHANGELOG.md (default: CHANGELOG.md)",
    )
    parser.add_argument(
        "--tag",
        default="",
        help="Release tag or semver (e.g. v2.0.10 or 2.0.10)",
    )
    parser.add_argument(
        "--check-package-json",
        default="",
        help="Verify CHANGELOG.md has a non-empty entry for the version in package.json",
    )
    parser.add_argument(
        "--out",
        default="",
        help="Optional file path to write extracted notes",
    )
    parser.add_argument(
        "--strict",
        action="store_true",
        help="Fail instead of falling back when CHANGELOG.md lacks the target version",
    )
    args = parser.parse_args()

    changelog_path = Path(args.changelog)

    if args.check_package_json:
        pkg = json.loads(Path(args.check_package_json).read_text(encoding="utf-8"))
        version = str(pkg.get("version", "")).strip()
        if not version:
            print("package.json missing version field", file=sys.stderr)
            return 1
        try:
            notes = resolve_release_notes(
                changelog_path, version, repo_dir=None, allow_fallback=False
            )
        except ValueError as exc:
            print(str(exc), file=sys.stderr)
            return 1
        print(f"CHANGELOG.md OK for v{version} ({len(notes.splitlines())} lines)")
        return 0

    if not args.tag:
        print("--tag or --check-package-json is required", file=sys.stderr)
        return 2

    try:
        notes = resolve_release_notes(
            changelog_path,
            args.tag,
            repo_dir=changelog_path.resolve().parent,
            allow_fallback=not args.strict,
        )
    except ValueError as exc:
        print(str(exc), file=sys.stderr)
        return 1

    if args.out:
        out_path = Path(args.out)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(notes + "\n", encoding="utf-8")
    else:
        print(notes)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
