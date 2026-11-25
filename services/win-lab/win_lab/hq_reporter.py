"""HQ Event & Tako Telemetry reporting with secret scrubbing and cooldown deduplication."""

from __future__ import annotations

import ipaddress
import json
import platform
import re
import socket
import time
from typing import Any, Callable
import urllib.error
import urllib.parse
import urllib.request

from . import SERVICE_VERSION
from .config import LabConfig
from .storage import LabStore


SECRET_PATTERNS: list[tuple[re.Pattern[str], str]] = [
    (re.compile(r"sk-[A-Za-z0-9_\-]{12,}"), "sk-[REDACTED]"),
    (re.compile(r"gh[pousr]_[A-Za-z0-9]{16,}"), "gh_token_[REDACTED]"),
    (re.compile(r"AKIA[0-9A-Z]{16}"), "AKIA[REDACTED]"),
    (re.compile(r"(?i)(authorization\s*[:=]\s*bearer\s+)[A-Za-z0-9._\-+/=]{8,}"), r"\1[REDACTED]"),
    (re.compile(r"(?i)(bearer\s+)[A-Za-z0-9._\-+/=]{16,}"), r"\1[REDACTED]"),
    (re.compile(r"(?i)(api[_-]?key|secret|token|password)\s*[:=]\s*['\"]?[A-Za-z0-9._\-+/=]{12,}['\"]?"), r"\1=[REDACTED]"),
    (re.compile(r"-----BEGIN [A-Z0-9 ]+ PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]+ PRIVATE KEY-----"), "[PRIVATE_KEY_REDACTED]"),
]


def scrub_text(text: str, max_len: int = 4000) -> str:
    if not text:
        return ""
    cleaned = text
    for pat, repl in SECRET_PATTERNS:
        cleaned = pat.sub(repl, cleaned)
    if len(cleaned) > max_len:
        cleaned = cleaned[: max_len - 20] + "\n...[truncated]"
    return cleaned


def scrub_obj(obj: Any) -> Any:
    if isinstance(obj, str):
        return scrub_text(obj)
    if isinstance(obj, list):
        return [scrub_obj(x) for x in obj]
    if isinstance(obj, dict):
        return {str(k): scrub_obj(v) for k, v in obj.items()}
    return obj


def _is_private_url(url: str) -> bool:
    host = (urllib.parse.urlparse(url).hostname or "").strip().lower()
    if host in ("localhost", "127.0.0.1", "::1"):
        return True
    try:
        ip = ipaddress.ip_address(host)
        return ip.is_loopback or ip.is_private
    except ValueError:
        return False


def default_http_post_json(url: str, payload: dict[str, Any], timeout: float = 8.0) -> tuple[int, str]:
    raw = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=raw,
        headers={
            "Content-Type": "application/json; charset=utf-8",
            "Accept": "application/json",
            "User-Agent": f"tako-win-lab/{SERVICE_VERSION}",
        },
        method="POST",
    )
    opener = (
        urllib.request.build_opener(urllib.request.ProxyHandler({}))
        if _is_private_url(url)
        else urllib.request.build_opener()
    )
    try:
        with opener.open(req, timeout=timeout) as resp:
            return int(resp.status), resp.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as exc:
        body = ""
        try:
            body = exc.read().decode("utf-8", errors="replace")
        except Exception:
            pass
        return int(exc.code), body or str(exc)
    except Exception as exc:
        return 0, f"{type(exc).__name__}: {exc}"


def probe_http_health(url: str, timeout: float = 3.0) -> dict[str, Any]:
    req = urllib.request.Request(url, headers={"User-Agent": f"tako-win-lab/{SERVICE_VERSION}"})
    opener = (
        urllib.request.build_opener(urllib.request.ProxyHandler({}))
        if _is_private_url(url)
        else urllib.request.build_opener()
    )
    started = time.monotonic()
    try:
        with opener.open(req, timeout=timeout) as resp:
            ms = int((time.monotonic() - started) * 1000)
            return {"ok": 200 <= resp.status < 300, "status": int(resp.status), "latency_ms": ms, "error": ""}
    except Exception as exc:
        ms = int((time.monotonic() - started) * 1000)
        return {"ok": False, "status": 0, "latency_ms": ms, "error": str(exc)}


