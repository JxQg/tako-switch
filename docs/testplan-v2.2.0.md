# Tako Switch v2.2.0 独立黑盒测试计划

> **测试范围**：EdgeOne `Tako Web Page` MCP 后端同步 · Antigravity 全量简体中文汉化引擎与词典 · 前端极简三域重设计与既有能力防回归  
> **契约基准**：`docs/dispatch/v2.2.0.md`  
> **工作分支**：`test/v2.2.0-blackbox`（独立工作树 `/tmp/tako-switch-wt-test`）

---

## 1. 基线差异验证（Baseline Differential Verification）

在合入 `v2.2.0` 三个开发分支前，已于独立测试工作树 `/tmp/tako-switch-wt-test`（基线 `main` / `v2.1.1`）完成基线代码核对与静态门禁实测：

| 验证维度 | `v2.1.1` 基线实测事实 | `v2.2.0` 契约目标 |
|---|---|---|
| `McpToolsConfig` 与 EdgeOne MCP 同步 | `src-tauri/src/config.rs` 中 `McpToolsConfig` 仅有 `image/video/audio/search/consult/output_dir/consult_model`，无 `web_page`/`edgeone_api_token`/`edgeone_project_name`；`adapters/mod.rs` 无 `EDGEONE_MCP_SERVER_KEY`、`build_edgeone_mcp_json_entry` 与 `detect_node_runtime` | 向后兼容反序列化旧版 `state.json`（`web_page` 默认 `false`）；按需生成 `tako-web-page` 条目并同步至 Claude `1p`/`3p`、Codex `config.toml`、Antigravity `mcp_config.json`，关闭或切回 `Official` 彻底清理且保留用户配置 |
| Antigravity 汉化词典与运行时引擎 | `antigravity_i18n.rs` 内联约 229 条手写词典，无独立 `src-tauri/src/adapters/antigravity_zh_dict.json`，无动态正则模板与 Shadow DOM 遍历 | 抽离独立词典 JSON（`>= 1,200` 条有效自然中文词条），`dist/preload.js` 升级精确查表（含末尾标点容错）+ 动态正则规则 + Shadow DOM/同源 `iframe` + 代码/编辑器跳过守卫，保持幂等与 100% SHA-256 字节级还原 |
| 前端信息架构与静态契约门禁 | 基线运行 `python3 scripts/verify_client_mcp_ui.py --self-test && python3 scripts/verify_client_mcp_ui.py` 全部通过（`76/76`），尚未包含 `mcp-toggle-webpage`、EdgeOne 入口及词典规模校验 | 落地左侧 `220px` 三域导航与右侧单域面板，新增 `Tako Web Page` 全宽卡片（`data-testid="mcp-toggle-webpage"`），全量保留既有核心 `data-testid`、媒体资产、CSP 与反篡改门禁 |

---

## 2. 双盲独立纪律与宿主机安全红线

1. **双盲独立边界**：
   - 仅依据共享契约文档 `docs/dispatch/v2.2.0.md` 与 `main` 基线设计全部测试用例，绝不读取三个并行开发 worktree 的内部实现。
   - 仅调用契约声明的公开符号与落盘配置格式，不耦合私有函数或内部状态。
2. **宿主机与进程安全红线**：
   - 所有 Rust 集成测试强制使用 `tempfile::TempDir` + `AppPaths::for_test(tmp.path())`（显式断言 `paths.is_test == true`），通过 `paths.agy_app_override_dir` 将模拟 `Antigravity.app` 重定向至临时沙盒。
   - 严禁触碰宿主机 `/Applications/Claude.app`、`/Applications/Antigravity.app` 或真实 `~/.tako` / `~/.codex` / `~/.claude` / `~/.gemini` / `~/Library/Application Support/Claude*`。
   - 所有测试辅助代码保持跨平台编译干净（Unix 专用代码必须加 `#[cfg(unix)]`），确保 Windows/macOS CI 下 `cargo clippy --all-targets -- -D warnings` 零告警。

---

## 3. 黑盒测试用例矩阵

### 3.1 模块 A：EdgeOne `Tako Web Page` MCP 后端同步（`TC-EO-01` .. `TC-EO-08`）

