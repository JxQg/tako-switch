# Tako Switch 开发文档

本文面向参与 Tako Switch 开发和维护的人。项目优先保持轻量、可验证、对非技术用户友好。

## 技术栈

- Tauri v2
- React 18
- TypeScript
- Vite
- Bun
- Rust 2021

## 本地环境

需要准备：

- Bun，版本以 `package.json` 的 `packageManager` 为准。
- Rust stable。
- Tauri v2 所需的系统依赖。
- Windows 打包需要 NSIS / MSI 相关环境，macOS 打包需要对应签名环境。

常用命令：

```bash
bun install
bun run tauri:dev
bun run typecheck
bun run build
bun run rust:test
bun run tauri:info
bun run tauri:build
```

如果 `bun run tauri:build` 在 Windows 上提示无法删除 `tako-switch.exe`，通常是旧的 Tako Switch 进程还在运行。先关闭应用或结束进程后再重试。

## 文档分工

- `.agents/`：Codex 和其他代理使用的项目约定。
- `.codex/docs/`：Codex 规划、实现计划和阶段性方案。
- `docs/`：面向用户和开发者的项目文档。
- `README.md`：项目简介和文档导航，保持简短。

新增文档时优先放在 `docs/`，除非内容只给代理或只记录规划过程。

## 前端结构

入口文件：

- `src/main.tsx`：React 启动入口。
- `src/App.tsx`：主界面、标签页状态和导入流程编排。
- `src/App.css`：应用样式。

Tako 前端集成放在 `src/integrations/tako/`：

- `api.ts`：Tauri invoke 封装。
- `auth.ts`：浏览器授权和 `takoswitch://` 回调流程。
- `providerConfig.ts`：服务商配置读取与默认服务商解析。
- `sessionStore.ts`：Tako 会话保存。
- `types.ts`：前端集成类型。

前端组件负责渲染、交互和流程编排。网络请求、Tauri 调用、服务商解析和配置写入规则不要堆进组件里。

## 后端结构

Tauri 后端位于 `src-tauri/src/`。

- `lib.rs`：Tauri 插件、单实例、deep-link 和 invoke handler 注册。
- `commands.rs`：通用命令、当前配置读取、预览、应用、恢复和 deep-link 解析。
- `models.rs`：前后端共享的命令 DTO。
- `config_paths.rs`：用户配置路径、程序目录和服务商配置路径。
- `backups.rs`：备份、恢复、最近一次写入结果持久化。
- `tools.rs`：Codex / Claude Code 本机命令检测。
- `env_vars.rs`：Codex 用户环境变量写入。
- `redaction.rs`：密钥遮罩和配置脱敏。
- `platforms/`：本机客户端写入器。
- `providers/`：服务商配置、校验和 Tako 远程 API。

`lib.rs` 应保持为入口装配层，不承载业务逻辑。新增平台或服务商能力时，优先扩展 `platforms/` 或 `providers/`，再从 invoke handler 暴露窄命令。

## 配置写入规则

所有写入都应经过同一条流程：

1. 前端构造 `ConfigInput`。
2. 后端通过 `validate_input` 读取并校验服务商配置。
3. `preview_changes` 生成写入预览。
4. `apply_configs` 先备份，再写入。
5. 写入结果保存到程序目录下的 `tako-config-backups.json`。
6. 恢复通过 `restore_backup` 回到写入前状态。

不要在新功能里绕过预览、备份和恢复流程直接写配置文件。

### Codex 写入

Codex 写入器在 `src-tauri/src/platforms/codex.rs`。

- 目标路径：`$CODEX_HOME/config.toml`，未设置时为 `~/.codex/config.toml`。
- 写入内容：`model`、`model_provider`、`model_providers.<provider>`。
- 密钥来源：`env_key = "TAKO_CODEX_API_KEY"`。
- Windows 上通过用户环境变量注册表写入；macOS / Linux 上写入 shell 配置文件。

Codex 配置使用 `toml_edit` 合并，目标是保留用户已有配置并保持幂等。

### Claude Code 写入

Claude Code 写入器在 `src-tauri/src/platforms/claude.rs`。

