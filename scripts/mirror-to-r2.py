#!/usr/bin/env python3
"""Mirror upstream desktop installers referenced by manifest.json into an S3-compatible bucket (Cloudflare R2).

Design principles:
- Zero third-party Python dependencies (stdlib + `aws` CLI + `curl` preinstalled on GitHub-hosted runners).
- Idempotent & incremental: skips download/upload if the exact object key already exists in R2.
- Safe manifest switching: only rewrites an artifact's `url` to the China CDN mirror (keeping the official
  URL as `fallbackUrl`) AFTER confirming the object exists in R2. Never publishes broken 404 mirror URLs.
- Automatic retention pruning: keeps only the newest N versions per (app, platform) prefix so storage stays
  well within Cloudflare R2's 10 GB free tier.
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request


DEFAULT_MIRROR_APPS = ("dsh-tako", "claude", "antigravity")
DEFAULT_KEY_PREFIX = "apps"
DEFAULT_KEEP_VERSIONS = 2

# Hard per-attempt timeouts for aws CLI subprocess calls. Cross-border half-open TCP
# connections can otherwise hang for hours with zero error output.
API_TIMEOUT_SECONDS = 60  # head-object / list-objects-v2 / delete-object
UPLOAD_TIMEOUT_SECONDS = 300  # baseline for a single PUT attempt
# Upload timeout scales with file size so large installers do not die spuriously:
# assume a worst-case sustained throughput floor of ~256 KiB/s across the Pacific.
UPLOAD_TIMEOUT_FLOOR_BPS = 256 * 1024
AWS_MAX_ATTEMPTS = 3  # 1 try + up to 2 retries for timed-out operations


def parse_app_filter(raw: str | None) -> set[str]:
    if not raw or not raw.strip():
        return set(DEFAULT_MIRROR_APPS)
    return {item.strip().lower() for item in raw.split(",") if item.strip()}


def build_object_key(prefix: str, app_id: str, plat: str, version: str, official_url: str) -> str:
    basename = os.path.basename(urllib.parse.urlparse(official_url).path)
    if not basename:
        raise ValueError(f"Cannot infer filename from URL: {official_url}")
    clean_prefix = prefix.strip("/")
    if clean_prefix:
        return f"{clean_prefix}/{app_id}/{plat}/{version}/{basename}"
    return f"{app_id}/{plat}/{version}/{basename}"


def build_mirror_url(mirror_base: str, app_id: str, plat: str, version: str, official_url: str) -> str:
    basename = os.path.basename(urllib.parse.urlparse(official_url).path)
    if not basename:
        raise ValueError(f"Cannot infer filename from URL: {official_url}")
    return f"{mirror_base.rstrip('/')}/{app_id}/{plat}/{version}/{basename}"


def select_keys_to_prune(
    objects: list[dict],
    current_version: str,
    keep_versions: int,
) -> list[str]:
    """Given S3 list-objects-v2 `Contents` under `<prefix>/<app>/<plat>/`, return keys to delete.

    Groups objects by their version directory segment immediately under `<prefix>/<app>/<plat>/`,
    always retaining `current_version` plus the most recently modified older versions up to `keep_versions`.
    """
    keep = max(1, keep_versions)
    by_version: dict[str, tuple[str, list[str]]] = {}
    for obj in objects:
        key = str(obj.get("Key") or "")
        if not key or key.endswith("/"):
            continue
        # Expected shape: .../<app>/<plat>/<version>/<filename>
        parts = key.split("/")
        if len(parts) < 2:
            continue
        ver = parts[-2]
        last_mod = str(obj.get("LastModified") or "")
        prev_mod, keys = by_version.get(ver, ("", []))
        keys.append(key)
        by_version[ver] = (max(prev_mod, last_mod), keys)

    if len(by_version) <= keep:
        return []

    retained: set[str] = set()
    if current_version in by_version:
        retained.add(current_version)

    others = sorted(
        ((ver, meta[0]) for ver, meta in by_version.items() if ver not in retained),
        key=lambda item: (item[1], item[0]),
        reverse=True,
    )
    for ver, _ in others:
        if len(retained) >= keep:
            break
        retained.add(ver)

    to_delete: list[str] = []
    for ver, (_, keys) in by_version.items():
        if ver not in retained:
            to_delete.extend(sorted(keys))
    return sorted(to_delete)


def compute_file_hashes(path: Path) -> tuple[int, str, str]:
    sha256 = hashlib.sha256()
    sha512 = hashlib.sha512()
    total = 0
    with path.open("rb") as f:
        while True:
            chunk = f.read(1024 * 1024)
            if not chunk:
                break
            total += len(chunk)
            sha256.update(chunk)
            sha512.update(chunk)
    return total, sha256.hexdigest(), sha512.hexdigest()


def infer_s3_region(endpoint_url: str, explicit_region: str | None = None) -> str:
    if explicit_region and explicit_region.strip():
        return explicit_region.strip()
    m = re.search(r"cos\.([a-z0-9-]+)\.myqcloud\.com", endpoint_url, re.IGNORECASE)
    if m:
        return m.group(1)
    m = re.search(r"oss-([a-z0-9-]+)\.aliyuncs\.com", endpoint_url, re.IGNORECASE)
    if m:
        return f"oss-{m.group(1)}"
    return "auto"


def fetch_dogecloud_s3_credentials(
    access_key: str,
    secret_key: str,
    bucket_name: str,
) -> tuple[str, str, str, dict[str, str]]:
    """Exchange DogeCloud master AK/SK for temporary S3-compatible STS credentials."""
    api_path = "/auth/tmp_token.json"
    body = json.dumps(
        {"channel": "OSS_FULL", "scopes": [f"{bucket_name}:*"]},
        separators=(",", ":"),
    )
    sign_str = f"{api_path}\n{body}"
    sign = hmac.new(
        secret_key.encode("utf-8"),
        sign_str.encode("utf-8"),
        hashlib.sha1,
    ).hexdigest()
    req = urllib.request.Request(
        f"https://api.dogecloud.com{api_path}",
        data=body.encode("utf-8"),
        headers={
            "Content-Type": "application/json",
            "Authorization": f"TOKEN {access_key}:{sign}",
        },
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=20) as resp:
        payload = json.loads(resp.read().decode("utf-8"))
    if payload.get("code") != 200:
        raise RuntimeError(f"DogeCloud tmp_token failed: {payload.get('msg') or payload}")
    data = payload.get("data") or {}
    creds = data.get("Credentials") or {}
    buckets = data.get("Buckets") or []
    if not creds.get("accessKeyId") or not creds.get("secretAccessKey") or not buckets:
        raise RuntimeError("DogeCloud tmp_token response missing Credentials or Buckets")
    target = next((b for b in buckets if b.get("name") == bucket_name), buckets[0])
    s3_bucket = str(target.get("s3Bucket") or "").strip()
    s3_endpoint = str(target.get("s3Endpoint") or "").strip()
    if not s3_bucket or not s3_endpoint:
        raise RuntimeError(f"DogeCloud bucket metadata incomplete: {target}")
    region = infer_s3_region(s3_endpoint)
    env_overrides = {
        "AWS_ACCESS_KEY_ID": str(creds["accessKeyId"]),
        "AWS_SECRET_ACCESS_KEY": str(creds["secretAccessKey"]),
        "AWS_SESSION_TOKEN": str(creds.get("sessionToken") or ""),
        "AWS_DEFAULT_REGION": region,
        "AWS_REQUEST_CHECKSUM_CALCULATION": "when_required",
        "AWS_RESPONSE_CHECKSUM_VALIDATION": "when_required",
    }
    return s3_bucket, s3_endpoint, region, env_overrides


class S3Client:
    def __init__(
        self,
        bucket: str,
        endpoint_url: str,
        region: str = "auto",
        env_overrides: dict[str, str] | None = None,
        multipart_threshold_mb: int = 5,
        max_concurrency: int = 2,
    ) -> None:
        self.bucket = bucket
        self.endpoint_url = endpoint_url
        self.region = region
        self.env = os.environ.copy()
        self.env.setdefault("AWS_REQUEST_CHECKSUM_CALCULATION", "when_required")
        self.env.setdefault("AWS_RESPONSE_CHECKSUM_VALIDATION", "when_required")
        if env_overrides:
            self.env.update({k: v for k, v in env_overrides.items() if v is not None})
        lower_ep = endpoint_url.lower()
        if "myqcloud.com" in lower_ep or "aliyuncs.com" in lower_ep:
            # Tencent COS (DogeCloud OSS) and Aliyun OSS require virtual-hosted-style addressing.
            # Keep multipart chunk size at the 5MB S3 minimum and concurrency low so cross-border
            # GitHub runners do not trigger COS UserNetworkTooSlow during peak hours. Callers may
            # raise multipart_threshold to force a single PUT for files that are small enough to
            # survive a slow trans-Pacific link in one request (UploadPart gets aborted by COS).
            cfg_file = tempfile.NamedTemporaryFile(
                mode="w",
                encoding="utf-8",
                prefix="tako-aws-cfg-",
                suffix=".ini",
                delete=False,
            )
            cfg_file.write(
                f"[default]\nregion = {region}\nretry_mode = standard\nmax_attempts = 6\n"
                "s3 =\n"
                "    addressing_style = virtual\n"
                "    multipart_chunksize = 5MB\n"
                f"    multipart_threshold = {multipart_threshold_mb}MB\n"
                f"    max_concurrent_requests = {max_concurrency}\n"
            )
            cfg_file.close()
            self.env["AWS_CONFIG_FILE"] = cfg_file.name

    def _base_cmd(self) -> list[str]:
        return [
            "aws",
            "--endpoint-url",
            self.endpoint_url,
            "--region",
            self.region,
        ]

    def _run_with_timeout(
        self,
        cmd: list[str],
        *,
        key: str,
        timeout: float,
        capture: bool,
        check: bool,
        attempts: int = AWS_MAX_ATTEMPTS,
    ) -> subprocess.CompletedProcess:
        """Run an aws CLI command with a hard per-attempt timeout. TimeoutExpired alone is
        retried (up to `attempts` total tries); non-zero exits follow `check` unchanged.
        Raises RuntimeError naming the object key and elapsed seconds when retries exhaust."""
        last_timeout: subprocess.TimeoutExpired | None = None
        start = time.monotonic()
        for attempt in range(1, max(1, attempts) + 1):
            try:
                if capture:
                    return subprocess.run(
                        cmd, capture_output=True, text=True, check=check, env=self.env, timeout=timeout
                    )
                return subprocess.run(cmd, check=check, env=self.env, timeout=timeout)
            except subprocess.TimeoutExpired as exc:
                last_timeout = exc
                if attempt < max(1, attempts):
                    print(
                        f"[retry] aws operation timed out after {timeout:.0f}s for {key} "
                        f"(attempt {attempt}/{attempts}); retrying...",
                        file=sys.stderr,
                    )
        elapsed = time.monotonic() - start
        assert last_timeout is not None
        raise RuntimeError(
            f"aws operation for {key} timed out after {elapsed:.0f}s total "
            f"({max(1, attempts)} attempts x {timeout:.0f}s cap)"
        ) from last_timeout

    def head_object(self, key: str) -> dict | None:
        cmd = self._base_cmd() + [
            "s3api",
            "head-object",
            "--bucket",
            self.bucket,
            "--key",
            key,
        ]
        res = self._run_with_timeout(cmd, key=key, timeout=API_TIMEOUT_SECONDS, capture=True, check=False)
        if res.returncode != 0:
            return None
        try:
            return json.loads(res.stdout)
        except json.JSONDecodeError:
            return {}

    def upload_file(
        self,
        local_path: Path,
        key: str,
        *,
        cache_control: str,
        content_type: str | None = None,
        metadata: dict[str, str] | None = None,
    ) -> None:
        cmd = self._base_cmd() + [
            "s3",
            "cp",
            str(local_path),
            f"s3://{self.bucket}/{key}",
            "--cache-control",
            cache_control,
        ]
        if content_type:
            cmd.extend(["--content-type", content_type])
        if metadata:
            meta_str = ",".join(f"{k}={v}" for k, v in metadata.items())
            cmd.extend(["--metadata", meta_str])
        size = local_path.stat().st_size if local_path.exists() else 0
        timeout = max(UPLOAD_TIMEOUT_SECONDS, int(size // UPLOAD_TIMEOUT_FLOOR_BPS))
        last_err: Exception | None = None
        start = time.monotonic()
        for attempt in range(1, AWS_MAX_ATTEMPTS + 1):
            try:
                subprocess.run(cmd, check=True, env=self.env, timeout=timeout)
                return
            except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as exc:
                last_err = exc
                if attempt < AWS_MAX_ATTEMPTS:
                    print(
                        f"[retry] upload attempt {attempt}/{AWS_MAX_ATTEMPTS} failed for {key}; retrying...",
                        file=sys.stderr,
                    )
        elapsed = time.monotonic() - start
        assert last_err is not None
        raise RuntimeError(
            f"upload failed for {key} after {elapsed:.0f}s total "
            f"({AWS_MAX_ATTEMPTS} attempts x {timeout:.0f}s cap): {last_err}"
        ) from last_err

    def list_objects(self, prefix: str) -> list[dict]:
        cmd = self._base_cmd() + [
            "s3api",
            "list-objects-v2",
            "--bucket",
            self.bucket,
            "--prefix",
            prefix,
        ]
        res = self._run_with_timeout(cmd, key=prefix, timeout=API_TIMEOUT_SECONDS, capture=True, check=True)
        if not res.stdout.strip():
            return []
        data = json.loads(res.stdout)
        return list(data.get("Contents") or [])

    def delete_object(self, key: str) -> None:
        cmd = self._base_cmd() + [
            "s3api",
            "delete-object",
            "--bucket",
            self.bucket,
            "--key",
            key,
        ]
        self._run_with_timeout(cmd, key=key, timeout=API_TIMEOUT_SECONDS, capture=False, check=True)


def download_file(url: str, dest: Path) -> None:
    cmd = [
        "curl",
        "-fL",
        "--retry",
        "3",
        "--retry-delay",
        "3",
        "--connect-timeout",
        "20",
        "--max-time",
        "600",
        "-sS",
        "-o",
        str(dest),
        url,
    ]
    subprocess.run(cmd, check=True, timeout=630)  # hard cap slightly above curl --max-time 600


def mirror_manifest_to_r2(
    manifest: dict,
    s3: S3Client,
    mirror_base: str | None,
    apps_to_mirror: set[str],
    key_prefix: str = DEFAULT_KEY_PREFIX,
    keep_versions: int = DEFAULT_KEEP_VERSIONS,
    dry_run: bool = False,
) -> dict:
    apps = manifest.get("apps") or {}
    for app_id, app_entry in apps.items():
        if app_id.lower() not in apps_to_mirror:
            continue
        platforms = app_entry.get("platforms") or {}
        for plat, art in platforms.items():
            # Prefer upstream/GitHub fallbackUrl as the download source so runner syncs
            # do not burn domestic OSS traffic (e.g., dsh-tako OSS -> GitHub Release).
            prev_url = str(art.get("url") or "").strip()
            existing_fallback = str(art.get("fallbackUrl") or "").strip()
            official_url = existing_fallback or prev_url
            version = str(art.get("version") or app_entry.get("version") or "").strip()
            if not official_url or not version:
                continue

            key = build_object_key(key_prefix, app_id, plat, version, official_url)
            expected_size = art.get("size") if isinstance(art.get("size"), int) else None
            head = None if dry_run else s3.head_object(key)

            obj_ready = False
            obj_sha256 = art.get("sha256")
            obj_size = expected_size

            if head is not None:
                remote_size = head.get("ContentLength")
                if expected_size is None or remote_size == expected_size:
                    print(f"[skip] {key} already exists in s3://{s3.bucket} ({remote_size} bytes)")
                    obj_ready = True
                    meta = {k.lower(): v for k, v in (head.get("Metadata") or {}).items()}
                    if not obj_sha256 and meta.get("sha256"):
                        obj_sha256 = meta["sha256"]
                    if isinstance(remote_size, int) and remote_size > 0:
                        obj_size = remote_size

            if not obj_ready:
                if dry_run:
                    print(f"[dry-run] would mirror {official_url} -> s3://{s3.bucket}/{key}")
                    obj_ready = True
                else:
                    with tempfile.TemporaryDirectory(prefix="tako-r2-mirror-") as tmp_dir:
                        local_file = Path(tmp_dir) / os.path.basename(key)
                        print(f"[download] {app_id}/{plat} {version} <- {official_url}")
                        download_file(official_url, local_file)
                        actual_size, actual_sha256, actual_sha512 = compute_file_hashes(local_file)

                        expected_sha256 = str(art.get("sha256") or "").strip().lower()
                        expected_sha512 = str(art.get("sha512") or "").strip().lower()
                        if expected_sha256 and actual_sha256 != expected_sha256:
                            raise RuntimeError(
                                f"SHA-256 mismatch for {official_url}: expected {expected_sha256}, got {actual_sha256}"
                            )
                        if expected_sha512 and actual_sha512 != expected_sha512:
                            raise RuntimeError(
                                f"SHA-512 mismatch for {official_url}: expected {expected_sha512}, got {actual_sha512}"
                            )

                        print(f"[upload] s3://{s3.bucket}/{key} ({actual_size} bytes, sha256={actual_sha256[:12]}...)")
                        s3.upload_file(
                            local_file,
                            key,
                            cache_control="public, max-age=31536000, immutable",
                            content_type="application/octet-stream",
                            metadata={"sha256": actual_sha256},
                        )
                        obj_ready = True
                        obj_sha256 = actual_sha256
                        obj_size = actual_size

            if obj_ready:
                if obj_sha256:
                    art["sha256"] = obj_sha256
                if obj_size:
                    art["size"] = obj_size
                if mirror_base:
                    mirror_prefix = mirror_base.rstrip("/") + "/"
                    art["url"] = build_mirror_url(mirror_base, app_id, plat, version, official_url)
                    # Keep the previous primary URL as fallbackUrl when it is not already
                    # pointing at our mirror (e.g., dsh-tako falls back to Aliyun OSS Shanghai;
                    # claude/antigravity fall back to their official upstream URLs).
                    if prev_url and not prev_url.startswith(mirror_prefix):
                        art["fallbackUrl"] = prev_url
                    else:
                        art["fallbackUrl"] = official_url

                if not dry_run:
                    clean_prefix = key_prefix.strip("/")
                    list_prefix = f"{clean_prefix}/{app_id}/{plat}/" if clean_prefix else f"{app_id}/{plat}/"
                    existing_objs = s3.list_objects(list_prefix)
                    stale_keys = select_keys_to_prune(existing_objs, version, keep_versions)
                    for stale in stale_keys:
                        print(f"[prune] deleting stale version object s3://{s3.bucket}/{stale}")
                        s3.delete_object(stale)

    return manifest


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Mirror Tako Switch installers into Cloudflare R2")
    parser.add_argument("--manifest", default="manifest/manifest.json", help="Path to manifest.json")
    parser.add_argument("--bucket", default=os.environ.get("R2_BUCKET", ""), help="R2 bucket name")
    parser.add_argument("--endpoint-url", default=os.environ.get("R2_ENDPOINT_URL", ""), help="R2 S3 endpoint URL")
    parser.add_argument(
        "--mirror-base",
        default=os.environ.get("MIRROR_APPS_BASE") or os.environ.get("TAKO_MIRROR_BASE") or "",
        help="Public China CDN base URL pointing at `<bucket>/<prefix>` (e.g. https://dl.lven.asia/apps)",
    )
    parser.add_argument(
        "--apps",
        default=os.environ.get("MIRROR_APPS", ",".join(DEFAULT_MIRROR_APPS)),
        help="Comma-separated app IDs to mirror (default: dsh-tako,claude,antigravity)",
    )
    parser.add_argument(
        "--key-prefix",
        default=os.environ.get("R2_KEY_PREFIX", DEFAULT_KEY_PREFIX),
        help="Object key prefix inside bucket (default: apps)",
    )
    parser.add_argument(
        "--keep-versions",
        type=int,
        default=int(os.environ.get("R2_KEEP_VERSIONS", str(DEFAULT_KEEP_VERSIONS))),
        help="Number of recent versions to retain per app/platform (default: 2)",
    )
    parser.add_argument(
        "--upload-manifest-key",
        default=os.environ.get("R2_MANIFEST_KEY", "tako-switch/manifest.json"),
        help="Optional bucket key to also upload updated manifest.json",
    )
    parser.add_argument(
        "--prev-manifest",
        default="",
        help="Optional path to previous manifest.json; preserves generatedAt when apps did not change",
    )
    parser.add_argument(
        "--prefer-doge",
        action="store_true",
        help="Upload directly to DogeCloud OSS instead of R2 (only recommended from China-local runners)",
    )
    parser.add_argument("--dry-run", action="store_true", help="Plan actions without uploading/pruning")
    args = parser.parse_args(argv)

    doge_ak = os.environ.get("DOGE_ACCESS_KEY", "").strip()
    doge_sk = os.environ.get("DOGE_SECRET_KEY", "").strip()
    doge_bucket = os.environ.get("DOGE_OSS_BUCKET", "").strip()

    s3_bucket = args.bucket.strip()
    s3_endpoint = args.endpoint_url.strip()
    s3_region = infer_s3_region(s3_endpoint, os.environ.get("AWS_DEFAULT_REGION"))
    env_overrides: dict[str, str] | None = None

    use_doge = (args.prefer_doge or not (s3_bucket and s3_endpoint)) and bool(
        doge_ak and doge_sk and doge_bucket
    )
    if use_doge:
        print(f"[auth] exchanging DogeCloud STS token for OSS space '{doge_bucket}'")
        s3_bucket, s3_endpoint, s3_region, env_overrides = fetch_dogecloud_s3_credentials(
            doge_ak, doge_sk, doge_bucket
        )
        print(f"[auth] DogeCloud S3 target: s3://{s3_bucket} ({s3_endpoint}, region={s3_region})")
    elif not s3_bucket or not s3_endpoint:
        print(
            "Neither R2 (R2_BUCKET/R2_ENDPOINT_URL) nor DogeCloud OSS "
            "(DOGE_ACCESS_KEY/DOGE_SECRET_KEY/DOGE_OSS_BUCKET) configured; skipping mirror.",
            file=sys.stderr,
        )
        return 0

    manifest_path = Path(args.manifest)
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

    s3 = S3Client(
        bucket=s3_bucket,
        endpoint_url=s3_endpoint,
        region=s3_region,
        env_overrides=env_overrides,
    )
    apps_to_mirror = parse_app_filter(args.apps)
    mirror_base = args.mirror_base.strip() or None

    updated = mirror_manifest_to_r2(
        manifest=manifest,
        s3=s3,
        mirror_base=mirror_base,
        apps_to_mirror=apps_to_mirror,
        key_prefix=args.key_prefix,
        keep_versions=args.keep_versions,
        dry_run=args.dry_run,
    )

    if args.prev_manifest:
        prev_path = Path(args.prev_manifest)
        if prev_path.exists():
            try:
                prev = json.loads(prev_path.read_text(encoding="utf-8"))
                if prev.get("apps") == updated.get("apps") and prev.get("generatedAt"):
                    updated["generatedAt"] = prev["generatedAt"]
            except Exception:
                pass

    if not args.dry_run:
        manifest_path.write_text(
            json.dumps(updated, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
        if args.upload_manifest_key:
            manifest_key = args.upload_manifest_key.lstrip("/")
            s3.upload_file(
                manifest_path,
                manifest_key,
                cache_control="public, max-age=300",
                content_type="application/json; charset=utf-8",
            )
            print(f"[upload] s3://{s3.bucket}/{manifest_key}")
            # Also overwrite the tiny mutable manifest.json directly into DogeCloud Shanghai OSS
            # so mirror-storage never serves a stale cached manifest after a new upstream release.
            if not use_doge and doge_ak and doge_sk and doge_bucket:
                try:
                    d_bucket, d_ep, d_reg, d_env = fetch_dogecloud_s3_credentials(
                        doge_ak, doge_sk, doge_bucket
                    )
                    doge_s3 = S3Client(
                        bucket=d_bucket,
                        endpoint_url=d_ep,
                        region=d_reg,
                        env_overrides=d_env,
                    )
                    doge_s3.upload_file(
                        manifest_path,
                        manifest_key,
                        cache_control="public, max-age=120",
                        content_type="application/json; charset=utf-8",
                    )
                    print(f"[upload] synced manifest directly to DogeCloud OSS s3://{d_bucket}/{manifest_key}")
                except Exception as exc:
                    print(f"[warn] direct DogeCloud manifest sync skipped: {exc}", file=sys.stderr)

    return 0


if __name__ == "__main__":
    sys.exit(main())