| 用例编号 | 对应契约条目 | 前置条件 | 客观可判定预期（Pass/Fail Oracle） | 测试类型 |
|---|---|---|---|---|
| `TC-EO-01` | §1.1 `McpToolsConfig` 扩展与向后兼容 | 在 `TempDir` 沙盒中写入旧版 v2.1.1 `state.json`（不含 `webPage`/`edgeoneApiToken`/`edgeoneProjectName`，并含自定义 `image: false`, `consultModel: "swe-2"`） | `serde_json::from_str::<SwitchState>` 与 `read_switch_state(&paths)` 均无报错；`mcp.web_page == false`、`edgeone_api_token.is_none()`、`edgeone_project_name.is_none()`，且旧字段完整保留；设置三字段并 `save_switch_state` 后 JSON 含 camelCase 键 `webPage`/`edgeoneApiToken`/`edgeoneProjectName` 且往返一致 | Rust 黑盒 (`v220_blackbox.rs`) |
| `TC-EO-02` | §1.2 默认免密钥 MCP 条目构造与空白清洗 | 构造 `McpToolsConfig`：场景 A（`edgeone_api_token=None, edgeone_project_name=None`）；场景 B（仅含空串/空白 `"   "`, `"\t\n "`），调用 `build_edgeone_mcp_json_entry(&mcp)` | `EDGEONE_MCP_SERVER_KEY == "tako-web-page"`；`command` 为 `"npx"`（或解析到的 `npx`/`npx.cmd` 可执行路径）；`args` 精确等于 `["-y", "@edgeone/makers-mcp@latest", "--region", "china"]`；两种场景下输出对象**均不含 `env` 键** | Rust 黑盒 (`v220_blackbox.rs`) |
| `TC-EO-03` | §1.2 可选 `env` 精准注入与 trim 清洗 | 分别测试仅 Token（带前后空格）、仅 ProjectName（带前后空格）、Token+ProjectName 同配三种 `McpToolsConfig` 调用 `build_edgeone_mcp_json_entry` | 仅 Token 时 `env` 只含 `EDGEONE_PAGES_API_TOKEN`（已 trim）且无 `EDGEONE_PAGES_PROJECT_NAME`；仅 ProjectName 时只含 `EDGEONE_PAGES_PROJECT_NAME`（已 trim）且无 `EDGEONE_PAGES_API_TOKEN`；同配时两键均精确写入且 `args` 不变 | Rust 黑盒 (`v220_blackbox.rs`) |
| `TC-EO-04` | §1.2 Claude Desktop (`1p` & `3p`) 同步与清理 | 沙盒中预先写入 `claude_1p/claude_desktop_config.json`（含 `user-1p-fs`）与 `claude_3p/claude_desktop_config.json`（含 `user-3p-db`）及登录态 | `AppMode::Tako` + `web_page=true`：`1p` 与 `3p` 均写入 `mcpServers["tako-web-page"]`（含正确 `args` 与可选 `env`），且原有 `user-1p-fs`/`user-3p-db` 完整保留；改为 `web_page=false` 再 `apply_mode(Tako)` 或切回 `AppMode::Official` 后，`1p`/`3p` 均彻底移除 `tako-web-page` 且不误删用户原有 `mcpServers` | Rust 黑盒 (`v220_blackbox.rs`) |
| `TC-EO-05` | §1.2 Codex Desktop (`config.toml`) 同步与清理 | 沙盒中预置 `~/.codex/config.toml`（含 `[mcp_servers.user_custom]`）及登录态 | `AppMode::Tako` + `web_page=true`（无凭证）：存在 `[mcp_servers."tako-web-page"]`（`command`/`args` 正确、无 `env` 表），同时保留 `[mcp_servers.tako]` 与 `[mcp_servers.user_custom]`；填入凭证重新同步后出现 `[mcp_servers."tako-web-page".env]`；关闭 `web_page=false` 仅移除 `tako-web-page` 保留 `tako` 与用户项；切回 `Official` 彻底移除 `tako-web-page` 与 `tako` 且保留 `user_custom` | Rust 黑盒 (`v220_blackbox.rs`) |
| `TC-EO-06` | §1.2 Google Antigravity (`mcp_config.json`) 同步与清理 | 沙盒中预置 `paths.agy_gemini_config_dir/mcp_config.json`（含 `user-agy`）及登录态 | `AppMode::Tako` + `web_page=true`：写入 `mcpServers["tako-web-page"]`（含正确 `args` 与 `env`），同时保留 `tako` 与 `user-agy`；关闭 `web_page=false` 移除 `tako-web-page` 保留 `tako` 与 `user-agy`；切回 `Official` 移除 `tako-web-page` 与 `tako` 保留 `user-agy` | Rust 黑盒 (`v220_blackbox.rs`) |
| `TC-EO-07` | §1.2 DSH Tako 安全边界 | 沙盒中配置 `web_page=true` 与登录态，依次调用 `DshTakoAdapter.apply_mode(Tako)`、`diagnose`、`apply_mode(Official)` | 全程返回 `Ok`、不报错、不破坏 `.credentials.yaml` 原有其他键，不臆造无效的外部 MCP JSON 配置文件 | Rust 黑盒 (`v220_blackbox.rs`) |
| `TC-EO-08` | §1.1 & §1.2 `detect_node_runtime()` 与 `save_settings` 联动 | 直接调用 `detect_node_runtime()`；并验证 `lib.rs` 中 `check_node_runtime` 命令注册与 `save_settings` 对 `mcp.web_page`/`edgeone_*` 变更触发在用 `Tako` 客户端重同步 | `detect_node_runtime()` 无 panic；满足不变量（`!available => !meets_min_version`；`meets_min_version => available && major >= Some(18)`；非空 `version` 字符串）；可序列化为 camelCase `NodeRuntimeInfo` JSON；修改开关或凭证保存后无需手动重切模式即可同步 | Rust 黑盒 (`v220_blackbox.rs`) + 静态/集成验证 |

