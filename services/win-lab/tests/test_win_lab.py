"""Unit & black-box tests for tako-win-lab (cross-platform, zero external deps)."""

from __future__ import annotations

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import stat
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock
import urllib.error
import urllib.request
import zipfile

WIN_LAB_ROOT = Path(__file__).resolve().parents[1]
if str(WIN_LAB_ROOT) not in sys.path:
    sys.path.insert(0, str(WIN_LAB_ROOT))

from win_lab.config import LabConfig  # noqa: E402
from win_lab import gates  # noqa: E402
from win_lab.gates import (  # noqa: E402
    ELECTRON_FUSE_SENTINEL,
    ELECTRON_INTEGRITY_SLOT,
    _extract_windows_installer_sandbox,
    _recursive_7z_extract,
    detect_installer_type,
    verify_windows_artifact,
)
from win_lab.hq_reporter import HqReporter, scrub_text  # noqa: E402
from win_lab.runner import LabRunner  # noqa: E402
from win_lab.server import WinLabServer, is_private_or_mesh_addr  # noqa: E402
from win_lab.storage import LabStore  # noqa: E402


def build_fake_asar(files: dict[str, bytes]) -> bytes:
    tree: dict[str, dict] = {"files": {}}
    body = bytearray()
    for rel, raw in sorted(files.items()):
        parts = rel.split("/")
        cur = tree["files"]
        for p in parts[:-1]:
            cur = cur.setdefault(p, {"files": {}})["files"]
        cur[parts[-1]] = {"size": len(raw), "offset": str(len(body))}
        body.extend(raw)
    hdr_json = json.dumps(tree, separators=(",", ":")).encode("utf-8")
    pad = (4 - (len(hdr_json) % 4)) % 4
    hdr_padded = hdr_json + (b"\x00" * pad)
    inner_pickle = struct.pack("<II", 4 + len(hdr_padded), len(hdr_json)) + hdr_padded
    outer_pickle = struct.pack("<II", 4, len(inner_pickle)) + inner_pickle
    return outer_pickle + bytes(body)


def build_fake_codex_msix(path: Path, version: str = "26.1120.6357.0") -> None:
    manifest = f"""<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
         xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10">
  <Identity Name="OpenAI.Codex" Publisher="CN=OpenAI" Version="{version}" ProcessorArchitecture="x64" />
  <Applications>
    <Application Id="App" Executable="app\\ChatGPT.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="Codex" />
      <Extensions>
        <uap:Extension Category="windows.protocol">
          <uap:Protocol Name="codex" />
        </uap:Extension>
      </Extensions>
    </Application>
    <Application Id="Helper" Executable="app\\Helper.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="Helper" AppListEntry="none" />
    </Application>
  </Applications>
  <Capabilities>
    <Capability Name="internetClient" />
  </Capabilities>
</Package>
"""
    asar_bytes = build_fake_asar(
        {
            "main.js": b"localeOverride model_catalog_url default_tools_approval_mode approval_mode",
        }
    )
    with zipfile.ZipFile(path, "w") as zf:
        zf.writestr("AppxManifest.xml", manifest)
        zf.writestr("app/resources/app.asar", asar_bytes)
        zf.writestr("app/locales/zh-CN.pak", b"zh-CN")
        zf.writestr("app/resources/owl-app.ini", "[Owl]\nUserDataDirectoryName=Codex\n")
        zf.writestr("app/resources/codex-cli/codex-package.json", json.dumps({"version": "0.172.0"}))
        zf.writestr("app/ChatGPT.exe", b"MZ" + ELECTRON_FUSE_SENTINEL + ELECTRON_INTEGRITY_SLOT)


