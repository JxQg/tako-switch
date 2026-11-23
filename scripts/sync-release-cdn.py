#!/usr/bin/env python3
"""Upload Tako Switch release binaries + China latest.json to the origins behind dl.lven.asia.

Two upload targets run when configured:
- Cloudflare R2   (R2_BUCKET / R2_ENDPOINT_URL + ambient AWS_* credentials)
- DogeCloud OSS   (DOGE_ACCESS_KEY / DOGE_SECRET_KEY / DOGE_OSS_BUCKET, exchanged for STS creds)

release.yml uses this in two places:
- normal publish: binaries come from the fresh build artifacts in release-files/
- `resync_tag` repair mode: binaries are re-downloaded from an existing GitHub Release
  and re-pushed to both buckets (fixes releases shipped before DogeCloud sync existed).
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import sys
import time
import urllib.request

MIRROR_MOD_PATH = Path(__file__).resolve().parent / "mirror-to-r2.py"

RELEASES_PREFIX = "tako-switch/releases"
LATEST_JSON_KEY = "tako-switch/latest.json"
BINARY_CACHE_CONTROL = "public, max-age=31536000, immutable"
LATEST_CACHE_CONTROL = "public, max-age=120"
LATEST_CONTENT_TYPE = "application/json; charset=utf-8"


def load_mirror_module():
    spec = importlib.util.spec_from_file_location("mirror_to_r2", MIRROR_MOD_PATH)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def iter_release_files(release_dir: Path) -> list[Path]:
    """All files to ship under tako-switch/releases/<tag>/ (latest.json excluded —
    it is the mutable pointer and is uploaded to LATEST_JSON_KEY separately)."""
    files = [
        p for p in sorted(release_dir.iterdir()) if p.is_file() and p.name != "latest.json"
    ]
    if not files:
        raise RuntimeError(f"no release files found in {release_dir}")
    return files


def sync_release_to_bucket(s3, *, tag: str, files: list[Path], latest_cn: Path) -> list[str]:
    uploaded: list[str] = []
    for local in files:
        key = f"{RELEASES_PREFIX}/{tag}/{local.name}"
        s3.upload_file(local, key, cache_control=BINARY_CACHE_CONTROL)
        uploaded.append(key)
    s3.upload_file(
        latest_cn,
        LATEST_JSON_KEY,
        cache_control=LATEST_CACHE_CONTROL,
        content_type=LATEST_CONTENT_TYPE,
    )
    uploaded.append(LATEST_JSON_KEY)
    return uploaded


def verify_platform_urls(
    latest_cn: Path,
    *,
    retries: int = 6,
    delay: float = 30.0,
    opener=urllib.request.urlopen,
) -> list[str]:
    """HEAD every platform url in latest-cn.json until all return 2xx (retries absorb
    negative-cache TTLs on the CDN edge). Raises RuntimeError on final failure."""
    data = json.loads(latest_cn.read_text(encoding="utf-8"))
    urls = [
        str(plat["url"])
        for plat in (data.get("platforms") or {}).values()
        if plat.get("url")
    ]
    if not urls:
        raise RuntimeError(f"no platform urls found in {latest_cn}")

    bad = list(urls)
    for attempt in range(1, max(1, retries) + 1):
        still_bad: list[str] = []
        for url in bad:
            ok = False
            try:
                req = urllib.request.Request(url, method="HEAD")
                with opener(req, timeout=30) as resp:
                    ok = 200 <= resp.status < 300
            except Exception:
                ok = False
            if not ok:
                still_bad.append(url)
        bad = still_bad
        if not bad:
            return urls
        if attempt < max(1, retries):
            print(
                f"[verify] {len(bad)}/{len(urls)} urls not ready; retry in {delay:.0f}s "
                f"(attempt {attempt}/{retries})",
                file=sys.stderr,
            )
            time.sleep(delay)
    raise RuntimeError("CDN verification failed for: " + ", ".join(sorted(bad)))


def build_targets(mod, required: set[str]) -> list[tuple[str, object]]:
    targets: list[tuple[str, object]] = []

    r2_bucket = os.environ.get("R2_BUCKET", "").strip()
    r2_endpoint = os.environ.get("R2_ENDPOINT_URL", "").strip()
    if r2_bucket and r2_endpoint:
        region = mod.infer_s3_region(r2_endpoint, os.environ.get("AWS_DEFAULT_REGION"))
        targets.append(("r2", mod.S3Client(bucket=r2_bucket, endpoint_url=r2_endpoint, region=region)))
    elif "r2" in required:
        raise RuntimeError("R2 target required but R2_BUCKET/R2_ENDPOINT_URL are not configured")

    doge_ak = os.environ.get("DOGE_ACCESS_KEY", "").strip()
    doge_sk = os.environ.get("DOGE_SECRET_KEY", "").strip()
    doge_bucket = os.environ.get("DOGE_OSS_BUCKET", "").strip()
    if doge_ak and doge_sk and doge_bucket:
        print(f"[auth] exchanging DogeCloud STS token for OSS space '{doge_bucket}'")
        sb, ep, reg, env = mod.fetch_dogecloud_s3_credentials(doge_ak, doge_sk, doge_bucket)
        print(f"[auth] DogeCloud S3 target: s3://{sb} ({ep}, region={reg})")
        # Cross-border runner -> Shanghai COS aborts multipart UploadPart calls with
        # UserNetworkTooSlow; release assets are ~10 MB so a single PUT is more reliable.
        targets.append(("dogecloud", mod.S3Client(
            bucket=sb,
            endpoint_url=ep,
            region=reg,
            env_overrides=env,
            multipart_threshold_mb=64,
            max_concurrency=1,
        )))
    elif "doge" in required:
        raise RuntimeError(
            "DogeCloud target required but DOGE_ACCESS_KEY/DOGE_SECRET_KEY/DOGE_OSS_BUCKET are not configured"
        )
    elif r2_bucket and r2_endpoint:
        print(
            "[warn] DOGE_* secrets not configured; skipping direct DogeCloud OSS upload "
            "(dl.lven.asia release objects may 404 without origin-pull)",
            file=sys.stderr,
        )
    return targets


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True, help="Release tag, e.g. v2.2.7")
    parser.add_argument("--release-dir", default="release-files", help="Directory holding release binaries")
    parser.add_argument("--latest-cn", default="latest-cn.json", help="China latest.json pointing at dl.lven.asia")
    parser.add_argument(
        "--require",
        action="append",
        choices=["r2", "doge"],
        default=[],
        help="Fail if the given target's credentials are not configured (repeatable)",
    )
    parser.add_argument("--verify", action="store_true", help="HEAD-verify every platform url after upload")
    parser.add_argument("--verify-retries", type=int, default=6)
    parser.add_argument("--verify-delay", type=float, default=30.0)
    args = parser.parse_args(argv)

    release_dir = Path(args.release_dir)
    latest_cn = Path(args.latest_cn)
    files = iter_release_files(release_dir)
    if not latest_cn.is_file():
        raise RuntimeError(f"{latest_cn} does not exist")

    mod = load_mirror_module()
    targets = build_targets(mod, set(args.require))
    if not targets:
        print("No CDN targets configured (R2_* / DOGE_*); skipping sync.", file=sys.stderr)
        return 0

    for name, s3 in targets:
        print(f"=== [{name}] uploading {len(files)} release files for {args.tag} to s3://{s3.bucket}")
        for key in sync_release_to_bucket(s3, tag=args.tag, files=files, latest_cn=latest_cn):
            print(f"  [{name}] uploaded {key}")

    if args.verify:
        urls = verify_platform_urls(latest_cn, retries=args.verify_retries, delay=args.verify_delay)
        print(f"[verify] all {len(urls)} platform urls reachable")
        for url in urls:
            print(f"  [ok] {url}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
