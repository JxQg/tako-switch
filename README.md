# Tako Switch

Tako Switch 是一个跨平台桌面工具，用来把 Tako 网关配置一键导入到 Codex 和 Claude Code。

它面向不想手动编辑配置文件的用户：打开应用后先检测本机客户端，再通过 Tako 登录或手动粘贴 ApiKey，确认写入预览后再应用配置。应用配置前会自动备份原文件，并在界面里提供恢复入口。

## 核心能力

- 检测本机 Codex / Claude Code 命令是否可用。
- 通过系统浏览器完成 Tako 授权，并使用 `takoswitch://` 回到桌面应用。
- 支持手动粘贴 Tako ApiKey。
- 显示 Tako 账号、套餐、用量和模型摘要。
- 为 Codex / Claude Code 生成写入预览，密钥会遮罩显示。
- 应用配置前自动备份，支持从最近一次写入结果恢复。
- 读取当前配置文件并以只读方式展示。
- 显示当前应用版本，并可从 GitHub Releases 检查和打开新版安装包。

## 文档导航

- [操作文档](docs/user-guide.md)：安装准备、首次配置、字段说明、备份恢复和常见问题。
- [开发文档](docs/development.md)：本地开发、项目结构、配置写入规则、服务商配置和发布流程。

## 快速开发

```bash
bun install
bun run typecheck
bun run build
bun run rust:test
bun run tauri:build
bun run version:set v0.3.0   # 同步 package.json / tauri.conf.json / Cargo.toml
cargo update --manifest-path src-tauri/Cargo.toml --package tako-switch --precise 0.3.0   # 顺手同步 Cargo.lock
bun run version:check          # 只校验是否一致，不写文件
```

## 配置目标

- Codex：`$CODEX_HOME/config.toml`，未设置 `CODEX_HOME` 时使用 `~/.codex/config.toml`。
- Claude Code：`~/.claude/settings.json`。
- Codex 密钥：写入用户环境变量 `TAKO_CODEX_API_KEY`，需要新开终端后生效。

详细说明见 [操作文档](docs/user-guide.md) 和 [开发文档](docs/development.md)。
