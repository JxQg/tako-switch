#!/usr/bin/env python3
"""Unit tests for scripts/manage-releases.py."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import plistlib
import struct
import tempfile
import unittest


def load_module():
    path = Path(__file__).resolve().parent / "manage-releases.py"
    spec = importlib.util.spec_from_file_location("manage_releases", path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def build_minimal_asar(files: dict[str, bytes]) -> bytes:
    """Create a valid Electron ASAR archive in memory for tests."""
    header_files: dict = {}
    blobs: list[bytes] = []
    offset = 0
    for rel, content in files.items():
        parts = rel.split("/")
        cur = header_files
        for part in parts[:-1]:
            cur = cur.setdefault(part, {"files": {}})["files"]
        cur[parts[-1]] = {"size": len(content), "offset": str(offset)}
        blobs.append(content)
        offset += len(content)
    header_json = json.dumps({"files": header_files}, separators=(",", ":")).encode("utf-8")
    pad = (4 - (len(header_json) % 4)) % 4
    padded_json = header_json + (b"\x00" * pad)
    header_pickle_size = 4 + 4 + len(padded_json)
    return (
        struct.pack("<IIII", 4, header_pickle_size, len(padded_json) + 4, len(header_json))
        + padded_json
        + b"".join(blobs)
    )


class ManageReleasesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.mod = load_module()

    def test_render_manifest_from_pinned_preserves_schema(self) -> None:
        pinned = json.loads(self.mod.DEFAULT_PINNED_PATH.read_text(encoding="utf-8"))
        rendered = self.mod.render_manifest_from_pinned(pinned, generated_at="2026-11-25T12:00:00Z")
        self.assertEqual(rendered["schemaVersion"], 1)
        self.assertEqual(rendered["generatedAt"], "2026-11-25T12:00:00Z")
        self.assertEqual(set(rendered["apps"].keys()), {"dsh-tako", "codex", "claude", "antigravity"})
        # Companion metadata must not leak into public manifest apps schema
        self.assertNotIn("companionArtifacts", rendered["apps"]["claude"])
        self.assertNotIn("gates", rendered["apps"]["claude"])

    def test_analyze_changelog_risks_flags_breaking_and_auth_keywords(self) -> None:
        risks = self.mod.analyze_changelog_risks(
            "Breaking change: Electron 45 asar integrity enabled; OAuth login required for MCP."
        )
        cats = {r["category"] for r in risks}
        self.assertIn("breaking", cats)
        self.assertIn("auth_region", cats)
        self.assertIn("packaging_runtime", cats)
        self.assertIn("config_mcp", cats)

    def test_build_check_report_detects_version_diff(self) -> None:
        pinned = {
            "schemaVersion": 1,
            "apps": {
                "claude": {
                    "displayName": "Claude Desktop",
                    "version": "7.37095.0",
                    "status": "approved",
                    "platforms": {"macos-universal": {"version": "7.37095.0", "size": 100}},
                }
            },
        }
        upstream = {
            "apps": {
                "claude": {
                    "displayName": "Claude Desktop",
                    "version": "7.38000.0",
                    "platforms": {"macos-universal": {"version": "7.38000.0", "size": 120}},
                }
            }
        }
        changelogs = {
            "claude": {
                "entries": [{"version": "7.38000.0", "publishedAt": "2026-11-26", "body": "Electron update"}],
                "riskHints": [{"category": "packaging_runtime", "keywords": ["electron"]}],
            }
        }
        report, md = self.mod.build_check_report(pinned, upstream, changelogs)
        self.assertTrue(report["hasAnyUpstreamUpdate"])
        self.assertTrue(report["apps"]["claude"]["hasUpstreamUpdate"])
        self.assertIn("7.38000.0", md)
        self.assertIn("禁止自动盲跟上游", md)

    def test_verify_macos_bundle_claude_three_gates_and_baseline_diff(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            app_dir = root / "Claude.app"
            res_dir = app_dir / "Contents" / "Resources"
            macos_dir = app_dir / "Contents" / "MacOS"
            res_dir.mkdir(parents=True)
            macos_dir.mkdir(parents=True)
            (macos_dir / "Claude").write_bytes(b"mach-o")
            with (app_dir / "Contents" / "Info.plist").open("wb") as f:
                plistlib.dump(
                    {
                        "CFBundleIdentifier": "com.anthropic.claudefordesktop",
                        "CFBundleShortVersionString": "7.37095.0",
                        "CFBundleVersion": "7.37095.0",
                        "CFBundleURLTypes": [{"CFBundleURLSchemes": ["claude"]}],
                    },
                    f,
                )
            ccd_json = json.dumps(
                {
                    "version": "2.1.331",
                    "manifest": {"version": "2.1.331", "platforms": {}},
                    "baseUrl": "https://downloads.claude.ai/claude-code-releases",
                },
                separators=(",", ":"),
            )
            build_js = (
                'const loc={en:"en-US",ja:"ja-JP",id:"id-ID"};const re=/[a-z]{2}-[A-Z]{2}/;'
                'const keys=["inferenceGatewayBaseUrl","managedMcpServers","inferenceModels",'
                '"disableAutoUpdates","disableNonessentialTelemetry"];'
                f"function nc(){{return JSON.parse('{ccd_json}')}}"
            ).encode("utf-8")
            en_i18n = json.dumps({"k1": "New chat", "k2": "Settings"}).encode("utf-8")
            (res_dir / "app.asar").write_bytes(
                build_minimal_asar(
                    {
                        "resources/i18n/en-US.json": en_i18n,
                        ".vite/build/index.js": build_js,
                    }
                )
            )
            (res_dir / "ion-dist.asar").write_bytes(
                build_minimal_asar({"chunk.js": b'const locales=["en-US","ja-JP"];'})
            )

            baselines_dir = root / "baselines"
            first = self.mod.verify_macos_bundle(
                "claude", app_dir, baselines_dir=baselines_dir, update_baseline=True
            )
            self.assertEqual(first["overall"], "pass")
            self.assertEqual(first["extraAudit"]["claudeCodeVersion"], "2.1.331")

            # Adding an unexpected Helper triggers Gate 3 warning
            (app_dir / "Contents" / "Frameworks" / "WeirdHelper.app").mkdir(parents=True)
            second = self.mod.verify_macos_bundle(
                "claude", app_dir, baselines_dir=baselines_dir, update_baseline=False
            )
            self.assertEqual(second["gates"]["anomalyAudit"]["status"], "warn")
            self.assertEqual(second["overall"], "warn")

    def test_promote_blocks_failed_gate_and_accepts_passing_gate(self) -> None:
        pinned = {"schemaVersion": 1, "apps": {}}
        candidate = {
            "apps": {
                "antigravity": {
                    "upstreamEntry": {
                        "displayName": "Google Antigravity",
                        "version": "2.36.0",
                        "platforms": {"macos-arm64": {"url": "https://example.com/a.zip", "version": "2.36.0"}},
                    },
                    "changelog": {"entries": [{"version": "2.36.0", "body": "Bug fixes"}]},
                }
            }
        }
        failed_gate = {"appId": "antigravity", "overall": "fail", "gates": {}}
        with self.assertRaises(ValueError):
            self.mod.promote_candidate(pinned, "antigravity", candidate, failed_gate)

        passing_gate = {
            "appId": "antigravity",
            "overall": "pass",
            "gates": {
                "localization": {"status": "pass"},
                "injection": {"status": "pass"},
                "anomalyAudit": {"status": "pass"},
            },
        }
        updated = self.mod.promote_candidate(pinned, "antigravity", candidate, passing_gate)
        self.assertEqual(updated["apps"]["antigravity"]["version"], "2.36.0")
        self.assertEqual(updated["apps"]["antigravity"]["gates"]["localization"], "pass")


if __name__ == "__main__":
    unittest.main()
