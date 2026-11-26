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


class TimeoutRetryTests(unittest.TestCase):
    def _client(self) -> object:
        return mod.S3Client(bucket="b", endpoint_url="https://ep.example.com", region="auto")

    def test_head_object_passes_timeout_to_subprocess(self) -> None:
        import subprocess as sp
        from unittest import mock

        client = self._client()
        fake_res = sp.CompletedProcess(args=[], returncode=0, stdout="{}", stderr="")
        with mock.patch.object(mod.subprocess, "run", return_value=fake_res) as run_mock:
            self.assertEqual(client.head_object("apps/k"), {})
        self.assertEqual(run_mock.call_count, 1)
        self.assertEqual(run_mock.call_args.kwargs.get("timeout"), mod.API_TIMEOUT_SECONDS)

    def test_head_object_timeout_retried_then_raises_with_key(self) -> None:
        import subprocess as sp
        from unittest import mock

        client = self._client()

        def always_timeout(*args, **kwargs):
            raise sp.TimeoutExpired(cmd=args[0], timeout=kwargs.get("timeout"))

        with mock.patch.object(mod.subprocess, "run", side_effect=always_timeout) as run_mock:
            with self.assertRaises(RuntimeError) as ctx:
                client.head_object("apps/stuck-key")
        self.assertEqual(run_mock.call_count, mod.AWS_MAX_ATTEMPTS)
        self.assertIn("apps/stuck-key", str(ctx.exception))
        self.assertIn("timed out", str(ctx.exception))

    def test_list_objects_single_timeout_recovers(self) -> None:
        import subprocess as sp
        from unittest import mock

        client = self._client()
        fake_res = sp.CompletedProcess(args=[], returncode=0, stdout='{"Contents": []}', stderr="")
        calls = []

        def flaky(*args, **kwargs):
            calls.append(1)
            if len(calls) == 1:
                raise sp.TimeoutExpired(cmd=args[0], timeout=kwargs.get("timeout"))
            return fake_res

        with mock.patch.object(mod.subprocess, "run", side_effect=flaky) as run_mock:
            self.assertEqual(client.list_objects("apps/claude/"), [])
        self.assertEqual(run_mock.call_count, 2)

    def test_upload_timeout_scales_with_file_size(self) -> None:
        import subprocess as sp
        import tempfile
        from unittest import mock

        client = self._client()
        with tempfile.TemporaryDirectory() as td:
            big = Path(td) / "big.bin"
            big.write_bytes(b"x" * 16)
            fake_res = sp.CompletedProcess(args=[], returncode=0)
            with mock.patch.object(mod.subprocess, "run", return_value=fake_res) as run_mock:
                client.upload_file(big, "apps/small.bin", cache_control="x")
            small_timeout = run_mock.call_args.kwargs.get("timeout")
            self.assertEqual(small_timeout, mod.UPLOAD_TIMEOUT_SECONDS)

            # Simulate a huge file by monkeypatching stat size.
            real_stat = Path.stat

            def fake_stat(self, *a, **k):
                if self == big:
                    import os
                    st = real_stat(self, *a, **k)
                    return os.stat_result((st[0], st[1], st[2], st[3], st[4], st[5], 512 * 1024 * 1024, st[7], st[8], st[9]))
                return real_stat(self, *a, **k)

            with mock.patch.object(Path, "stat", fake_stat):
                with mock.patch.object(mod.subprocess, "run", return_value=fake_res) as run_mock:
                    client.upload_file(big, "apps/huge.bin", cache_control="x")
                huge_timeout = run_mock.call_args.kwargs.get("timeout")
            self.assertGreater(huge_timeout, mod.UPLOAD_TIMEOUT_SECONDS)

    def test_upload_timeout_error_mentions_key_and_elapsed(self) -> None:
        import subprocess as sp
        import tempfile
        from unittest import mock

        client = self._client()
        with tempfile.TemporaryDirectory() as td:
            f = Path(td) / "x.bin"
            f.write_bytes(b"x")

            def always_timeout(*args, **kwargs):
                raise sp.TimeoutExpired(cmd=args[0], timeout=kwargs.get("timeout"))

            with mock.patch.object(mod.subprocess, "run", side_effect=always_timeout) as run_mock:
                with self.assertRaises(RuntimeError) as ctx:
                    client.upload_file(f, "apps/timeout.bin", cache_control="x")
        self.assertEqual(run_mock.call_count, mod.AWS_MAX_ATTEMPTS)
        msg = str(ctx.exception)
        self.assertIn("apps/timeout.bin", msg)
        self.assertIn("attempts", msg)

    def test_mirror_companion_artifacts_uploads_verifies_and_prunes(self) -> None:
        payload = b"fake-zst-claude-code"
        import hashlib
        sha = hashlib.sha256(payload).hexdigest()
        pinned = {
            "apps": {
                "claude": {
                    "companionArtifacts": {
                        "claude-code": {
                            "version": "2.1.331",
                            "platforms": {
                                "win32-x64": {
                                    "url": "https://downloads.claude.ai/claude-code-releases/2.1.331/win32-x64/claude.exe.zst",
                                    "sha256": sha,
                                    "size": len(payload),
                                }
                            },
                        }
                    }
                }
            }
        }
        uploaded: list[str] = []
        deleted: list[str] = []

        class FakeS3:
            bucket = "tako-apps-mirror"
            def head_object(self, key: str):
                return None
            def upload_file(self, local_path: Path, key: str, *, cache_control: str, content_type: str | None = None):
                assert cache_control
                uploaded.append(key)
            def list_objects(self, prefix: str):
                return [
                    {"Key": "apps/claude-code/2.1.331/win32-x64/claude.exe.zst", "LastModified": "2026-11-25T00:00:00Z"},
                    {"Key": "apps/claude-code/2.1.300/win32-x64/claude.exe.zst", "LastModified": "2026-11-20T00:00:00Z"},
                    {"Key": "apps/claude-code/2.1.200/win32-x64/claude.exe.zst", "LastModified": "2026-11-10T00:00:00Z"},
                ]
            def delete_object(self, key: str):
                deleted.append(key)

        def fake_download(url: str, dest: Path):
            dest.write_bytes(payload)

        with unittest.mock.patch.object(mod, "download_file", side_effect=fake_download):
            keys = mod.mirror_companion_artifacts_to_r2(pinned, FakeS3(), keep_versions=2)

        self.assertEqual(keys, ["apps/claude-code/2.1.331/win32-x64/claude.exe.zst"])
        self.assertEqual(uploaded, ["apps/claude-code/2.1.331/win32-x64/claude.exe.zst"])
        self.assertEqual(deleted, ["apps/claude-code/2.1.200/win32-x64/claude.exe.zst"])

    def test_mirror_agy_cli_manifests_verifies_sha512_rewrites_urls_and_syncs_doge(self) -> None:
        import hashlib
        import json
        import tempfile
        import unittest.mock

        payload = b"fake-agy-cli-tarball-content"
        sha512 = hashlib.sha512(payload).hexdigest()

        with tempfile.TemporaryDirectory() as tmp:
            mf_dir = Path(tmp) / "manifests"
            mf_dir.mkdir(parents=True)
            official_url = (
                "https://storage.googleapis.com/antigravity-public/antigravity-cli/"
                "1.5.17-4823843970351104/darwin-arm/cli_mac_arm64.tar.gz"
            )
            (mf_dir / "darwin_arm64.json").write_text(
                json.dumps(
                    {
                        "version": "1.5.17",
                        "url": official_url,
                        "sha512": sha512,
                        "size": len(payload),
                        "minSupportedVersion": "1.5.16",
                        "supportedVersions": ["1.5.16", "1.5.17"],
                    }
                ),
                encoding="utf-8",
            )
            (mf_dir / "index.json").write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "tool": "agy",
                        "version": "1.5.17",
                        "platforms": {"darwin_arm64": {"url": official_url, "sha512": sha512, "size": len(payload)}},
                    }
                ),
                encoding="utf-8",
            )

            fake_r2 = FakeS3(
                listed={
                    "apps/agy-cli/darwin_arm64/": [
                        {"Key": "apps/agy-cli/darwin_arm64/1.5.15/cli_mac_arm64.tar.gz", "LastModified": "2026-11-20T00:00:00Z"},
                        {"Key": "apps/agy-cli/darwin_arm64/1.5.16/cli_mac_arm64.tar.gz", "LastModified": "2026-11-23T00:00:00Z"},
                        {"Key": "apps/agy-cli/darwin_arm64/1.5.17/cli_mac_arm64.tar.gz", "LastModified": "2026-11-26T00:00:00Z"},
                    ]
                }
            )
            fake_doge = FakeS3()

            def fake_download(url: str, dest: Path) -> None:
                dest.write_bytes(payload)

            with unittest.mock.patch.object(mod, "download_file", side_effect=fake_download):
                mirrored = mod.mirror_agy_cli_manifests_to_r2(
                    manifests_dir=mf_dir,
                    s3=fake_r2,
                    mirror_base="https://dl.lven.asia/apps",
                    key_prefix="apps",
                    manifest_s3_prefix="tako-switch/agy-cli/manifests",
                    keep_versions=2,
                    doge_s3=fake_doge,
                )

            self.assertEqual(mirrored, ["apps/agy-cli/darwin_arm64/1.5.17/cli_mac_arm64.tar.gz"])
            updated_plat = json.loads((mf_dir / "darwin_arm64.json").read_text(encoding="utf-8"))
            self.assertEqual(
                updated_plat["url"],
                "https://dl.lven.asia/apps/agy-cli/darwin_arm64/1.5.17/cli_mac_arm64.tar.gz",
            )
            self.assertEqual(updated_plat["fallbackUrl"], official_url)
            updated_idx = json.loads((mf_dir / "index.json").read_text(encoding="utf-8"))
            self.assertEqual(updated_idx["platforms"]["darwin_arm64"]["url"], updated_plat["url"])
            self.assertEqual(updated_idx["platforms"]["darwin_arm64"]["fallbackUrl"], official_url)
            self.assertEqual(fake_r2.deleted, ["apps/agy-cli/darwin_arm64/1.5.15/cli_mac_arm64.tar.gz"])
            doge_keys = [k for k, _, _ in fake_doge.uploaded]
            self.assertEqual(
                doge_keys,
                [
                    "tako-switch/agy-cli/manifests/darwin_arm64.json",
                    "tako-switch/agy-cli/manifests/index.json",
                ],
            )


if __name__ == "__main__":
    unittest.main()
