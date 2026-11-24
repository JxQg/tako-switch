# 通用扩展中心

## 入口与范围

首页的“通用扩展中心”提供通用 MCP、Skills 技能、插件管理和提示词预设四个入口。当前实现通用 MCP 和提示词预设，Skills 和插件页签保留入口。

扩展中心只管理 Claude、Codex 和 Antigravity。Tako 超级 MCP 仍在“配置与演示”页面独立管理，不出现在通用 MCP 列表中。

## 通用 MCP

进入页面时，扩展中心扫描三端已有配置，将第三方服务收录到中心库。扫描不改写客户端文件，解析失败的文件会显示警告。

- 点击“添加 MCP”，可填写 stdio 命令或 HTTP 地址，也可选择 Context7、Playwright、Fetch 和 Sequential Thinking 模板。
- “智能粘贴”接受 `mcpServers` JSON、单服务 JSON 或 `[mcp_servers.<id>]` TOML。无名称的单服务配置使用 `custom-mcp`，保存前可修改标识。
- 三端开关独立生效。Claude 同时更新 Desktop 1p、已有 3p 目录和 Claude Code 的 `.claude.json`；其他顶层字段保持不变。
- “有差异”显示中心配置与本机配置的参数差异。行内双箭头按中心配置统一已启用的平台；编辑器可选择某端参数作为来源。
- 三端全部关闭后，中心库仍保留该服务。只有“删除”才同时移除三端配置和中心记录。

`tako`、`tako-web-page` 和 `node_repl` 为保留标识，不能手工创建或覆盖。

“更多”支持重新扫描、全部差异同步和 JSON 备份。恢复备份会合并中心记录，并按备份中的开关与参数恢复各端配置，不删除无关服务。

## 提示词预设

每个平台最多启用一个预设。开启新预设后，旧预设保留在列表中。保存已启用预设会同步目标文件；保存未启用预设只更新中心库。

| 平台 | 目标文件 |
|---|---|
| Claude | `~/.claude/CLAUDE.md` |
| Codex | `~/.codex/AGENTS.md`，遵循 `CODEX_HOME` |
| Antigravity | `~/.gemini/AGENTS.md` |

目标文件的用户正文和 `tako-switch:chinese-ui` 汉化托管块独立保存。切换、修改、停用或删除预设时，扩展中心原样保留汉化块。

首次发现手写正文时，扩展中心先备份原文件，再创建已启用的“导入的原始提示词备份”。外部编辑器修改已启用正文后，重新读取会更新中心预设。平台栏大小表示目标文件的 UTF-8 总字节数，包括汉化块。

“同步到其他端”只复制到勾选的平台。同名预设会更新，其他预设不变；可选择同步后立即启用。若目标同名预设已启用，更新正文也会同步其目标文件。

## 存储与恢复

| 路径 | 内容 |
|---|---|
| `~/.tako/switch/extensions/mcp.json` | 中心 MCP 配置、各端开关和参数差异 |
| `~/.tako/switch/extensions/prompts.json` | 三端提示词预设与启用标识 |
| `~/.tako/switch/backups/<platform>/` | 首次修改前的客户端原文件 |

所有客户端修改先解析，再备份并原子写入。一次多文件操作失败时，会尝试恢复本次写入前的文件，并返回失败信息。不要公开含环境变量或请求头的 MCP 备份。

## 开发验证

Rust 黑盒测试只使用临时目录：

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test ext_hub_phase1_blackbox
```

浏览器脚本使用模拟 Tauri IPC，覆盖 `820×650` 和 `720×560`、深浅色及紧凑/舒适密度。脚本不调用真实客户端，也不使用付费工具。

在本工作树安装测试依赖并启动预览：

```powershell
npm install --prefix src-tauri/target/browser-tools --no-save --package-lock=false playwright
npm run dev -- --port 1420
```

在另一终端运行：

```powershell
$env:PLAYWRIGHT_MODULE = (Resolve-Path src-tauri/target/browser-tools/node_modules/playwright/index.mjs).Path
$env:EXT_HUB_PREVIEW_URL = "http://127.0.0.1:1420"
$env:PLAYWRIGHT_CHANNEL = "msedge"
node scripts/test_ext_hub_phase1_ui.mjs
```

脚本将截图和 `layout-results.json` 写入 `src-tauri/target/ext-phase1-ui/`。可用 `PLAYWRIGHT_CHANNEL=chrome` 选择 Chrome。完整提交门禁仍以 `AGENTS.md` 为准。