- 目标路径：`~/.claude/settings.json`。
- 写入内容：`env.ANTHROPIC_BASE_URL`、`env.ANTHROPIC_AUTH_TOKEN`、可选的 `env.ANTHROPIC_MODEL`。
- 模型留空时会移除 `ANTHROPIC_MODEL`，让 Claude Code 使用默认模型。

Claude 设置使用 JSON 合并，目标是保留用户已有字段。

## 服务商配置

默认服务商配置位于：

```text
src-tauri/config/providers.json
```

打包时该文件会作为资源复制到程序目录下：

```text
config/providers.json
```

运行时优先读取程序目录下的 `config/providers.json`。如果文件不存在或无效，会回退到内置默认配置，并在界面显示警告。

当前内置服务商是 Tako：

- Codex 默认地址：`https://tako.shiroha.tech/v1`
- Codex 默认模型：`gpt-5.4`
- Claude Code 默认地址：`https://tako.shiroha.tech`
- Codex writer：`codexConfigToml`
- Claude Code writer：`claudeSettingsJson`

新增服务商时，优先扩展 provider schema 和校验逻辑，再接入现有 preview/apply 管线。

## Tako 登录和 deep-link

前端 `startTakoLogin` 会打开：

```text
https://tako.shiroha.tech/app/authorize?state=<state>&redirect=takoswitch
```

授权成功后，浏览器通过 `takoswitch://` 回到桌面应用。后端在 `commands.rs` 解析 `resource=auth`、`key` 和 `state`，并向前端发送 `tako-auth` 事件。

调试 Windows debug 构建时，`lib.rs` 会注册 deep-link 协议。正式打包由 Tauri 配置负责。

## 备份与恢复

备份由后端统一管理：

- Codex：`<程序目录>/backups/codex/`
- Claude Code：`<程序目录>/backups/claude-code/`
- 最近一次写入结果：`<程序目录>/tako-config-backups.json`

如果目标文件原本不存在，备份文件会写入缺失文件标记；恢复时会删除后来创建的目标文件。

## 测试与验证

提交前建议至少运行：

```bash
bun run typecheck
bun run build
bun run rust:test
```

涉及 Tauri 配置、打包资源、deep-link、Windows 子系统或发布流程时，再运行：

```bash
bun run tauri:build
```

涉及 UI 的改动应同时检查桌面窗口尺寸下的布局，确保首页、导入配置、当前配置和弹窗都没有文字溢出或交互遮挡。

## 发布流程

发布由 `.github/workflows/release.yml` 负责，触发条件是推送语义化版本 tag：

```bash
git tag v0.1.0
git push origin v0.1.0
```

流程会先校验 tag，再生成 Release Notes，并创建或更新同名 GitHub Release。构建任务会从 tag 同步版本号到：

- `package.json`
- `src-tauri/tauri.conf.json`
- `src-tauri/Cargo.toml`

随后构建并上传产物：

- Windows：NSIS `.exe` 和 MSI `.msi`
- macOS：Apple Silicon + Intel universal `.dmg`

Release Notes 由 `scripts/generate-release-notes.mjs` 根据 Conventional Commit 生成，常用分类：

- `feat:`：Features
- `fix:`：Fixes
- `build:` / `ci:`：Build
- `docs:`：Documentation
- `test:`：Tests
- `chore:` / `refactor:` / `style:`：Maintenance

破坏性变更使用 `!` 或 `BREAKING CHANGE:` footer。

macOS 签名优先读取仓库密钥 `APPLE_SIGNING_IDENTITY`。如果密钥为空，CI 会使用 `APPLE_SIGNING_IDENTITY=-` 进行 ad-hoc signing。

## 维护原则

- 面向用户的流程优先保持“可预览、可备份、可恢复”。
- Home 保持轻量状态页，导入页负责写入，当前配置页保持只读。
- 服务商和账号相关逻辑放在 `integrations` / `providers` 下，不把根组件或入口文件变成大杂烩。
- 依赖升级优先兼容性和可验证性，避免为安全修复引入大范围无关升级。
