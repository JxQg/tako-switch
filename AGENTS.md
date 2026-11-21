# AGENTS.md

> `AGENTS.md`（Codex / 通用 Agent 读取）与 `CLAUDE.md`（Claude Code 读取）内容完全相同，修改任一文件必须同步另一份。面向人类用户的产品与使用说明见 [`README.md`](./README.md)。

本仓库是 **Tako Switch**（`tako-dev/tako-switch`）的源码仓库：基于 Tauri v2（Rust + React/TypeScript/Tailwind v4）构建的跨平台桌面端 AI 客户端配置器、本地统一网关与内置 6 合 1 MCP 宿主。

---

## 单一二进制四角色架构（Single-Binary Multi-Role Architecture）

`src-tauri/src/main.rs` 编出的单一可执行文件 `tako-switch` 承担 4 种运行时角色：

| 启动命令 / 参数 | 运行时角色 | 核心入口 | 说明 |
|---|---|---|---|
| `tako-switch`（无子命令） | Tauri GUI 主应用 | `tako_switch_lib::run()` (`src-tauri/src/lib.rs`) | 管理界面、托盘常驻、客户端检测/安装/切换、保活本地网关后台 |
| `tako-switch gateway-proxy [--port 18419]`（别名 `agy-proxy`） | 本地统一网关 HTTP 服务 | `agy_proxy::run_proxy_server(port)` | 默认监听 `127.0.0.1:18419`，提供 `/claude`、`/codex`、Gemini 协议转换、动态模型目录与 `/__tako_proxy_health` |
| `tako-switch mcp-server` | Rust `stdio` MCP 服务器 | `mcp_server::run_stdio_server()` | JSON-RPC 2.0 宿主机内置 MCP，提供 6 合 1 工具；每次请求动态重读 `~/.tako/switch/` 配置 |
| `tako-switch status-line` | 状态栏额度输出辅助命令 | `auth::render_status_line()` | 读取 `~/.tako/switch/auth.json` 与标准输入会话用量并输出一行状态摘要 |

> ⚠️ **关键进程纪律**：开发、调试或本地升级时，**严禁执行 `pkill -x tako-switch` 或 `killall tako-switch`**！三端客户端（Claude Desktop / Codex Desktop / Antigravity）会拉起 `tako-switch mcp-server` 与 `gateway-proxy` 作为子/守护进程；粗暴按进程名全杀会直接打断正在运行中的 AI 会话。如需关闭 GUI，请按完整 `.app/Contents/MacOS/tako-switch`（末尾无子命令参数）精确匹配 PID，或调用应用内关闭能力。

---

## 代码地图（Codebase Map）