---

### 3.2 模块 B：Antigravity 全量汉化引擎与词典（`TC-AGY-01` .. `TC-AGY-06`）

| 用例编号 | 对应契约条目 | 前置条件 | 客观可判定预期（Pass/Fail Oracle） | 测试类型 |
|---|---|---|---|---|
| `TC-AGY-01` | §1.3 独立词典 `antigravity_zh_dict.json` 规模与质量 | 读取 `src-tauri/src/adapters/antigravity_zh_dict.json` | 文件存在且为合法 JSON Object；无空 key / 纯空白 key / 空 value / 非字符串 value；有效词条数 `>= 1,200`；核心高频词条（`New Conversation`、`Scheduled Tasks`、`Skills & Customizations`、`Global Settings`、`MCP Servers` 等）均存在且包含中文字符；全量译文中含中文字符比例 `>= 90%` | Rust 黑盒 (`v220_blackbox.rs`) + Python 静态校验 |
| `TC-AGY-02` | §1.3 运行时引擎升级（独立词典 + 动态模板 + Shadow DOM） | 构造最小完整模拟 `Antigravity.app`（`package.json`, `dist/menu.js`, `dist/tray.js`, `dist/updater.js`, `dist/preload.js`, `unpacked: true` 原生模块, `Info.plist`），执行 `patch_antigravity_ui_for_paths(&paths)` | `inspect_antigravity_ui_patch` 返回 `UiPatchStatus::Patched` 且 `has_valid_backup == true`；解包补丁后的 `app.asar` 并解码 `dist/preload.js`，包含独立词典核心译文、动态正则模板模式（`Thinking for` / `files changed` / `ago` 等）、Shadow DOM 支持（`shadowRoot`）及 `127.0.0.1`/`localhost` 主机安全校验 | Rust 黑盒 (`v220_blackbox.rs`) |
| `TC-AGY-03` | §1.3 严格安全跳过守卫（防误翻代码与聊天正文） | 检查 `TC-AGY-02` 补丁注入后的 `dist/preload.js` | 明确包含代码与输入元素跳过守卫：`monaco-editor`、`xterm`、`CODE`、`PRE`、`SCRIPT`、`STYLE`、`TEXTAREA`、`INPUT`（及 `contenteditable`） | Rust 黑盒 (`v220_blackbox.rs`) |
| `TC-AGY-04` | §1.3 补丁幂等性与 ASAR / `Info.plist` 完整性 | 在 `Patched` 状态下再次调用 `patch_antigravity_ui_for_paths(&paths)` | 第二次调用返回 `Ok`；ASAR Pickle 4 字节对齐与 `unpacked: true` 原生模块完整保留；`dist/preload.js`、`dist/menu.js`、`dist/tray.js`、`dist/updater.js` 均恰好含 1 次 `PATCH_GUARD_COMMENT` 且 `dist/preload.js` 内容与首次补丁后完全一致；`Info.plist` `ElectronAsarIntegrity` 哈希同步更新 | Rust 黑盒 (`v220_blackbox.rs`) |
| `TC-AGY-05` | §1.3 100% SHA-256 字节级无损还原 | 在 `Patched` 状态下调用 `restore_antigravity_ui_for_paths(&paths)` | 还原后 `app.asar` 的 SHA-256 与原始备份 100% 字节级一致（`sha256` 与原始字节数组双相等）；`Info.plist` 100% 字节级一致；`inspect_antigravity_ui_patch` 回归 `UiPatchStatus::Unpatched` | Rust 黑盒 (`v220_blackbox.rs`) |
| `TC-AGY-06` | §1.3 升级感知与重新备份 | 已打补丁的模拟 `Antigravity.app` 被覆盖为新版清洁包，再依次调用 `inspect` → `patch` → `restore` | `inspect` 识别 `UiPatchStatus::OutdatedAfterUpgrade`；重打补丁先以新版清洁包刷新 `app.asar.orig` 再注入；还原后 100% 字节级恢复为新版清洁包而非旧版 | Rust 黑盒 (`v220_blackbox.rs`) |

