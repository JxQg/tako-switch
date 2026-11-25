"""HTTP server for tako-win-lab implementing the cloud-ops-console v4 plugin contract."""

from __future__ import annotations

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import ipaddress
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import time
from typing import Any
from urllib.parse import parse_qs, unquote, urlparse

from . import APP_ORDER, SERVICE_NAME, SERVICE_VERSION
from .config import LabConfig
from .gates import load_pinned_versions
from .hq_reporter import HqReporter, probe_http_health
from .runner import LabRunner
from .storage import LabStore, utc_now_iso


UI_DIR = Path(__file__).resolve().parent / "ui"
MAX_BODY_BYTES = 2 * 1024 * 1024


def is_private_or_mesh_addr(remote_addr: str) -> bool:
    host = (remote_addr or "").strip().strip("[]")
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        return False
    if ip.is_loopback or ip.is_private or ip.is_link_local:
        return True
    # CGNAT 100.64.0.0/10 (Tailscale/overlay)
    if ip.version == 4 and (int(ip) & 0xFFC00000) == 0x64400000:
        return True
    return False


def collect_host_info(cfg: LabConfig) -> dict[str, Any]:
    disk_free_gb = 0.0
    disk_total_gb = 0.0
    try:
        usage = shutil.disk_usage(str(cfg.data_dir))
        disk_free_gb = round(usage.free / (1024**3), 1)
        disk_total_gb = round(usage.total / (1024**3), 1)
    except OSError:
        pass
    return {
        "hostname": socket.gethostname(),
        "os": platform.system(),
        "osRelease": platform.release(),
        "osVersion": platform.version(),
        "arch": platform.machine(),
        "python": platform.python_version(),
        "diskFreeGB": disk_free_gb,
        "diskTotalGB": disk_total_gb,
        "allowLiveSmoke": cfg.allow_live_smoke,
        "scheduleEnabled": cfg.schedule_enabled,
        "hqUrl": cfg.hq_url,
        "telemetryUrl": cfg.telemetry_url,
    }


