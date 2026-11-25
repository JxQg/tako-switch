"""Background job runner and periodic scheduler for tako-win-lab."""

from __future__ import annotations

from datetime import datetime, timezone
import queue
import subprocess
import threading
import time
from typing import Any

from . import APP_ORDER
from .config import LabConfig
from .gates import (
    download_artifact,
    load_pinned_versions,
    resolve_windows_artifact_spec,
    run_upstream_candidate_check,
    verify_windows_artifact,
)
from .hq_reporter import HqReporter
from .smoke import run_windows_runtime_smoke
from .storage import LabStore, utc_now_iso


class LabRunner:
    """Serializes heavy Windows install/unpack/smoke tasks cleanly and handles scheduled checks."""

    def __init__(self, cfg: LabConfig, store: LabStore, reporter: HqReporter) -> None:
        self.cfg = cfg
        self.store = store
        self.reporter = reporter
        self._q: queue.Queue[dict[str, Any]] = queue.Queue()
        self._stop = threading.Event()
        self._worker_thread: threading.Thread | None = None
        self._sched_thread: threading.Thread | None = None
        self._active_run_id: str = ""
        self._lock = threading.Lock()

    def start(self) -> None:
        if self._worker_thread is None:
            self._worker_thread = threading.Thread(target=self._worker_loop, name="win-lab-worker", daemon=True)
            self._worker_thread.start()
        if self.cfg.schedule_enabled and self._sched_thread is None:
            self._sched_thread = threading.Thread(target=self._scheduler_loop, name="win-lab-sched", daemon=True)
            self._sched_thread.start()

    def stop(self) -> None:
        self._stop.set()
        self._q.put({"_stop": True})

    def active_run_id(self) -> str:
        with self._lock:
            return self._active_run_id

    def enqueue_gate(
        self,
        *,
        app_id: str,
        candidate_url: str = "",
        version_override: str = "",
        update_baseline: bool = False,
        report_hq: bool = True,
        trigger: str = "manual",
    ) -> list[dict[str, Any]]:
        targets = list(APP_ORDER) if app_id == "all" else [app_id]
        created: list[dict[str, Any]] = []
        for aid in targets:
            if aid not in APP_ORDER:
                raise ValueError(f"Unsupported app_id: {aid}")
            run = self.store.create_run(
                kind="gate",
                app_id=aid,
                target_version=version_override,
                source_url=candidate_url,
                trigger=trigger,
            )
            self._q.put(
                {
                    "type": "gate",
                    "run_id": run["id"],
                    "app_id": aid,
                    "candidate_url": candidate_url if app_id != "all" else "",
                    "version_override": version_override if app_id != "all" else "",
                    "update_baseline": update_baseline,
                    "report_hq": report_hq,
                }
            )
            created.append(run)
        return created

    def enqueue_smoke(
        self,
        *,
        app_id: str,
        candidate_url: str = "",
        version_override: str = "",
        hold_seconds: int = 15,
        keep_installed: bool = False,
        report_hq: bool = True,
        trigger: str = "manual",
    ) -> dict[str, Any]:
        if app_id not in APP_ORDER:
            raise ValueError(f"Unsupported app_id for smoke test: {app_id}")
        run = self.store.create_run(
            kind="smoke",
            app_id=app_id,
            target_version=version_override,
            source_url=candidate_url,
            trigger=trigger,
        )
        self._q.put(
            {
                "type": "smoke",
                "run_id": run["id"],
                "app_id": app_id,
                "candidate_url": candidate_url,
                "version_override": version_override,
                "hold_seconds": hold_seconds,
                "keep_installed": keep_installed,
                "report_hq": report_hq,
            }
        )
        return run

    def enqueue_upstream_check(self, *, report_hq: bool = True, trigger: str = "manual") -> dict[str, Any]:
        run = self.store.create_run(kind="upstream_check", app_id="all", trigger=trigger)
        self._q.put({"type": "upstream_check", "run_id": run["id"], "report_hq": report_hq})
        return run

    def sync_repo(self) -> dict[str, Any]:
        """Pull latest manifest/pinned-versions.json and i18n dicts from git remote (fast-forward only)."""
        git_dir = self.cfg.repo_root / ".git"
        if not git_dir.exists():
            return {"ok": False, "message": "WIN_LAB_REPO_ROOT 不是 Git 仓库，跳过 git pull"}
        proc = subprocess.run(
            ["git", "-C", str(self.cfg.repo_root), "pull", "--ff-only"],
            capture_output=True,
            text=True,
            timeout=60,
        )
        sha_proc = subprocess.run(
            ["git", "-C", str(self.cfg.repo_root), "rev-parse", "--short", "HEAD"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        return {
            "ok": proc.returncode == 0,
            "head": sha_proc.stdout.strip(),
            "stdout": proc.stdout.strip(),
            "stderr": proc.stderr.strip(),
        }

    def _worker_loop(self) -> None:
        while not self._stop.is_set():
            try:
                job = self._q.get(timeout=1.0)
            except queue.Empty:
                continue
            if job.get("_stop"):
                return
            run_id = str(job.get("run_id") or "")
            with self._lock:
                self._active_run_id = run_id
            try:
                jtype = job.get("type")
                if jtype == "gate":
                    self._execute_gate(job)
                elif jtype == "smoke":
                    self._execute_smoke(job)
                elif jtype == "upstream_check":
                    self._execute_upstream_check(job)
            except Exception as exc:
                if run_id:
                    self.store.update_run(
                        run_id,
                        status="error",
                        overall_gate="error",
                        finished_at=utc_now_iso(),
                        summary=f"内部执行异常: {exc}",
                    )
            finally:
                with self._lock:
                    self._active_run_id = ""
                self._q.task_done()

    def _execute_gate(self, job: dict[str, Any]) -> None:
        run_id = job["run_id"]
        app_id = job["app_id"]
        t0 = time.monotonic()
        logs: list[str] = []

        def _log(msg: str) -> None:
            logs.append(f"[{utc_now_iso()}] {msg}")

        self.store.update_run(run_id, status="running")
        try:
            spec = resolve_windows_artifact_spec(
                self.cfg,
                app_id,
                candidate_url=job.get("candidate_url", ""),
                version_override=job.get("version_override", ""),
            )
            self.store.update_run(run_id, target_version=spec["version"], source_url=spec["url"])
            _log(f"start Windows 3-gate for {app_id} v{spec['version']}")
            dl = download_artifact(
                spec["url"],
                spec["cache_path"],
                expected_sha256=spec["expected_sha256"],
                fallback_url=spec["fallback_url"],
                log=_log,
            )
            gate_res = verify_windows_artifact(
                self.cfg,
                app_id=app_id,
                artifact_path=dl["path"],
                version_hint=spec["version"],
                pinned=spec["pinned"],
                update_baseline=bool(job.get("update_baseline")),
                log=_log,
            )
            overall = str(gate_res["overall"])
            gates = gate_res["gates"]
            summary_parts = [
                f"{k}={gates[k]['status']}" for k in ("localization", "injection", "anomalyAudit") if k in gates
            ]
            summary = f"{app_id} {gate_res['version']} Windows 三门禁: {', '.join(summary_parts)}"
            _log(f"completed overall={overall} | {summary}")

            dur_ms = int((time.monotonic() - t0) * 1000)
            updated = self.store.update_run(
                run_id,
                target_version=gate_res["version"] or spec["version"],
                status=overall,
                overall_gate=overall,
                finished_at=utc_now_iso(),
                duration_ms=dur_ms,
                summary=summary,
                gates_json=gates,
                artifacts_json={
                    "profile": gate_res.get("profile") or {},
                    "extraAudit": gate_res.get("extraAudit") or {},
                    "cached": dl["cached"],
                },
                log_text="\n".join(logs),
            )
            if updated and job.get("report_hq", True):
                ev = self.reporter.report_completed_run(updated)
                if ev:
                    self.store.update_run(
                        run_id,
                        hq_event_id=ev.get("id", ""),
                        hq_ticket_id=ev.get("ticket_id", ""),
                    )
        except Exception as exc:
            dur_ms = int((time.monotonic() - t0) * 1000)
            _log(f"ERROR: {exc}")
            updated = self.store.update_run(
                run_id,
                status="error",
                overall_gate="error",
                finished_at=utc_now_iso(),
                duration_ms=dur_ms,
                summary=f"{app_id} 门禁执行失败: {exc}",
                log_text="\n".join(logs),
            )
            if updated and job.get("report_hq", True):
                ev = self.reporter.report_completed_run(updated)
                if ev:
                    self.store.update_run(
                        run_id,
                        hq_event_id=ev.get("id", ""),
                        hq_ticket_id=ev.get("ticket_id", ""),
                    )

    def _execute_smoke(self, job: dict[str, Any]) -> None:
        run_id = job["run_id"]
        app_id = job["app_id"]
        t0 = time.monotonic()
        logs: list[str] = []

        def _log(msg: str) -> None:
            logs.append(f"[{utc_now_iso()}] {msg}")

        self.store.update_run(run_id, status="running")
        try:
            spec = resolve_windows_artifact_spec(
                self.cfg,
                app_id,
                candidate_url=job.get("candidate_url", ""),
                version_override=job.get("version_override", ""),
            )
            self.store.update_run(run_id, target_version=spec["version"], source_url=spec["url"])
            _log(f"start Windows Gate-4 live smoke for {app_id} v{spec['version']}")
            dl = download_artifact(
                spec["url"],
                spec["cache_path"],
                expected_sha256=spec["expected_sha256"],
                fallback_url=spec["fallback_url"],
                log=_log,
            )
            smoke_status, smoke_data = run_windows_runtime_smoke(
                self.cfg,
                app_id=app_id,
                artifact_path=dl["path"],
                hold_seconds=int(job.get("hold_seconds") or 15),
                keep_installed=bool(job.get("keep_installed")),
                log=_log,
            )
            crashes = len(smoke_data.get("crash_events") or [])
            if smoke_data.get("skipped"):
                summary = str(smoke_data.get("reason") or "已跳过")
            elif smoke_data.get("install_error"):
                summary = f"{app_id} {spec['version']} 安装失败: {smoke_data['install_error'][:160]}"
            elif not smoke_data.get("launched"):
                summary = f"{app_id} {spec['version']} 未拉起进程（crash_events={crashes}）"
            else:
                summary = (
                    f"{app_id} {spec['version']} 冷启动 {smoke_status}: "
                    f"存活 {smoke_data.get('survived_sec', 0)}s / 峰值进程 {smoke_data.get('peak_processes', 0)} / 崩溃事件 {crashes}"
                )
            _log(f"completed smoke status={smoke_status} | {summary}")

            dur_ms = int((time.monotonic() - t0) * 1000)
            updated = self.store.update_run(
                run_id,
                status=smoke_status,
                overall_gate=smoke_status,
                finished_at=utc_now_iso(),
                duration_ms=dur_ms,
                summary=summary,
                smoke_json=smoke_data,
                log_text="\n".join(logs),
            )
            if updated and job.get("report_hq", True):
                ev = self.reporter.report_completed_run(updated)
                if ev:
                    self.store.update_run(
                        run_id,
                        hq_event_id=ev.get("id", ""),
                        hq_ticket_id=ev.get("ticket_id", ""),
                    )
        except Exception as exc:
            dur_ms = int((time.monotonic() - t0) * 1000)
            _log(f"ERROR: {exc}")
            updated = self.store.update_run(
                run_id,
                status="error",
                overall_gate="error",
                finished_at=utc_now_iso(),
                duration_ms=dur_ms,
                summary=f"{app_id} 实机冒烟异常: {exc}",
                log_text="\n".join(logs),
            )
            if updated and job.get("report_hq", True):
                ev = self.reporter.report_completed_run(updated)
                if ev:
                    self.store.update_run(
                        run_id,
                        hq_event_id=ev.get("id", ""),
                        hq_ticket_id=ev.get("ticket_id", ""),
                    )

    def _execute_upstream_check(self, job: dict[str, Any]) -> None:
        run_id = job["run_id"]
        t0 = time.monotonic()
        self.store.update_run(run_id, status="running")
        try:
            rep_json, rep_md = run_upstream_candidate_check(self.cfg)
            has_upd = bool(rep_json.get("hasAnyUpstreamUpdate"))
            upd_apps = [
                f"{aid}({info.get('pinnedVersion')}->{info.get('upstreamVersion')})"
                for aid, info in (rep_json.get("apps") or {}).items()
                if isinstance(info, dict) and info.get("hasUpstreamUpdate")
            ]
            summary = f"发现上游新版: {', '.join(upd_apps)}" if upd_apps else "4 个桌面客户端均与锁定版本一致"
            self.store.save_upstream_snapshot(
                has_updates=has_upd,
                summary=summary,
                report_json=rep_json,
                report_md=rep_md,
            )
            self.store.set_meta("last_upstream_check_at", utc_now_iso())

            hq_events = self.reporter.report_upstream_candidates(rep_json) if (has_upd and job.get("report_hq", True)) else []
            first_ev = hq_events[0] if hq_events else {}
            dur_ms = int((time.monotonic() - t0) * 1000)
            self.store.update_run(
                run_id,
                status="warn" if has_upd else "pass",
                overall_gate="warn" if has_upd else "pass",
                finished_at=utc_now_iso(),
                duration_ms=dur_ms,
                summary=summary,
                artifacts_json={"updatedApps": upd_apps, "hqEventsCount": len(hq_events)},
                hq_event_id=str(first_ev.get("id") or ""),
                hq_ticket_id=str(first_ev.get("ticket_id") or ""),
                log_text=rep_md[:20000],
            )
        except Exception as exc:
            dur_ms = int((time.monotonic() - t0) * 1000)
            self.store.update_run(
                run_id,
                status="error",
                overall_gate="error",
                finished_at=utc_now_iso(),
                duration_ms=dur_ms,
                summary=f"上游版本巡检失败: {exc}",
                log_text=str(exc),
            )

    def _scheduler_loop(self) -> None:
        # Wait 45s after startup before checking whether periodic runs are due.
        if self._stop.wait(45.0):
            return
        while not self._stop.is_set():
            try:
                now_ts = time.time()
                # 1. Periodic upstream candidate check
                last_up_iso = self.store.get_meta("last_upstream_check_at", "")
                due_up = True
                if last_up_iso:
                    try:
                        prev_dt = datetime.strptime(last_up_iso, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
                        due_up = (now_ts - prev_dt.timestamp()) >= self.cfg.upstream_check_interval_min * 60
                    except ValueError:
                        due_up = True
                if due_up and self.store.count_active_runs() == 0:
                    self.enqueue_upstream_check(report_hq=True, trigger="schedule")
            except Exception:
                pass
            if self._stop.wait(300.0):
                return