---

### 3.3 模块 C：前端极简重设计信息架构与回归项（`TC-UI-01` .. `TC-UI-07`）

| 用例编号 | 对应契约条目 | 前置条件 | 客观可判定预期（Pass/Fail Oracle） | 测试类型 |
|---|---|---|---|---|
| `TC-UI-01` | §1.3 / §1.4 静态契约、资产与安全防回归门禁 | 运行 `python3 scripts/verify_client_mcp_ui.py --self-test` 与 `python3 scripts/verify_client_mcp_ui.py` | 自身单测全通；既有 7 项 WebP、H.264 MP4、3 项 MP3 资产校验通过；CSP 无通配符/`unsafe-eval`；零未授权 `app.asar` 篡改；零内部 `tako_generate_*` 工具名泄漏到 JSX 文本；既有核心 `data-testid` 全部保留；新增 `mcp-toggle-webpage`、EdgeOne / `Tako Web Page` 入口与 `antigravity_zh_dict.json >= 1200` 全部通过 | Python 静态与契约校验 |
| `TC-UI-02` | §1.4 三域信息架构与极简标题栏 | 启动前端 dev server 并在真实浏览器中打开（桌面视口宽度 `1120px` / `960px`） | 左侧固定导航宽度 `220px`，上段「全局」（总览 / 通用设置 / MCP 能力）+ 下段「软件独立设置」（Claude Desktop / Codex / Antigravity / DSH Tako + 单行紧凑状态短标签），`Claude Desktop` 及短标签无折行；顶部标题栏含窗口标题、网关状态点、余额短显与账号入口 | 浏览器真实 UI 验证 |
| `TC-UI-03` | §1.4 软件独立设置单域面板 | 逐一点击左侧 4 个软件导航项 | 右侧仅渲染当前选中软件的单域面板（无四端大卡堆叠）；含页头、三项摘要卡、专属汉化状态带（Claude & Antigravity 具备单一开关与 `agy-ui-patch-btn`/`agy-ui-restore-btn`/`claude-ui-patch-btn`）、连接与模型（`open-model-square-btn` 可打开模型广场且 `plaza-*` 交互正常）、按需折叠的安装与维护区 | 浏览器真实 UI 验证 |
| `TC-UI-04` | §1.4 通用设置面板与权限引导弹窗 | 切至「通用设置」并触发主汉化开关及权限受阻模拟态 | 呈现账号余额卡、网关与网络、更新策略、外观与菜单栏、高级折叠；`main-chinese-ui-toggle` 可交互；权限受阻时正确渲染 `ui-patch-permission-modal` 并明确告知配置级汉化已生效 | 浏览器真实 UI 验证 |
| `TC-UI-05` | §1.4 MCP 能力中心 · `Tako Web Page (EdgeOne)` | 切至「MCP 能力」面板 | 顶部横跨全宽卡片展示「网站上线 · Tako Web Page」，主开关具备 `data-testid="mcp-toggle-webpage"`，右侧「查看授权流程」可打开免密钥授权说明弹窗，下方折叠「高级：Token 与项目名」表单与本机 Node.js >= 18 状态提示 | 浏览器真实 UI 验证 |
| `TC-UI-06` | §1.4 5 项核心 MCP 能力卡与 `McpInlineTheater` | 在「MCP 能力」面板点击 2 列 5 项能力卡上的 `体验能力 →`（`open-mcp-demo`） | 打开侧滑抽屉/弹窗展示 `mcp-inline-theater`，5 个 `mcp-demo-tab-*` 标签、图片对比滑块、H.264 视频、冰糖音频、复制提示词与实时运行入口全部可交互，页面可见文本零暴露内部 `tako_*` 函数名 | 浏览器真实 UI 验证 |
| `TC-UI-07` | §1.4 视觉降噪与零回归检查 | 检查所有面板与品牌图标渲染及浏览器 Console | `BrandIcons.tsx` 无白底+边框+阴影三层冗余容器，普通设置行无装饰图标，无水平溢出/元素遮挡，Console 无 JS Error 与未捕获 Promise Rejection | 浏览器真实 UI 验证 |