class WinLabServer:
    def __init__(self, cfg: LabConfig, store: LabStore, reporter: HqReporter, runner: LabRunner) -> None:
        self.cfg = cfg
        self.store = store
        self.reporter = reporter
        self.runner = runner
        self.started_monotonic = time.monotonic()
        self._health_cache: dict[str, Any] = {}
        self._health_cache_at: float = 0.0

    def _cached_upstream_health(self) -> dict[str, Any]:
        now = time.monotonic()
        if now - self._health_cache_at < 30.0 and self._health_cache:
            return self._health_cache
        hq_h = probe_http_health(f"{self.cfg.hq_url}/v1/health", timeout=2.5) if self.cfg.hq_url else {"ok": False}
        tel_h = (
            probe_http_health(f"{self.cfg.telemetry_url}/healthz", timeout=2.5)
            if self.cfg.telemetry_url
            else {"ok": False}
        )
        self._health_cache = {"hq": hq_h, "telemetry": tel_h, "checkedAt": utc_now_iso()}
        self._health_cache_at = now
        return self._health_cache

    def build_badge(self) -> dict[str, int]:
        latest = self.store.latest_runs_by_app()
        failing = 0
        for aid in APP_ORDER:
            g = (latest.get(aid) or {}).get("gate")
            s = (latest.get(aid) or {}).get("smoke")
            if (g and g.get("status") in ("fail", "error")) or (s and s.get("status") in ("fail", "error")):
                failing += 1
        snap = self.store.latest_upstream_snapshot()
        pending_cand = 0
        if snap and isinstance(snap.get("report"), dict):
            for info in (snap["report"].get("apps") or {}).values():
                if isinstance(info, dict) and info.get("hasUpstreamUpdate"):
                    pending_cand += 1
        hq_24h = self.store.count_hq_events_last_24h()
        return {
            "failing_gates": failing,
            "pending_candidates": pending_cand,
            "running_jobs": self.store.count_active_runs(),
            "hq_events_24h": hq_24h["sent"],
        }

    def build_plugin_manifest(self) -> dict[str, Any]:
        return {
            "id": "win-lab",
            "title": "Windows 测试台",
            "group": "tako",
            "route": "/tako/win-lab",
            "routes": [
                {"path": "/tako/win-lab", "key": "overview", "title": "门禁矩阵"},
                {"path": "/tako/win-lab/runs", "key": "runs", "title": "测试记录"},
                {"path": "/tako/win-lab/candidates", "key": "candidates", "title": "上游版本与日志"},
                {"path": "/tako/win-lab/hq", "key": "hq", "title": "HQ 事件上报"},
            ],
            "embed_entry": "/win-lab?embed=1&theme={theme}",
            "upstream_port": self.cfg.port,
            "version": SERVICE_VERSION,
            "capabilities": [
                "embedded-v4-ui",
                "windows-release-gate",
                "windows-live-smoke",
                "changelog-audit",
                "hq-event-reporter",
            ],
            "badge": self.build_badge(),
        }

    def build_overview(self) -> dict[str, Any]:
        pinned = load_pinned_versions(self.cfg)
        pinned_apps = pinned.get("apps") or {}
        latest = self.store.latest_runs_by_app()
        snap = self.store.latest_upstream_snapshot()
        up_apps = ((snap or {}).get("report") or {}).get("apps") or {}

        matrix: list[dict[str, Any]] = []
        for aid in APP_ORDER:
            p = pinned_apps.get(aid) or {}
            win_plat = (p.get("platforms") or {}).get("windows-x64") or {}
            up = up_apps.get(aid) or {}
            lr = latest.get(aid) or {}
            matrix.append(
                {
                    "appId": aid,
                    "displayName": p.get("displayName") or aid,
                    "pinnedVersion": str(win_plat.get("version") or p.get("version") or ""),
                    "pinnedMacVersion": str(p.get("version") or ""),
                    "pinnedStatus": str(p.get("status") or "unknown"),
                    "installType": str(win_plat.get("installType") or ""),
                    "hasUpstreamUpdate": bool(up.get("hasUpstreamUpdate")),
                    "upstreamVersion": str(up.get("upstreamVersion") or ""),
                    "changelogSummary": str(p.get("changelogSummary") or ""),
                    "latestGate": lr.get("gate"),
                    "latestSmoke": lr.get("smoke"),
                }
            )

        return {
            "service": SERVICE_NAME,
            "version": SERVICE_VERSION,
            "uptimeSec": round(time.monotonic() - self.started_monotonic, 1),
            "host": collect_host_info(self.cfg),
            "upstreams": self._cached_upstream_health(),
            "badge": self.build_badge(),
            "activeRunId": self.runner.active_run_id(),
            "hq24h": self.store.count_hq_events_last_24h(),
            "matrix": matrix,
            "recentRuns": self.store.list_runs(limit=15),
            "latestUpstream": {
                "checkedAt": snap["checked_at"] if snap else "",
                "hasUpdates": snap["has_updates"] if snap else False,
                "summary": snap["summary"] if snap else "尚未执行上游巡检",
            },
        }

    def make_handler_class(self) -> type[BaseHTTPRequestHandler]:
        srv = self

        class Handler(BaseHTTPRequestHandler):
            server_version = f"{SERVICE_NAME}/{SERVICE_VERSION}"

            def log_message(self, fmt: str, *args: Any) -> None:
                if os.environ.get("WIN_LAB_QUIET_LOG", "0") == "1":
                    return
                super().log_message(fmt, *args)

            def _send_json(self, status: int, payload: Any) -> None:
                raw = json.dumps(payload, ensure_ascii=False).encode("utf-8")
                self.send_response(status)
                self.send_header("Content-Type", "application/json; charset=utf-8")
                self.send_header("Content-Length", str(len(raw)))
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                if self.command != "HEAD":
                    self.wfile.write(raw)

            def _send_bytes(self, status: int, data: bytes, content_type: str) -> None:
                self.send_response(status)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(data)))
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                if self.command != "HEAD":
                    self.wfile.write(data)

            def _require_internal_mesh(self) -> bool:
                for hdr in ("X-Forwarded-For", "X-Real-IP", "Forwarded", "X-Forwarded-Host"):
                    if (self.headers.get(hdr) or "").strip():
                        self._send_json(
                            403, {"ok": False, "error": "internal plugin endpoints reject proxied public traffic"}
                        )
                        return False
                remote_ip = self.client_address[0] if self.client_address else ""
                if not is_private_or_mesh_addr(remote_ip):
                    self._send_json(
                        403, {"ok": False, "error": "internal plugin endpoints require private mesh direct connection"}
                    )
                    return False
                return True

            def _read_json(self) -> dict[str, Any]:
                length = int(self.headers.get("Content-Length") or 0)
                if length <= 0:
                    return {}
                if length > MAX_BODY_BYTES:
                    raise ValueError("request body too large")
                raw = self.rfile.read(length)
                data = json.loads(raw.decode("utf-8"))
                if not isinstance(data, dict):
                    raise ValueError("JSON body must be an object")
                return data

            def _serve_ui(self, _norm_path: str) -> None:
                index_file = UI_DIR / "index.html"
                if not index_file.is_file():
                    self._send_json(500, {"ok": False, "error": "UI index.html missing"})
                    return
                html = index_file.read_text(encoding="utf-8")
                css = (UI_DIR / "app.css").read_text(encoding="utf-8") if (UI_DIR / "app.css").is_file() else ""
                embed_js = (
                    (UI_DIR / "ops-console-embed.js").read_text(encoding="utf-8")
                    if (UI_DIR / "ops-console-embed.js").is_file()
                    else ""
                )
                app_js = (UI_DIR / "app.js").read_text(encoding="utf-8") if (UI_DIR / "app.js").is_file() else ""
                html = (
                    html.replace("/*__APP_CSS__*/", css, 1)
                    .replace("/*__OPS_EMBED_JS__*/", embed_js, 1)
                    .replace("/*__APP_JS__*/", app_js, 1)
                )
                self._send_bytes(200, html.encode("utf-8"), "text/html; charset=utf-8")

            def do_HEAD(self) -> None:  # noqa: N802
                self.do_GET()

            def do_GET(self) -> None:  # noqa: N802
                parsed = urlparse(self.path)
                path = parsed.path.rstrip("/") or "/"
                qs = parse_qs(parsed.query)

                if path in ("/healthz", "/api/health"):
                    db_ok = srv.store.ping()
                    badge = srv.build_badge() if db_ok else {}
                    self._send_json(
                        200 if db_ok else 503,
                        {
                            "ok": db_ok,
                            "service": SERVICE_NAME,
                            "version": SERVICE_VERSION,
                            "host": socket.gethostname(),
                            "os": platform.system(),
                            "uptime_sec": round(time.monotonic() - srv.started_monotonic, 1),
                            "db_ok": db_ok,
                            "db_size_bytes": srv.store.db_size_bytes(),
                            "badge": badge,
                        },
                    )
                    return

                if not self._require_internal_mesh():
                    return

                if path == "/api/plugin/manifest":
                    self._send_json(200, srv.build_plugin_manifest())
                    return

                if path == "/" or path == "/win-lab" or path.startswith("/win-lab/"):
                    self._serve_ui("/win-lab" if path == "/" else path)
                    return

                if path == "/api/overview":
                    self._send_json(200, {"ok": True, "data": srv.build_overview()})
                    return

                if path == "/api/runs":
                    app_f = (qs.get("app_id", [""])[0] or "").strip()
                    kind_f = (qs.get("kind", [""])[0] or "").strip()
                    st_f = (qs.get("status", [""])[0] or "").strip()
                    lim = int(qs.get("limit", ["50"])[0] or "50")
                    self._send_json(
                        200,
                        {"ok": True, "data": {"items": srv.store.list_runs(app_id=app_f, kind=kind_f, status=st_f, limit=lim)}},
                    )
                    return

                m_run = re.fullmatch(r"/api/runs/([A-Za-z0-9._-]+)", path)
                if m_run:
                    run = srv.store.get_run(unquote(m_run.group(1)))
                    if not run:
                        self._send_json(404, {"ok": False, "error": "run not found"})
                        return
                    self._send_json(200, {"ok": True, "data": run})
                    return

                if path == "/api/candidates":
                    snap = srv.store.latest_upstream_snapshot()
                    self._send_json(200, {"ok": True, "data": snap or {"checked_at": "", "has_updates": False, "report": {}, "report_md": ""}})
                    return

                if path == "/api/hq/events":
                    lim = int(qs.get("limit", ["60"])[0] or "60")
                    ch = (qs.get("channel", [""])[0] or "").strip()
                    self._send_json(
                        200,
                        {
                            "ok": True,
                            "data": {
                                "items": srv.store.list_hq_events(limit=lim, channel=ch),
                                "stats24h": srv.store.count_hq_events_last_24h(),
                                "upstreams": srv._cached_upstream_health(),
                            },
                        },
                    )
                    return

                self._send_json(404, {"ok": False, "error": f"not found: {path}"})

            def do_POST(self) -> None:  # noqa: N802
                if not self._require_internal_mesh():
                    return
                parsed = urlparse(self.path)
                path = parsed.path.rstrip("/") or "/"
                try:
                    body = self._read_json()
                except Exception as exc:
                    self._send_json(400, {"ok": False, "error": f"invalid JSON body: {exc}"})
                    return

                try:
                    if path == "/api/runs/gate":
                        app_id = str(body.get("app_id") or "all").strip()
                        runs = srv.runner.enqueue_gate(
                            app_id=app_id,
                            candidate_url=str(body.get("candidate_url") or ""),
                            version_override=str(body.get("version") or ""),
                            update_baseline=bool(body.get("update_baseline")),
                            report_hq=bool(body.get("report_hq", True)),
                            trigger=str(body.get("trigger") or "ops_ui"),
                        )
                        self._send_json(202, {"ok": True, "data": {"runs": runs}})
                        return

                    if path == "/api/runs/smoke":
                        app_id = str(body.get("app_id") or "").strip()
                        if not app_id:
                            self._send_json(400, {"ok": False, "error": "app_id is required"})
                            return
                        run = srv.runner.enqueue_smoke(
                            app_id=app_id,
                            candidate_url=str(body.get("candidate_url") or ""),
                            version_override=str(body.get("version") or ""),
                            hold_seconds=int(body.get("hold_seconds") or 15),
                            keep_installed=bool(body.get("keep_installed")),
                            report_hq=bool(body.get("report_hq", True)),
                            trigger=str(body.get("trigger") or "ops_ui"),
                        )
                        self._send_json(202, {"ok": True, "data": {"run": run}})
                        return

                    if path == "/api/candidates/check":
                        run = srv.runner.enqueue_upstream_check(
                            report_hq=bool(body.get("report_hq", True)),
                            trigger=str(body.get("trigger") or "ops_ui"),
                        )
                        self._send_json(202, {"ok": True, "data": {"run": run}})
                        return

                    if path == "/api/hq/report":
                        run_id = str(body.get("run_id") or "").strip()
                        if run_id:
                            run = srv.store.get_run(run_id)
                            if not run:
                                self._send_json(404, {"ok": False, "error": f"run not found: {run_id}"})
                                return
                            ev = srv.reporter.report_completed_run(run, force_hq=True)
                            if ev:
                                srv.store.update_run(
                                    run_id,
                                    hq_event_id=ev.get("id", ""),
                                    hq_ticket_id=ev.get("ticket_id", ""),
                                )
                            self._send_json(200, {"ok": True, "data": {"event": ev}})
                            return

                        ev_type = str(body.get("event_type") or "winlab.gate_passed").strip()
                        title = str(body.get("title") or "Windows 测试台 HQ 连通性自检").strip()
                        msg = str(body.get("body") or f"来自 {socket.gethostname()} 的测试台连通性探活事件").strip()
                        create_ticket = bool(body.get("create_ticket", False))
                        priority = str(body.get("priority") or "P3").strip().upper()
                        ev = srv.reporter.report_hq_event(
                            event_type=ev_type,
                            title=title,
                            body=msg,
                            priority=priority,
                            create_ticket=create_ticket,
                            tags=["selftest"],
                            source_ref=f"tako-win-lab:manual:{int(time.time())}",
                            force=True,
                        )
                        self._send_json(200, {"ok": True, "data": {"event": ev}})
                        return

                    if path == "/api/sync-repo":
                        res = srv.runner.sync_repo()
                        self._send_json(200 if res["ok"] else 500, {"ok": res["ok"], "data": res})
                        return

                    self._send_json(404, {"ok": False, "error": f"not found: {path}"})
                except ValueError as exc:
                    self._send_json(400, {"ok": False, "error": str(exc)})
                except Exception as exc:
                    self._send_json(500, {"ok": False, "error": f"{type(exc).__name__}: {exc}"})

        return Handler

    def create_http_server(self) -> ThreadingHTTPServer:
        return ThreadingHTTPServer((self.cfg.host, self.cfg.port), self.make_handler_class())
