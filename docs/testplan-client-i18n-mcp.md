# 客户端三端简体中文汉化与超级 MCP 演示黑盒测试计划

## 1. 测试范围与隔离原则

- **测试职责**：依据需求规格与基线公开契约编写黑盒测试计划、Rust 集成测试与前端静态/资源校验脚本；严格遵循测试与开发分离，不读取或依赖开发分支实现细节。
- **后端公开接口**：`tako_switch_lib::adapters::{get_adapter, AppId, AppMode}` 与 `tako_switch_lib::config::*`（沙盒统一使用 `AppPaths::for_test(temp_dir)`，`is_test = true`，禁止触碰宿主机真实目录或 macOS `defaults`）。
- **前端与资源范围**：`src/`、`public/mcp-demo/`、`src-tauri/tauri.conf.json` 与仓库级 `app.asar` 禁改扫描。
- **危险操作禁令**：严禁执行 `pkill -x tako-switch`、`killall tako-switch` 或修改主仓。

---

## 2. 后端三端无损简体中文汉化测试用例表

| 用例编号 | 对应需求 | 前置条件 | 触发步骤 | 可判定预期 | 测试类型 |
|---|---|---|---|---|---|
| TC-CODEX-01 | Codex 启用简体中文与幂等性 | `AppPaths::for_test` 沙盒；已登录（`save_switch_auth`）；`prefer_chinese_ui = true`；预置 `codex_dir/config.toml`（含 `model = "gpt-5.4"` 与 `[desktop].reviewDelivery = "inline"`）及 `codex_dir/AGENTS.md`（含用户自定义规则） | 连续两次调用 `get_adapter(AppId::Codex).apply_mode(&paths, AppMode::Tako)`；调用 `diagnose(&paths)` | 1) `config.toml` 中 `[desktop].localeOverride == "zh-CN"`，`[desktop].reviewDelivery == "inline"` 与 `model` 原样保留；2) `AGENTS.md` 原有自定义规则保留，起止标记 `<!-- tako-switch:chinese-ui:start -->` 与 `<!-- tako-switch:chinese-ui:end -->` 各出现且仅出现 1 次，块内含 `"简体中文"`；3) `diagnose(&paths).notes` 至少 1 条包含 `"中文增强: 已启用"` | Rust 集成黑盒 |
| TC-CODEX-02 | Codex 关闭中文增强与切回 Official 无损还原（原无 `localeOverride`） | 同 TC-CODEX-01 完成首次 `Tako` 写入 | 1) 将 `SwitchState.prefer_chinese_ui` 设为 `false` 后重新 `apply_mode(Tako)`；2) 再将 `prefer_chinese_ui` 设为 `true` 后 `apply_mode(Tako)`，随后 `apply_mode(Official)` | 1) 两次还原后 `config.toml` 中 `[desktop].localeOverride` 均被移除，而 `reviewDelivery == "inline"` 保留；2) `AGENTS.md` 不再包含起止受控标记，原有用户规则文本完整保留 | Rust 集成黑盒 |
| TC-CODEX-03 | Codex 还原用户原有 `localeOverride`（如 `"ja-JP"`） | 沙盒预置 `codex_dir/config.toml` 含 `[desktop]\nlocaleOverride = "ja-JP"\nreviewDelivery = "inline"`；`prefer_chinese_ui = true` | 1) `apply_mode(Tako)`；2) 改 `prefer_chinese_ui = false` 后 `apply_mode(Tako)`；3) 改 `prefer_chinese_ui = true` 后 `apply_mode(Tako)` 再 `apply_mode(Official)` | 步骤 1 后 `localeOverride == "zh-CN"`；步骤 2 与步骤 3 还原后 `[desktop].localeOverride` 均精确恢复为 `"ja-JP"`，且 `reviewDelivery == "inline"` 不丢失 | Rust 集成黑盒 |
| TC-CODEX-04 | Codex 原无 `config.toml` 与 `AGENTS.md` 时清理彻底 | 沙盒 `codex_dir` 为空目录（无 `config.toml` 与 `AGENTS.md`）；`prefer_chinese_ui = true` | 1) `apply_mode(Tako)`；2) `prefer_chinese_ui = false` 后 `apply_mode(Tako)`；3) `prefer_chinese_ui = true` 后 `apply_mode(Tako)` 再 `apply_mode(Official)` | 步骤 2 后 `[desktop].localeOverride` 与空 `[desktop]` 表均不存在，`AGENTS.md` 被删除或内容为空；步骤 3 切回 `Official` 后 `config.toml` 被清理（不存在或为空且无残留 `[desktop]` 表），`AGENTS.md` 被删除或为空 | Rust 集成黑盒 |
| TC-CLAUDE-01 | Claude 三处配置注入简体中文与原有键保留 | 沙盒预置 `claude_3p_dir/config.json`（`{"darkMode":"dark","customFlag":true}`）、`claude_1p_dir/config.json`（`{"darkMode":"dark","windowBounds":{"width":1200}}`）、`.claude/settings.json`（`{"model":"claude-sonnet-5-5","permissions":{"allow":["Bash(git status)"]}}`）；`prefer_chinese_ui = true` | 调用 `get_adapter(AppId::Claude).apply_mode(&paths, AppMode::Tako)` 与 `diagnose(&paths)` | 1) `claude_3p_dir/config.json` 与 `claude_1p_dir/config.json` 均含 `"locale": "zh-Hans"` 且保留 `darkMode` / `customFlag` / `windowBounds`；2) `.claude/settings.json` 含 `"language": "Simplified Chinese"`，保留 `model` 与 `"Bash(git status)"`，且写入 Tako MCP `permissions.allow`；3) `diagnose(&paths).notes` 至少 1 条包含 `"中文增强: 已启用"` | Rust 集成黑盒 |
| TC-CLAUDE-02 | Claude 原无 `"locale"` / `"language"` 时关闭与切回 Official 移除注入键 | 完成 TC-CLAUDE-01 首次 `Tako` 写入 | 1) `prefer_chinese_ui = false` 后 `apply_mode(Tako)`；2) 恢复 `prefer_chinese_ui = true` 后 `apply_mode(Tako)` 再 `apply_mode(Official)` | 步骤 1 后三份 JSON 分别移除 `"locale"` / `"language"`，原业务键保留，`.claude/settings.json` 仍保留 Tako MCP 权限；步骤 2 切回 `Official` 后三份 JSON 均不含 `"locale"` / `"language"`，且 `.claude/settings.json` 移除 `mcp__tako__*` 保留 `"Bash(git status)"` | Rust 集成黑盒 |
| TC-CLAUDE-03 | Claude 精确恢复用户原有 `"locale"` / `"language"` 值 | 预置 `3p/config.json`（`locale: "en-US"`）、`1p/config.json`（`locale: "fr-FR"`）、`.claude/settings.json`（`language: "Japanese"`）及其他自定义键；`prefer_chinese_ui = true` | 1) `apply_mode(Tako)`；2) `prefer_chinese_ui = false` 后 `apply_mode(Tako)`；3) 恢复 `prefer_chinese_ui = true` 后 `apply_mode(Tako)` 再 `apply_mode(Official)` | 步骤 1 后分别覆盖为 `"zh-Hans"` / `"zh-Hans"` / `"Simplified Chinese"`；步骤 2 与步骤 3 还原后精确恢复为 `"en-US"` / `"fr-FR"` / `"Japanese"` 且原有其他键不丢失 | Rust 集成黑盒 |
| TC-CLAUDE-04 | Claude 原无三份 JSON 文件时开启与切回清理 | 沙盒中 `claude_1p_dir/config.json`、`claude_3p_dir/config.json`、`.claude/settings.json` 均不存在；`prefer_chinese_ui = true` | 1) `apply_mode(Tako)`；2) `apply_mode(Official)` | 步骤 1 后三份文件均生成并含正确中文键；步骤 2 切回 `Official` 后三份文件中均不再存在 `"locale"` 与 `"language"` 键（文件已删除或 JSON 中无该键） | Rust 集成黑盒 |
| TC-AGY-01 | Antigravity 注入 `~/.gemini/AGENTS.md` 与幂等性 | 沙盒预置 `paths.home/.gemini/AGENTS.md`（含用户自定义规则）；`prefer_chinese_ui = true` | 连续两次调用 `get_adapter(AppId::Antigravity).apply_mode(&paths, AppMode::Tako)` 与 `diagnose(&paths)` | 1) 用户原有规则原样保留；2) `<!-- tako-switch:chinese-ui:start -->` 与 `<!-- tako-switch:chinese-ui:end -->` 各出现 1 次且中间包含 `"简体中文"`；3) `diagnose(&paths).notes` 包含 `"中文增强: 已启用"` | Rust 集成黑盒 |
| TC-AGY-02 | Antigravity 关闭中文增强与切回 Official 剥离受控块 | 同 TC-AGY-01 完成首次 `Tako` 写入 | 1) `prefer_chinese_ui = false` 后 `apply_mode(Tako)`；2) 恢复 `prefer_chinese_ui = true` 后 `apply_mode(Tako)` 再 `apply_mode(Official)` | 两次还原后 `~/.gemini/AGENTS.md` 中的受控起止块均被彻底剥离，用户原有规则完整保留 | Rust 集成黑盒 |
| TC-AGY-03 | Antigravity 原无 `~/.gemini/AGENTS.md` 时自动删除 | 沙盒初始不存在 `paths.home/.gemini/AGENTS.md`；`prefer_chinese_ui = true` | 1) `apply_mode(Tako)`；2) `prefer_chinese_ui = false` 后 `apply_mode(Tako)`；3) 恢复 `prefer_chinese_ui = true` 后 `apply_mode(Tako)` 再 `apply_mode(Official)` | 步骤 1 生成含受控块的 `AGENTS.md`；步骤 2 与步骤 3 还原后 `~/.gemini/AGENTS.md` 均被自动删除（`!exists()`） | Rust 集成黑盒 |
| TC-CROSS-01 | 三端测试沙盒隔离与 `is_test` 宿主机保护 | 使用 `AppPaths::for_test(tmp.path())` 构造路径 | 分别对 Codex / Claude / Antigravity 执行 `apply_mode(Tako)` 与 `apply_mode(Official)` | `paths.is_test == true`，全部配置与备份文件仅写入 `tmp.path()` 内；不依赖且不改写宿主机真实 `~/.codex`、`~/Library/Application Support/Claude*`、`~/.gemini` 或 macOS `defaults` | Rust 集成黑盒 |

