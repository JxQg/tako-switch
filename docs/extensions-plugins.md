# 插件中心

## 入口与平台

从首页“通用扩展中心”进入插件管理。插件中心管理 Claude、Codex 和 Antigravity，提供已安装、发现和市场源视图。MCP 和 Skills 入口继续打开现有同步功能。

插件中心依赖对应客户端的 CLI（命令行程序）。探测顺序是桌面安装包内置 CLI、用户 CLI 安装目录、系统 `PATH`。状态徽章区分内置和 PATH 来源。未检出 CLI 时显示空状态，不安装额外软件。

## 三端行为

| 平台 | 读取与操作 | 配置边界 |
| --- | --- | --- |
| Claude | `plugin list [--available] --json`，支持安装、更新、启停、卸载和市场管理 | 从安装目录补读清单；用户 scope 可修改，项目和管理员 scope 只读 |
| Codex | `plugin list [--available] --json`，安装、更新、卸载和市场管理使用 CLI | 启停修改 `config.toml` 的 `[plugins."<id>"].enabled`，保留其余字段、表和注释 |
| Antigravity | `agy plugin list --json`；启停和卸载使用已安装目录名 | 安装和更新均调用 `agy plugin install <name>@<marketplace>`；不支持增删市场源 |

Claude 清单位于安装目录的 `.claude-plugin/plugin.json`。Antigravity 从 `plugins/<目录名>/` 及其 `.antigravity-plugin`、`.gemini-plugin`、`.claude-plugin` 子目录读取 `plugin.json`。清单支持 JSONC 的行注释、块注释和尾随逗号，字符串中的网址和注释符号保持原样。

Antigravity 安装或更新后会重新读取已安装状态。插件仍停用时，界面提示“需客户端授权”。插件开关和卸载始终使用 CLI 返回的目录名，包括旧版负数目录名。

## 存储与网络

所有核心入口接收 `AppPaths`。Codex 子进程使用 `paths.codex_dir` 作为 `CODEX_HOME`。首次修改 Codex 配置前，使用现有备份机制保存原文件；配置和缓存均通过 `write_atomic` 写入。

Antigravity 并发请求官方目录与伙伴目录：

- `https://www.gstatic.com/antigravity/plugins/marketplace.json`
- `https://www.gstatic.com/antigravity/plugins-partners/marketplace.json`

请求遵循 `bypassSystemProxy`。两个来源分别缓存到 `paths.switch_dir()/extensions/plugin_cache.json`。单源失败时保留该来源的旧缓存；全部失败时使用已有缓存。没有缓存时，已安装列表仍可用，发现视图显示来源不可用提示。快照中的 `catalogFromCache` 和 `warnings` 区分缓存与实时结果。

## 执行与错误

插件 ID 接受安全名称或 `name@marketplace`。名称段只能包含 ASCII 字母、数字、点、下划线和连字符，首字符必须是字母或数字。Antigravity 已安装目录额外兼容负号纯数字，但必须先匹配实际已安装记录。市场来源接受 `owner/repo` 或不带凭据、查询参数和片段的 HTTPS 地址。

CLI 子进程关闭 stdin。Windows 使用无窗口进程，批处理通过 `cmd.exe /d /v:off /s /c` 执行。列表、启停、卸载和移除市场的超时为 60 秒；安装、更新和添加市场为 180 秒。超时会中止子进程。插件修改操作串行执行，避免并发覆盖配置。

| 错误码 | 界面处理 |
| --- | --- |
| `PLUGIN_IN_USE` | 提示关闭对应客户端会话后重试，不自动关闭会话 |
| `PLUGIN_NEEDS_TERMINAL_CONFIRM` | 提示在终端确认并信任市场来源后重试 |
| `PLUGIN_INVALID_ARGUMENT` | 在执行 CLI 前拒绝非法参数 |
| `PLUGIN_SCOPE_READ_ONLY` | 拒绝修改项目或管理员管理的插件 |
| `PLUGIN_CLI_TIMEOUT` | 提示命令超时，不自动重试 |

## 测试

Rust 黑盒测试使用 `TempDir` 和 `AppPaths::for_test`。测试不读取宿主机 PATH。`PluginTestOverrides` 仅接受沙盒内 CLI 路径和本地 HTTP 目录地址，不能通过 Tauri IPC 注入。

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test ext_hub_phase3_blackbox
```

浏览器回归使用模拟 IPC，不连接本机客户端。先启动 Vite，再运行测试：

```bash
npm run dev -- --port 1433
npm run test:extensions-ui
```

默认测试地址为 `http://127.0.0.1:1433/`，可通过 `EXT_HUB_PREVIEW_URL` 覆盖。Windows 默认使用已安装的 Chrome；其他平台使用 Playwright Chromium。可以设置 `EXT_HUB_CHROME` 指定浏览器可执行文件。

测试覆盖 `720×560` 和 `820×650` 两种窗口、深浅色、紧凑与舒适密度，以及搜索、安装、更新、卸载、市场管理、未检出 CLI、错误提示和平台切换。截图保存在 `src-tauri/target/ext-hub-phase3-ui/`。
