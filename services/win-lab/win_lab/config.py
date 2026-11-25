"""Configuration loader for tako-win-lab (env + sensible fleet defaults)."""

from __future__ import annotations

from dataclasses import dataclass
import os
from pathlib import Path


def _env_bool(name: str, default: bool) -> bool:
    raw = os.environ.get(name)
    if raw is None or not raw.strip():
        return default
    return raw.strip().lower() in ("1", "true", "yes", "on")


def _env_int(name: str, default: int, minimum: int = 0) -> int:
    raw = os.environ.get(name)
    if raw is None or not raw.strip():
        return default
    try:
        return max(minimum, int(raw.strip()))
    except ValueError:
        return default


@dataclass(frozen=True)
class LabConfig:
    host: str
    port: int
    repo_root: Path
    data_dir: Path
    cache_dir: Path
    work_dir: Path
    db_path: Path
    hq_url: str
    telemetry_url: str
    manifest_cdn_url: str
    auto_report_hq: bool
    auto_report_telemetry: bool
    hq_cooldown_sec: int
    candidate_cooldown_sec: int
    schedule_enabled: bool
    upstream_check_interval_min: int
    pinned_gate_interval_min: int
    allow_live_smoke: bool
    ops_console_url: str

    @classmethod
    def from_env(cls, *, repo_root_override: Path | None = None, data_dir_override: Path | None = None) -> "LabConfig":
        default_repo = Path(__file__).resolve().parents[3]
        repo_root = (
            repo_root_override
            or Path(os.environ.get("WIN_LAB_REPO_ROOT") or default_repo)
        ).resolve()

        default_data = repo_root / "services" / "win-lab" / ".data"
        data_dir = (
            data_dir_override
            or Path(os.environ.get("WIN_LAB_DATA_DIR") or default_data)
        ).resolve()
        cache_dir = Path(os.environ.get("WIN_LAB_CACHE_DIR") or (data_dir / "cache")).resolve()
        work_dir = Path(os.environ.get("WIN_LAB_WORK_DIR") or (data_dir / "work")).resolve()
        db_path = Path(os.environ.get("WIN_LAB_DB_PATH") or (data_dir / "win-lab.db")).resolve()

        return cls(
            host=os.environ.get("WIN_LAB_HOST", "0.0.0.0").strip() or "0.0.0.0",
            port=_env_int("WIN_LAB_PORT", 18795, minimum=1),
            repo_root=repo_root,
            data_dir=data_dir,
            cache_dir=cache_dir,
            work_dir=work_dir,
            db_path=db_path,
            hq_url=os.environ.get("WIN_LAB_HQ_URL", "http://172.17.66.26:8789").rstrip("/"),
            telemetry_url=os.environ.get("WIN_LAB_TELEMETRY_URL", "http://172.17.66.15:18080").rstrip("/"),
            manifest_cdn_url=os.environ.get(
                "WIN_LAB_MANIFEST_CDN_URL", "https://dl.lven.asia/tako-switch/manifest.json"
            ).strip(),
            auto_report_hq=_env_bool("WIN_LAB_AUTO_REPORT_HQ", True),
            auto_report_telemetry=_env_bool("WIN_LAB_AUTO_REPORT_TELEMETRY", True),
            hq_cooldown_sec=_env_int("WIN_LAB_HQ_COOLDOWN_SEC", 6 * 3600, minimum=0),
            candidate_cooldown_sec=_env_int("WIN_LAB_CANDIDATE_COOLDOWN_SEC", 24 * 3600, minimum=0),
            schedule_enabled=_env_bool("WIN_LAB_SCHEDULE_ENABLED", True),
            upstream_check_interval_min=_env_int("WIN_LAB_UPSTREAM_INTERVAL_MIN", 360, minimum=15),
            pinned_gate_interval_min=_env_int("WIN_LAB_GATE_INTERVAL_MIN", 1440, minimum=30),
            allow_live_smoke=_env_bool("WIN_LAB_ALLOW_LIVE_SMOKE", True),
            ops_console_url=os.environ.get(
                "WIN_LAB_OPS_CONSOLE_URL", "http://172.17.66.26:8767/tako/win-lab"
            ).rstrip("/"),
        )

    @property
    def pinned_path(self) -> Path:
        return self.repo_root / "manifest" / "pinned-versions.json"

    @property
    def manifest_path(self) -> Path:
        return self.repo_root / "manifest" / "manifest.json"

    @property
    def baselines_dir(self) -> Path:
        return self.repo_root / "manifest" / "baselines"

    def ensure_dirs(self) -> None:
        self.data_dir.mkdir(parents=True, exist_ok=True)
        self.cache_dir.mkdir(parents=True, exist_ok=True)
        self.work_dir.mkdir(parents=True, exist_ok=True)
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