---

## 3. 前端五大超级 MCP 与 UI 验收测试用例表

| 用例编号 | 对应需求 | 前置条件 | 触发步骤 | 可判定预期 | 测试类型 |
|---|---|---|---|---|---|
| TC-UI-01 | 偏好设置中文副标题更新 | 仓库 `src/` 源码就绪 | 扫描 `src/` TSX 源码 | 存在精确文案 `Codex / Claude 界面与中文回复 · 反重力系统菜单与中文交互` | Python 静态黑盒 |
| TC-UI-02 | 主界面「Tako 超级 MCP」入口与 5 工具能力短说明 | 仓库 `src/` 源码就绪 | 扫描 `src/` TSX 源码并定位「Tako 超级 MCP」区块 | 1) 存在 `data-testid="open-mcp-demo"` 入口；2) 同时涵盖 5 个工具（`tako_generate_image` / `tako_generate_video` / `tako_generate_audio` / `tako_web_search` / `tako_consult`）及其能力短说明（生图 / 生视频 / 声音 / 联网 / 外脑） | Python 静态黑盒 + E2E 人工 |
| TC-UI-03 | 演示弹窗 5 个 Tab 与核心交互控件 `data-testid` | 仓库 `src/` 源码就绪 | 扫描演示弹窗组件或 `App.tsx` | 存在以下全部 `data-testid`（字面量或受控模板循环）：`mcp-demo-tab-image`、`mcp-demo-tab-video`、`mcp-demo-tab-audio`、`mcp-demo-tab-search`、`mcp-demo-tab-consult`、`mcp-demo-play-audio`、`consult-preset-gpt-6.2-sol`、`mcp-demo-copy-prompt` | Python 静态黑盒 + E2E 人工 |
| TC-UI-04 | `public/mcp-demo/` 静态样本文件存在与字节合法性 | 仓库 `public/mcp-demo/` 目录就绪 | 读取 `ceramic-mug.webp`、`ceramic-mug-edited.webp`、`voice-sample.wav` 二进制头与体积 | 1) 三个文件均存在且非空；2) 两张 `.webp` 满足 `RIFF....WEBP` 容器头及 `VP8`/`VP8L`/`VP8X` 块且字节互异；3) `.wav` 满足 `RIFF....WAVE`、`fmt ` 与非空 `data` 块 | Python 资源黑盒 |
| TC-UI-05 | `tauri.conf.json` CSP 放行媒体与本地资源 | 读取 `src-tauri/tauri.conf.json` | 解析 `app.security.csp` 各指令 | `default-src` 含 `'self'` 且无通配 `*`；`img-src` 含 `'self'` 与 `data:`；若前端使用 `blob:` / `data:` 播放音频，则 `media-src` 必须显式包含相应源（否则 `'self'` 允许 `/mcp-demo/voice-sample.wav`） | Python 配置黑盒 |
| TC-SEC-01 | 禁止硬改 / 解包第三方客户端 `app.asar` | 仓库 `src-tauri/`、`src/`、`scripts/` 就绪 | 全量扫描源码与构建脚本中的 `app.asar` / `@electron/asar` / 解包修补调用 | 业务源码与脚本中零 `app.asar` 修改、解包、重打包代码命中 | Python 安全黑盒 |
| TC-E2E-01 | 演示弹窗端到端交互与剪贴板/音频试听 | `npm run dev` 或 Tauri 桌面窗口启动 | 点击 `open-mcp-demo` 打开弹窗 -> 逐一切换 5 个 Tab -> 在音频 Tab 点击 `mcp-demo-play-audio` -> 在外脑 Tab 点击 `consult-preset-gpt-6.2-sol` -> 点击 `mcp-demo-copy-prompt` | 弹窗正常渲染原图/改图对比；音频播放无 CSP 报错且可播放/暂停；外脑模型切换高亮更新；点击复制后剪贴板写入对应提示词 | E2E / 手工验收 |