def build_fake_claude_squirrel_exe(path: Path) -> None:
    app_asar = build_fake_asar(
        {
            "resources/i18n/en-US.json": json.dumps({"k1": "Settings", "k2": "Help"}).encode("utf-8"),
            "resources/i18n/zh-Hans.json": json.dumps({"k1": "设置", "k2": "帮助"}).encode("utf-8"),
            ".vite/build/index.js": (
                b'const locales=["en-US","ja-JP","id-ID"];const re=/[a-z]{2}-[A-Z]{2}/;'
                b'const cfg={inferenceGatewayBaseUrl:1,managedMcpServers:1,inferenceModels:1,'
                b'disableAutoUpdates:1,disableNonessentialTelemetry:1};'
                b'const ccd={baseUrl:"https://downloads.claude.ai/claude-code-releases",version:"2.1.331"};'
            ),
        }
    )
    ion_asar = build_fake_asar({"chunk.js": b'const wl=["en-US","ja-JP","fr-FR"];'})

    nupkg_path = path.parent / "AnthropicClaude-7.37095.0-full.nupkg"
    with zipfile.ZipFile(nupkg_path, "w") as nf:
        nf.writestr("lib/net45/resources/app.asar", app_asar)
        nf.writestr("lib/net45/resources/ion-dist.asar", ion_asar)
        nf.writestr("lib/net45/Claude.exe", b"MZ" + ELECTRON_FUSE_SENTINEL + ELECTRON_INTEGRITY_SLOT)

    # Simulate Squirrel Setup.exe: PE stub prefix + embedded zip archive containing .nupkg
    outer_zip = path.parent / "outer.zip"
    with zipfile.ZipFile(outer_zip, "w") as oz:
        oz.write(nupkg_path, arcname=nupkg_path.name)
    path.write_bytes(b"MZ_FAKE_PE_STUB_HEADER____" + outer_zip.read_bytes())


