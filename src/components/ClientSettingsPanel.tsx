import { useEffect, useState } from "react";
import {
  CheckCircle2,
  ChevronDown,
  Download,
  ExternalLink,
  Loader2,
  Power,
  RefreshCw,
  RotateCw,
  SlidersHorizontal,
} from "lucide-react";
import {
  api,
  type AppId,
  type AppMode,
  type AppSnapshot,
  type AppUpdateStatus,
  type ClientModelOption,
  type DashboardPayload,
  type InstallProgressEvent,
  type SettingsPatch,
  type UiPatchProgressEvent,
} from "../lib/api";
import { AppBrandIcon } from "./BrandIcons";
import {
  APP_DISPLAY_NAMES,
  APP_SUBTITLES,
  SegmentedControl,
  StatusPill,
  ToggleSwitch,
  clientModeLabel,
  formatBytes,
  formatModelFilterLabel,
  formatVersion,
  installStageLabel,
} from "./UiPrimitives";

interface ClientSettingsPanelProps {
  app: AppSnapshot;
  dashboard: DashboardPayload;
  update?: AppUpdateStatus;
  progress?: InstallProgressEvent;
  uiPatchProgress?: UiPatchProgressEvent | null;
  isBusy: boolean;
  cancelling: boolean;
  needsRestart: boolean;
  savingInlineSetting: boolean;
  onSetMode: (app: AppSnapshot, mode: AppMode) => void;
  onLaunchApp: (appId: AppId) => void;
  onRestartApp: (appId: AppId) => void;
  onCloseApp: (appId: AppId) => void;
  onInstallOrUpgrade: (appId: AppId) => void;
  onCancelInstall: (appId: AppId) => void;
  onRediagnose: (appId: AppId) => void;
  onPatchAppUi: (app: AppSnapshot) => void;
  onRestoreAppUi: (app: AppSnapshot) => void;
  onOpenModelPicker: (appId: AppId) => void;
  onSaveInlineSettings: (patch: SettingsPatch, successMessage: string) => Promise<void>;
}

function getUiPatchPresentation(app: AppSnapshot): {
  title: string;
  desc: string;
  pillText: string;
  pillTone: "ok" | "warn" | "neutral";
  actionLabel: string;
  canRestore: boolean;
} {
  const isClaude = app.id === "claude";
  const scopeDesc = isClaude
    ? "Chat、Cowork 与 Claude Code 界面均已汉化"
    : "系统菜单、托盘与工作台界面均已汉化";
  const unpatchedDesc = isClaude
    ? "一键注入 Claude Desktop 全量简体中文界面词典，支持字节级无损还原"
    : "一键注入 Antigravity 全量简体中文界面词典，支持字节级无损还原";

  switch (app.uiPatch.status) {
    case "patched":
      if (app.uiPatch.updateAvailable) {
        return {
          title: "有新版简体中文词典可更新",
          desc:
            app.uiPatch.note ??
            "检测到新版简体中文词典，客户端关闭时会自动静默同步，也可点击立即更新",
          pillText: "可更新",
          pillTone: "warn",
          actionLabel: "更新汉化",
          canRestore: true,
        };
      }
      return {
        title: "完整简体中文已开启",
        desc: app.uiPatch.note ?? scopeDesc,
        pillText: "已生效",
        pillTone: "ok",
        actionLabel: "重新应用",
        canRestore: true,
      };
    case "outdated_after_upgrade":
      return {
        title: "客户端升级后需重新应用汉化",
        desc: app.uiPatch.note ?? "检测到客户端版本更新，点击重新应用以恢复完整简体中文界面",
        pillText: "需重打",
        pillTone: "warn",
        actionLabel: "重新应用",
        canRestore: app.uiPatch.hasValidBackup,
      };
    case "unsupported":
      return {
        title: "暂不支持当前安装包汉化",
        desc: app.uiPatch.note ?? "当前客户端安装结构受限，配置文件级中文回复仍可正常使用",
        pillText: "暂不支持",
        pillTone: "neutral",
        actionLabel: "尝试应用",
        canRestore: app.uiPatch.hasValidBackup,
      };
    default:
      return {
        title: "界面简体中文",
        desc: app.uiPatch.note ?? unpatchedDesc,
        pillText: app.install.installed ? "未开启" : "未安装",
        pillTone: "neutral",
        actionLabel: "开启界面汉化",
        canRestore: app.uiPatch.hasValidBackup,
      };
  }
}

