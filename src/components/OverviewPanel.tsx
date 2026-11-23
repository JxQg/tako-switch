import {
  ArrowUpCircle,
  Brain,
  CheckCircle2,
  ChevronRight,
  Download,
  ExternalLink,
  FileText,
  Film,
  Globe,
  Globe2,
  Headphones,
  Image as ImageIcon,
  LayoutGrid,
  Loader2,
  Power,
  RotateCw,
  SlidersHorizontal,
  Sparkles,
} from "lucide-react";
import type {
  AppId,
  AppMode,
  AppSnapshot,
  AppUpdateStatus,
  DashboardPayload,
  InstallProgressEvent,
  UiPatchProgressEvent,
} from "../lib/api";
import { AppBrandIcon } from "./BrandIcons";
import type { McpToolKey } from "./McpInlineTheater";
import {
  APP_DISPLAY_NAMES,
  CLIENT_ORDER,
  formatVersion,
  installStageLabel,
  type NavRoute,
} from "./UiPrimitives";

interface SelfUpdateSummary {
  status: "idle" | "checking" | "latest" | "available" | "downloading" | "installing" | "error";
  version?: string;
  source?: string;
  percent?: number;
}

interface OverviewPanelProps {
  dashboard: DashboardPayload | null;
  loading: boolean;
  updatesMap: Partial<Record<AppId, AppUpdateStatus>>;
  installProgress: Partial<Record<AppId, InstallProgressEvent>>;
  needsRestart: Partial<Record<AppId, boolean>>;
  updatableApps: AppSnapshot[];
  batchUpdatingApps: boolean;
  busyApp: AppId | null;
  cancellingApp: AppId | null;
  togglingChineseUi: boolean;
  uiPatchProgress: UiPatchProgressEvent | null;
  savingMcp: boolean;
  mcpEnabledCount: number;
  mcpTotalCount: number;
  selfUpdateSummary: SelfUpdateSummary;
  onNavigate: (route: NavRoute) => void;
  onOpenMcpDetail: (tab?: McpToolKey) => void;
  onSetMode: (app: AppSnapshot, mode: AppMode) => void;
  onLaunchApp: (appId: AppId) => void;
  onRestartApp: (appId: AppId) => void;
  onCloseApp: (appId: AppId) => void;
  onInstallOrUpgrade: (appId: AppId) => void;
  onCancelInstall: (appId: AppId) => void;
  onToggleMainChineseUi: () => void;
  onToggleCoreMcp: (key: McpToolKey) => void;
  onToggleWebPageMcp: (next: boolean) => void;
  onOpenModelSquare: () => void;
  onBatchUpgradeApps: () => void;
  onInstallSelfUpdate: () => void;
  onOpenChangelog: () => void;
}

const MODE_OPTIONS: { id: AppMode; label: string }[] = [
  { id: "official", label: "官方" },
  { id: "tako", label: "Tako" },
];

type HomeMcpItem =
  | {
      kind: "core";
      key: McpToolKey;
      label: string;
      subtitle: string;
      Icon: typeof ImageIcon;
    }
  | {
      kind: "webpage";
      key: "webPage";
      label: string;
      subtitle: string;
      Icon: typeof Globe2;
    };

const HOME_MCP_ITEMS: HomeMcpItem[] = [
  {
    kind: "core",
    key: "image",
    label: "图像生成",
    subtitle: "文生图 / 改图 / 多图融合",
    Icon: ImageIcon,
  },
  {
    kind: "core",
    key: "video",
    label: "视频生成",
    subtitle: "首尾帧控制 · 动态成片",
    Icon: Film,
  },
  {
    kind: "core",
    key: "audio",
    label: "声音生成",
    subtitle: "多角色多情绪自然语音",
    Icon: Headphones,
  },
  {
    kind: "core",
    key: "search",
    label: "联网搜索",
    subtitle: "实时信源检索与溯源",
    Icon: Globe,
  },
  {
    kind: "core",
    key: "consult",
    label: "多模型会诊",
    subtitle: "第二外脑交叉审阅",
    Icon: Brain,
  },
  {
    kind: "webpage",
    key: "webPage",
    label: "网站上线",
    subtitle: "腾讯云 EdgeOne 一键发布",
    Icon: Globe2,
  },
];

