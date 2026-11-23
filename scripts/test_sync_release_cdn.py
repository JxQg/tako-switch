from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    "sync_release_cdn",
    Path(__file__).resolve().parent / "sync-release-cdn.py",
)
assert SPEC and SPEC.loader
mod = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mod)


class FakeResp:
    def __init__(self, status: int):
        self.status = status

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


class FakeS3:
    def __init__(self, bucket: str = "bucket-x"):
        self.bucket = bucket
        self.uploaded: list[tuple[str, str, str | None]] = []

    def upload_file(self, local_path: Path, key: str, *, cache_control: str, content_type=None, metadata=None) -> None:
        self.uploaded.append((key, cache_control, content_type))


def make_release_dir(tmp: Path, names: list[str]) -> Path:
    d = tmp / "release-files"
    d.mkdir()
    for name in names:
        (d / name).write_text("blob-" + name, encoding="utf-8")
    return d


def write_latest_cn(tmp: Path, urls: list[str]) -> Path:
    p = tmp / "latest-cn.json"
    platforms = {f"plat-{i}": {"signature": "sig", "url": u} for i, u in enumerate(urls)}
    p.write_text(
        json.dumps({"version": "2.2.7", "notes": "n", "pub_date": "t", "platforms": platforms}),
        encoding="utf-8",
    )
    return p


class SyncReleaseCdnTests(unittest.TestCase):
    def test_iter_release_files_excludes_latest_json(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            d = make_release_dir(
                Path(td), ["a.dmg", "b.exe", "SHA256SUMS.txt", "latest.json", "x.sig"]
            )
            names = [p.name for p in mod.iter_release_files(d)]
            self.assertEqual(names, ["SHA256SUMS.txt", "a.dmg", "b.exe", "x.sig"])

    def test_iter_release_files_requires_nonempty(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            d = make_release_dir(Path(td), ["latest.json"])
            with self.assertRaises(RuntimeError):
                mod.iter_release_files(d)

    def test_sync_release_to_bucket_keys_and_cache(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            d = make_release_dir(tmp, ["latest.json", "app.dmg", "SHA256SUMS.txt"])
            files = mod.iter_release_files(d)
            latest = write_latest_cn(tmp, ["https://dl.lven.asia/tako-switch/releases/v9.9.9/app.dmg"])
            fake = FakeS3("mybucket")
            keys = mod.sync_release_to_bucket(fake, tag="v9.9.9", files=files, latest_cn=latest)
            self.assertEqual(
                keys,
                [
                    "tako-switch/releases/v9.9.9/SHA256SUMS.txt",
                    "tako-switch/releases/v9.9.9/app.dmg",
                    "tako-switch/latest.json",
                ],
            )
            by_key = {k: (cc, ct) for k, cc, ct in fake.uploaded}
            self.assertEqual(by_key["tako-switch/releases/v9.9.9/app.dmg"], (mod.BINARY_CACHE_CONTROL, None))
            self.assertEqual(
                by_key["tako-switch/latest.json"],
                (mod.LATEST_CACHE_CONTROL, mod.LATEST_CONTENT_TYPE),
            )

    def test_verify_platform_urls_retries_then_succeeds(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            latest = write_latest_cn(
                Path(td), ["https://dl.lven.asia/tako-switch/releases/v1/a", "https://dl.lven.asia/tako-switch/releases/v1/b"]
            )
            statuses = iter([404, 200, 200])  # a:404,b:200 on pass1; a:200 on pass2
            methods: list[str] = []

            def opener(req, timeout=None):
                methods.append(req.get_method())
                status = next(statuses)
                if status != 200:
                    raise OSError(f"HTTP {status}")
                return FakeResp(200)

            urls = mod.verify_platform_urls(latest, retries=3, delay=0, opener=opener)
            self.assertEqual(len(urls), 2)
            self.assertEqual(methods, ["HEAD", "HEAD", "HEAD"])  # retry only re-checks failing urls

    def test_verify_platform_urls_raises_after_retries(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            latest = write_latest_cn(Path(td), ["https://dl.lven.asia/x/never"])

            def opener(req, timeout=None):
                raise OSError("HTTP 404")

            with self.assertRaises(RuntimeError):
                mod.verify_platform_urls(latest, retries=2, delay=0, opener=opener)

    def test_verify_platform_urls_requires_platforms(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            p = Path(td) / "latest-cn.json"
            p.write_text(json.dumps({"version": "1", "platforms": {}}), encoding="utf-8")
            with self.assertRaises(RuntimeError):
                mod.verify_platform_urls(p, retries=1, delay=0)

    def test_build_targets_require_missing_doge_fails(self) -> None:
        mirror = mod.load_mirror_module()
        env = {
            "DOGE_ACCESS_KEY": "",
            "DOGE_SECRET_KEY": "",
            "DOGE_OSS_BUCKET": "",
            "R2_BUCKET": "",
            "R2_ENDPOINT_URL": "",
        }
        with mock.patch.dict("os.environ", env, clear=False):
            with self.assertRaises(RuntimeError):
                mod.build_targets(mirror, {"doge"})

    def test_build_targets_none_configured_returns_empty(self) -> None:
        mirror = mod.load_mirror_module()
        env = {
            "DOGE_ACCESS_KEY": "",
            "DOGE_SECRET_KEY": "",
            "DOGE_OSS_BUCKET": "",
            "R2_BUCKET": "",
            "R2_ENDPOINT_URL": "",
        }
        with mock.patch.dict("os.environ", env, clear=False):
            self.assertEqual(mod.build_targets(mirror, set()), [])

    def test_main_exits_zero_when_nothing_configured(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            make_release_dir(tmp, ["a.dmg"])
            write_latest_cn(tmp, ["https://dl.lven.asia/x"])
            env = {
                "DOGE_ACCESS_KEY": "",
                "DOGE_SECRET_KEY": "",
                "DOGE_OSS_BUCKET": "",
                "R2_BUCKET": "",
                "R2_ENDPOINT_URL": "",
            }
            with mock.patch.dict("os.environ", env, clear=False):
                rc = mod.main(
                    ["--tag", "v0.0.0", "--release-dir", str(tmp / "release-files"), "--latest-cn", str(tmp / "latest-cn.json")]
                )
            self.assertEqual(rc, 0)

    def test_s3client_writes_multipart_tuning_for_cos(self) -> None:
        mirror = mod.load_mirror_module()
        s3 = mirror.S3Client(
            bucket="b",
            endpoint_url="https://cos.ap-shanghai.myqcloud.com",
            region="ap-shanghai",
            multipart_threshold_mb=64,
            max_concurrency=1,
        )
        cfg = Path(s3.env["AWS_CONFIG_FILE"]).read_text(encoding="utf-8")
        self.assertIn("multipart_threshold = 64MB", cfg)
        self.assertIn("max_concurrent_requests = 1", cfg)
        self.assertIn("multipart_chunksize = 5MB", cfg)


if __name__ == "__main__":
    unittest.main()
