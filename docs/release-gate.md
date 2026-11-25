# 桌面客户端受控发版与三门禁验收指南

Tako Switch 不再自动跟随上游客户端最新版。所有客户端版本统一由 [`manifest/pinned-versions.json`](../manifest/pinned-versions.json) 锁定管理，[`manifest/manifest.json`](../manifest/manifest.json) 仅由此锁文件生成。

## 三条强制发布门禁

每个桌面客户端新版本在提升（`promote`）到线上之前，必须在隔离环境中完成以下三项验收：

1. **能够汉化（`localization`）**
   - `Antigravity`：`app.asar` 可解析、Electron integrity fuse/槽位可兼容、渲染层补丁入口及汉化词典命中正常。
   - `Claude Desktop`：`app.asar` + `ion-dist.asar` 完整、`resources/i18n/en-US.json` 与 `.vite/build/*.js` 语言映射/正则入口、`ion` locale 白名单全部命中，词典覆盖率无大幅回退。
   - `Codex`：`zh_CN.lproj` 存在且 `app.asar` 保留 `desktop.localeOverride` 锚点。
   - `DSH Tako`：原生中文客户端 bundle 完整。
2. **里面的内容都能正常放进去（`injection`）**
   - 网关地址、模型目录、3P profile、MCP（`tako` / `tako-web-page`）、中文规则与禁自更新配置契约锚点完整。
   - 对 `Claude Desktop`，同步检查内嵌 `claude-code-releases` CLI 版本（如 `2.1.331`）；若内嵌版本变化，须一并更新 `pinned-versions.json` 里的 `companionArtifacts.claude-code` 并镜像到 `dl.lven.asia/apps/claude-code/`。
3. **没有额外奇奇怪怪的东西（`anomalyAudit`）**
   - 与 `manifest/baselines/<app>.macos.json` 对比 `Info.plist`（Bundle ID、URL Schemes、隐私权限）、可执行文件、Helper App、后台 LoginItem/LaunchService 与内嵌外部下载源（如 VM/CLI 下载器），任何异常新增项都会告警拦截。

## 标准操作流程（macOS 优先）

### 1. 巡检上游新版本与官方更新日志（只读，不改线上）

```bash
python3 scripts/manage-releases.py check \
  --out-json /tmp/upstream-candidates.json \
  --out-md /tmp/upstream-candidates.md
```

定时 CI（`.github/workflows/sync-manifest.yml`）每天也会自动跑这一步并产出候选报告，但**不会自动修改或推送 `manifest/manifest.json`**。

### 2. 在 macOS 隔离环境中跑三门禁验收

对候选安装包（`.zip` / `.dmg`，自动解包到临时目录，不碰 `/Applications`）：

```bash
python3 scripts/manage-releases.py verify-macos \
  --app claude \
  --archive /path/to/Claude-candidate.zip \
  --out-json /tmp/gate-claude.json
```

或对本机已安装版本做基线校验 / 更新基线：

```bash
python3 scripts/manage-releases.py verify-macos --app all
python3 scripts/manage-releases.py verify-macos --app claude --update-baseline
```

### 3. 审核通过后 Promote 并同步国内镜像

```bash
python3 scripts/manage-releases.py promote \
  --app claude \
  --candidate /tmp/upstream-candidates.json \
  --gate-report /tmp/gate-claude.json
```

提交 `manifest/pinned-versions.json`、`manifest/manifest.json`（及更新后的基线文件）到 `main` 后，在 GitHub Actions 手动运行 **Managed Desktop Release Pipeline** 并选择 `mode = mirror-pinned`，自动完成安装包及 `claude-code` 伴随二进制的国内 CDN 镜像与 `manifest.json` 同步。

## Windows 测试台（河南 `.6` 常驻服务 `tako-win-lab`）

Windows 端已接入常驻测试台微服务 [`services/win-lab/`](../services/win-lab/)，运行在河南 `.6`（`172.17.66.6:18795`），提供：

1. **Windows 3 门禁静态解包与注入验收**（不污染宿主配置）：
   - `Codex`（`.msix`）：纯解包解析 `AppxManifest.xml`（校验 `OpenAI.Codex` Identity、可见主入口 vs 后台辅助入口、Protocol/Capability/StartupTask）、`app.asar` 锚点（`localeOverride` / `model_catalog_url` / `approval_mode`）、`zh-CN` 语言资源与 `config.toml` 反斜杠路径隔离干跑。
   - `Claude Desktop`（`.exe`）：免安装提取内嵌 `.nupkg` 校验 `app.asar` + `ion-dist.asar` 汉化覆盖率与 3P 契约字段、`Claude.exe` Electron fuse/integrity 槽位、以及国内 `dl.lven.asia/apps/claude-code/<ver>/win32-x64/claude.exe.zst` 镜像可达性。
   - `Antigravity`（Inno `.exe`）与 `DSH Tako`（NSIS `.exe`）：隔离目录静默解包，校验 `app.asar` / PE Electron fuse / Authenticode 签名与 Windows 基线差分（`manifest/baselines/<app>.windows.json`）。
2. **Gate 4 实机冷启动与 EventLog 崩溃探针**：
   - 在 `.6` 交互桌面会话中真实拉起候选包，持续轮询进程存活与主窗口标题，并抓取 `Application`（Event ID 1000/1001/1026）、`AppxDeploymentServer`、`AppModel-Runtime` 及客户端启动日志尾部，自动脱敏保存并清理现场。
3. **运维控制台与 HQ 事件上报闭环**：
   - 以标准 v4 插件挂载至 `.26:8767/tako/win-lab`（`parent_nav: tako`），Prometheus 黑盒探活 `http://172.17.66.6:18795/healthz`。
   - 门禁失败（`winlab.gate_failed`）、实机崩溃（`winlab.smoke_crash`）、上游发现待审新版（`winlab.upstream_candidate`）自动脱敏并按冷却去重上报至 HQ（`http://172.17.66.26:8789/v1/events`）建单分诊，通过回执走轻量 SSE（`winlab.gate_passed`），同时同步异常至客户端数据台（`http://172.17.66.15:18080/v1/{events,errors}`）。

### 命令行一键调用 `.6` Windows 测试台

```bash
# 全量跑 Windows 三门禁（或指定单个客户端 --app codex）
python3 scripts/manage-releases.py verify-windows --app all --out-json /tmp/win-gates.json

# 更新 Windows 基线档案
python3 scripts/manage-releases.py verify-windows --app codex --update-baseline

# 执行 Gate 4 实机安装 + 冷启动 + EventLog 崩溃复测
python3 scripts/manage-releases.py verify-windows --app codex --smoke --hold-seconds 15
```
