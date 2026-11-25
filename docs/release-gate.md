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

## Windows 环境演进规划

当前先以 macOS 环境跑通三门禁与基线差分；后续接入独立 Windows 测试机 / GitHub `windows-latest` runner 时，沿用同一份 `pinned-versions.json` 与 `baselines/<app>.windows.json` 扩展 `verify-windows`（重点校验 MSIX `AppxManifest.xml` 入口、PE ASAR integrity 校验、杀软/UAC 静默安装与冷启动存活探针）。
