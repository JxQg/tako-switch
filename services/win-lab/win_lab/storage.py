"""SQLite WAL storage for test runs, HQ event outbox, and upstream snapshots."""

from __future__ import annotations

from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
import json
from pathlib import Path
import sqlite3
import threading
import uuid
from typing import Any, Iterator

from . import APP_ORDER


def utc_now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def new_short_id(prefix: str = "") -> str:
    tok = uuid.uuid4().hex[:10]
    return f"{prefix}{tok}" if prefix else tok


class LabStore:
    def __init__(self, db_path: Path) -> None:
        self.db_path = db_path
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        self._lock = threading.RLock()
        self._init_schema()

    @contextmanager
    def _connect(self) -> Iterator[sqlite3.Connection]:
        conn = sqlite3.connect(str(self.db_path), timeout=15.0)
        conn.row_factory = sqlite3.Row
        try:
            conn.execute("PRAGMA journal_mode=WAL")
            conn.execute("PRAGMA synchronous=NORMAL")
            conn.execute("PRAGMA foreign_keys=ON")
            with conn:
                yield conn
        finally:
            conn.close()

    def _init_schema(self) -> None:
        with self._lock, self._connect() as conn:
            conn.executescript(
                """
                CREATE TABLE IF NOT EXISTS test_runs (
                    id TEXT PRIMARY KEY,
                    kind TEXT NOT NULL,
                    app_id TEXT NOT NULL,
                    target_version TEXT NOT NULL DEFAULT '',
                    source_url TEXT NOT NULL DEFAULT '',
                    trigger TEXT NOT NULL DEFAULT 'manual',
                    status TEXT NOT NULL DEFAULT 'queued',
                    overall_gate TEXT NOT NULL DEFAULT 'pending',
                    started_at TEXT NOT NULL,
                    finished_at TEXT NOT NULL DEFAULT '',
                    duration_ms INTEGER NOT NULL DEFAULT 0,
                    summary TEXT NOT NULL DEFAULT '',
                    gates_json TEXT NOT NULL DEFAULT '{}',
                    smoke_json TEXT NOT NULL DEFAULT '{}',
                    artifacts_json TEXT NOT NULL DEFAULT '{}',
                    hq_event_id TEXT NOT NULL DEFAULT '',
                    hq_ticket_id TEXT NOT NULL DEFAULT '',
                    log_text TEXT NOT NULL DEFAULT ''
                );

                CREATE INDEX IF NOT EXISTS idx_runs_started ON test_runs(started_at DESC);
                CREATE INDEX IF NOT EXISTS idx_runs_app_kind ON test_runs(app_id, kind, started_at DESC);

                CREATE TABLE IF NOT EXISTS hq_events (
                    id TEXT PRIMARY KEY,
                    run_id TEXT NOT NULL DEFAULT '',
                    event_type TEXT NOT NULL,
                    channel TEXT NOT NULL,
                    dedupe_key TEXT NOT NULL DEFAULT '',
                    priority TEXT NOT NULL DEFAULT 'P2',
                    title TEXT NOT NULL,
                    create_ticket INTEGER NOT NULL DEFAULT 0,
                    payload_json TEXT NOT NULL DEFAULT '{}',
                    status TEXT NOT NULL DEFAULT 'pending',
                    http_status INTEGER NOT NULL DEFAULT 0,
                    ticket_id TEXT NOT NULL DEFAULT '',
                    response_text TEXT NOT NULL DEFAULT '',
                    created_at TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_hq_created ON hq_events(created_at DESC);
                CREATE INDEX IF NOT EXISTS idx_hq_dedupe ON hq_events(dedupe_key, status, created_at DESC);

                CREATE TABLE IF NOT EXISTS upstream_snapshots (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    checked_at TEXT NOT NULL,
                    has_updates INTEGER NOT NULL DEFAULT 0,
                    summary TEXT NOT NULL DEFAULT '',
                    report_json TEXT NOT NULL DEFAULT '{}',
                    report_md TEXT NOT NULL DEFAULT ''
                );

                CREATE INDEX IF NOT EXISTS idx_upstream_checked ON upstream_snapshots(checked_at DESC);

                CREATE TABLE IF NOT EXISTS kv_meta (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                """
            )
            # Recover any stale "running" runs left over from an unexpected process crash.
            conn.execute(
                """
                UPDATE test_runs
                SET status = 'error',
                    overall_gate = CASE WHEN overall_gate = 'pending' THEN 'error' ELSE overall_gate END,
                    finished_at = ?,
                    summary = CASE
                        WHEN summary = '' THEN '服务重启时任务中断，已自动回收为 error'
                        ELSE summary
                    END
                WHERE status IN ('queued', 'running')
                """,
                (utc_now_iso(),),
            )

    def ping(self) -> bool:
        try:
            with self._lock, self._connect() as conn:
                conn.execute("SELECT 1").fetchone()
            return True
        except Exception:
            return False

    def db_size_bytes(self) -> int:
        try:
            return self.db_path.stat().st_size
        except OSError:
            return 0

    def set_meta(self, key: str, value: str) -> None:
        with self._lock, self._connect() as conn:
            conn.execute(
                """
                INSERT INTO kv_meta(key, value, updated_at)
                VALUES(?, ?, ?)
                ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at
                """,
                (key, value, utc_now_iso()),
            )

    def get_meta(self, key: str, default: str = "") -> str:
        with self._lock, self._connect() as conn:
            row = conn.execute("SELECT value FROM kv_meta WHERE key=?", (key,)).fetchone()
            return str(row["value"]) if row else default

    # --- Test Runs ---

    def create_run(
        self,
        *,
        kind: str,
        app_id: str,
        target_version: str = "",
        source_url: str = "",
        trigger: str = "manual",
    ) -> dict[str, Any]:
        run_id = new_short_id("run_")
        now = utc_now_iso()
        with self._lock, self._connect() as conn:
            conn.execute(
                """
                INSERT INTO test_runs(
                    id, kind, app_id, target_version, source_url, trigger,
                    status, overall_gate, started_at
                ) VALUES (?, ?, ?, ?, ?, ?, 'queued', 'pending', ?)
                """,
                (run_id, kind, app_id, target_version, source_url, trigger, now),
            )
            row = conn.execute("SELECT * FROM test_runs WHERE id=?", (run_id,)).fetchone()
        return self._row_to_run(row)

    def update_run(self, run_id: str, **fields: Any) -> dict[str, Any] | None:
        allowed = {
            "target_version",
            "source_url",
            "status",
            "overall_gate",
            "finished_at",
            "duration_ms",
            "summary",
            "gates_json",
            "smoke_json",
            "artifacts_json",
            "hq_event_id",
            "hq_ticket_id",
            "log_text",
        }
        updates: list[str] = []
        params: list[Any] = []
        for k, v in fields.items():
            if k not in allowed:
                continue
            if k.endswith("_json") and not isinstance(v, str):
                v = json.dumps(v, ensure_ascii=False)
            updates.append(f"{k}=?")
            params.append(v)
        if not updates:
            return self.get_run(run_id)
        params.append(run_id)
        with self._lock, self._connect() as conn:
            conn.execute(f"UPDATE test_runs SET {', '.join(updates)} WHERE id=?", params)
            row = conn.execute("SELECT * FROM test_runs WHERE id=?", (run_id,)).fetchone()
        return self._row_to_run(row) if row else None

    def get_run(self, run_id: str) -> dict[str, Any] | None:
        with self._lock, self._connect() as conn:
            row = conn.execute("SELECT * FROM test_runs WHERE id=?", (run_id,)).fetchone()
        return self._row_to_run(row) if row else None

    def list_runs(
        self,
        *,
        app_id: str = "",
        kind: str = "",
        status: str = "",
        limit: int = 50,
    ) -> list[dict[str, Any]]:
        limit = max(1, min(200, int(limit)))
        clauses: list[str] = []
        params: list[Any] = []
        if app_id:
            clauses.append("app_id=?")
            params.append(app_id)
        if kind:
            clauses.append("kind=?")
            params.append(kind)
        if status:
            clauses.append("status=?")
            params.append(status)
        where = f"WHERE {' AND '.join(clauses)}" if clauses else ""
        params.append(limit)
        with self._lock, self._connect() as conn:
            rows = conn.execute(
                f"SELECT * FROM test_runs {where} ORDER BY started_at DESC LIMIT ?",
                params,
            ).fetchall()
        return [self._row_to_run(r, include_logs=False) for r in rows]

    def latest_runs_by_app(self) -> dict[str, dict[str, Any]]:
        out: dict[str, dict[str, Any]] = {}
        with self._lock, self._connect() as conn:
            for app_id in APP_ORDER:
                gate_row = conn.execute(
                    """
                    SELECT * FROM test_runs
                    WHERE app_id=? AND kind='gate' AND status NOT IN ('queued', 'running')
                    ORDER BY started_at DESC LIMIT 1
                    """,
                    (app_id,),
                ).fetchone()
                smoke_row = conn.execute(
                    """
                    SELECT * FROM test_runs
                    WHERE app_id=? AND kind='smoke' AND status NOT IN ('queued', 'running')
                    ORDER BY started_at DESC LIMIT 1
                    """,
                    (app_id,),
                ).fetchone()
                out[app_id] = {
                    "gate": self._row_to_run(gate_row, include_logs=False) if gate_row else None,
                    "smoke": self._row_to_run(smoke_row, include_logs=False) if smoke_row else None,
                }
        return out

    def count_active_runs(self) -> int:
        with self._lock, self._connect() as conn:
            row = conn.execute(
                "SELECT COUNT(*) AS cnt FROM test_runs WHERE status IN ('queued', 'running')"
            ).fetchone()
            return int(row["cnt"]) if row else 0

    def _row_to_run(self, row: sqlite3.Row, *, include_logs: bool = True) -> dict[str, Any]:
        def _parse_json(raw: str) -> Any:
            try:
                return json.loads(raw) if raw else {}
            except Exception:
                return {}

        obj = {
            "id": str(row["id"]),
            "kind": str(row["kind"]),
            "app_id": str(row["app_id"]),
            "target_version": str(row["target_version"] or ""),
            "source_url": str(row["source_url"] or ""),
            "trigger": str(row["trigger"] or ""),
            "status": str(row["status"]),
            "overall_gate": str(row["overall_gate"]),
            "started_at": str(row["started_at"]),
            "finished_at": str(row["finished_at"] or ""),
            "duration_ms": int(row["duration_ms"] or 0),
            "summary": str(row["summary"] or ""),
            "gates": _parse_json(str(row["gates_json"] or "{}")),
            "smoke": _parse_json(str(row["smoke_json"] or "{}")),
            "artifacts": _parse_json(str(row["artifacts_json"] or "{}")),
            "hq_event_id": str(row["hq_event_id"] or ""),
            "hq_ticket_id": str(row["hq_ticket_id"] or ""),
        }
        if include_logs:
            obj["log_text"] = str(row["log_text"] or "")
        return obj

    # --- HQ Event Outbox ---

    def find_recent_sent_event(self, dedupe_key: str, cooldown_sec: int) -> dict[str, Any] | None:
        if not dedupe_key or cooldown_sec <= 0:
            return None
        cutoff = (datetime.now(timezone.utc) - timedelta(seconds=cooldown_sec)).strftime("%Y-%m-%dT%H:%M:%SZ")
        with self._lock, self._connect() as conn:
            row = conn.execute(
                """
                SELECT * FROM hq_events
                WHERE dedupe_key=? AND status='sent' AND created_at >= ?
                ORDER BY created_at DESC LIMIT 1
                """,
                (dedupe_key, cutoff),
            ).fetchone()
        return self._row_to_hq_event(row) if row else None

    def record_hq_event(
        self,
        *,
        run_id: str,
        event_type: str,
        channel: str,
        dedupe_key: str,
        priority: str,
        title: str,
        create_ticket: bool,
        payload: dict[str, Any],
        status: str,
        http_status: int = 0,
        ticket_id: str = "",
        response_text: str = "",
    ) -> dict[str, Any]:
        ev_id = new_short_id("hqe_")
        now = utc_now_iso()
        with self._lock, self._connect() as conn:
            conn.execute(
                """
                INSERT INTO hq_events(
                    id, run_id, event_type, channel, dedupe_key, priority, title,
                    create_ticket, payload_json, status, http_status, ticket_id,
                    response_text, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    ev_id,
                    run_id,
                    event_type,
                    channel,
                    dedupe_key,
                    priority,
                    title,
                    1 if create_ticket else 0,
                    json.dumps(payload, ensure_ascii=False),
                    status,
                    int(http_status),
                    ticket_id,
                    response_text[:4000],
                    now,
                ),
            )
            row = conn.execute("SELECT * FROM hq_events WHERE id=?", (ev_id,)).fetchone()
        return self._row_to_hq_event(row)

    def list_hq_events(self, *, limit: int = 60, channel: str = "") -> list[dict[str, Any]]:
        limit = max(1, min(200, int(limit)))
        params: list[Any] = []
        where = ""
        if channel:
            where = "WHERE channel=?"
            params.append(channel)
        params.append(limit)
        with self._lock, self._connect() as conn:
            rows = conn.execute(
                f"SELECT * FROM hq_events {where} ORDER BY created_at DESC LIMIT ?",
                params,
            ).fetchall()
        return [self._row_to_hq_event(r) for r in rows]

    def count_hq_events_last_24h(self) -> dict[str, int]:
        cutoff = (datetime.now(timezone.utc) - timedelta(hours=24)).strftime("%Y-%m-%dT%H:%M:%SZ")
        with self._lock, self._connect() as conn:
            rows = conn.execute(
                """
                SELECT status, COUNT(*) AS cnt
                FROM hq_events
                WHERE created_at >= ? AND channel = 'hq_v1_events'
                GROUP BY status
                """,
                (cutoff,),
            ).fetchall()
        counts = {"sent": 0, "deduped": 0, "failed": 0, "total": 0}
        for r in rows:
            st = str(r["status"])
            c = int(r["cnt"])
            counts["total"] += c
            if st in counts:
                counts[st] += c
        return counts

    def _row_to_hq_event(self, row: sqlite3.Row) -> dict[str, Any]:
        try:
            payload = json.loads(str(row["payload_json"] or "{}"))
        except Exception:
            payload = {}
        return {
            "id": str(row["id"]),
            "run_id": str(row["run_id"] or ""),
            "event_type": str(row["event_type"]),
            "channel": str(row["channel"]),
            "dedupe_key": str(row["dedupe_key"] or ""),
            "priority": str(row["priority"]),
            "title": str(row["title"]),
            "create_ticket": bool(row["create_ticket"]),
            "payload": payload,
            "status": str(row["status"]),
            "http_status": int(row["http_status"] or 0),
            "ticket_id": str(row["ticket_id"] or ""),
            "response_text": str(row["response_text"] or ""),
            "created_at": str(row["created_at"]),
        }

    # --- Upstream Candidate Snapshots ---

    def save_upstream_snapshot(
        self,
        *,
        has_updates: bool,
        summary: str,
        report_json: dict[str, Any],
        report_md: str,
    ) -> dict[str, Any]:
        now = utc_now_iso()
        with self._lock, self._connect() as conn:
            cur = conn.execute(
                """
                INSERT INTO upstream_snapshots(checked_at, has_updates, summary, report_json, report_md)
                VALUES (?, ?, ?, ?, ?)
                """,
                (
                    now,
                    1 if has_updates else 0,
                    summary,
                    json.dumps(report_json, ensure_ascii=False),
                    report_md,
                ),
            )
            # Keep last 30 snapshots to prevent unbounded growth.
            conn.execute(
                """
                DELETE FROM upstream_snapshots
                WHERE id NOT IN (SELECT id FROM upstream_snapshots ORDER BY id DESC LIMIT 30)
                """
            )
            row = conn.execute("SELECT * FROM upstream_snapshots WHERE id=?", (cur.lastrowid,)).fetchone()
        return self._row_to_snapshot(row)

    def latest_upstream_snapshot(self) -> dict[str, Any] | None:
        with self._lock, self._connect() as conn:
            row = conn.execute(
                "SELECT * FROM upstream_snapshots ORDER BY id DESC LIMIT 1"
            ).fetchone()
        return self._row_to_snapshot(row) if row else None

    def _row_to_snapshot(self, row: sqlite3.Row) -> dict[str, Any]:
        try:
            rep = json.loads(str(row["report_json"] or "{}"))
        except Exception:
            rep = {}
        return {
            "id": int(row["id"]),
            "checked_at": str(row["checked_at"]),
            "has_updates": bool(row["has_updates"]),
            "summary": str(row["summary"] or ""),
            "report": rep,
            "report_md": str(row["report_md"] or ""),
        }
