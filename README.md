# Tako Switch

跨平台桌面工具，用于一键生成并导入 Codex 和 Claude Code 的个人全局配置。

## MVP 功能

- 使用 Tauri v2 + React + TypeScript + Vite。
- 检测 `codex --version` 和 `claude --version`，未安装时只提示，不阻塞配置写入。
- 写入 Codex：`CODEX_HOME/config.toml` 或 `~/.codex/config.toml`。
- 写入 Claude Code：`~/.claude/settings.json`。
- 写入前创建同目录 `*.tako-backup-*` 备份。
- 结构化合并 TOML / JSON，保留已有无关配置。
- 预览中遮罩密钥。

## 开发

```bash
bun install
bun run icons:generate
bun run typecheck
bun run build
bun run tauri:info
bun run tauri:dev
```

## Rust / Tauri 前置条件

Windows 上构建 Tauri 桌面应用需要：

- Rust / Cargo / rustup
- Visual Studio Build Tools，包含 MSVC 和 Windows SDK
- WebView2 Runtime

当前仓库已能通过前端类型检查和 Vite 构建；Rust 测试需要安装上述工具链后运行：

```bash
bun run rust:test
bun run tauri:build
```
