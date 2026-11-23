# Tako Switch 产品演进路线图（v2.0.x）

本文档记录 Tako Switch 桌面端自 `v2.0.x` 单一二进制架构定型以来的核心迭代轨迹，以及 `v2.0.8` 的两大交付模块。

---

## 1. v2.0.x 历史演进概览

| 版本阶段 | 核心主题 | 关键工程交付 |
|---|---|---|
| `v2.0.0` – `v2.0.4` | 单一二进制四角色底座 | 定型 `tako-switch` 单可执行文件四角色架构（GUI 主界面、本地网关 `gateway-proxy`、内置 6 合 1 `stdio` MCP 宿主、`status-line`）；打通 DSH Tako、Codex、Claude Desktop（`Claude-3p` 原生隔离槽）、Google Antigravity 一键安装、模式无损切换与双端点（阿里云 OSS + GitHub）自更新。 |
| `v2.0.5` – `v2.0.6` | 内联超级 MCP 实景剧场与现场试跑 | 移除旧弹窗，升级为主界面内联「效果演示 + 现场试跑（Live Playground）」双模式剧场，覆盖生图、生视频、冰糖多情绪真声、实时联网溯源与多模型外脑参谋。 |
| `v2.0.7` | 多机 OAuth 设备隔离与基础中文增强 | 引入持久化 8 位设备指纹 `~/.tako/switch/device-id` 防止多台电脑登录互顶 Token；为三端注入首版配置文件级中文回复与系统语言偏好。 |
| `v2.0.8` | 三端汉化一步到位 + 模型广场 PR #296 同步 | 主界面一级入口一键三端汉化、Antigravity 纯 Rust 界面汉化补丁器与字节级原厂签名无损还原；完整同步网关模型广场人民币实付计价、明细/均价双视图、官方折扣倍率、限时特惠与 6 场景快捷筛选。 |
| `v2.0.9` – `v2.0.10` | 代理直连流式优化与更新日志体系 | 新增默认绕过系统代理直连 Tako 网关、大请求体流式透传与下游断连即时中止；建立以 `CHANGELOG.md` 为单一真相源的自动提取发版流水线与客户端内更新日志弹窗。 |
| `v2.1.0` – `v2.1.1` | Claude Desktop 全量界面汉化 + 跨平台权限引导解耦 | 新增纯 Rust 双 ASAR（`app.asar` + `ion-dist.asar`）Claude Desktop 全量简体中文补丁与字节级无损还原，覆盖 Chat / Cowork / Claude Code 工作台与 ICU 占位符安全校验；解耦配置文件级汉化与 ASAR 写入，新增 macOS「App 管理」权限弹窗直达、Windows Squirrel 数值排序、PE32+ 完整性修补与临时 UAC 提权工作进程。 |
| `v2.2.0` | 极简界面重设计 + EdgeOne 网站上线 MCP + Antigravity 全量汉化 | 左侧栏「全局 / 软件独立设置」单域分栏架构与 MCP 抽屉剧场降噪重设计；新增 `Tako Web Page`（腾讯云 EdgeOne Pages）免密钥浏览器授权网站上线 MCP 全客户端同步与 Node.js 运行时检测；Antigravity 独立简体中文词典扩充至 3,041 条并升级动态模板/Shadow DOM/同源本地 iframe 零误伤翻译引擎。 |
| `v2.2.1` | Claude 汉化启动修复 + Antigravity 词典扩充与防拆词误翻 + 3p MCP 去重 | 修复 macOS ad-hoc 重签携带受限 entitlements 导致 Claude Desktop 汉化后 AMFI SIGKILL 无法启动的问题并优化 ASAR 重打包性能；Antigravity 词典扩充至 3,630 条并引入 React 安全相邻文本节点合并翻译与短词守卫；修复 Claude 3p `tako-web-page` 重复写入触发组织同名告警的问题。 |
| `v2.2.2` | 新版 Claude 汉化启动修复 + 清爽单栏主页与二级设置页 + 汉化进度条 | 修复新版 Claude Desktop 汉化后无法启动并保持无损还原；回归全宽单栏主页加清晰二级设置页；新增汉化实时进度条及客户端升级后自动补打汉化。 |
| `v2.2.3` | 客户端下载自动测速选路 + 国内加速正式接入 | 安装/升级客户端时并发测速自动优选最快线路（国内加速与官方源谁快走谁），下载停滞自动切备用源；更新说明改用通俗易懂的表达方式。 |
| **`v2.2.4`（当前）** | **恢复 Claude 完整推理强度 + Claude 汉化词典扩充至 1.1 万条** | 放开 Claude 模型 `maxEffort` 上限至 `max`，恢复模型原生「超高 / 极致」推理强度；新增近 9,000 条 Claude Desktop 常用界面中文词条，补齐项目、侧边栏、设置、Cowork 与 Claude Code 等页面漏翻。 |

