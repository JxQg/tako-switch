import tempfile
import unittest
from pathlib import Path

from scripts.extract_release_notes import (
    extract_section_from_markdown,
    resolve_release_notes,
)


SAMPLE_CHANGELOG = """# 更新日志

说明行。

## v2.0.10 (2026-11-22)

- 新增版本更新说明体系
- 升级客户端更新横幅

## [2.0.9] - 2026-11-22

- 默认绕过系统代理
- 修复僵尸上传

## Unreleased

- 草稿内容
"""


class ExtractReleaseNotesTests(unittest.TestCase):
    def test_extracts_exact_version_with_or_without_v_prefix(self) -> None:
        expected = "- 新增版本更新说明体系\n- 升级客户端更新横幅"
        self.assertEqual(extract_section_from_markdown(SAMPLE_CHANGELOG, "v2.0.10"), expected)
        self.assertEqual(extract_section_from_markdown(SAMPLE_CHANGELOG, "2.0.10"), expected)

    def test_extracts_bracketed_version_and_stops_at_next_section(self) -> None:
        expected = "- 默认绕过系统代理\n- 修复僵尸上传"
        self.assertEqual(extract_section_from_markdown(SAMPLE_CHANGELOG, "v2.0.9"), expected)

    def test_returns_none_when_version_missing(self) -> None:
        self.assertIsNone(extract_section_from_markdown(SAMPLE_CHANGELOG, "v2.0.8"))

    def test_strict_mode_raises_on_missing_version(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "CHANGELOG.md"
            path.write_text(SAMPLE_CHANGELOG, encoding="utf-8")
            with self.assertRaises(ValueError):
                resolve_release_notes(path, "v9.9.9", repo_dir=None, allow_fallback=False)

    def test_fallback_default_when_no_git_and_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "CHANGELOG.md"
            path.write_text(SAMPLE_CHANGELOG, encoding="utf-8")
            notes = resolve_release_notes(path, "v9.9.9", repo_dir=None, allow_fallback=True)
            self.assertEqual(notes, "Tako Switch v9.9.9")


if __name__ == "__main__":
    unittest.main()
