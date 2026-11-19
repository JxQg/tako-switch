# Tako Switch

极简跨平台（macOS + Windows）桌面 AI 客户端配置器与内置 **Tako 超级 MCP** 宿主，使用 Tauri v2 (Rust + React/TypeScript/Tailwind v4) 构建。

## 核心能力

- **3 端桌面客户端统一托管**：
  - **Codex Desktop** (`com.openai.codex` / `OpenAI.Codex`)：`~/.codex/config.toml` 无损写入（跳过 ChatGPT OAuth，保留用户现有 `auth.json` 与其他 TOML 配置）。
  - **Claude Desktop** (`com.anthropic.claudefordesktop`)：启用原生 `Claude-3p` 隔离配置槽与 `1p`/`3p` 双目录 MCP 隔离，切回官方零重登、零破坏会话历史。
  - **Google Antigravity Desktop** (`com.google.antigravity`)：零 root 用户态网关注入（macOS LaunchAgent `launchctl setenv` / Windows `HKCU\Environment` + `WM_SETTINGCHANGE`，固定 `WIRE_PROTOCOL=genai`）与 `~/.gemini/config/mcp_config.json` 管理；支持专用反重力 Key 覆盖。
- **三态一键切换** + **首次修改前自动无损备份** (`~/.tako/switch/backups/`)：
  - `官方`：彻底还原原厂直连状态。
  - `聚合`：保留官方订阅主模型，注入 Tako 超级 MCP 工具。
  - `Tako`：主模型与 MCP 全量走 Tako 企业网关，直接免官方登录使用。
- **内置 Rust stdio MCP 服务器 (`tako-switch mcp-server`)**：
  - 5 合 1 工具：`tako_generate_image`、`tako_generate_video`、`tako_generate_audio`、`tako_web_search`、`tako_consult`。
  - 每次 `tools/list` / `tools/call` 动态读取 `~/.tako/config.json` 与 `~/.tako/switch/state.json`，开关即时生效，无需重装 MCP。
- **双层自动化更新闭环**：
  1. `tako-switch` 自身通过 `tauri-plugin-updater`（Ed25519 minisign）优先检查镜像 `downloads.tako.shiroha.tech/tako-switch/latest.json`，自动回退至 GitHub Releases。
  2. GitHub Actions 每 6 小时自动同步 Antigravity / Claude / Codex 官方无墙源生成 `manifest/manifest.json`，客户端支持一键安装与无损覆盖升级。

## 本地开发与验证

```bash
# 前端类型检查与构建
npm install
npm run build

# Rust 单元测试与 Clippy 检查
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

# 清单同步脚本测试与更新
python3 -m unittest -v scripts/test_sync_manifest.py
python3 scripts/sync-manifest.py --out manifest/manifest.json

# Tauri 桌面开发 / 本地打包
npx tauri dev
npx tauri build --bundles app
```

## GitHub Actions Secrets / Variables 说明

- `TAURI_SIGNING_PRIVATE_KEY`（已配置）：Tauri v2 updater Ed25519 签名私钥（无口令）。
- macOS 签名与公证（发布签名前从 `tako-dev/dsh-tako-desktop` 复用）：
  `APPLE_CERTIFICATE`、`APPLE_CERTIFICATE_PASSWORD`、`APPLE_ID`、`APPLE_PASSWORD`、`APPLE_TEAM_ID`、`KEYCHAIN_PASSWORD`。
- 国内加速镜像（可选，配置 OSS/S3 后自动启用）：
  Variables: `MIRROR_SELF_BASE`、`MIRROR_APPS_BASE`；Secrets: `MIRROR_S3_BUCKET`、`MIRROR_ACCESS_KEY_ID`、`MIRROR_SECRET_ACCESS_KEY`、`MIRROR_ENDPOINT_URL`。
