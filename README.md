# Tako Switch

Tako Switch 是一个跨平台桌面工具，用于一键生成并导入 Codex 和 Claude Code 的个人全局配置。

它提供 Tako 浏览器授权、ApiKey 自动填充、写入预览、备份和恢复能力，让配置过程尽量适合非技术用户。

## 核心功能

- 检测本机 Codex / Claude Code 安装状态。
- 通过浏览器授权登录 Tako，并通过 `takoswitch://` 深链回到客户端。
- 自动验证并填入 Tako `cr_` ApiKey。
- 展示 Tako 账号、用量和模型摘要。
- 生成 Codex / Claude Code 配置写入预览。
- 应用配置前自动创建同目录备份，并支持恢复。

## 文档导航

- [使用指南](docs/user-guide.md)
- [开发指南](docs/development.md)

## 快速开发

```bash
bun install
bun run typecheck
bun run build
cargo test --manifest-path src-tauri/Cargo.toml
bun run tauri:build
```

## 配置目标

- Codex：`CODEX_HOME/config.toml` 或 `~/.codex/config.toml`
- Claude Code：`~/.claude/settings.json`

更多细节见 [使用指南](docs/user-guide.md) 和 [开发指南](docs/development.md)。
