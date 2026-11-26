import unittest
from pathlib import Path
import importlib.util

SPEC = importlib.util.spec_from_file_location(
    "sync_manifest",
    Path(__file__).resolve().parent / "sync-manifest.py",
)
assert SPEC and SPEC.loader
mod = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mod)


class SyncManifestParserTests(unittest.TestCase):
    def test_parse_electron_builder_yml_normalizes_base64_sha512(self) -> None:
        raw = """
version: 2.33.0
files:
  - url: https://storage.googleapis.com/antigravity-public/antigravity-hub/2.33.0-1/darwin-arm/Antigravity.zip
    sha512: akGKBoAWOxZmddrQyvSHDxmnCtaaLnz7jaBTDg+8g8vG/lytBldvIkHRPxXA+WWsm6Xq2FP+fSzWD/BKFmsBvQ==
    size: 208556255
path: Antigravity.zip
sha512: akGKBoAWOxZmddrQyvSHDxmnCtaaLnz7jaBTDg+8g8vG/lytBldvIkHRPxXA+WWsm6Xq2FP+fSzWD/BKFmsBvQ==
"""
        parsed = mod.parse_electron_builder_yml(raw)
        self.assertEqual(parsed["version"], "2.33.0")
        self.assertEqual(parsed["size"], 208556255)
        self.assertEqual(len(parsed["sha512"]), 128)
        self.assertTrue(parsed["url"].endswith("Antigravity.zip"))

    def test_parse_claude_feeds(self) -> None:
        mac_json = (
            '{"currentRelease":"6.6395.1","releases":[{"version":"6.6395.1",'
            '"updateTo":{"url":"https://downloads.claude.ai/releases/darwin/universal/6.6395.1/Claude-8fc566f7b97e371bae421fc79c260de86c6a4106.zip"}}]}'
        )
        mac = mod.parse_claude_mac_releases(mac_json)
        self.assertEqual(mac["version"], "6.6395.1")
        self.assertEqual(
            mac["commitByVersion"]["6.6395.1"],
            "8fc566f7b97e371bae421fc79c260de86c6a4106",
        )

        win_txt = "\ufeffA08B1434D6886ACC997A9EDD12D53D71835E2A21 AnthropicClaude-6.6395.1-full.nupkg 273966483\n"
        ver, sha1 = mod.parse_claude_win_releases(win_txt)
        self.assertEqual(ver, "6.6395.1")
        self.assertEqual(sha1, "a08b1434d6886acc997a9edd12d53d71835e2a21")

    def test_parse_sparkle_appcast(self) -> None:
        xml = """<?xml version='1.0' encoding='utf-8'?>
<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle" version="2.0">
  <channel>
    <item>
      <title>26.1111.71704</title>
      <sparkle:shortVersionString>26.1111.71704</sparkle:shortVersionString>
      <enclosure url="https://persistent.oaistatic.com/codex-app-prod/ChatGPT-darwin-arm64-26.1111.71704.zip" length="835851598" />
    </item>
  </channel>
</rss>"""
        parsed = mod.parse_sparkle_appcast(xml)
        self.assertEqual(parsed["version"], "26.1111.71704")
        self.assertEqual(parsed["size"], 835851598)

    def test_apply_mirror(self) -> None:
        art = {"url": "https://origin.example/App.zip", "version": "1.0.0", "installType": "zip"}
        direct = mod.apply_mirror(art, None, "claude", "macos-universal", "1.0.0")
        self.assertEqual(direct["url"], "https://origin.example/App.zip")
        self.assertNotIn("fallbackUrl", direct)

        mirrored = mod.apply_mirror(
            art, "https://downloads.tako.shiroha.tech/apps", "claude", "macos-universal", "1.0.0"
        )
        self.assertEqual(
            mirrored["url"],
            "https://downloads.tako.shiroha.tech/apps/claude/macos-universal/1.0.0/App.zip",
        )
        self.assertEqual(mirrored["fallbackUrl"], "https://origin.example/App.zip")

    def test_parse_dsh_tako_latest_json(self) -> None:
        raw = """{
          "version": "0.1.55",
          "assets": [
            {
              "name": "DSH-Tako-0.1.55-mac-arm64.dmg",
              "role": "mac-arm64",
              "ossUrl": "https://dsh-tako-dl.oss-cn-shanghai.aliyuncs.com/releases/v0.1.55/DSH-Tako-0.1.55-mac-arm64.dmg",
              "githubUrl": "https://github.com/tako-dev/dsh-tako-desktop/releases/download/v0.1.55/DSH-Tako-0.1.55-mac-arm64.dmg",
              "sha256": "BC603022",
              "size": 270740588
            },
            {
              "name": "DSH-Tako-0.1.55-win-x64.exe",
              "role": "win-x64",
              "ossUrl": "https://dsh-tako-dl.oss-cn-shanghai.aliyuncs.com/releases/v0.1.55/DSH-Tako-0.1.55-win-x64.exe",
              "githubUrl": "https://github.com/tako-dev/dsh-tako-desktop/releases/download/v0.1.55/DSH-Tako-0.1.55-win-x64.exe",
              "sha256": "BC5A39A7",
              "size": 313575106
            }
          ]
        }"""
        parsed = mod.parse_dsh_tako_latest_json(raw)
        self.assertEqual(parsed["version"], "0.1.55")
        mac = parsed["platforms"]["macos-arm64"]
        self.assertEqual(mac["installType"], "dmg")
        self.assertEqual(mac["appBundleName"], "DSH Tako.app")
        self.assertTrue(mac["url"].startswith("https://dsh-tako-dl.oss-cn-shanghai.aliyuncs.com/"))
        self.assertTrue(mac["fallbackUrl"].startswith("https://github.com/tako-dev/dsh-tako-desktop/"))
        self.assertEqual(mac["sha256"], "bc603022")
        self.assertEqual(mac["size"], 270740588)

        win = parsed["platforms"]["windows-x64"]
        self.assertEqual(win["installType"], "exe")
        self.assertEqual(win["silentArgs"], ["/S"])
        self.assertEqual(win["sha256"], "bc5a39a7")
        self.assertEqual(win["size"], 313575106)

    def test_parse_workbuddy_download_history(self) -> None:
        html = """
        <a href="https://acc-1258344699.cos.ap-guangzhou.myqcloud.com/workbuddy/saas/darwin-arm64/WorkBuddy-darwin-arm64-5.11.2.43614525-fe9da600.dmg">old</a>
        <a href="https://acc-1258344699.cos.ap-guangzhou.myqcloud.com/workbuddy/saas/darwin-arm64/WorkBuddy-darwin-arm64-5.11.3.43806934-ecace0f7.dmg">arm64</a>
        <a href="https://acc-1258344699.cos.ap-guangzhou.myqcloud.com/workbuddy/saas/darwin-x64/WorkBuddy-darwin-x64-5.11.3.43806934-ecace0f7.dmg">x64</a>
        <a href="https://acc-1258344699.cos.ap-guangzhou.myqcloud.com/workbuddy/saas/win32-x64-user/WorkBuddy-win32-x64-user-5.11.3.43806934-ecace0f7.exe">win</a>
        """
        parsed = mod.parse_workbuddy_download_history(html)
        self.assertEqual(parsed["version"], "5.11.3")
        arm = parsed["platforms"]["macos-arm64"]
        self.assertEqual(arm["installType"], "dmg")
        self.assertEqual(arm["appBundleName"], "WorkBuddy.app")
        self.assertIn("cos.accelerate.myqcloud.com", arm["url"])
        self.assertIn("cos.ap-guangzhou.myqcloud.com", arm["fallbackUrl"])
        win = parsed["platforms"]["windows-x64"]
        self.assertEqual(win["installType"], "exe")
        self.assertEqual(win["silentArgs"], ["/S"])
        self.assertIn("WorkBuddy-win32-x64-user-5.11.3", win["url"])


if __name__ == "__main__":
    unittest.main()
