# Tako Switch 通用扩展中心（v2.3.0）分阶段验收与总合流测试计划

> **契约基准**：[`docs/dispatch/v2.3.0-extensions-hub.md`](./dispatch/v2.3.0-extensions-hub.md) · [`AGENTS.md`](../AGENTS.md)
> **覆盖模块**：首页拆分与超级 MCP 提纯 · 通用 MCP 管理 · 提示词预设管理 · Skills 技能管理 · 三端 CLI 插件管理 · 低版本平滑升级与响应式防越界

---

## 1. 验收总则与沙盒红线

1. **三阶段各自独立闭环验收**：
   - `feat/ext-hub-phase1`、`feat/ext-hub-phase2`、`feat/ext-hub-phase3` 三个分支在各自独立 Git Worktree 中完成开发后，必须在各自工作树内独立跑通：`npm run typecheck && npm run build`、`cargo test`、`cargo clippy --all-targets -- -D warnings`、以及 `scripts/verify_client_mcp_ui.py`。
2. **宿主机零污染纪律**：
   - 所有 Rust 测试强制使用 `tempfile::TempDir` + `AppPaths::for_test(tmp.path())`，断言 `paths.is_test == true`。
   - Phase 2 的 GitHub 仓库归档拉取测试与 Phase 3 的 `gstatic` 目录拉取 / CLI 插件命令测试，全部通过本地临时 HTTP Mock Server 与沙盒临时假 CLI 脚本（Windows `.cmd` / Unix `.sh`）完成确定性闭环测试，绝不依赖外网稳定性或污染本机真实 `~/.claude`、`~/.codex`、`~/.gemini`。
3. **合流抽象与合规审计**：
   - 三个分支合入 `dev_jxqg` 后，审查并合并前后端重复逻辑至 `src-tauri/src/extensions/mod.rs` 与 `src/components/extensions/ExtensionPrimitives.tsx`；清理废弃组件与多余参数/死代码；同步更新 `AGENTS.md` 与 `CLAUDE.md`（保持字节级一致）。

---

## 2. 分阶段验收用例矩阵

### 2.1 Phase 1 验收矩阵（`feat/ext-hub-phase1`）

| 用例编号 | 验收维度 | 预期判定标准（Pass/Fail Oracle） |
| :--- | :--- | :--- |
| `TC-P1-01` | 首页三层拆分与纯净文案 | `OverviewPanel` 中 `Tako 超级 MCP` 副标题精确为 `Tako 模式下自动注入，开关即时生效`，无 `本项目特有王牌模组`，旧 `open-multi-sync` 按钮已移除；底部新增「通用扩展中心」4 卡片条（`grid-cols-2 min-[760px]:grid-cols-4`），`McpCenterPanel` 保留全部既有 `REQUIRED_TESTIDS`。 |
| `TC-P1-02` | 低版本升级 MCP 自动收编与内置隔离 | 沙盒预置旧版 Claude `1p`、Codex `config.toml`、Antigravity `mcp_config.json`（含自定义服务及内置 `"tako"`、`"tako-web-page"`、`"node_repl"`）；首次读取快照自动收编全部第三方 MCP 至 `extensions/mcp.json`，且绝不暴露或误删内置 `"tako"` / `"tako-web-page"` / `"node_repl"`。 |
| `TC-P1-03` | Claude 三处联动落盘与全关保留 | 在通用 MCP 中开启 `Claude` 列时，同步写入 `claude_1p/claude_desktop_config.json`、`claude_3p/claude_desktop_config.json`（若存在或 Tako 模式）与 `~/.claude.json`（保留 `.claude.json` 其它顶层字段不丢失）；将某 MCP 三端开关全关后，三端配置文件中均已移除，但中心库 `extensions/mcp.json` 仍保留该条目可随时重开。 |
| `TC-P1-04` | MCP 智能粘贴解析与备份恢复 | 支持解析标准 `{"mcpServers":{...}}` JSON、单对象 JSON 与 Codex `[mcp_servers.xxx]` TOML 文本；导出备份 JSON 后再导入可无损恢复中心库并同步落盘。 |
| `TC-P1-05` | 提示词预设与一键汉化双区无损合成 | 在 `~/.codex/AGENTS.md`、`~/.claude/CLAUDE.md`、`~/.gemini/AGENTS.md` 已含 `<!-- tako-switch:chinese-ui:start -->...` 托管块的条件下，执行预设新建、启用、切换、保存修改、停用，底部汉化块 100% 完好无损；反之再调用 `sync_standalone_chinese_locale(true/false)` 切换汉化开关，上方激活的提示词正文 100% 完好无损。 |
| `TC-P1-06` | 提示词历史自动归档、外部回写与跨端同步 | 目标文件已有未纳管手写内容时，首次加载自动备份原文件并生成「导入的原始提示词备份」；外部编辑器修改已启用预设文件后重新读取可自动回写并计算准确字节数；「同步到其他端 ▾」可精确同步至勾选的其余一端或两端。 |

