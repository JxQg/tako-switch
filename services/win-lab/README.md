# tako-win-lab — Tako Switch Windows 发版门禁与实机测试台

运行在河南 `.6` Windows 11 主机（`172.17.66.6:18795`，`C:\fleet\tako-win-lab\`），负责：

1. **Windows 3 门禁静态解包与注入验收**（不污染宿主配置）：
   - `localization`：检查语言包、ASAR 汉化入口锚点、词典覆盖率、PE Electron fuse/integrity 槽位。
   - `injection`：检查 3P 网关 / MCP / `model_catalog_url` / `AppxManifest.xml` 主入口锚点、国内 `claude-code` 伴随镜像可达性，以及隔离沙箱 `config.toml` / JSON 干跑校验。
   - `anomalyAudit`：核验 Authenticode 签名、PE 可执行清单、URL Protocol / Capability / StartupTask，并与 `manifest/baselines/<app>.windows.json` 基线差分。
2. **Gate 4 实机冷启动与 EventLog 崩溃探针**：
   - 在 `.6` 交互桌面会话中真实安装/拉起客户端，轮询进程存活与主窗口标题，采集 `Application`（Event ID 1000/1001/1026）、`AppxDeploymentServer`、`AppModel-Runtime` 与脱敏客户端启动日志尾部。
3. **上游候选与官方 Changelog 巡检**：
   - 定时拉取上游最新版本与更新日志、标注风险关键词，**绝不自动覆写线上 `manifest.json`**。
4. **运维中枢与 HQ 事件上报**：
   - 遵循 `PLUGIN-MOUNTING.md` v4 契约挂载至 `.26:8767/win-lab`（仅 EasyTier Mesh / 回环直连，拒绝公网反代透传头）。
   - Prometheus 黑盒探活 `http://172.17.66.6:18795/healthz`。
   - 门禁失败（`winlab.gate_failed`）、实机崩溃（`winlab.smoke_crash`）、发现上游新版（`winlab.upstream_candidate`）脱敏后经冷却去重上报至 HQ（`http://172.17.66.26:8789/v1/events`）建单分诊；通过回执走轻量 SSE（`winlab.gate_passed`）；同时同步至客户端数据台（`http://172.17.66.15:18080/v1/{events,errors}`）。

## 本地启动与测试

```bash
# 运行全部单测与黑盒测试（零第三方依赖）
WIN_LAB_QUIET_LOG=1 python3 -m unittest discover -s services/win-lab/tests -p 'test_*.py' -v

# 本地启动（默认端口 18795）
python3 services/win-lab/server.py --port 18795
```

## Windows `.6` 部署与运维

```powershell
# 安装 / 重启常驻计划任务 \TakoWinLabService
powershell -NoProfile -ExecutionPolicy Bypass -File C:\fleet\tako-win-lab\repo\services\win-lab\deploy\install-win6-service.ps1

# 健康检查
curl.exe -fsS http://172.17.66.6:18795/healthz
```

## HTTP API 一览

| 路径 | 方法 | 说明 |
|---|---|---|
| `/healthz` | `GET` | 存活与数据库状态探针（Prometheus `http_2xx`） |
| `/api/plugin/manifest` | `GET` | OPS Console v4 插件自描述与角标 |
| `/win-lab` | `GET` | 内嵌 v4 看板 UI（支持 `?embed=1&theme=light\|dark&host_nav=1`） |
| `/api/overview` | `GET` | 节点资源、4 款客户端门禁矩阵、上游链路与最近任务 |
| `/api/runs` / `/api/runs/:id` | `GET` | 历史任务列表与单次完整日志/门禁/EventLog 详情 |
| `/api/runs/gate` | `POST` | 入队 Windows 三门禁任务（`app_id`, `candidate_url`, `update_baseline`） |
| `/api/runs/smoke` | `POST` | 入队 Gate 4 实机安装+冷启动+EventLog 崩溃探针 |
| `/api/candidates` / `/api/candidates/check` | `GET`/`POST` | 查看/触发上游版本与官方 Changelog 风险巡检 |
| `/api/hq/events` / `/api/hq/report` | `GET`/`POST` | 查看 HQ 上报台账 / 手动触发 HQ 上报或连通性自检 |
| `/api/sync-repo` | `POST` | `git pull --ff-only` 同步最新锁版本与词典 |