---

## 2. v2.0.8 两大核心交付模块

### 模块一：三端汉化一步到位 + Antigravity 界面汉化补丁器

1. **主界面一级汉化开关（`main-chinese-ui-toggle`）**：
   - 将原本深藏在设置抽屉内的中文偏好提升到「桌面客户端」标题栏，直观展示当前汉化状态，支持一键开启或关闭还原。
   - 新增三端独立无损中文同步能力（`sync_standalone_chinese_locale`）：无论客户端当前处于 `Official` 还是 `Tako` 模式，点击一键汉化均可立即写入/还原：
     - **Codex Desktop / CLI**：`~/.codex/config.toml` `[desktop].localeOverride = "zh-CN"`、`~/.codex/AGENTS.md` 受控中文块、macOS `AppleLanguages`。
     - **Claude Desktop / Claude Code**：`1p` 与 `3p` `config.json` `"locale": "zh-Hans"`、`~/.claude/settings.json` `"language": "Simplified Chinese"`、macOS `AppleLanguages`。
     - **Google Antigravity**：`~/.gemini/AGENTS.md` 受控中文块、macOS `AppleLanguages` 及可选界面汉化补丁。
2. **Antigravity 纯 Rust ASAR 界面汉化与原厂签名级无损还原（`antigravity_i18n.rs`）**：
   - **零外部 Node 依赖**：纯 Rust 实现 Chromium Pickle 4 字节对齐的 Electron ASAR v1 解析与重打包器，保留 `unpacked: true` 条目原样不动，自动重算打包文件偏移与 4MB 分块 SHA-256 `integrity`。
   - **安全菜单与 Hub 界面汉化**：注入 `__tako_i18n_patch.json` 版本标记，对 `dist/menu.js`、`dist/tray.js`、`dist/updater.js`（保留英文与中文 `updateActions` 双键映射及顶层菜单查找兼容性）及 `dist/preload.js`（仅限本地 `127.0.0.1`/`localhost` 顶层窗口的错误隔离精确词典替换，严格跳过代码块、Monaco 编辑器与 Xterm 终端）执行幂等汉化。
   - **升级感知与字节级还原**：在 `~/.tako/switch/backups/antigravity/ui_patch/` 备份 `app.asar.orig` 及 macOS `Info.plist.orig`、`MacOS_Antigravity.orig`、`CodeResources.orig` 与 `patch-meta.json`；自动识别客户端自升级后的 `OutdatedAfterUpgrade` 状态（防止旧备份降级覆盖新版客户端），支持一键重打补丁或 100% 字节级恢复原厂签名。

### 模块二：同步网关模型广场 PR #296 新特性

1. **免登录直达模型广场（`open-model-square-btn` & `list_plaza_models`）**：
   - 主界面新增「模型广场」一级入口按钮，无需先登录或切入特定客户端即可直接拉取公共 `/api/pricing` 与 `/api/perf-metrics/summary` 浏览全广场模型（涵盖对话、推理、视觉、编程、生图、生视频、语音、向量全品类），并在同一弹窗内自由切换 `广场总览` / `Codex` / `Claude` / `Antigravity` 视图。
2. **人民币实付计价与双视图体系（`src/lib/plaza-pricing.ts`）**：
   - 默认以人民币实付价（`¥`，按平台充值比例 `CNY = internalUSD / 3` 折算）展示，支持一键切换 `$ 额度`（附醒目橙色提示 `按充值 ¥1 = $3 折算，非美元现金`）及 `/1M` · `/1K` Token 单位（按次计费模型不受 `/1K` 缩放影响）。
   - 支持 **「明细价 | 均价」** 双视图：`明细价` 清晰列出 `输入 / 缓存读 / 输出` 三列单价及限时特惠划线原价对比；`均价` 突出展示 `7日实付`（无近期流量回退 `标准负载`）并附 `标准负载 · 缓存命中率` 对照。
3. **官方折扣绿标、限时特惠橙标与 6 场景快捷筛选**：
   - 自动计算渠道最低倍率相对官方目录价的折扣倍数（`officialMultiplier = min(channel_ratio) / 20.4`），渲染绿色折扣徽章（如 `≈ 0.06x 官方 0.6折`）与限时半价徽章（`🔥 限时半价 · 11-30`）。
   - 顶部集成 `全部`、`🔥 限时特惠`、`≤9折`、`编程`、`1M+上下文`、`生图` 6 个高频场景胶囊，配合左侧厂商与 8 类能力筛选，帮助用户快速选型。