---

## 4. 契约对齐与合流验收清单

### 4.1 `v220_blackbox.rs` 依赖的 v2.2.0 公开符号契约

- `tako_switch_lib::config::McpToolsConfig` 字段：
  - `pub web_page: bool`（serde camelCase: `webPage`，默认 `false`）
  - `pub edgeone_api_token: Option<String>`（serde camelCase: `edgeoneApiToken`）
  - `pub edgeone_project_name: Option<String>`（serde camelCase: `edgeoneProjectName`）
- `tako_switch_lib::adapters` 公开符号：
  - `pub const EDGEONE_MCP_SERVER_KEY: &str = "tako-web-page";`
  - `pub fn build_edgeone_mcp_json_entry(mcp: &McpToolsConfig) -> serde_json::Value`
  - `pub fn detect_node_runtime() -> NodeRuntimeInfo`（含 `available`, `version`, `major`, `meets_min_version`, `node_path`, `npx_path`）
- `tako_switch_lib::adapters::antigravity_i18n`：
  - `PATCH_GUARD_COMMENT`、`inspect_antigravity_ui_patch`、`patch_antigravity_ui_for_paths`、`restore_antigravity_ui_for_paths`
  - `src-tauri/src/adapters/antigravity_zh_dict.json`（合法 JSON Object，`>= 1,200` 条有效中文词条）

### 4.2 规格边界与消除歧义说明（Ambiguity Notes）

1. **`build_edgeone_mcp_json_entry` 的 `command` 字段**：契约 §1.2 允许输出 `"npx"` 或本机解析到的 `npx` 绝对路径（Windows 上可为 `npx.cmd`）。黑盒断言校验 `command == "npx"` 或其路径文件名 stem 忽略大小写等于 `"npx"`，既兼容两种合规实现，又能拦截错误命令。
2. **`Antigravity` MCP 配置文件路径**：测试通过 `paths.agy_gemini_config_dir.join("mcp_config.json")` 定位沙盒内文件，自动随 `AppPaths::for_test` 定义保持一致，避免路径硬编码漂移。
3. **`dist/preload.js` 中中文词典编码形式**：无论是直接 UTF-8 中文还是 `\uXXXX` 转义序列，在 JS 运行时完全等价；黑盒测试先对 `\uXXXX` 解码归一化再断言独立词典词条已嵌入。

### 4.3 合流后标准验收命令

```bash
# 1. Python 静态契约、资产与安全门禁（含自测）
python3 scripts/verify_client_mcp_ui.py --self-test
python3 scripts/verify_client_mcp_ui.py

# 2. Rust 全量集成测试与跨平台 Clippy 严格门禁
cargo test --manifest-path src-tauri/Cargo.toml --test v220_blackbox
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

# 3. 前端类型检查与生产构建
npm run typecheck && npm run build
```