| 路径 | 职责与核心符号 |
|---|---|
| `src/App.tsx` | 主界面：账号登录/登出、四大客户端卡片（一键安装 / 模式切换 / 启动 / 关闭 / 诊断 / 模型筛选）、设置抽屉与更新检查 |
| `src/components/McpInlineTheater.tsx` | 主界面下方 5 合 1 超级 MCP 内联实景剧场（「效果演示」零扣费媒体交互 + 「现场试跑」Live Playground） |
| `src/components/ModelPickerModal.tsx` | 分客户端模型选择弹窗：按推荐/自定义/全量过滤，展示 7 日实付均价、输入输出单价与吞吐/延迟 |
| `src/components/BrandIcons.tsx` / `ModelIcons.tsx` | 客户端品牌图标与模型厂商图标（Codex / Claude / Antigravity / DeepSeek 等） |
| `src/lib/api.ts` | Tauri `invoke` Typed IPC 封装与数据类型定义（`AppSnapshot`、`SwitchState`、`ClientModelsResponse`、`LiveDemoResult` 等） |
| `src/index.css` | 全局设计 Token（浅色 + 深色 `[data-theme="dark"]`）与 `.tako-running-row` / `.tako-live-dot` 呼吸指示器样式 |
| `src-tauri/src/main.rs` | 4 角色子命令路由（`mcp-server`、`status-line`、`gateway-proxy` / `agy-proxy`、GUI） |
| `src-tauri/src/lib.rs` | Tauri 命令注册（含 `run_mcp_live_demo`）、`ManifestCheckResult`、OSS/GitHub 双端点更新策略 (`choose_self_update_endpoints`)、网关守护 |
| `src-tauri/src/config.rs` | `AppPaths`、备份存取 (`has_backup`)、`SwitchState`、`ClientModelFilter`、持久化设备指纹 (`read_or_create_device_id` / `build_desktop_device_name`)、原子写 (`write_atomic`) |
| `src-tauri/src/auth.rs` | 独立于 `tako-cli` 的 OAuth PKCE 登录（`DESKTOP_OAUTH_CLIENT_ID = "takoswitch"`、`build_oauth_authorize_url`、`exchange_desktop_token`）与 `~/.tako/switch/auth.json` 存取、`AccountStatus`、`BalanceCache` |
| `src-tauri/src/agy_proxy.rs` | 本地网关服务、`ClientModelsResponse` 构建、`/claude` 与 `/codex` 模型目录生成与过滤、Antigravity 32 槽位展开 |
| `src-tauri/src/mcp_server.rs` | 6 合 1 `stdio` MCP 工具实现（`tako_web_search`、`tako_generate_image`、`tako_generate_video`、`tako_generate_audio`、`tako_consult`、`tako_account_status`）及现场试跑核心 `execute_live_demo_for_paths` |
| `src-tauri/src/manifest.rs` | `Manifest` 解析、平台匹配 (`resolve_platform_artifact`)、Semver + Calver 版本比较 (`compare_versions`) 与更新计算 (`compute_updates`) |
| `src-tauri/src/installer.rs` | 带 `InstallProgressEvent` 与 SHA-256/SHA-512 校验的客户端下载安装器 |
| `src-tauri/src/adapters/mod.rs` | `DesktopAppAdapter` trait、`AppId` / `AppMode`、`snapshot_app`、`merge_tako_mcp_server`、`build_stdio_mcp_json_entry` |
| `src-tauri/src/adapters/dsh_tako.rs` | `DshTakoAdapter`：OSS/GitHub 安装升级、`~/.dsh-tako/.credentials.yaml` (`0600`) 无损同步 `refs.TAKO_API_KEY` |
| `src-tauri/src/adapters/claude.rs` | `ClaudeAdapter`：原生 `Claude-3p` 隔离槽与受管 `stdio` MCP 写入 |
| `src-tauri/src/adapters/codex.rs` | `CodexAdapter`：`~/.codex/config.toml` 无损 TOML 合并与官方 `auth.json` 暂存还原 |
| `src-tauri/src/adapters/antigravity.rs` | `AntigravityAdapter`：用户态环境变量网关注入与 `~/.gemini/config/mcp_config.json` 管理 |
| `scripts/sync-manifest.py` / `test_sync_manifest.py` | 抓取 DSH Tako 与三端官方无墙源并更新 `manifest/manifest.json` 的同步脚本与单测 |
| `scripts/verify_client_mcp_ui.py` | V2 超级 MCP 内联剧场二进制媒体（WebP/MP4/MP3）、`data-testid`、CSP 与反篡改静态门禁校验脚本 |
| `manifest/manifest.json` | 托管桌面端官方安装源清单（由 `.github/workflows/sync-manifest.yml` 每 6 小时自动刷新） |
| `.github/workflows/ci.yml` | PR 与 `main` push 验证流水线：前端构建 + 清单脚本单测 + Linux/macOS/Windows 三平台 `cargo test` & `clippy` |
| `.github/workflows/release.yml` | `v*` tag 发布流水线：macOS 双架构签名公证 + Windows NSIS + `latest.json` / `SHA256SUMS.txt` + 旧 `v2.*` Release 自动清理 |

---

## 核心工程不变量与适配器契约（Invariants & Contracts）

1. **`DesktopAppAdapter` 无损切换契约（`src-tauri/src/adapters/mod.rs`）**：
   - 任何首次修改目标客户端配置或凭据文件前，必须先通过 `config.rs` 写入 `~/.tako/switch/backups/<app>/`。
   - `apply_mode(AppMode::Official)` 必须幂等且完整恢复原始文件（无备份则安全移除注入键，保留用户原有配置）。
   - 所有配置文件写入一律使用 `write_atomic` 防止半截文件损坏。
2. **`ClaudeAdapter` 隔离与 MCP 投递规则（`src-tauri/src/adapters/claude.rs` + `agy_proxy.rs`）**：
   - 写入官方原生 `Claude-3p` 隔离配置槽（`inferenceCredentialKind: "static"` + `_meta.json` `entries`）与 `managedMcpServers`（`transport: "stdio"` + `toolPolicy`）；`1p` 官方登录槽与对话历史保持隔离。
   - 切换为 `AppMode::Tako` 时，Tako MCP **只允许**通过 3p profile 的 `managedMcpServers`（`stdio`）注入，**严禁**同时往 3p 用户态 `claude_desktop_config.json` 写入同名 `tako` 服务（否则 Claude Desktop 会重复拉起两个 MCP 进程）。
   - `/claude/v1/models` 只暴露真实 `claude-*` 模型（保持 `supports_1m: false` 避免桌面端出现重复 `[1m]` 条目），并在转发前剥离请求体里的 `[1m]` 后缀。
3. **`CodexAdapter` 无损合并规则（`src-tauri/src/adapters/codex.rs` + `agy_proxy.rs`）**：
   - 对 `~/.codex/config.toml` 做增量表合并而非整文件覆盖，保留用户自定义 MCP 与非冲突字段。
   - `/codex/models` 过滤模型时（Recommended / Custom / All）必须始终保留隐藏的 `codex-auto-review` 条目。
