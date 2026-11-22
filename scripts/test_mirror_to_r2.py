from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location(
    "mirror_to_r2",
    Path(__file__).resolve().parent / "mirror-to-r2.py",
)
assert SPEC and SPEC.loader
mod = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mod)


class FakeS3:
    def __init__(self, existing: dict[str, dict] | None = None, listed: dict[str, list[dict]] | None = None):
        self.bucket = "tako-apps-mirror"
        self.existing = existing or {}
        self.listed = listed or {}
        self.uploaded: list[tuple[str, str, dict | None]] = []
        self.deleted: list[str] = []

    def head_object(self, key: str) -> dict | None:
        return self.existing.get(key)

    def upload_file(self, local_path: Path, key: str, *, cache_control: str, content_type=None, metadata=None) -> None:
        self.uploaded.append((key, cache_control, metadata))

    def list_objects(self, prefix: str) -> list[dict]:
        return self.listed.get(prefix, [])

    def delete_object(self, key: str) -> None:
        self.deleted.append(key)


class MirrorToR2Tests(unittest.TestCase):
    def test_build_object_key_and_mirror_url(self) -> None:
        url = "https://downloads.claude.ai/releases/darwin/universal/6.28899.6/Claude-abc123.zip"
        key = mod.build_object_key("apps", "claude", "macos-universal", "6.28899.6", url)
        self.assertEqual(key, "apps/claude/macos-universal/6.28899.6/Claude-abc123.zip")

        murl = mod.build_mirror_url("https://dl.lven.asia/apps/", "claude", "macos-universal", "6.28899.6", url)
        self.assertEqual(murl, "https://dl.lven.asia/apps/claude/macos-universal/6.28899.6/Claude-abc123.zip")

    def test_select_keys_to_prune_keeps_current_and_newest_previous(self) -> None:
        objs = [
            {"Key": "apps/claude/macos-universal/6.1.0/Claude-1.zip", "LastModified": "2026-10-01T00:00:00Z"},
            {"Key": "apps/claude/macos-universal/6.2.0/Claude-2.zip", "LastModified": "2026-10-15T00:00:00Z"},
            {"Key": "apps/claude/macos-universal/6.3.0/Claude-3.zip", "LastModified": "2026-11-01T00:00:00Z"},
        ]
        stale = mod.select_keys_to_prune(objs, current_version="6.3.0", keep_versions=2)
        self.assertEqual(stale, ["apps/claude/macos-universal/6.1.0/Claude-1.zip"])

        # Even if current_version has an older timestamp, it must never be pruned.
        stale_pinned = mod.select_keys_to_prune(objs, current_version="6.1.0", keep_versions=2)
        self.assertEqual(stale_pinned, ["apps/claude/macos-universal/6.2.0/Claude-2.zip"])

    def test_mirror_skips_existing_and_rewrites_url_only_for_selected_apps(self) -> None:
        manifest = {
            "schemaVersion": 1,
            "generatedAt": "2026-11-22T00:00:00Z",
            "apps": {
                "claude": {
                    "displayName": "Claude Desktop",
                    "version": "6.28899.6",
                    "platforms": {
                        "macos-universal": {
                            "url": "https://downloads.claude.ai/releases/darwin/universal/6.28899.6/Claude-2f5fc5.zip",
                            "version": "6.28899.6",
                            "installType": "zip",
                            "size": 12345,
                        }
                    },
                },
                "codex": {
                    "displayName": "Codex Desktop",
                    "version": "26.1117.41037",
                    "platforms": {
                        "macos-arm64": {
                            "url": "https://persistent.oaistatic.com/codex-app-prod/ChatGPT.zip",
                            "version": "26.1117.41037",
                            "installType": "zip",
                            "size": 99999,
                        }
                    },
                },
            },
        }
        key = "apps/claude/macos-universal/6.28899.6/Claude-2f5fc5.zip"
        fake = FakeS3(
            existing={key: {"ContentLength": 12345, "Metadata": {"sha256": "deadbeef"}}},
            listed={
                "apps/claude/macos-universal/": [
                    {"Key": "apps/claude/macos-universal/6.0.0/old.zip", "LastModified": "2026-09-01T00:00:00Z"},
                    {"Key": "apps/claude/macos-universal/6.1.0/prev.zip", "LastModified": "2026-10-01T00:00:00Z"},
                    {"Key": key, "LastModified": "2026-11-22T00:00:00Z"},
                ]
            },
        )
        out = mod.mirror_manifest_to_r2(
            manifest,
            fake,
            mirror_base="https://dl.lven.asia/apps",
            apps_to_mirror={"claude", "antigravity"},
            keep_versions=2,
        )
        claude_mac = out["apps"]["claude"]["platforms"]["macos-universal"]
        self.assertEqual(
            claude_mac["url"],
            "https://dl.lven.asia/apps/claude/macos-universal/6.28899.6/Claude-2f5fc5.zip",
        )
        self.assertEqual(
            claude_mac["fallbackUrl"],
            "https://downloads.claude.ai/releases/darwin/universal/6.28899.6/Claude-2f5fc5.zip",
        )
        self.assertEqual(claude_mac["sha256"], "deadbeef")
        self.assertEqual(fake.deleted, ["apps/claude/macos-universal/6.0.0/old.zip"])

        # Codex untouched when not in apps_to_mirror
        codex_mac = out["apps"]["codex"]["platforms"]["macos-arm64"]
        self.assertEqual(codex_mac["url"], "https://persistent.oaistatic.com/codex-app-prod/ChatGPT.zip")
        self.assertNotIn("fallbackUrl", codex_mac)

    def test_dsh_tako_preserves_oss_as_fallback_when_mirrored(self) -> None:
        manifest = {
            "schemaVersion": 1,
            "generatedAt": "2026-11-22T00:00:00Z",
            "apps": {
                "dsh-tako": {
                    "displayName": "DSH Tako",
                    "version": "0.1.57",
                    "platforms": {
                        "macos-arm64": {
                            "url": "https://dsh-tako-dl.oss-cn-shanghai.aliyuncs.com/releases/v0.1.57/DSH-Tako-0.1.57-mac-arm64.dmg",
                            "fallbackUrl": "https://github.com/tako-dev/dsh-tako-desktop/releases/download/v0.1.57/DSH-Tako-0.1.57-mac-arm64.dmg",
                            "version": "0.1.57",
                            "installType": "dmg",
                            "sha256": "d44e4293",
                            "size": 270914084,
                        }
                    },
                }
            },
        }
        key = "apps/dsh-tako/macos-arm64/0.1.57/DSH-Tako-0.1.57-mac-arm64.dmg"
        fake = FakeS3(existing={key: {"ContentLength": 270914084, "Metadata": {"sha256": "d44e4293"}}})
        out = mod.mirror_manifest_to_r2(
            manifest,
            fake,
            mirror_base="https://dl.lven.asia/apps",
            apps_to_mirror=mod.parse_app_filter(None),
            keep_versions=2,
        )
        dsh_mac = out["apps"]["dsh-tako"]["platforms"]["macos-arm64"]
        self.assertEqual(
            dsh_mac["url"],
            "https://dl.lven.asia/apps/dsh-tako/macos-arm64/0.1.57/DSH-Tako-0.1.57-mac-arm64.dmg",
        )
        self.assertEqual(
            dsh_mac["fallbackUrl"],
            "https://dsh-tako-dl.oss-cn-shanghai.aliyuncs.com/releases/v0.1.57/DSH-Tako-0.1.57-mac-arm64.dmg",
        )


if __name__ == "__main__":
    unittest.main()