class TestWinLabCore(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.repo_root = WIN_LAB_ROOT.parent.parent
        self.cfg = LabConfig.from_env(repo_root_override=self.repo_root, data_dir_override=self.tmp / "data")
        self.cfg = LabConfig(
            **{
                **self.cfg.__dict__,
                "schedule_enabled": False,
                "hq_url": "http://127.0.0.1:1",
                "telemetry_url": "",
                "hq_cooldown_sec": 3600,
            }
        )
        self.cfg.ensure_dirs()
        self.store = LabStore(self.cfg.db_path)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def test_secret_scrubber_and_mesh_guard(self) -> None:
        raw = "Authorization: Bearer supersecretvalue12345678 sk-abcdef1234567890 ghp_1234567890abcdef1234"
        cleaned = scrub_text(raw)
        self.assertNotIn("supersecretvalue12345678", cleaned)
        self.assertNotIn("sk-abcdef1234567890", cleaned)
        self.assertNotIn("ghp_1234567890abcdef1234", cleaned)

        self.assertTrue(is_private_or_mesh_addr("127.0.0.1"))
        self.assertTrue(is_private_or_mesh_addr("172.17.66.26"))
        self.assertTrue(is_private_or_mesh_addr("100.64.1.2"))
        self.assertFalse(is_private_or_mesh_addr("8.8.8.8"))

    def test_bsdtar_extraction_is_attempted_before_sandbox_install(self) -> None:
        # Fabricate a "tar" binary that drops resources/app.asar into the -C target dir;
        # verifies the bsdtar stage is wired before any installer execution.
        installer = self.tmp / "Antigravity-x64.exe"
        installer.write_bytes(b"MZ" + b"\x00" * 90000 + b"\xef\xbe\xad\xdeNullsoftInst" + b"\x00" * 4096)

        fake_tar = self.tmp / "faketar.sh"
        fake_tar.write_text(
            "#!/bin/sh\n"
            "out=\"\"\n"
            "prev=\"\"\n"
            "for a in \"$@\"; do\n"
            "  if [ \"$prev\" = \"-C\" ]; then out=\"$a\"; fi\n"
            "  prev=\"$a\"\n"
            "done\n"
            "mkdir -p \"$out/resources\"\n"
            "printf fake > \"$out/resources/app.asar\"\n"
            "exit 0\n",
            encoding="utf-8",
        )
        fake_tar.chmod(fake_tar.stat().st_mode | stat.S_IXUSR)

        def fake_which(name):
            return str(fake_tar) if name == "tar" else None

        with mock.patch.object(gates.shutil, "which", side_effect=fake_which):
            work = self.tmp / "work_tar"
            work.mkdir()
            unpack_dir, method = _extract_windows_installer_sandbox("antigravity", installer, work)
            self.assertEqual(method, "bsdtar")
            self.assertTrue(any(unpack_dir.rglob("app.asar")))

    def test_detect_installer_type_sniffs_pe_stub(self) -> None:
        nsis_exe = self.tmp / "nsis.exe"
        nsis_exe.write_bytes(b"MZ" + b"\x00" * 90000 + b"\xef\xbe\xad\xdeNullsoftInst" + b"\x00" * 64)
        self.assertEqual(detect_installer_type(nsis_exe), "nsis")

        inno_exe = self.tmp / "inno.exe"
        inno_exe.write_bytes(b"MZ" + b"\x00" * 90000 + b"Inno Setup Setup Data (6.4.3)" + b"\x00" * 64)
        self.assertEqual(detect_installer_type(inno_exe), "inno")

        plain_exe = self.tmp / "plain.exe"
        plain_exe.write_bytes(b"MZ" + b"\x00" * 4096)
        self.assertEqual(detect_installer_type(plain_exe), "unknown")

    def test_recursive_7z_extract_unwraps_nested_archives(self) -> None:
        # Fake 7z binary: a shell script that copies the archive into a zip-extracted form.
        # We simulate electron-builder NSIS layout: outer dir contains app-64.7z which itself is
        # a zip in disguise; the fake extractor "extracts" it as a real zip.
        inner_zip = self.tmp / "inner_payload.zip"
        with zipfile.ZipFile(inner_zip, "w") as zf:
            zf.writestr("resources/app.asar", b"fake-asar")

        outer = self.tmp / "7z_unpacked"
        plugins = outer / "$PLUGINSDIR"
        plugins.mkdir(parents=True)
        (plugins / "app-64.7z").write_bytes(inner_zip.read_bytes())

        fake_7z = self.tmp / "fake7z.sh"
        fake_7z.write_text(
            "#!/bin/sh\n"
            "# fake7z x -y -o<out> <arc> -> python unzip arc into out\n"
            "out=\"\"\n"
            "arc=\"\"\n"
            "for a in \"$@\"; do\n"
            "  case \"$a\" in\n"
            "    -o*) out=\"${a#-o}\" ;;\n"
            "    x|-y) ;;\n"
            "    *) arc=\"$a\" ;;\n"
            "  esac\n"
            "done\n"
            "exec python3 -c \"import sys,zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])\" \"$arc\" \"$out\"\n",
            encoding="utf-8",
        )
        fake_7z.chmod(fake_7z.stat().st_mode | stat.S_IXUSR)

        logs: list[str] = []
        _recursive_7z_extract(str(fake_7z), outer, log=logs.append)
        extracted = outer / "$PLUGINSDIR" / "app-64_7z" / "resources" / "app.asar"
        self.assertTrue(extracted.is_file(), f"missing {extracted}; logs={logs}")

    def test_nsis_sandbox_install_routes_by_sniffed_type(self) -> None:
        # Fake NSIS installer: embedded zip scan must fail, 7z absent, so the flow falls
        # through to the sandbox install. Mock subprocess.run to emulate a silent NSIS install
        # that honors /D=<dir> by dropping resources/app.asar into the target dir.
        installer = self.tmp / "Antigravity-x64.exe"
        installer.write_bytes(b"MZ" + b"\x00" * 90000 + b"\xef\xbe\xad\xdeNullsoftInst" + b"\x00" * 4096)

        asar_bytes = build_fake_asar({"main.js": b"localization anchor"})

        def fake_run(cmd, capture_output=False, timeout=None, shell=False, **kwargs):
            cmd_str = cmd if isinstance(cmd, str) else " ".join(str(c) for c in cmd)
            if shell and "/S /D=" in cmd_str:
                target = cmd_str.split("/D=", 1)[1].strip()
                dest = Path(target) / "resources"
                dest.mkdir(parents=True, exist_ok=True)
                (dest / "app.asar").write_bytes(asar_bytes)
                return subprocess.CompletedProcess(cmd, 0)
            return subprocess.CompletedProcess(cmd, 1)

        with mock.patch.object(gates.platform, "system", return_value="Windows"), mock.patch.object(
            gates.shutil, "which", return_value=None
        ), mock.patch.object(gates.subprocess, "run", side_effect=fake_run):
            work = self.tmp / "work_nsis"
            work.mkdir()
            unpack_dir, method = _extract_windows_installer_sandbox("antigravity", installer, work)
            self.assertEqual(method, "nsis_sandbox")
            self.assertTrue(any(unpack_dir.rglob("app.asar")))

    def test_inno_sandbox_still_used_for_inno_installers(self) -> None:
        installer = self.tmp / "legacy-inno.exe"
        installer.write_bytes(b"MZ" + b"\x00" * 90000 + b"Inno Setup Setup Data (6.4.3)" + b"\x00" * 4096)

        def fake_run(cmd, capture_output=False, timeout=None, shell=False, **kwargs):
            if isinstance(cmd, list) and any(str(c).startswith("/DIR=") for c in cmd):
                target = [str(c) for c in cmd if str(c).startswith("/DIR=")][0].split("=", 1)[1]
                dest = Path(target) / "resources"
                dest.mkdir(parents=True, exist_ok=True)
                (dest / "app.asar").write_bytes(build_fake_asar({"main.js": b"x"}))
                return subprocess.CompletedProcess(cmd, 0)
            return subprocess.CompletedProcess(cmd, 1)

        with mock.patch.object(gates.platform, "system", return_value="Windows"), mock.patch.object(
            gates.shutil, "which", return_value=None
        ), mock.patch.object(gates.subprocess, "run", side_effect=fake_run):
            work = self.tmp / "work_inno"
            work.mkdir()
            unpack_dir, method = _extract_windows_installer_sandbox("antigravity", installer, work)
            self.assertEqual(method, "inno_sandbox")
            self.assertTrue(any(unpack_dir.rglob("app.asar")))

    def test_nsis_install_timeout_does_not_raise(self) -> None:
        installer = self.tmp / "hanging-nsis.exe"
        installer.write_bytes(b"MZ" + b"\x00" * 90000 + b"\xef\xbe\xad\xdeNullsoftInst" + b"\x00" * 4096)

        def fake_run(cmd, capture_output=False, timeout=None, shell=False, **kwargs):
            cmd_str = cmd if isinstance(cmd, str) else " ".join(str(c) for c in cmd)
            if shell and "/S /D=" in cmd_str:
                raise subprocess.TimeoutExpired(cmd_str, timeout or 0)
            return subprocess.CompletedProcess(cmd, 1)

        with mock.patch.object(gates.platform, "system", return_value="Windows"), mock.patch.object(
            gates.shutil, "which", return_value=None
        ), mock.patch.object(gates.subprocess, "run", side_effect=fake_run):
            work = self.tmp / "work_hang"
            work.mkdir()
            unpack_dir, method = _extract_windows_installer_sandbox("antigravity", installer, work)
            # Falls back gracefully instead of raising/crashing the gate run
            self.assertEqual(method, "raw_exe_only")

    def test_verify_codex_msix_3_gates_and_baseline(self) -> None:
        msix = self.tmp / "ChatGPT-x64.msix"
        build_fake_codex_msix(msix)

        # Use isolated baselines dir inside tmp so we never touch repo baselines in unit tests
        fake_repo = self.tmp / "repo"
        (fake_repo / "manifest" / "baselines").mkdir(parents=True)
        cfg_iso = LabConfig(**{**self.cfg.__dict__, "repo_root": fake_repo})

        res1 = verify_windows_artifact(
            cfg_iso, app_id="codex", artifact_path=msix, update_baseline=True, check_remote_mirrors=False
        )
        self.assertEqual(res1["overall"], "pass")
        self.assertEqual(res1["gates"]["localization"]["status"], "pass")
        self.assertEqual(res1["gates"]["injection"]["status"], "pass")
        self.assertEqual(res1["gates"]["anomalyAudit"]["status"], "pass")
        self.assertEqual(res1["profile"]["packageName"], "OpenAI.Codex")
        self.assertTrue((fake_repo / "manifest" / "baselines" / "codex.windows.json").is_file())

        # Re-run against captured baseline -> still pass
        res2 = verify_windows_artifact(
            cfg_iso, app_id="codex", artifact_path=msix, update_baseline=False, check_remote_mirrors=False
        )
        self.assertEqual(res2["overall"], "pass")

    def test_verify_claude_squirrel_embedded_exe(self) -> None:
        exe = self.tmp / "Claude-test.exe"
        build_fake_claude_squirrel_exe(exe)
        fake_repo = self.tmp / "repo_claude"
        (fake_repo / "manifest" / "baselines").mkdir(parents=True)
        cfg_iso = LabConfig(**{**self.cfg.__dict__, "repo_root": fake_repo})

        res = verify_windows_artifact(
            cfg_iso, app_id="claude", artifact_path=exe, update_baseline=True, check_remote_mirrors=False
        )
        self.assertEqual(res["overall"], "pass")
        self.assertEqual(res["extraAudit"]["claudeCodeVersion"], "2.1.331")
        self.assertTrue(res["extraAudit"]["pe_Claude.exe"]["hasElectronFuse"])

    def test_hq_reporter_delivery_and_cooldown_dedupe(self) -> None:
        sent_calls: list[tuple[str, dict]] = []

        def fake_post(url: str, payload: dict, _timeout: float) -> tuple[int, str]:
            sent_calls.append((url, payload))
            return 201, json.dumps({"ok": True, "ticket_id": "abcd1234"})

        reporter = HqReporter(self.cfg, self.store, post_fn=fake_post)
        run = self.store.create_run(kind="smoke", app_id="codex", target_version="26.1120.6357.0")
        run = self.store.update_run(
            run["id"],
            status="fail",
            overall_gate="fail",
            summary="Codex 启动闪退 sk-secret1234567890",
            smoke_json={"launched": True, "survived_sec": 2.0, "crash_events": [{"id": 1000, "message": "0xc0000005"}]},
        )

        ev1 = reporter.report_completed_run(run)
        self.assertIsNotNone(ev1)
        self.assertEqual(ev1["status"], "sent")
        self.assertEqual(ev1["ticket_id"], "abcd1234")
        self.assertEqual(len(sent_calls), 1)
        self.assertTrue(sent_calls[0][1]["create_ticket"])
        self.assertNotIn("sk-secret1234567890", sent_calls[0][1]["body"])

        # Second identical failure within cooldown should be deduped
        ev2 = reporter.report_completed_run(run)
        self.assertEqual(ev2["status"], "deduped")
        self.assertEqual(len(sent_calls), 1)

    def test_http_server_plugin_manifest_mesh_guard_and_hq_endpoint(self) -> None:
        # Spin up a tiny mock HQ server on localhost
        received_hq: list[dict] = []

        class MockHQ(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_GET(self):  # noqa: N802
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'{"ok":true}')

            def do_POST(self):  # noqa: N802
                n = int(self.headers.get("Content-Length") or 0)
                received_hq.append(json.loads(self.rfile.read(n).decode("utf-8")))
                raw = json.dumps({"ok": True, "event_type": "test", "ticket_id": "t99887766"}).encode("utf-8")
                self.send_response(201)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

        hq_srv = ThreadingHTTPServer(("127.0.0.1", 0), MockHQ)
        hq_port = hq_srv.server_address[1]
        threading.Thread(target=hq_srv.serve_forever, daemon=True).start()

        cfg = LabConfig(
            **{
                **self.cfg.__dict__,
                "host": "127.0.0.1",
                "port": 0,
                "hq_url": f"http://127.0.0.1:{hq_port}",
            }
        )
        reporter = HqReporter(cfg, self.store)
        runner = LabRunner(cfg, self.store, reporter)
        runner.start()
        app = WinLabServer(cfg, self.store, reporter, runner)
        httpd = app.create_http_server()
        port = httpd.server_address[1]
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        try:
            # 1. /healthz
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/healthz", timeout=5) as r:
                h = json.loads(r.read().decode("utf-8"))
                self.assertTrue(h["ok"])
                self.assertEqual(h["service"], "tako-win-lab")

            # 2. /api/plugin/manifest
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/api/plugin/manifest", timeout=5) as r:
                m = json.loads(r.read().decode("utf-8"))
                self.assertEqual(m["id"], "win-lab")
                self.assertEqual(m["route"], "/win-lab")
                self.assertIn("badge", m)

            # 3. Mesh guard rejects X-Forwarded-For
            req_forb = urllib.request.Request(
                f"http://127.0.0.1:{port}/api/overview",
                headers={"X-Forwarded-For": "1.2.3.4"},
            )
            with self.assertRaises(urllib.error.HTTPError) as ctx:
                urllib.request.urlopen(req_forb, timeout=5)
            self.assertEqual(ctx.exception.code, 403)

            # 4. Manual HQ report ping
            req_hq = urllib.request.Request(
                f"http://127.0.0.1:{port}/api/hq/report",
                data=json.dumps({"title": "自检探活", "create_ticket": False}).encode("utf-8"),
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            with urllib.request.urlopen(req_hq, timeout=5) as r:
                res = json.loads(r.read().decode("utf-8"))
                self.assertTrue(res["ok"])
                self.assertEqual(res["data"]["event"]["status"], "sent")
            self.assertEqual(len(received_hq), 1)
            self.assertEqual(received_hq[0]["create_ticket"], False)
        finally:
            runner.stop()
            httpd.shutdown()
            httpd.server_close()
            hq_srv.shutdown()
            hq_srv.server_close()


if __name__ == "__main__":
    unittest.main()