4. **`AntigravityAdapter` 环境变量与 32 槽位规则（`src-tauri/src/adapters/antigravity.rs` + `agy_proxy.rs`）**：
   - 固定 `WIRE_PROTOCOL=genai`，通过用户态（macOS `launchctl setenv` / LaunchAgent，Windows `HKCU\Environment`）注入环境变量，支持可选专用 Key 覆盖。
   - 展开 `/v1beta/models` 推理档位前，必须先按 `ClientModelFilter` 过滤再截断 **32 个自定义模型槽位上限**（`expand_models_catalog_with_filter`），否则用户勾选的主力模型会被截掉。
5. **账号与配置隔离（`config.rs`、`auth.rs`、`mcp_server.rs`）**：
   - `~/.tako/switch/auth.json` 与 `tako-cli` 的 `~/.tako/config.json` 必须保持物理隔离，不得互写。
   - `~/.tako/switch/device-id` 持久化 8 位本机设备指纹（登出/清理 `auth.json` 时不得删除），OAuth 登录固定携带 `client_id = "takoswitch"` 与 `{host}-{device_id}`（总长 ≤ 32）的 `device_name`，保证多机互不顶号、同机幂等。
   - `mcp_server.rs` 每次处理 `tools/list` / `tools/call` 时直接读取磁盘上的最新 `auth.json` 与 `state.json`，不得只靠启动期快照缓存状态。
6. **双端点自更新与官方清单约定（`lib.rs`、`manifest.rs`、`scripts/sync-manifest.py`）**：
   - `tako-switch` 自身更新器在 `tauri.conf.json` 与 `lib.rs` 中同时支持阿里云上海 OSS 与 GitHub Releases 回退，通过 `choose_self_update_endpoints` 按 Semver 决定优先端点。
   - `manifest/manifest.json` 中的第三方客户端安装 URL 始终指向原厂官方无墙分发域名，不在仓库或自建 OSS 中转存第三方二进制包。

---

## 提交前必跑门禁（Verification Gate）

与 [`.github/workflows/ci.yml`](./.github/workflows/ci.yml) 对齐，每次改完代码必须全部通过后再提交：

```bash
# 1. 前端类型检查与生产构建
npm run typecheck && npm run build

# 2. Rust 全量单测与 Clippy 零告警门禁
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

# 3. 清单同步脚本测试与结构校验
python3 -m unittest -v scripts/test_sync_manifest.py
python3 -c 'import json; d=json.load(open("manifest/manifest.json")); assert set(d["apps"])=={"dsh-tako","codex","claude","antigravity"}'

# 4. 超级 MCP 内联剧场与安全门禁校验
python3 scripts/verify_client_mcp_ui.py --self-test && python3 scripts/verify_client_mcp_ui.py
```

---

## 发版规程（Release SOP）

1. **同步升级版本号（三处必须严格一致）**：
   - `package.json`（及 `package-lock.json` 根 `"version"`）
   - `src-tauri/tauri.conf.json`
   - `src-tauri/Cargo.toml`（跑一次 `cargo check` 更新 `src-tauri/Cargo.lock`）
2. **先同步远端清单自动提交**：`sync-manifest.yml` 每 6 小时可能向 `main` 推送 `chore(manifest): sync desktop app versions [skip ci]`，push 前先 `git fetch origin main && git merge origin/main --no-edit`（不要 `git push --force`）。
3. **显式路径提交并推 tag**：
   - 用 `git commit ... -- <文件列表>` 提交，并核对 `git show --stat HEAD`。
   - 先 `git push origin main`，再打与版本号严格一致的 tag（`git tag vX.Y.Z && git push origin vX.Y.Z`）。
   - **禁止移动或强推已推送的 `v*` tag**；若构建或公证失败，修复后递增补丁号重新发版。
4. **Release CI 产物核验**：`.github/workflows/release.yml` 会在 macOS（aarch64 + x86_64 签名公证）与 Windows（x64 NSIS）构建后自动生成 `latest.json`、`SHA256SUMS.txt` 并清理更早的 `v2.*` GitHub Release（保留历史 `v4.*` 不动）。

---

## 安全与合规红线（Hard Boundaries）

- **公开仓库零敏感信息**：严禁向仓库写入或提交任何 API Key（`cr_`/`sk-`）、Apple ID / App-Specific Password / `.p12` / `.cer`、私密服务器 IP 或内部主机名；文档与测试夹具中一律使用明显占位符。
- **禁止客户端绕过与篡改**：严禁加入伪造模型 ID 绕过客户端校验（如伪装模型前缀/行为字段）、欺骗上游身份检查、或反编译/修补第三方桌面客户端二进制（例如 `app.asar`）的代码。