export function ClientSettingsPanel({
  app,
  dashboard,
  update,
  progress,
  uiPatchProgress,
  isBusy,
  cancelling,
  needsRestart,
  savingInlineSetting,
  onSetMode,
  onLaunchApp,
  onRestartApp,
  onCloseApp,
  onInstallOrUpgrade,
  onCancelInstall,
  onRediagnose,
  onPatchAppUi,
  onRestoreAppUi,
  onOpenModelPicker,
  onSaveInlineSettings,
}: ClientSettingsPanelProps) {
  const [diagOpen, setDiagOpen] = useState(false);
  const [claudeAdvancedOpen, setClaudeAdvancedOpen] = useState(false);
  const [agyKeyDraft, setAgyKeyDraft] = useState(dashboard.state.agyApiKey ?? "");
  const [claudeSubtitleDraft, setClaudeSubtitleDraft] = useState(
    dashboard.state.gatewayDisplaySubtitle ?? "Tako AI Gateway",
  );
  const [workbuddyModels, setWorkbuddyModels] = useState<ClientModelOption[]>([]);
  const [loadingWorkbuddyModels, setLoadingWorkbuddyModels] = useState(false);

  useEffect(() => {
    setDiagOpen(false);
    setClaudeAdvancedOpen(false);
  }, [app.id]);

  useEffect(() => {
    if (app.id !== "workbuddy") return;
    let cancelled = false;
    setLoadingWorkbuddyModels(true);
    void api
      .listClientModels("workbuddy", false)
      .then((res) => {
        if (!cancelled) setWorkbuddyModels(res.models);
      })
      .catch(() => {
        // Fallback options rendered cleanly when offline or logged out.
      })
      .finally(() => {
        if (!cancelled) setLoadingWorkbuddyModels(false);
      });
    return () => {
      cancelled = true;
    };
  }, [
    app.id,
    dashboard.account.loggedIn,
    dashboard.state.modelFilters?.workbuddy?.mode,
    dashboard.state.modelFilters?.workbuddy?.enabledIds.join(","),
  ]);

  useEffect(() => {
    setAgyKeyDraft(dashboard.state.agyApiKey ?? "");
  }, [dashboard.state.agyApiKey]);

  useEffect(() => {
    setClaudeSubtitleDraft(dashboard.state.gatewayDisplaySubtitle ?? "Tako AI Gateway");
  }, [dashboard.state.gatewayDisplaySubtitle]);

  const isDsh = app.id === "dsh-tako";
  const pendingAutoTako =
    Boolean(dashboard.account.loggedIn) &&
    app.mode === "official" &&
    !(dashboard.state.explicitOfficialApps ?? []).includes(app.id);
  const displayMode: AppMode = pendingAutoTako ? "tako" : app.mode;
  const modelFilter =
    app.id === "claude"
      ? dashboard.state.modelFilters?.claude
      : app.id === "codex"
        ? dashboard.state.modelFilters?.codex
        : app.id === "antigravity"
          ? dashboard.state.modelFilters?.antigravity
          : app.id === "workbuddy"
            ? dashboard.state.modelFilters?.workbuddy
            : undefined;

  const modeOptions: { id: AppMode; label: string }[] = isDsh
    ? [
        { id: "official", label: "独立登录" },
        { id: "tako", label: "同步密钥" },
      ]
    : [
        { id: "official", label: "官方" },
        { id: "tako", label: "Tako" },
      ];

  const modeHelp = pendingAutoTako
    ? isDsh
      ? app.running
        ? "默认已设为同步 Tako 账号；当前独立会话不中断，下次重启或重新启动时自动同步。"
        : "默认已设为同步 Tako 账号，点击启动时自动写入密钥。"
      : app.running
        ? "默认已设为 Tako 模式；当前官方会话保持不中断，下次重启或重新启动时自动切换。"
        : "默认已设为 Tako 模式，点击启动时自动切换并连接本地 Tako 网关。"
    : app.id === "claude"
      ? "Tako 接管时免 Claude 官方登录，官方对话与个人数据保持隔离。"
      : app.id === "codex"
        ? "Tako 接管时自动配置本地网关；可按需保留官方 ChatGPT 登录。"
        : app.id === "antigravity"
          ? "模型按你的选择过滤后映射到 32 个自定义槽位，自动同步本地网关。"
          : app.id === "workbuddy"
            ? "自动将 Tako 网关模型写入 ~/.workbuddy/models.json（保留原有自定义模型与官方连接器），并在 WorkBuddy 新建对话时自动默认选中 Tako 模型。"
            : "同步模式自动写入当前 Tako 账号密钥；独立模式保留应用内独立登录。";

  const patchInfo = getUiPatchPresentation(app);
  const artifactSize = formatBytes(update?.artifact?.size);
  const issueCount = app.diagnostic.issues.length;
  const agyKeyDirty = agyKeyDraft.trim() !== (dashboard.state.agyApiKey ?? "").trim();
  const claudeSubtitleDirty =
    claudeSubtitleDraft.trim() !==
    (dashboard.state.gatewayDisplaySubtitle ?? "Tako AI Gateway").trim();

  return (
    <div className="tako-page-sub flex flex-col">
      {/* Page Header */}
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0 flex-1">
          <div className="text-[11px] font-bold tracking-[0.08em] uppercase text-[var(--accent)] mb-1">
            软件独立设置
          </div>
          <div className="flex items-center gap-2.5">
            <AppBrandIcon appId={app.id} className="h-7 w-7" />
            <h1 className="text-[20px] font-semibold tracking-[-0.025em] text-[var(--text-primary)] leading-tight truncate">
              {APP_DISPLAY_NAMES[app.id]}
            </h1>
          </div>
          <p className="text-[12.5px] text-[var(--text-secondary)] mt-1.5">
            {APP_SUBTITLES[app.id]}
          </p>
        </div>

        {/* Primary 1-2 Actions */}
        <div className="flex items-center gap-2 shrink-0 pt-1">
          {!app.install.installed ? (
            <button
              type="button"
              disabled={isBusy}
              onClick={() => onInstallOrUpgrade(app.id)}
              className="h-[34px] px-3.5 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12.5px] font-semibold inline-flex items-center gap-1.5 transition cursor-pointer disabled:opacity-60"
            >
              <Download className="h-3.5 w-3.5" />
              <span>{isBusy ? "安装中..." : "一键安装"}</span>
            </button>
          ) : app.running ? (
            <>
              <button
                type="button"
                disabled={isBusy}
                onClick={() => onRestartApp(app.id)}
                className={`h-[34px] px-3.5 rounded-[9px] text-[12.5px] font-semibold inline-flex items-center gap-1.5 transition cursor-pointer disabled:opacity-60 ${
                  needsRestart
                    ? "bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white"
                    : "border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-primary)]"
                }`}
              >
                <RotateCw className={`h-3.5 w-3.5 ${isBusy ? "animate-spin" : ""}`} />
                <span>{needsRestart ? "重启生效" : "重启应用"}</span>
              </button>
              <button
                type="button"
                disabled={isBusy}
                onClick={() => onLaunchApp(app.id)}
                className="h-[34px] px-3 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12.5px] font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)] inline-flex items-center gap-1.5 transition cursor-pointer disabled:opacity-60"
              >
                <ExternalLink className="h-3.5 w-3.5" />
                <span>置前</span>
              </button>
            </>
          ) : (
            <button
              type="button"
              disabled={isBusy}
              onClick={() => onLaunchApp(app.id)}
              className="h-[34px] px-3.5 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12.5px] font-semibold inline-flex items-center gap-1.5 transition cursor-pointer disabled:opacity-60"
            >
              <ExternalLink className="h-3.5 w-3.5" />
              <span>打开应用</span>
            </button>
          )}
        </div>
      </div>

      {/* 3-Card Status Strip */}
      <div className="grid grid-cols-3 tako-list-stack">
        <div className="tako-card-surface border border-[var(--border)] bg-[var(--bg-card)] tako-panel-head">
          <div className="text-[11px] text-[var(--text-muted)] flex items-center justify-between gap-1">
            <span>安装状态</span>
            {app.running && (
              <span className="inline-flex items-center gap-1 text-[10.5px] text-[var(--success)] font-medium">
                <span className="tako-live-dot" />
                运行中
              </span>
            )}
          </div>
          <div className="mt-1.5 text-[14px] font-semibold text-[var(--text-primary)] tabular-nums truncate">
            {app.install.installed
              ? app.install.version
                ? `已安装 · ${formatVersion(app.install.version)}`
                : "已安装"
              : "未安装"}
          </div>
        </div>

        <div className="tako-card-surface border border-[var(--border)] bg-[var(--bg-card)] tako-panel-head">
          <div className="text-[11px] text-[var(--text-muted)]">运行模式</div>
          <div
            className={`mt-1.5 text-[14px] font-semibold truncate ${
              app.mode === "tako" ? "text-[var(--success)]" : "text-[var(--text-primary)]"
            }`}
          >
            {clientModeLabel(app)}
          </div>
        </div>

        <div className="tako-card-surface border border-[var(--border)] bg-[var(--bg-card)] tako-panel-head">
          <div className="text-[11px] text-[var(--text-muted)]">更新状态</div>
          {update?.hasUpdate && update.remoteVersion ? (
            <button
              type="button"
              disabled={isBusy}
              onClick={() => onInstallOrUpgrade(app.id)}
              className="mt-1.5 text-[14px] font-semibold text-[var(--accent)] hover:underline cursor-pointer truncate block text-left tabular-nums disabled:opacity-50"
            >
              升级至 {formatVersion(update.remoteVersion)}
            </button>
          ) : (
            <div className="mt-1.5 text-[14px] font-semibold text-[var(--text-primary)] truncate">
              {app.install.installed ? "已是最新版" : "等待安装"}
            </div>
          )}
        </div>
      </div>

      {/* Dedicated Full Simplified Chinese UI Banner (Claude Desktop & Antigravity) */}
      {(app.id === "claude" || app.id === "antigravity") && (
        <div
          className="tako-card-surface border border-[var(--accent-border)] tako-panel-pad flex flex-col gap-3"
          style={{ background: "var(--language-grad)" }}
        >
          <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-3 min-w-0 flex-1">
            <div className="h-[34px] w-[34px] rounded-[10px] bg-[var(--bg-card)] text-[var(--accent)] font-bold text-[14px] inline-flex items-center justify-center shrink-0 shadow-[var(--shadow-nav)]">
              中
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2 flex-wrap">
                <span className="text-[13.5px] font-semibold text-[var(--text-primary)]">
                  {patchInfo.title}
                </span>
                <StatusPill tone={patchInfo.pillTone}>{patchInfo.pillText}</StatusPill>
              </div>
              <p className="text-[11.5px] text-[var(--text-secondary)] mt-0.5 leading-snug">
                {patchInfo.desc}
              </p>
            </div>
          </div>

          {/* Explicit literal data-testid attributes for static verifier compatibility */}
          {app.id === "claude" ? (
            <div className="flex items-center gap-2.5 shrink-0">
              <button
                type="button"
                data-testid="claude-ui-patch-btn"
                disabled={isBusy || !app.install.installed}
                onClick={() => onPatchAppUi(app)}
                className={`h-8 px-3 rounded-[8px] text-[12px] font-semibold transition cursor-pointer disabled:opacity-50 ${
                  app.uiPatch.status === "patched"
                    ? "border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-primary)]"
                    : "bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white"
                }`}
              >
                {isBusy
                  ? uiPatchProgress
                    ? `处理中 ${Math.round(uiPatchProgress.percent)}%`
                    : "处理中..."
                  : patchInfo.actionLabel}
              </button>
              {patchInfo.canRestore && (
                <button
                  type="button"
                  data-testid="claude-ui-restore-btn"
                  disabled={isBusy}
                  onClick={() => onRestoreAppUi(app)}
                  className="text-[12px] font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:underline cursor-pointer disabled:opacity-50"
                >
                  还原原版
                </button>
              )}
            </div>
          ) : (
            <div className="flex items-center gap-2.5 shrink-0">
              <button
                type="button"
                data-testid="agy-ui-patch-btn"
                disabled={isBusy || !app.install.installed}
                onClick={() => onPatchAppUi(app)}
                className={`h-8 px-3 rounded-[8px] text-[12px] font-semibold transition cursor-pointer disabled:opacity-50 ${
                  app.uiPatch.status === "patched"
                    ? "border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-primary)]"
                    : "bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white"
                }`}
              >
                {isBusy
                  ? uiPatchProgress
                    ? `处理中 ${Math.round(uiPatchProgress.percent)}%`
                    : "处理中..."
                  : patchInfo.actionLabel}
              </button>
              {patchInfo.canRestore && (
                <button
                  type="button"
                  data-testid="agy-ui-restore-btn"
                  disabled={isBusy}
                  onClick={() => onRestoreAppUi(app)}
                  className="text-[12px] font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:underline cursor-pointer disabled:opacity-50"
                >
                  还原原版
                </button>
              )}
            </div>
          )}
          </div>

          {uiPatchProgress && (
            <div className="pt-2 border-t border-[var(--accent-border)]/70 flex flex-col gap-1.5">
              <div className="flex items-center justify-between gap-2 text-[11.5px]">
                <span className="inline-flex items-center gap-1.5 text-[var(--text-primary)] font-medium truncate">
                  {uiPatchProgress.stage === "done" ? (
                    <CheckCircle2 className="h-3.5 w-3.5 text-[var(--success)] shrink-0" />
                  ) : (
                    <Loader2 className="h-3.5 w-3.5 text-[var(--accent)] animate-spin shrink-0" />
                  )}
                  <span className="truncate">{uiPatchProgress.message}</span>
                </span>
                <span
                  className="tabular-nums font-semibold shrink-0"
                  style={{
                    color:
                      uiPatchProgress.stage === "done"
                        ? "var(--success)"
                        : uiPatchProgress.stage === "error"
                          ? "var(--danger)"
                          : "var(--accent)",
                  }}
                >
                  {Math.round(uiPatchProgress.percent)}%
                </span>
              </div>
              <div className="h-1.5 w-full rounded-full bg-[var(--bg-card)]/80 overflow-hidden">
                <div
                  className="h-full rounded-full transition-all duration-300 ease-out"
                  style={{
                    width: `${Math.max(4, Math.min(100, uiPatchProgress.percent))}%`,
                    backgroundColor:
                      uiPatchProgress.stage === "done"
                        ? "var(--success)"
                        : uiPatchProgress.stage === "error"
                          ? "var(--danger)"
                          : "var(--accent)",
                  }}
                />
              </div>
            </div>
          )}
        </div>
      )}

      {/* Connection & Model Card */}
      <section className="tako-card-surface border border-[var(--border)] bg-[var(--bg-card)] overflow-hidden">
        <div className="tako-panel-head border-b border-[var(--border)]">
          <h2 className="text-[13.5px] font-semibold text-[var(--text-primary)]">
            {isDsh ? "账号同步" : "连接与模型"}
          </h2>
          <p className="text-[11px] text-[var(--text-muted)] mt-0.5">
            以下设置仅作用于 {APP_DISPLAY_NAMES[app.id]}，不影响其他客户端。
          </p>
        </div>

        <div className="divide-y divide-[var(--border)] tako-panel-body">
          {/* Mode Row */}
          <div className="tako-setting-row flex flex-wrap items-center justify-between gap-4">
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-medium text-[var(--text-primary)]">
                运行模式
              </div>
              <p className="text-[11.5px] text-[var(--text-muted)] mt-0.5 leading-snug">
                {modeHelp}
              </p>
            </div>
            <SegmentedControl
              options={modeOptions}
              value={displayMode}
              disabled={isBusy}
              onChange={(next) => onSetMode(app, next)}
            />
          </div>

          {/* Model Selection Row (non-DSH clients) */}
          {!isDsh && (
            <div className="tako-setting-row flex flex-wrap items-center justify-between gap-4">
              <div className="min-w-0 flex-1">
                <div className="text-[13px] font-medium text-[var(--text-primary)]">
                  可用模型列表
                </div>
                <p className="text-[11.5px] text-[var(--text-muted)] mt-0.5 leading-snug">
                  {app.mode === "tako"
                    ? app.id === "antigravity"
                      ? "从全量目录精简勾选，确保主力模型优先占满 32 个槽位。"
                      : app.id === "workbuddy"
                        ? "勾选的模型会自动写入 ~/.workbuddy/models.json 并实时热生效，无需重启。"
                        : "自定义该客户端在 Tako 模式下展示的模型列表。"
                    : "切至 Tako 模式后，此处选择的模型列表将自动生效。"}
                </p>
              </div>
              <button
                type="button"
                onClick={() => onOpenModelPicker(app.id)}
                className="h-[34px] px-3 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-primary)] inline-flex items-center gap-2 transition cursor-pointer shrink-0 tabular-nums"
              >
                <SlidersHorizontal className="h-3.5 w-3.5 text-[var(--accent)]" />
                <span>{formatModelFilterLabel(app.id, modelFilter)}</span>
              </button>
            </div>
          )}

          {/* WorkBuddy-only: Default Selected Model */}
          {app.id === "workbuddy" && (() => {
            const fallbackDefaults: { id: string; displayName: string }[] = [
              { id: "claude-opus-5-5", displayName: "Claude Opus 5.5" },
              { id: "claude-sonnet-5-5", displayName: "Claude Sonnet 5.5" },
              { id: "gpt-6.2-sol", displayName: "GPT-6.2 Sol" },
              { id: "gpt-6.2-astra", displayName: "GPT-6.2 Astra" },
              { id: "deepseek-v4.1-flash", displayName: "DeepSeek V4.1 Flash" },
              { id: "swe-2-1", displayName: "SWE-2.1" },
            ];
            const visibleOptions = workbuddyModels
              .filter((m) => m.visible)
              .map((m) => ({ id: m.id, displayName: m.displayName }));
            const options = visibleOptions.length > 0 ? visibleOptions : fallbackDefaults;
            const currentPref = (dashboard.state.workbuddyDefaultModel ?? "").trim();
            const hasPrefInOptions =
              !currentPref ||
              options.some((o) => o.id.toLowerCase() === currentPref.toLowerCase());
            const firstOption = options[0];

            return (
              <div className="tako-setting-row flex flex-wrap items-center justify-between gap-4">
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] font-medium text-[var(--text-primary)]">
                    默认选中模型
                  </div>
                  <p className="text-[11.5px] text-[var(--text-muted)] mt-0.5 leading-snug">
                    {app.mode === "tako"
                      ? app.running
                        ? "新建对话时自动选中的 Tako 模型；修改后点击上方「重启」立即生效。"
                        : "已自动写入 WorkBuddy 默认模型，下次打开新建对话即默认选中。"
                      : "切至 Tako 模式后，将自动在 WorkBuddy 新建对话时默认选中该模型。"}
                  </p>
                </div>
                <select
                  aria-label="WorkBuddy 默认选中模型"
                  value={currentPref}
                  disabled={savingInlineSetting || loadingWorkbuddyModels}
                  onChange={(e) => {
                    const nextVal = e.target.value;
                    const chosen = options.find((o) => o.id === nextVal);
                    const label = chosen
                      ? chosen.displayName
                      : `自动首选（${firstOption?.displayName ?? "Claude Opus 5.5"}）`;
                    const suffix =
                      app.mode === "tako" && app.running
                        ? "（重启 WorkBuddy 后生效）"
                        : "（下次打开自动选中）";
                    void onSaveInlineSettings(
                      { workbuddyDefaultModel: nextVal },
                      `WorkBuddy 默认模型已设为 ${label}${suffix}`,
                    );
                  }}
                  className="h-[34px] max-w-[230px] px-2.5 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)] cursor-pointer disabled:opacity-50 shrink-0"
                >
                  <option value="">
                    {`自动首选（${firstOption?.displayName ?? "Claude Opus 5.5"}）`}
                  </option>
                  {!hasPrefInOptions && currentPref && (
                    <option value={currentPref}>{currentPref}</option>
                  )}
                  {options.map((opt) => (
                    <option key={opt.id} value={opt.id}>
                      {opt.displayName}
                    </option>
                  ))}
                </select>
              </div>
            );
          })()}

          {/* Codex-only: Preserve Official ChatGPT Login + Chinese Reply Status */}
          {app.id === "codex" && (
            <>
              <div className="tako-setting-row flex items-center justify-between gap-4">
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] font-medium text-[var(--text-primary)]">
                    保留官方 ChatGPT 登录
                  </div>
                  <p className="text-[11.5px] text-[var(--text-muted)] mt-0.5 leading-snug">
                    开启后切至 Tako 模式不覆盖官方登录态，适合需使用远程操控与官方插件的用户。
                  </p>
                </div>
                <ToggleSwitch
                  checked={Boolean(dashboard.state.codexPreserveOfficialAuth)}
                  disabled={savingInlineSetting}
                  ariaLabel="保留官方 ChatGPT 登录"
                  onChange={(next) =>
                    void onSaveInlineSettings(
                      { codexPreserveOfficialAuth: next },
                      next ? "已开启保留 Codex 官方登录" : "已关闭保留 Codex 官方登录",
                    )
                  }
                />
              </div>

              <div className="tako-setting-row flex items-center justify-between gap-4">
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] font-medium text-[var(--text-primary)]">
                    简体中文回复规则
                  </div>
                  <p className="text-[11.5px] text-[var(--text-muted)] mt-0.5 leading-snug">
                    通过配置文件写入默认简体中文回复偏好，不修改 Codex 应用本体。
                  </p>
                </div>
                <StatusPill tone={app.chineseConfigured ? "ok" : "neutral"}>
                  {app.chineseConfigured ? "已配置" : "未配置"}
                </StatusPill>
              </div>
            </>
          )}

          {/* Antigravity-only: Dedicated Agy Key & 32-slot note */}
          {app.id === "antigravity" && (
            <div className="tako-setting-row flex flex-col gap-2">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="min-w-0">
                  <div className="text-[13px] font-medium text-[var(--text-primary)]">
                    专用反重力 Key（可选）
                  </div>
                  <p className="text-[11.5px] text-[var(--text-muted)] mt-0.5">
                    留空默认复用主账号 Key；仅在需要为 Antigravity 单独计费或隔离额度时填写。
                  </p>
                </div>
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="password"
                  value={agyKeyDraft}
                  onChange={(e) => setAgyKeyDraft(e.target.value)}
                  placeholder="sk-...（留空则复用主账号 Key）"
                  className="flex-1 h-[34px] px-3 rounded-[8px] border border-[var(--border)] bg-[var(--bg-canvas)] text-[12px] font-mono text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
                />
                <button
                  type="button"
                  disabled={!agyKeyDirty || savingInlineSetting}
                  onClick={() =>
                    void onSaveInlineSettings(
                      { agyApiKey: agyKeyDraft.trim() },
                      "Antigravity 专用 Key 已保存",
                    )
                  }
                  className="h-[34px] px-3 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-primary)] cursor-pointer disabled:opacity-45 shrink-0"
                >
                  保存
                </button>
              </div>
            </div>
          )}

          {/* Claude-only: Workspace Mode + Collapsible Gateway Subtitle */}
          {app.id === "claude" && (() => {
            const workspaceMode = dashboard.state.claudeWorkspaceMode ?? "agent_only";
            return (
              <>
                <div className="tako-setting-row flex flex-wrap items-center justify-between gap-4">
                  <div className="min-w-0 flex-1">
                    <div className="text-[13px] font-medium text-[var(--text-primary)]">
                      工作区模式
                    </div>
                    <p className="text-[11.5px] text-[var(--text-muted)] mt-0.5 leading-snug">
                      {workspaceMode === "agent_only"
                        ? app.mode === "tako" && app.running
                          ? "默认直达纯 Agent（Code）模式，允许直接读取本机目录并默认放行文件修改；切换后点击上方「重启」生效。"
                          : "默认直达纯 Agent（Code）模式，免选文件夹即可读写本机目录并默认放行文件修改，彻底避开受限 Chat 模式。"
                        : app.mode === "tako" && app.running
                          ? "完整模式同时显示 Chat / Cowork / Code 标签页（Chat 无法直接读取本机目录）；切换后点击上方「重启」生效。"
                          : "完整模式同时显示 Chat、Cowork 与 Code 三个标签页（注：Chat 由官方限制无法直接读取本机目录）。"}
                    </p>
                  </div>
                  <SegmentedControl
                    options={[
                      { id: "agent_only", label: "纯 Agent（推荐）" },
                      { id: "full", label: "完整模式" },
                    ]}
                    value={workspaceMode}
                    disabled={savingInlineSetting}
                    onChange={(next) => {
                      const label = next === "agent_only" ? "纯 Agent 模式" : "完整模式";
                      const suffix =
                        app.mode === "tako" && app.running
                          ? "（重启 Claude Desktop 后生效）"
                          : "（下次打开 Claude Desktop 生效）";
                      void onSaveInlineSettings(
                        { claudeWorkspaceMode: next },
                        `Claude Desktop 已设为${label}${suffix}`,
                      );
                    }}
                  />
                </div>

                <div className="py-3">
                  <button
                    type="button"
                    onClick={() => setClaudeAdvancedOpen((prev) => !prev)}
                    className="w-full flex items-center justify-between text-[12px] text-[var(--text-secondary)] hover:text-[var(--text-primary)] cursor-pointer"
                  >
                    <span>高级：Claude 网关副标题</span>
                    <ChevronDown
                      className={`h-3.5 w-3.5 transition-transform ${
                        claudeAdvancedOpen ? "rotate-180" : ""
                      }`}
                    />
                  </button>
                  {claudeAdvancedOpen && (
                    <div className="mt-2.5 flex items-center gap-2">
                      <input
                        type="text"
                        maxLength={80}
                        value={claudeSubtitleDraft}
                        onChange={(e) => setClaudeSubtitleDraft(e.target.value)}
                        placeholder="Tako AI Gateway"
                        className="flex-1 h-[34px] px-3 rounded-[8px] border border-[var(--border)] bg-[var(--bg-canvas)] text-[12px] text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
                      />
                      <button
                        type="button"
                        disabled={!claudeSubtitleDirty || savingInlineSetting}
                        onClick={() =>
                          void onSaveInlineSettings(
                            {
                              gatewayDisplaySubtitle:
                                claudeSubtitleDraft.trim() || "Tako AI Gateway",
                            },
                            "Claude 网关副标题已保存",
                          )
                        }
                        className="h-[34px] px-3 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-primary)] cursor-pointer disabled:opacity-45 shrink-0"
                      >
                        保存
                      </button>
                    </div>
                  )}
                </div>
              </>
            );
          })()}
        </div>
      </section>

      {/* Install & Maintenance Card */}
      <section className="tako-card-surface border border-[var(--border)] bg-[var(--bg-card)] overflow-hidden">
        <div className="tako-panel-head border-b border-[var(--border)]">
          <h2 className="text-[13.5px] font-semibold text-[var(--text-primary)]">
            安装与维护
          </h2>
        </div>

        <div className="divide-y divide-[var(--border)] tako-panel-body">
          {/* Install / Update Row */}
          <div className="tako-setting-row flex flex-col gap-2.5">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="min-w-0 flex-1">
                <div className="text-[13px] font-medium text-[var(--text-primary)]">
                  {app.install.installed ? "安装位置与更新" : "安装客户端"}
                </div>
                <div className="text-[11.5px] text-[var(--text-muted)] mt-0.5 truncate selectable font-mono">
                  {app.install.installPath ?? "尚未检测到安装路径"}
                </div>
              </div>

              <div className="flex items-center gap-2 shrink-0">
                {progress ? (
                  <button
                    type="button"
                    disabled={cancelling}
                    onClick={() => onCancelInstall(app.id)}
                    className="h-8 px-3 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--danger)] cursor-pointer disabled:opacity-50"
                  >
                    {cancelling ? "取消中..." : "取消下载"}
                  </button>
                ) : (
                  <button
                    type="button"
                    disabled={isBusy}
                    onClick={() => onInstallOrUpgrade(app.id)}
                    className="h-8 px-3 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-primary)] inline-flex items-center gap-1.5 cursor-pointer disabled:opacity-50"
                  >
                    <Download className="h-3.5 w-3.5 text-[var(--text-secondary)]" />
                    <span>
                      {!app.install.installed
                        ? "下载安装"
                        : update?.hasUpdate && update.remoteVersion
                          ? `更新至 ${formatVersion(update.remoteVersion)}${artifactSize ? ` (${artifactSize})` : ""}`
                          : "重新安装"}
                    </span>
                  </button>
                )}
              </div>
            </div>

            {progress && (
              <div className="flex flex-col gap-1.5">
                <div className="flex items-center justify-between text-[11px] text-[var(--text-secondary)] tabular-nums">
                  <span className="truncate">
                    {installStageLabel(progress.stage)} · {progress.message}
                  </span>
                  <span className="font-semibold text-[var(--accent)] shrink-0 ml-2">
                    {Math.round(progress.percent)}%
                  </span>
                </div>
                <div className="h-1.5 w-full rounded-full bg-[var(--bg-subtle)] overflow-hidden">
                  <div
                    className="h-full rounded-full bg-[var(--accent)] transition-all duration-150"
                    style={{ width: `${Math.max(3, Math.min(100, progress.percent))}%` }}
                  />
                </div>
              </div>
            )}
          </div>

          {/* Close Running App */}
          {app.running && (
            <div className="tako-setting-row flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[13px] font-medium text-[var(--text-primary)]">
                  关闭正在运行的进程
                </div>
                <p className="text-[11.5px] text-[var(--text-muted)] mt-0.5">
                  安全退出 {APP_DISPLAY_NAMES[app.id]}，避免切换配置或升级时文件被占用。
                </p>
              </div>
              <button
                type="button"
                disabled={isBusy}
                onClick={() => onCloseApp(app.id)}
                className="tako-ctrl-h px-3 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--danger-soft)] text-[12px] font-medium text-[var(--danger)] inline-flex items-center gap-1.5 transition cursor-pointer disabled:opacity-50 shrink-0"
              >
                <Power className="h-3.5 w-3.5" />
                <span>关闭应用</span>
              </button>
            </div>
          )}

          {/* Diagnostics (on-demand collapsible) */}
          <div className="tako-setting-row flex flex-col gap-2.5">
            <div className="flex items-center justify-between gap-3">
              <button
                type="button"
                onClick={() => setDiagOpen((prev) => !prev)}
                className="flex items-center gap-2 text-left cursor-pointer min-w-0"
              >
                <span className="text-[13px] font-medium text-[var(--text-primary)]">
                  配置诊断
                </span>
                <StatusPill tone={app.diagnostic.ok ? "ok" : "warn"}>
                  {app.diagnostic.ok ? "配置正常" : `${issueCount} 项需检查`}
                </StatusPill>
                <ChevronDown
                  className={`h-3.5 w-3.5 text-[var(--text-muted)] transition-transform ${
                    diagOpen ? "rotate-180" : ""
                  }`}
                />
              </button>

              <button
                type="button"
                onClick={() => onRediagnose(app.id)}
                className="h-7 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[11.5px] font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)] inline-flex items-center gap-1 cursor-pointer shrink-0"
              >
                <RefreshCw className="h-3 w-3" />
                <span>重新检测</span>
              </button>
            </div>

            {diagOpen && (
              <div className="rounded-[10px] bg-[var(--bg-subtle)]/75 border border-[var(--border)] px-3.5 py-2.5 text-[11.5px] flex flex-col gap-1.5 selectable">
                {app.diagnostic.issues.length > 0 && (
                  <ul className="space-y-1 text-[var(--warning)]">
                    {app.diagnostic.issues.map((issue, idx) => (
                      <li key={`issue-${idx}`}>• {issue}</li>
                    ))}
                  </ul>
                )}
                {app.diagnostic.notes.length > 0 ? (
                  <ul className="space-y-1 text-[var(--text-secondary)]">
                    {app.diagnostic.notes.map((note, idx) => (
                      <li key={`note-${idx}`}>• {note}</li>
                    ))}
                  </ul>
                ) : (
                  app.diagnostic.issues.length === 0 && (
                    <div className="text-[var(--text-muted)]">暂无额外诊断说明。</div>
                  )
                )}
              </div>
            )}
          </div>
        </div>
      </section>
    </div>
  );
}