class HqReporter:
    """Delivers structured gate/crash/candidate events to HQ (.26:8789) and Tako Telemetry (.15:18080)."""

    def __init__(
        self,
        cfg: LabConfig,
        store: LabStore,
        *,
        post_fn: Callable[[str, dict[str, Any], float], tuple[int, str]] | None = None,
    ) -> None:
        self.cfg = cfg
        self.store = store
        self._post = post_fn or default_http_post_json
        self.hostname = socket.gethostname()

    def report_hq_event(
        self,
        *,
        event_type: str,
        title: str,
        body: str,
        ok_criteria: str = "",
        priority: str = "P2",
        create_ticket: bool = True,
        tags: list[str] | None = None,
        source_ref: str = "",
        lock_key: str = "",
        run_id: str = "",
        dedupe_key: str = "",
        cooldown_sec: int | None = None,
        force: bool = False,
        template_name: str = "quick_fix",
    ) -> dict[str, Any]:
        clean_title = scrub_text(title, max_len=140)
        clean_body = scrub_text(body, max_len=3600)
        clean_ok = scrub_text(ok_criteria, max_len=600)
        eff_dedupe = dedupe_key or source_ref or f"{event_type}:{clean_title}"
        eff_cooldown = self.cfg.hq_cooldown_sec if cooldown_sec is None else cooldown_sec

        payload: dict[str, Any] = {
            "event_type": event_type,
            "title": clean_title,
            "body": clean_body,
            "ok_criteria": clean_ok,
            "source_ref": source_ref or eff_dedupe,
            "template_name": template_name,
            "priority": priority,
            "tags": sorted(set(["tako-switch", "win-lab", *(tags or [])])),
            "lock_key": lock_key,
            "create_ticket": bool(create_ticket),
        }

        if not force and create_ticket and eff_cooldown > 0:
            prev = self.store.find_recent_sent_event(eff_dedupe, eff_cooldown)
            if prev is not None:
                return self.store.record_hq_event(
                    run_id=run_id,
                    event_type=event_type,
                    channel="hq_v1_events",
                    dedupe_key=eff_dedupe,
                    priority=priority,
                    title=clean_title,
                    create_ticket=create_ticket,
                    payload=payload,
                    status="deduped",
                    http_status=0,
                    ticket_id=prev.get("ticket_id", ""),
                    response_text=f"Cooldown deduped ({eff_cooldown}s) against {prev['id']} at {prev['created_at']}",
                )

        if not self.cfg.hq_url:
            return self.store.record_hq_event(
                run_id=run_id,
                event_type=event_type,
                channel="hq_v1_events",
                dedupe_key=eff_dedupe,
                priority=priority,
                title=clean_title,
                create_ticket=create_ticket,
                payload=payload,
                status="failed",
                http_status=0,
                response_text="WIN_LAB_HQ_URL not configured",
            )

        url = f"{self.cfg.hq_url}/v1/events"
        code, resp_text = self._post(url, payload, 10.0)
        ticket_id = ""
        ok = 200 <= code < 300
        if ok:
            try:
                parsed = json.loads(resp_text)
                ticket_id = str(parsed.get("ticket_id") or "")
            except Exception:
                pass

        return self.store.record_hq_event(
            run_id=run_id,
            event_type=event_type,
            channel="hq_v1_events",
            dedupe_key=eff_dedupe,
            priority=priority,
            title=clean_title,
            create_ticket=create_ticket,
            payload=payload,
            status="sent" if ok else "failed",
            http_status=code,
            ticket_id=ticket_id,
            response_text=resp_text,
        )

    def report_run_to_telemetry(self, run: dict[str, Any]) -> dict[str, Any] | None:
        if not self.cfg.auto_report_telemetry or not self.cfg.telemetry_url:
            return None

        now_ts = int(time.time())
        app_id = str(run.get("app_id") or "unknown")
        kind = str(run.get("kind") or "gate")
        status = str(run.get("status") or "error")
        version = str(run.get("target_version") or "")
        ok = status in ("pass", "warn")

        event_payload = {
            "device_id": f"win6-lab-{self.hostname.lower()}",
            "app_version": f"win-lab-{SERVICE_VERSION}",
            "os": "windows" if platform.system().lower() == "windows" else platform.system().lower(),
            "os_version": platform.version()[:64],
            "arch": platform.machine().lower() or "x86_64",
            "events": [
                {
                    "event_name": f"win_lab_{kind}",
                    "target_app": app_id,
                    "ok": ok,
                    "duration_ms": int(run.get("duration_ms") or 0),
                    "ts": now_ts,
                    "props": {
                        "run_id": run.get("id", ""),
                        "status": status,
                        "overall_gate": run.get("overall_gate", ""),
                        "target_version": version,
                        "trigger": run.get("trigger", ""),
                    },
                }
            ],
        }
        code, resp = self._post(f"{self.cfg.telemetry_url}/v1/events", event_payload, 6.0)
        rec = self.store.record_hq_event(
            run_id=str(run.get("id") or ""),
            event_type=f"telemetry.win_lab_{kind}",
            channel="tako_telemetry",
            dedupe_key=f"tel:{run.get('id', '')}",
            priority="P3",
            title=f"Telemetry: {app_id} {kind} ({status})",
            create_ticket=False,
            payload=event_payload,
            status="sent" if 200 <= code < 300 else "failed",
            http_status=code,
            response_text=resp,
        )

        # Also push to /v1/errors on failure/crash so /telemetry/errors catches Windows regressions.
        if status in ("fail", "error"):
            smoke = run.get("smoke") or {}
            gates = run.get("gates") or {}
            err_type = "WinSmokeCrash" if kind == "smoke" else "WinReleaseGateFailed"
            stack_lines: list[str] = []
            for gname, ginfo in gates.items():
                if isinstance(ginfo, dict) and ginfo.get("status") in ("fail", "warn"):
                    stack_lines.append(f"[{gname}:{ginfo.get('status')}] " + "; ".join(ginfo.get("details") or []))
            for ev in (smoke.get("crash_events") or [])[:5]:
                stack_lines.append(f"[EventLog {ev.get('id')}] {ev.get('message', '')[:300]}")
            err_payload = {
                "device_id": f"win6-lab-{self.hostname.lower()}",
                "app_version": f"win-lab-{SERVICE_VERSION}",
                "os": "windows" if platform.system().lower() == "windows" else platform.system().lower(),
                "os_version": platform.version()[:64],
                "arch": platform.machine().lower() or "x86_64",
                "source": "win_lab",
                "category": f"win_lab_{app_id}",
                "error_type": err_type,
                "message": scrub_text(str(run.get("summary") or f"{app_id} {kind} failed"), max_len=500),
                "location": f"{app_id}:{version}:{kind}",
                "stack": scrub_text("\n".join(stack_lines), max_len=2400),
                "context": {
                    "run_id": run.get("id", ""),
                    "app_id": app_id,
                    "target_version": version,
                    "kind": kind,
                    "status": status,
                },
                "ts": now_ts,
            }
            self._post(f"{self.cfg.telemetry_url}/v1/errors", err_payload, 6.0)
        return rec

    def report_completed_run(self, run: dict[str, Any], *, force_hq: bool = False) -> dict[str, Any] | None:
        self.report_run_to_telemetry(run)
        if not self.cfg.auto_report_hq and not force_hq:
            return None

        app_id = str(run.get("app_id") or "unknown")
        kind = str(run.get("kind") or "gate")
        status = str(run.get("status") or "error")
        version = str(run.get("target_version") or "unknown")
        run_id = str(run.get("id") or "")
        summary = str(run.get("summary") or "")
        gates = run.get("gates") or {}
        smoke = run.get("smoke") or {}

        detail_lines = [
            f"Windows 测试台（河南 .6）执行完成：{app_id} {version}（模式：{kind}，结果：{status}）",
            f"摘要：{summary}",
        ]
        for gname, ginfo in gates.items():
            if isinstance(ginfo, dict):
                st = ginfo.get("status", "unknown")
                det = "；".join(str(x) for x in (ginfo.get("details") or [])[:4])
                detail_lines.append(f"- 门禁 {gname} [{st}]：{det}")
        if smoke:
            detail_lines.append(
                f"- 实机冷启动：installed={smoke.get('installed')} launched={smoke.get('launched')} "
                f"survived_sec={smoke.get('survived_sec')} crash_events={len(smoke.get('crash_events') or [])}"
            )
            for ev in (smoke.get("crash_events") or [])[:3]:
                detail_lines.append(
                    f"  - EventLog #{ev.get('id')} ({ev.get('provider', '')}): {str(ev.get('message', ''))[:240]}"
                )
        detail_lines.append(f"查看详情：{self.cfg.ops_console_url}/runs?id={run_id}")
        body_text = "\n".join(detail_lines)

        if status in ("fail", "error"):
            is_smoke = kind == "smoke"
            ev_type = "winlab.smoke_crash" if is_smoke else "winlab.gate_failed"
            title = (
                f"Windows 实机冒烟失败：{app_id} {version}"
                if is_smoke
                else f"Windows 发版门禁未通过：{app_id} {version}"
            )
            return self.report_hq_event(
                event_type=ev_type,
                title=title,
                body=body_text,
                ok_criteria=f"在 .6 Windows 测试台重新运行 {app_id} {version} 的 {kind} 验收并达到 pass。",
                priority="P1" if is_smoke else "P2",
                create_ticket=True,
                tags=[app_id, kind, "regression"],
                source_ref=f"tako-win-lab:{kind}:{app_id}:{version}",
                lock_key=f"tako-win-lab:{app_id}",
                run_id=run_id,
                dedupe_key=f"{ev_type}:{app_id}:{version}",
                force=force_hq,
            )

        # Passing or warning runs publish a lightweight SSE notification without creating noisy HQ tickets.
        return self.report_hq_event(
            event_type="winlab.gate_passed",
            title=f"Windows 验收通过：{app_id} {version}（{kind}/{status}）",
            body=body_text,
            priority="P3",
            create_ticket=False,
            tags=[app_id, kind],
            source_ref=f"tako-win-lab:{kind}:{app_id}:{version}",
            run_id=run_id,
            dedupe_key=f"winlab.gate_passed:{app_id}:{version}:{kind}",
            force=True,
        )

    def report_upstream_candidates(self, report_json: dict[str, Any], *, force: bool = False) -> list[dict[str, Any]]:
        if not self.cfg.auto_report_hq and not force:
            return []
        apps = report_json.get("apps") or {}
        emitted: list[dict[str, Any]] = []
        for app_id, info in apps.items():
            if not isinstance(info, dict) or not info.get("hasUpstreamUpdate"):
                continue
            pinned_v = str(info.get("pinnedVersion") or "")
            up_v = str(info.get("upstreamVersion") or "")
            cl = info.get("changelog") or {}
            hints = [h.get("category", "") for h in (cl.get("riskHints") or []) if isinstance(h, dict)]
            entries = cl.get("entries") or []
            excerpt = ""
            if entries and isinstance(entries[0], dict):
                excerpt = str(entries[0].get("body") or "")[:600]
            body = "\n".join(
                [
                    f"检测到桌面客户端 {app_id} 存在上游新版本（当前锁定已冻结，不会自动推送给用户）：",
                    f"- 当前锁定版本：{pinned_v}",
                    f"- 上游最新版本：{up_v}",
                    f"- 更新日志风险关键词：{', '.join(hints) if hints else '无高危词命中'}",
                    f"- 更新日志摘录：{excerpt or '上游未附正文说明，需在测试台解包差分审核'}",
                    f"- 请在 macOS 门禁与 .6 Windows 测试台（{self.cfg.ops_console_url}）完成三门禁验收后再人工 promote。",
                ]
            )
            rec = self.report_hq_event(
                event_type="winlab.upstream_candidate",
                title=f"上游发现新版待验收：{app_id} {pinned_v} -> {up_v}",
                body=body,
                ok_criteria=f"审核 {app_id} {up_v} 官方更新日志，完成 macOS 与 Windows 三门禁验收并决定是否 promote。",
                priority="P2",
                create_ticket=True,
                tags=[app_id, "upstream-candidate"],
                source_ref=f"tako-win-lab:candidate:{app_id}:{up_v}",
                lock_key=f"tako-candidate:{app_id}",
                dedupe_key=f"winlab.upstream_candidate:{app_id}:{up_v}",
                cooldown_sec=self.cfg.candidate_cooldown_sec,
                force=force,
            )
            emitted.append(rec)
        return emitted
