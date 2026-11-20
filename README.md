# Tako Switch

跨平台（macOS + Windows）AI 桌面客户端一键配置器与内置 **Tako 超级 MCP** 宿主（基于 Tauri v2 + Rust + React 构建）。

[官网与一键下载](https://tako.shiroha.tech/tako-switch) · [详细配置指南](https://tako.shiroha.tech/docs/desktop/tako-switch) · [GitHub Releases](https://github.com/tako-dev/tako-switch/releases/latest) · [反馈 Bug / 功能建议](https://github.com/tako-dev/tako-switch/issues) · Agent 开发规范见 [`AGENTS.md`](./AGENTS.md)

---

## 下载与安装（Download & Install）

| 平台 | 安装包 | 系统要求 | 签名与安装说明 |
|---|---|---|---|
| **macOS (Apple Silicon)** | `Tako-Switch_<version>_darwin-arm64.dmg` | macOS 14 或更高，M1–M4 | Apple Developer ID 签名与 Apple 公证；打开 `.dmg` 拖入「应用程序」即可 |
| **macOS (Intel)** | `Tako-Switch_<version>_darwin-x64.dmg` | macOS 14 或更高，x86_64 | Apple Developer ID 签名与 Apple 公证；打开 `.dmg` 拖入「应用程序」即可 |
| **Windows (x64)** | `Tako-Switch_<version>_windows-x64-setup.exe` | Windows 10/11 64 位 | 首次启动若遇 Windows SmartScreen 提示，点「更多信息 → 仍要运行」 |

- **自更新加速**：客户端内置 `tauri-plugin-updater`（Ed25519 minisign 签名校验），优先从阿里云上海 OSS 镜像检查并拉取更新，异常时自动无感回退 GitHub Releases。
- **单版本保留策略**：为控制分发与存储成本，上海 OSS 与 GitHub Releases 的 `v2.*` 版本仅保留最新一版发布包。

---

## 三步快速开始（Quick Start）

1. **登录账号**：打开 Tako Switch，点击右上角 **「登录」** 拉起浏览器完成 OAuth PKCE 一键授权（也支持在设置中直接粘贴 `cr_` 开头的 Tako API Key）。
2. **安装或接管客户端**：在客户端列表中点击 **「一键安装」**（直接从官方无墙源下载并安装缺失的桌面客户端，带实时进度），或将已安装的客户端从 **「官方」** 一键切换到 **「Tako」**。
3. **按需筛选模型与随时切回**：点击客户端行右侧的 **「模型」** 按钮，可按 **7 日实付均价** 与 **平均吞吐 / 延迟** 挑选展示模型；随时点回 **「官方」** 即可一键无损还原原厂直连状态。

---

## 四大托管客户端与无损隔离机制（Managed Clients）

Tako Switch 在每次写入前都会自动备份原配置至 `~/.tako/switch/backups/`，并针对每个客户端采用最稳妥的无损隔离策略：

| 客户端 | 托管内容 | 隔离与安全保障 | 切回「官方 / 独立」行为 |
|---|---|---|---|
| **DSH Tako** (`tech.shiroha.dsh-tako`) | 一键从上海 OSS / GitHub 安装升级，并同步当前 Tako API Key 至 `~/.dsh-tako/.credentials.yaml`（严格保持 `0600` 权限，装完即用） | 原生内置完整 Tako 模型与联网工具，无需占用本地网关端口；保留原有 `records` 与其他密钥条目 | 恢复备份或清理同步写入的 `TAKO_API_KEY`，改回在 DSH Tako 内独立登录 |
| **Claude Desktop** (`com.anthropic.claudefordesktop`) | 启用官方原生 `Claude-3p` 隔离配置槽、本地 `/claude` 网关与受管 `stdio` MCP 工具策略 | 与 `1p` 官方个人登录槽完全物理隔离，零重登、不触碰官方对话历史 | 关闭 3p 网关接管并原样恢复备份 |
| **Codex Desktop** (`com.openai.codex` / `OpenAI.Codex`) | 无损编辑 `~/.codex/config.toml`、本地 `/codex` 网关、动态模型目录与预授权 MCP | 保留用户其他 TOML 配置段；自动暂存官方 `auth.json` 实现免登录直进（设置中可选保留官方登录） | 从 `~/.tako/switch/backups/` 原样还原配置与官方登录态 |
| **Google Antigravity** (`com.google.antigravity`) | 用户态环境变量注入（macOS LaunchAgent / Windows `HKCU\Environment`，固定 `WIRE_PROTOCOL=genai`）、推理档位展开与 `~/.gemini/config/mcp_config.json` | 零 root 权限修改系统；支持在设置中单独指定 Antigravity 专用 Key 覆盖主 Key | 清理用户态网关环境变量并还原备份配置 |

---

## 内置本地统一网关与 6 合 1 Tako 超级 MCP

- **本地统一网关（`http://127.0.0.1:18419`）**：随应用自动托管运行，为三端桌面客户端提供本地代理路由（`/claude`、`/codex` 及 Gemini 协议适配）、动态模型目录、推理档位展开与用量/额度查询支撑。
- **内置 Rust `stdio` MCP 服务器（`tako-switch mcp-server`）**：每次 `tools/list` 与 `tools/call` 均动态读取 `~/.tako/switch/auth.json` 与 `~/.tako/switch/state.json`，在 Tako Switch 设置中开关工具后立即生效，无需重装 MCP。

| 工具名 | 能力说明 | 默认后端 / 模型 |
|---|---|---|
| `tako_web_search` | 实时联网搜索，返回结构化摘要与可点击引用来源 | DeepSeek / Grok |
| `tako_generate_image` | 文生图与参考图编辑，支持自动保存生成图片 | `gpt-image-2` |
| `tako_generate_video` | 文生视频与图生视频，支持异步轮询与 MP4 下载 | `grok-imagine-video-1.5` (480p) |
| `tako_generate_audio` | 多语种语音合成（TTS）与音频转写（ASR） | `speech_tts` / `speech_asr` |
| `tako_consult` | 向外部专家模型咨询疑难推理或获取第二意见 | `gpt-6.2-sol`（可配置） |
| `tako_account_status` | 实时查询当前 Tako 账号余额、所属分组与速率限制 | Live quota API |

---

## 本地目录结构与隐私安全

Tako Switch 的所有配置均保存在用户主目录的独立子目录下，与 `tako-cli`（`~/.tako/config.json`）**完全隔离，互不覆盖**：

| 路径 | 用途 |
|---|---|
| `~/.tako/switch/auth.json` | 当前登录态与 Tako API Key（支持 OAuth PKCE 或手动导入） |
| `~/.tako/switch/state.json` | 网关地址、客户端切换模式、MCP 工具开关与各客户端模型筛选策略 |
| `~/.tako/switch/backups/<app>/` | 首次写入前的原始配置文件与官方登录态快照，用于无损恢复 |
| `~/.tako/switch/manifest-cache.json` | 托管桌面客户端安装包清单本地缓存 |

> **上游安装包清单同步**：仓库内 [`manifest/manifest.json`](./manifest/manifest.json) 由 GitHub Actions 每 6 小时自动同步三端官方无墙源链接（仅几十 KB 的文本清单）；用户一键安装客户端时，3–4 GB 的第三方原厂安装包直接从官方 CDN 拉取，不经第三方中转。

---

## 故障排查与反馈 Issue

1. **行内一键诊断**：点击客户端行内的 **「诊断」** 按钮，可自动检查客户端安装路径、配置文件写入状态、MCP 注入与本地网关监听。
2. **检查本地网关健康度**：在终端或浏览器访问 `http://127.0.0.1:18419/__tako_proxy_health`，确认网关进程正常响应。
3. **提交反馈**：前往 [GitHub Issues](https://github.com/tako-dev/tako-switch/issues) 时请附上系统版本、Tako Switch 版本号、目标客户端与复现步骤。**请勿在 Issue 中粘贴任何真实的 `cr_` API Key、`auth.json` 内容或个人凭据**。

---

## 本地开发构建（Contributing）

环境要求：`Node.js >= 22`、`Rust stable`、`Python >= 3.12`。

```bash
# 安装前端依赖并执行类型检查与生产构建
npm install
npm run typecheck && npm run build

# 运行 Rust 单元测试与 Clippy 严格检查
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

# 运行清单同步脚本单测
python3 -m unittest -v scripts/test_sync_manifest.py

# 启动 Tauri 桌面热重载开发模式
npx tauri dev
```

面向 AI Coding Agent 的架构细节、适配器不变量、进程边界与发版规范，请参阅 [`AGENTS.md`](./AGENTS.md)（与 [`CLAUDE.md`](./CLAUDE.md) 内容一致）。