### 2.2 Phase 2 验收矩阵（`feat/ext-hub-phase2`）

| 用例编号 | 验收维度 | 预期判定标准（Pass/Fail Oracle） |
| :--- | :--- | :--- |
| `TC-P2-01` | 主副本收编与软链优先/复制自动回退 | 沙盒三端已有技能目录可自动收编至 `~/.tako/switch/skills/<id>`；`auto` 策略下优先建立软链，无软链权限或显式 `copy` 策略下自动完成原子目录复制（跳过 `.git`、`node_modules`），Antigravity 额外 CLI 目录存在时同步联动。 |
| `TC-P2-02` | 三端独立开关与主副本防丢 | 关闭单端或全部三端开关时，目标端技能目录被清理，但 `~/.tako/switch/skills/<id>` 主副本依然保留，重新打开开关立即恢复；执行「彻底卸载」时同时移除三端目录与主副本。 |
| `TC-P2-03` | 本地目录与 `.zip` 安全导入 | 支持从本地目录或 `.zip` 归档（含单技能根目录与多技能子目录）导入并解析 `SKILL.md` frontmatter；若 `.zip` 内含 `../evil` 路径穿越条目，必须拒绝并报错拦截，零越界文件写入。 |
| `TC-P2-04` | GitHub 仓库归档发现与哈希更新检测 | 通过本地 HTTP Server 模拟仓库 ZIP 归档，验证仓库源增删、技能发现列表解析、一键安装到选定端；当远端归档中 `SKILL.md` 或附属脚本更新后，准确识别 `updateAvailable == true` 并支持一键更新同步至已启用端。 |

### 2.3 Phase 3 验收矩阵（`feat/ext-hub-phase3`）

| 用例编号 | 验收维度 | 预期判定标准（Pass/Fail Oracle） |
| :--- | :--- | :--- |
| `TC-P3-01` | CLI 探测优先级与未装 CLI 空状态 | 验证内置桌面端路径优先于系统 `PATH`；当三端均无可用 CLI 时返回 `cliAvailable == false`，前端渲染 `ext-plugins-empty-unsupported`（「未检测到对应 CLI，当前客户端暂不支持该功能」），不抛未捕获异常。 |
| `TC-P3-02` | Claude & Codex 插件列表、市场与 `config.toml` 启停 | 通过沙盒假 CLI 脚本模拟 `plugin list [--available] --json` 与 `marketplace list --json`，验证已安装/可发现解析、`.claude-plugin/plugin.json` 补读、以及 Codex 插件启停无损读写 `config.toml` `[plugins."<id>"].enabled`（保留文件中其它配置与注释）。 |
| `TC-P3-03` | Antigravity JSONC 清单、`gstatic` 缓存与授权提示 | 验证带 `//` 与尾随逗号的 JSONC `plugin.json` 正常解析；模拟官方与伙伴两份 `marketplace.json` 拉取并落盘 `extensions/plugin_cache.json`；安装返回停用态时标记 `needsConfiguration == true`；Antigravity 下市场源入口禁用。 |
| `TC-P3-04` | 参数防注入与占用锁错误识别 | 传入 `-malicious`、`; rm -rf` 或非法格式 ID 时在执行子进程前直接拦截返回校验错误；CLI 输出目录占用错误时准确映射为友好提示（`PLUGIN_IN_USE`）。 |

### 2.4 合流总验收矩阵（`dev_jxqg`）

| 用例编号 | 验收维度 | 预期判定标准（Pass/Fail Oracle） |
| :--- | :--- | :--- |
| `TC-ALL-01` | 通用抽象复用与零冗余代码 | 前后端公共组件与方法统一收敛至 `ExtensionPrimitives.tsx` 与 `extensions/mod.rs`，旧临时组件 `MultiPlatformSyncPanel.tsx` 与无用参数/方法清理干净，`tsc --noEmit` 与 `cargo clippy --all-targets -- -D warnings` 零告警。 |
| `TC-ALL-02` | `820×650` 与 `720×560` 双尺寸 + 深浅色双密度视觉实测 | 启动开发预览，在 `820×650` 默认尺寸与 `720×560` 极限最小窗口宽度（覆盖浅色/深色、紧凑/舒适密度）下逐一核验首页与扩展中心全部 4 个子模块：`document.documentElement.scrollWidth <= window.innerWidth`，按钮、搜索框与预设 Tab 零位移、零截断、零刺眼硬编码底色。 |
| `TC-ALL-03` | 文档同步与全量 CI 门禁 | `AGENTS.md` 与 `CLAUDE.md` 同步更新且字节级完全一致；`AGENTS.md` 所列四道提交前门禁命令 100% 通过。 |
