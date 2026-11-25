#!/usr/bin/env python3
"""Entrypoint for Tako Switch Windows Release Gate & Test Lab service (`tako-win-lab`)."""

from __future__ import annotations

import argparse
from pathlib import Path
import signal
import sys

from win_lab import SERVICE_NAME, SERVICE_VERSION
from win_lab.config import LabConfig
from win_lab.hq_reporter import HqReporter
from win_lab.runner import LabRunner
from win_lab.server import WinLabServer
from win_lab.storage import LabStore


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=f"{SERVICE_NAME} v{SERVICE_VERSION}")
    parser.add_argument("--host", default=None, help="Bind host (default env WIN_LAB_HOST or 0.0.0.0)")
    parser.add_argument("--port", type=int, default=None, help="Bind port (default env WIN_LAB_PORT or 18795)")
    parser.add_argument("--repo-root", default=None, help="Override tako-switch-v2 repo root")
    parser.add_argument("--data-dir", default=None, help="Override persistent data/cache/db directory")
    args = parser.parse_args(argv)

    cfg = LabConfig.from_env(
        repo_root_override=Path(args.repo_root) if args.repo_root else None,
        data_dir_override=Path(args.data_dir) if args.data_dir else None,
    )
    if args.host or args.port:
        cfg = LabConfig(
            **{
                **cfg.__dict__,
                "host": args.host or cfg.host,
                "port": args.port or cfg.port,
            }
        )
    cfg.ensure_dirs()

    store = LabStore(cfg.db_path)
    reporter = HqReporter(cfg, store)
    runner = LabRunner(cfg, store, reporter)
    runner.start()

    app = WinLabServer(cfg, store, reporter, runner)
    httpd = app.create_http_server()

    def _shutdown(_sig: int, _frame: object) -> None:
        runner.stop()
        raise SystemExit(0)

    signal.signal(signal.SIGINT, _shutdown)
    signal.signal(signal.SIGTERM, _shutdown)

    print(
        f"[{SERVICE_NAME}] v{SERVICE_VERSION} listening on http://{cfg.host}:{cfg.port} "
        f"(repo={cfg.repo_root}, data={cfg.data_dir}, hq={cfg.hq_url})",
        flush=True,
    )
    try:
        httpd.serve_forever()
    except SystemExit:
        pass
    finally:
        runner.stop()
        httpd.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