---

## 4. 待澄清边界（不自行臆测）

1. **后端登录写入函数命名**：需求规格描述为「`write_auth_token` 写入测试 token」，但基线 `tako_switch_lib::config` 公开接口为 `save_switch_auth(&paths, api_key, api_id, auth_source)`。当前集成测试按基线实际公开接口 `save_switch_auth` 编写；请确认是否需要额外暴露 `write_auth_token` 别名。
2. **Codex `config.toml` 空 `[desktop]` 表边界**：当用户原本的 `config.toml` 含有其他配置项但原本**没有** `[desktop]` 表时，切换 `prefer_chinese_ui = false` 或切回 `Official` 后，移除 `localeOverride` 导致 `[desktop]` 变空时是否应连带移除空 `[desktop]` 表；若用户原本就显式写了空 `[desktop]` 表又该如何处理？
3. **Claude 原本不存在配置文件时的删除 vs 置空**：当 `claude_1p_dir/config.json`、`claude_3p_dir/config.json` 或 `~/.claude/settings.json` 在首次切入 Tako 前原本不存在（存在 `.absent` 标记），切回 `Official` 或关闭 `prefer_chinese_ui` 时，是仅移除 `"locale"` / `"language"` 字段保留 `{}`，还是在对象为空时删除文件？当前测试仅断言「文件被删除或对应键已被移除」。
4. **Claude `diagnose` 的「中文增强: 已启用」判定条件**：当 `mode == Official` 或三处配置文件中仅部分包含中文键（例如用户手工删除 `1p/config.json` 的 `locale`）时，`diagnose(&paths).notes` 是否仍输出 `"中文增强: 已启用"`，规格未明确说明。
5. **Antigravity 原本空白 `AGENTS.md` 与 Windows 端语言联动**：
   - 若 `~/.gemini/AGENTS.md` 原本存在但仅含空白字符，剥离受控块后应保留原空白文件还是删除，需确认。
   - 「反重力系统菜单与中文交互」在 macOS 上通过 `/usr/bin/defaults`（`is_test = true` 时跳过），Windows 端是否有对应菜单语言配置未在规格中定义。
6. **前端 5 工具文案与外脑预设 `data-testid` 规律**：
   - 主界面 5 个工具能力短说明（生图 / 生视频 / 声音 / 联网 / 外脑）的精确字面文案未给定，且基线 `MCP_TOOLS` 中音频按钮为 `"音频"`，是否将按钮标签或副说明统一改为 `"声音"` 需确认。
   - 外脑参谋模型切换项仅明确列举 `consult-preset-gpt-6.2-sol`，其余预设模型是否统一采用 `consult-preset-<modelId>` 规则需确认。
   - 若演示弹窗音频播放器通过 `URL.createObjectURL` 或 `data:` URI 播放音频，`tauri.conf.json` 的 `csp` 是否已补充 `media-src 'self' data: blob:` 需结合实际播放方式确认。
