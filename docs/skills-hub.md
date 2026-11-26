# Skills 技能管理

## 管理与同步

从首页进入“通用扩展中心 · Skills 技能”。首次打开时，中心会扫描 Claude、Codex 和 Antigravity 的已有技能，并将尚未纳管的技能复制到主副本目录。同名技能内容不同时，各端原文件保持不变，列表标记“有差异”。

| 数据 | 路径 |
| --- | --- |
| 技能主副本 | `~/.tako/switch/skills/<id>/` |
| 技能记录、同步策略和默认同步端 | `~/.tako/switch/extensions/skills.json` |
| GitHub 仓库源 | `~/.tako/switch/extensions/skill_repos.json` |
| 首次替换前的客户端文件备份 | `~/.tako/switch/backups/extensions-skills/` |

Claude 的目标是 `~/.claude/skills/<id>/`。Codex 的目标是 `CODEX_HOME/skills/<id>/`，未设置 `CODEX_HOME` 时使用 `~/.codex/skills/<id>/`。Antigravity 的目标是 `~/.gemini/config/skills/<id>/`；如果 `~/.gemini/antigravity/skills/` 或 `~/.gemini/antigravity-cli/skills/` 已存在，也会同步更新。

“更多”菜单提供三种同步策略：

| 策略 | 行为 |
| --- | --- |
| 软链优先，自动回退复制 | 默认策略。先创建目录软链；无法创建时复制技能。Windows 未开启开发者模式时也可使用。 |
| 仅使用软链 | 创建失败时显示错误，并保留原有内容。 |
| 强制复制 | 将文件复制到临时目录，再替换目标目录。 |

开关下方显示实际方式。复制时跳过 `.git`、`node_modules`、`__pycache__`、`.venv`、`venv`、`.pytest_cache`、`.mypy_cache`、`.ruff_cache`、`.DS_Store` 和 `Thumbs.db`（保留 `dist`、`build` 和 `assets`）。扫描收编时若个别技能目录超限、断链或格式损坏，会自动隔离跳过并在页面顶部展示原因，其余技能与首页计数不受影响。

关闭平台开关只移除对应平台的链接或副本。**三端全部关闭后，主副本仍然保留。**“三端对齐”会按主副本刷新三端；“彻底卸载”经确认后删除三端目标和主副本。已有客户端文件的首次备份不随卸载删除。

## 导入技能

“导入技能”支持本地目录和 ZIP 文件。目录必须包含 `SKILL.md`。ZIP 可通过选择文件、拖放文件或后端路径参数导入，支持根目录单技能，以及一级、二级子目录中的多个技能。

导入前可以选择同步端，也可以全部不选，仅保存主副本。默认同步端在“仓库发现”中设置。重复标识符会报错，批量导入不会覆盖已有技能。目标客户端中已有同名目录时，先执行“扫描收编本机技能”。

`SKILL.md` 的 YAML frontmatter 支持 `name`、`description`、单引号、双引号和多行文本。缺少名称时使用目录名。普通目录和 ZIP 子目录的名称用作技能 ID；根目录 ZIP 使用 frontmatter 的 `name`。

本地目录导入与收编单技能限制为 512 MiB；ZIP 压缩体积和总解压体积均限制为 200 MiB（大体积 ZIP 自动按 4 MiB 分片流式上传解压），最多 20,000 个条目、16 层路径，单次最多 200 个技能。路径中的 `..`、绝对路径、盘符、反斜杠、特殊设备名、重复路径和链接条目会导致整个导入失败。本地目录中的链接只能指向技能目录内部。

## GitHub 仓库

中心默认订阅 `anthropics/skills` 的 `main` 分支。可以添加 `owner/repo`、指定分支、停用或删除仓库源。刷新仓库后展示其中包含 `SKILL.md` 的技能。此功能不接入第三方技能市场。

下载使用 GitHub 仓库 ZIP 归档。`main` 返回 404 时尝试 `master`。每次请求都重新读取“绕过系统代理”设置；开启时直连，关闭时遵循系统代理。仓库配置中的可选 `archiveUrlOverride` 用于指定归档源，支持 `{branch}` 占位符；测试使用本地 HTTP 地址。

中心按相对路径排序计算内容 SHA-256。ZIP 条目顺序、时间戳和外层仓库目录名不影响内容指纹。`SKILL.md` 或附属脚本变化后，“检查更新”会标记对应来源的已安装技能。更新只刷新已启用平台，不重新开启停用端。

主副本或已启用的复制副本存在本地修改时，更新会停止并显示原因。先备份修改，再决定恢复原版或使用主副本重新同步。技能安装和更新不会执行其中的脚本。

## 开发验证

后端接口均提供接收 `AppPaths` 的 `*_for_paths` 入口。黑盒测试使用 `TempDir`、`AppPaths::for_test` 和本地 HTTP 服务，不读取真实客户端配置，也不依赖 GitHub 公网服务。

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test ext_hub_phase2_blackbox
npx playwright install chromium
npm run test:skills-ui
npm run dev:skills-preview
```

UI 测试在浏览器中模拟 Tauri IPC，覆盖 `720×560` 和 `820×650`、浅色和深色、紧凑和舒适密度，共 8 组组合。测试检查列表、搜索、仓库发现、菜单、导入抽屉、ZIP Base64 参数、开关和卸载确认。截图保存在 `src-tauri/target/phase2-ui/`。原生拖放通过 Tauri 的文件拖放事件接收路径；浏览器文件选择通过 Base64 提交。

`dev:skills-preview` 启动隔离浏览器预览，默认端口为 `18042`。端口占用时自动选择其他端口，并输出实际地址。预览只使用内存测试数据，不读取或写入客户端配置。