function briefClientSummary(app: AppSnapshot, update?: AppUpdateStatus): string {
  if (!app.install.installed) return "本机尚未安装 · 点击右侧安装或进设置查看引导";
  const parts: string[] = [];
  if (app.id === "dsh-tako") {
    parts.push(app.mode === "tako" ? "已同步当前账号密钥" : "使用独立账号登录");
  } else {
    parts.push(app.mode === "tako" ? "免官方登录直连 Tako" : "官方原版直连");
  }
  if (app.id === "claude" || app.id === "antigravity") {
    if (app.uiPatch.status === "patched") {
      parts.push("界面简体中文已开启");
    } else if (app.uiPatch.status === "outdated_after_upgrade") {
      parts.push("升级后需重新汉化");
    }
  } else if (app.id === "codex" && app.chineseConfigured) {
    parts.push("中文回复已配置");
  }
  if (update?.hasUpdate && update.remoteVersion) {
    parts.push(`可升级 ${formatVersion(update.remoteVersion)}`);
  }
  return parts.join(" · ");
}

export function OverviewPanel({
  dashboard,
  loading,
  updatesMap,
  installProgress,
  needsRestart,
  updatableApps,
  batchUpdatingApps,
  busyApp,
  cancellingApp,
  togglingChineseUi,
  uiPatchProgress,
  savingMcp,
  mcpEnabledCount,
  mcpTotalCount,
  selfUpdateSummary,
  onNavigate,
  onOpenMcpDetail,
  onSetMode,
  onLaunchApp,
  onRestartApp,
  onCloseApp,
  onInstallOrUpgrade,
  onCancelInstall,
  onToggleMainChineseUi,
  onToggleCoreMcp,
  onToggleWebPageMcp,
  onOpenModelSquare,
  onBatchUpgradeApps,
  onInstallSelfUpdate,
  onOpenChangelog,
}: OverviewPanelProps) {
  const appsById = new Map((dashboard?.apps ?? []).map((app) => [app.id, app]));
  const orderedApps = CLIENT_ORDER.map((id) => appsById.get(id)).filter(
    (app): app is AppSnapshot => Boolean(app),
  );

  const preferZh = dashboard?.state.preferChineseUi ?? true;
  const anyMissingUiPatch = orderedApps.some(
    (a) =>
      (a.id === "antigravity" || a.id === "claude") &&
      a.install.installed &&
      (a.uiPatch.status === "unpatched" || a.uiPatch.status === "outdated_after_upgrade"),
  );
  const anyMissingZhConfig = orderedApps.some(
    (a) => a.id !== "dsh-tako" && a.install.installed && !a.chineseConfigured,
  );
  const chineseFullyActive = preferZh && !anyMissingUiPatch && !anyMissingZhConfig;

  const mcp = dashboard?.state.mcp ?? null;

  return (
    <div className="w-full px-4 py-3.5 flex flex-col gap-3.5">
      {/* Compact Self-Update Banner */}
      {(selfUpdateSummary.status === "available" ||
        selfUpdateSummary.status === "downloading" ||
        selfUpdateSummary.status === "installing") && (
        <div className="rounded-[12px] border border-[var(--accent-border)] bg-[var(--accent-soft)]/80 px-3.5 py-2.5 flex flex-wrap items-center justify-between gap-2.5">
          <div className="flex items-center gap-2 min-w-0">
            <ArrowUpCircle className="h-4 w-4 text-[var(--accent)] shrink-0" />
            <div className="text-[12.5px] font-medium text-[var(--text-primary)] truncate">
              {selfUpdateSummary.status === "downloading"
                ? `正在下载 Tako Switch v${selfUpdateSummary.version ?? ""}（${Math.round(selfUpdateSummary.percent ?? 0)}%）`
                : selfUpdateSummary.status === "installing"
                  ? "正在安装新版本，完成后将自动重启..."
                  : `Tako Switch v${selfUpdateSummary.version ?? ""} 可更新`}
            </div>
          </div>
          {selfUpdateSummary.status === "available" && (
            <div className="flex items-center gap-1.5 shrink-0">
              <button
                type="button"
                onClick={onOpenChangelog}
                className="h-7 px-2.5 rounded-[7px] border border-[var(--accent-border)] bg-[var(--bg-card)]/90 hover:bg-[var(--bg-card)] text-[11.5px] font-medium text-[var(--text-primary)] inline-flex items-center gap-1 cursor-pointer"
              >
                <FileText className="h-3 w-3" />
                <span>更新日志</span>
              </button>
              <button
                type="button"
                onClick={onInstallSelfUpdate}
                className="h-7 px-2.5 rounded-[7px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[11.5px] font-medium cursor-pointer"
              >
                立即更新
              </button>
            </div>
          )}
        </div>
      )}

      {/* Section 1: Desktop Clients (Classic Quick-Action Rows + Secondary Page Entry) */}
      <section className="flex flex-col gap-2 min-w-0">
        <div className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1.5 px-0.5 min-w-0">
          <div className="flex items-center flex-wrap gap-1.5 min-w-0">
            <h2 className="text-[12.5px] font-semibold text-[var(--text-secondary)] shrink-0">
              桌面客户端
            </h2>

            <button
              type="button"
              data-testid="main-chinese-ui-toggle"
              disabled={!dashboard || togglingChineseUi || busyApp !== null}
              onClick={onToggleMainChineseUi}
              title={
                chineseFullyActive
                  ? "三端简体中文配置与界面汉化已生效 · 点击可一键关闭并恢复"
                  : "一键同步 Codex / Claude / Antigravity 简体中文配置与界面汉化"
              }
              className={`h-6 px-2.5 rounded-full border text-[11px] font-medium inline-flex items-center gap-1 transition cursor-pointer disabled:opacity-60 shrink-0 ${
                chineseFullyActive
                  ? "bg-[var(--success-soft)] border-[var(--success-border)] text-[var(--success)] hover:opacity-90"
                  : "bg-[var(--accent-soft)] border-[var(--accent-border)] text-[var(--accent)] hover:bg-[var(--accent-border)]"
              }`}
            >
              <Globe className="h-3 w-3 shrink-0" />
              <span>
                {togglingChineseUi
                  ? "汉化同步中..."
                  : chineseFullyActive
                    ? "三端汉化：已开启"
                    : "一键三端汉化"}
              </span>
            </button>

            <button
              type="button"
              data-testid="open-model-square-btn"
              onClick={onOpenModelSquare}
              title="免登录浏览全量模型单价、折扣倍率与吞吐速度"
              className="h-6 px-2.5 rounded-full border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] hover:border-[var(--border-strong)] text-[11px] font-medium text-[var(--text-primary)] inline-flex items-center gap-1 transition cursor-pointer shrink-0"
            >
              <LayoutGrid className="h-3 w-3 text-[var(--accent)] shrink-0" />
              <span>模型广场</span>
            </button>
          </div>

          <div className="flex items-center gap-2 shrink-0">
            {updatableApps.length >= 2 ? (
              <button
                type="button"
                disabled={batchUpdatingApps || busyApp !== null}
                onClick={onBatchUpgradeApps}
                className="h-6 px-2.5 rounded-full bg-[var(--accent-soft)] hover:bg-[var(--accent-border)] text-[var(--accent)] border border-[var(--accent-border)] text-[11px] font-medium inline-flex items-center gap-1 transition cursor-pointer disabled:opacity-60 shrink-0"
              >
                <ArrowUpCircle className="h-3 w-3" />
                <span>
                  {batchUpdatingApps
                    ? "批量更新中..."
                    : `全部更新 (${updatableApps.length})`}
                </span>
              </button>
            ) : (
              <span className="hidden sm:inline text-[11.5px] text-[var(--text-muted)]">
                点右侧「设置」进入各软件独立配置
              </span>
            )}
          </div>
        </div>

        {/* Live Chinese Localization Progress Card */}
        {uiPatchProgress && (
          <div
            data-testid="ui-patch-progress-card"
            className="rounded-[12px] border px-3.5 py-2.5 flex flex-col gap-1.5 transition-all"
            style={{
              backgroundColor:
                uiPatchProgress.stage === "done"
                  ? "var(--success-soft)"
                  : uiPatchProgress.stage === "error"
                    ? "var(--danger-soft)"
                    : "var(--accent-soft)",
              borderColor:
                uiPatchProgress.stage === "done"
                  ? "var(--success-border)"
                  : uiPatchProgress.stage === "error"
                    ? "rgba(201, 76, 76, 0.28)"
                    : "var(--accent-border)",
            }}
          >
            <div className="flex items-center justify-between gap-2 text-[12px]">
              <div className="flex items-center gap-2 min-w-0 font-medium text-[var(--text-primary)]">
                {uiPatchProgress.stage === "done" ? (
                  <CheckCircle2 className="h-3.5 w-3.5 text-[var(--success)] shrink-0" />
                ) : (
                  <Loader2 className="h-3.5 w-3.5 text-[var(--accent)] animate-spin shrink-0" />
                )}
                <span className="truncate">{uiPatchProgress.message}</span>
              </div>
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
            <div className="h-1.5 w-full rounded-full bg-[var(--bg-card)]/75 overflow-hidden">
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

        {loading ? (
          <div className="rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] p-6 text-center text-[13px] text-[var(--text-secondary)]">
            正在读取本机客户端与配置...
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            {orderedApps.map((app) => {
              const update = updatesMap[app.id];
              const progress = installProgress[app.id];
              const isBusy = busyApp === app.id;
              const restartRequired = Boolean(needsRestart[app.id]);
              const hasIssue = app.diagnostic.issues.length > 0;

              return (
                <div
                  key={app.id}
                  className={`rounded-[13px] border bg-[var(--bg-card)] shadow-[var(--shadow-card)] transition-colors overflow-hidden ${
                    app.running
                      ? "border-[var(--success-border)]"
                      : "border-[var(--border)] hover:border-[var(--border-strong)]"
                  }`}
                >
                  <div className="px-3.5 py-2.5 flex flex-wrap sm:flex-nowrap items-center gap-x-3 gap-y-2">
                    {/* Brand & Summary (click enters client subpage) */}
                    <button
                      type="button"
                      onClick={() => onNavigate(app.id)}
                      className="w-full sm:w-auto sm:flex-1 min-w-0 flex items-center gap-3 text-left group cursor-pointer"
                    >
                      <AppBrandIcon appId={app.id} className="h-8 w-8" />
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-1.5 min-w-0">
                          <span className="text-[13.5px] font-semibold tracking-[-0.01em] text-[var(--text-primary)] group-hover:text-[var(--accent)] transition truncate">
                            {APP_DISPLAY_NAMES[app.id]}
                          </span>
                          {app.install.installed && app.install.version && (
                            <span className="text-[11px] text-[var(--text-muted)] tabular-nums shrink-0">
                              {formatVersion(app.install.version)}
                            </span>
                          )}
                          {app.running && (
                            <span
                              className="inline-flex items-center gap-1 text-[11px] text-[var(--success)] shrink-0"
                              title="正在运行"
                            >
                              <span className="tako-live-dot" />
                              <span>运行中</span>
                            </span>
                          )}
                          {update?.hasUpdate && (
                            <span className="text-[10px] font-semibold px-1.5 py-0.5 rounded bg-[var(--accent-soft)] text-[var(--accent)] shrink-0">
                              有更新
                            </span>
                          )}
                          {hasIssue && (
                            <span className="text-[10px] font-semibold px-1.5 py-0.5 rounded bg-[var(--warning-soft)] text-[var(--warning)] shrink-0">
                              需检查
                            </span>
                          )}
                        </div>
                        <div className="text-[11.5px] text-[var(--text-muted)] truncate mt-0.5">
                          {briefClientSummary(app, update)}
                        </div>
                      </div>
                    </button>

                    {/* Fixed-Width Mode Switcher */}
                    <div className="w-[116px] shrink-0 grid grid-cols-2 p-0.5 rounded-[9px] bg-[var(--bg-pill)] border border-[var(--border)]">
                      {MODE_OPTIONS.map((option) => {
                        const active = app.mode === option.id;
                        const label =
                          app.id === "dsh-tako"
                            ? option.id === "tako"
                              ? "同步"
                              : "独立"
                            : option.label;
                        return (
                          <button
                            key={option.id}
                            type="button"
                            disabled={isBusy}
                            onClick={() => onSetMode(app, option.id)}
                            className={`h-7 rounded-[7px] text-[12px] transition cursor-pointer disabled:opacity-60 ${
                              active
                                ? "bg-[var(--bg-card)] font-semibold shadow-[var(--shadow-segment)]"
                                : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                            }`}
                            style={
                              active
                                ? {
                                    color:
                                      option.id === "tako"
                                        ? "var(--accent)"
                                        : "var(--text-primary)",
                                  }
                                : undefined
                            }
                          >
                            {label}
                          </button>
                        );
                      })}
                    </div>

                    {/* Fixed-Width Primary Action Slot */}
                    <div className="w-[96px] shrink-0 flex items-center justify-end">
                      {progress ? (
                        progress.stage === "downloading" ? (
                          <div className="w-full h-8 px-2 rounded-[9px] bg-[var(--accent-soft)] border border-[var(--accent-border)] flex items-center justify-between gap-1 text-[11px] font-medium text-[var(--accent)] tabular-nums">
                            <span className="truncate">{Math.round(progress.percent)}%</span>
                            <button
                              type="button"
                              disabled={cancellingApp === app.id}
                              onClick={() => onCancelInstall(app.id)}
                              className="px-1 py-0.5 rounded bg-[var(--bg-card)]/80 hover:bg-[var(--bg-card)] text-[var(--text-secondary)] hover:text-[var(--danger)] text-[10.5px] shrink-0 cursor-pointer disabled:opacity-50"
                            >
                              {cancellingApp === app.id ? "..." : "取消"}
                            </button>
                          </div>
                        ) : (
                          <div className="w-full h-8 px-2 rounded-[9px] bg-[var(--accent-soft)] border border-[var(--accent-border)] flex items-center justify-center text-[11px] font-medium text-[var(--accent)] tabular-nums truncate">
                            {installStageLabel(progress.stage)} {Math.round(progress.percent)}%
                          </div>
                        )
                      ) : !app.install.installed ? (
                        update && !update.platformSupported ? (
                          <span className="w-full h-8 rounded-[9px] bg-[var(--bg-subtle)] text-[var(--text-muted)] text-[12px] font-medium flex items-center justify-center">
                            暂不支持
                          </span>
                        ) : (
                          <button
                            type="button"
                            data-testid={`overview-action-${app.id}`}
                            disabled={isBusy}
                            onClick={() => onInstallOrUpgrade(app.id)}
                            className="w-full h-8 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium flex items-center justify-center gap-1.5 transition cursor-pointer disabled:opacity-60"
                          >
                            <Download className="h-3.5 w-3.5" />
                            <span>{isBusy ? "安装中" : "安装"}</span>
                          </button>
                        )
                      ) : restartRequired ? (
                        <button
                          type="button"
                          data-testid={`overview-action-${app.id}`}
                          disabled={isBusy}
                          onClick={() => onRestartApp(app.id)}
                          title="重启应用使最新配置生效"
                          className="w-full h-8 rounded-[9px] text-[12px] font-medium flex items-center justify-center gap-1.5 transition cursor-pointer disabled:opacity-60"
                          style={{
                            backgroundColor: "var(--warning-soft)",
                            color: "var(--warning)",
                          }}
                        >
                          <RotateCw className={`h-3.5 w-3.5 ${isBusy ? "animate-spin" : ""}`} />
                          <span>重启生效</span>
                        </button>
                      ) : app.running ? (
                        <button
                          type="button"
                          data-testid={`overview-action-${app.id}`}
                          disabled={isBusy}
                          onClick={() => onCloseApp(app.id)}
                          title={`关闭 ${APP_DISPLAY_NAMES[app.id]}`}
                          className="w-full h-8 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-primary)] text-[12px] font-medium flex items-center justify-center gap-1.5 transition cursor-pointer disabled:opacity-60"
                        >
                          <Power className="h-3.5 w-3.5 text-[var(--text-secondary)]" />
                          <span>{isBusy ? "关闭中" : "关闭"}</span>
                        </button>
                      ) : (
                        <button
                          type="button"
                          data-testid={`overview-action-${app.id}`}
                          disabled={isBusy}
                          onClick={() => onLaunchApp(app.id)}
                          title={`一键打开 ${APP_DISPLAY_NAMES[app.id]}`}
                          className="w-full h-8 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium flex items-center justify-center gap-1.5 transition cursor-pointer disabled:opacity-60"
                        >
                          <ExternalLink className="h-3.5 w-3.5" />
                          <span>{isBusy ? "打开中" : "打开"}</span>
                        </button>
                      )}
                    </div>

                    {/* Secondary Page Entry: Per-Client Settings */}
                    <button
                      type="button"
                      data-testid={`open-client-settings-${app.id}`}
                      onClick={() => onNavigate(app.id)}
                      title={`进入 ${APP_DISPLAY_NAMES[app.id]} 独立设置（模型、汉化、升级与诊断）`}
                      className="h-8 px-2.5 rounded-[9px] border border-[var(--border)] bg-[var(--bg-subtle)]/55 hover:bg-[var(--bg-subtle)] hover:border-[var(--border-strong)] text-[12px] font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)] inline-flex items-center gap-1 transition cursor-pointer shrink-0"
                    >
                      <SlidersHorizontal className="h-3.5 w-3.5" />
                      <span>设置</span>
                    </button>
                  </div>

                  {/* Inline Download Progress Strip */}
                  {progress && (
                    <div className="px-3.5 pb-2.5 flex flex-col gap-1">
                      <div className="h-1 w-full rounded-full bg-[var(--bg-subtle)] overflow-hidden">
                        <div
                          className="h-full rounded-full transition-all duration-200"
                          style={{
                            width: `${Math.max(4, Math.min(100, progress.percent))}%`,
                            backgroundColor:
                              progress.stage === "error"
                                ? "var(--danger)"
                                : "var(--accent)",
                          }}
                        />
                      </div>
                      <span className="tabular-nums text-[11px] text-[var(--text-secondary)] truncate">
                        {progress.message}
                      </span>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </section>

      {/* Section 2: Tako Super MCP Compact Card + Secondary Page Entry */}
      <section className="rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] shadow-[var(--shadow-card)] p-3.5 min-w-0">
        <div className="flex flex-wrap items-center justify-between gap-2 mb-2.5">
          <div className="flex items-center gap-2 min-w-0">
            <Sparkles className="h-4 w-4 text-[var(--accent)] shrink-0" />
            <span className="text-[13px] font-semibold text-[var(--text-primary)] shrink-0">
              Tako 超级 MCP
            </span>
            <span className="hidden sm:inline text-[11.5px] text-[var(--text-muted)] truncate">
              Tako 模式下自动注入，开关即时生效
            </span>
          </div>

          <div className="flex items-center gap-2 shrink-0">
            <span className="tabular-nums text-[11.5px] font-medium text-[var(--text-secondary)]">
              {mcpEnabledCount}/{mcpTotalCount} 已开启
            </span>
            <button
              type="button"
              data-testid="open-mcp-center"
              onClick={() => onOpenMcpDetail()}
              className="h-7 px-2.5 rounded-full border border-[var(--accent-border)] bg-[var(--accent-soft)] hover:bg-[var(--accent-border)] text-[var(--accent)] text-[11.5px] font-medium inline-flex items-center gap-1 transition cursor-pointer"
            >
              <span>配置与演示</span>
              <ChevronRight className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>

        <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
          {HOME_MCP_ITEMS.map((item) => {
            const Icon = item.Icon;
            const enabled =
              item.kind === "webpage"
                ? Boolean(mcp?.webPage)
                : Boolean(mcp?.[item.key]);
            const interactive = Boolean(mcp) && !savingMcp;

            return (
              <div
                key={item.key}
                role="button"
                tabIndex={0}
                onClick={() =>
                  onOpenMcpDetail(item.kind === "core" ? item.key : undefined)
                }
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onOpenMcpDetail(item.kind === "core" ? item.key : undefined);
                  }
                }}
                className={`rounded-[10px] border px-2.5 py-2 flex flex-col justify-between gap-1.5 transition select-none min-w-0 cursor-pointer ${
                  enabled
                    ? "bg-[var(--accent-soft)]/50 border-[var(--accent-border)] text-[var(--accent)] hover:bg-[var(--accent-soft)]"
                    : "bg-[var(--bg-subtle)]/55 border-[var(--border)] text-[var(--text-secondary)] hover:bg-[var(--bg-subtle)] hover:text-[var(--text-primary)]"
                }`}
              >
                <div className="flex items-center justify-between gap-1.5 min-w-0">
                  <span className="flex items-center gap-1.5 min-w-0 text-[12px] font-semibold">
                    <Icon className="h-3.5 w-3.5 shrink-0" />
                    <span className="truncate">{item.label}</span>
                  </span>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={enabled}
                    aria-label={`切换${item.label}开关`}
                    data-testid={
                      item.kind === "webpage"
                        ? "toggle-mcp-webpage"
                        : `toggle-mcp-${item.key}`
                    }
                    disabled={!interactive}
                    onClick={(e) => {
                      e.stopPropagation();
                      if (!interactive) return;
                      if (item.kind === "webpage") {
                        onToggleWebPageMcp(!enabled);
                      } else {
                        onToggleCoreMcp(item.key);
                      }
                    }}
                    title={enabled ? "已开启 · 点击关闭" : "已关闭 · 点击开启"}
                    className={`h-[18px] px-1.5 rounded-full border inline-flex items-center shrink-0 transition cursor-pointer disabled:opacity-50 ${
                      enabled
                        ? "bg-[var(--success-soft)] border-[var(--success-border)]"
                        : "bg-[var(--bg-card)]/90 border-[var(--border-strong)]"
                    }`}
                  >
                    <span
                      className={`h-2 w-2 rounded-full transition-colors ${
                        enabled
                          ? "bg-[var(--success)] shadow-[0_0_6px_var(--success-ring)]"
                          : "bg-[var(--text-muted)]/60"
                      }`}
                    />
                  </button>
                </div>

                <div className="flex items-center justify-between gap-1 min-w-0">
                  <span className="text-[10.5px] leading-tight text-[var(--text-muted)] truncate">
                    {item.subtitle}
                  </span>
                  <span className="text-[10px] font-medium text-[var(--text-muted)] shrink-0">
                    {item.kind === "webpage" ? "授权" : "演示"} ›
                  </span>
                </div>
              </div>
            );
          })}
        </div>
      </section>
    </div>
  );
}
