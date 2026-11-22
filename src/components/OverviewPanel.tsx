import {
  ArrowUpCircle,
  ChevronRight,
  Download,
  FileText,
  LayoutGrid,
} from "lucide-react";
import type {
  AppId,
  AppSnapshot,
  AppUpdateStatus,
  DashboardPayload,
} from "../lib/api";
import { AppBrandIcon } from "./BrandIcons";
import {
  APP_DISPLAY_NAMES,
  CLIENT_ORDER,
  StatusPill,
  ToggleSwitch,
  clientModeLabel,
  formatVersion,
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
  updatableApps: AppSnapshot[];
  batchUpdatingApps: boolean;
  busyApp: AppId | null;
  togglingChineseUi: boolean;
  mcpEnabledCount: number;
  mcpTotalCount: number;
  selfUpdateSummary: SelfUpdateSummary;
  onNavigate: (route: NavRoute) => void;
  onToggleMainChineseUi: () => void;
  onOpenModelSquare: () => void;
  onBatchUpgradeApps: () => void;
  onInstallSelfUpdate: () => void;
  onOpenChangelog: () => void;
}

function briefClientSummary(app: AppSnapshot, update?: AppUpdateStatus): string {
  if (!app.install.installed) return "本机尚未安装 · 点击进入安装引导";
  const parts: string[] = [];
  if (app.id === "dsh-tako") {
    parts.push(app.mode === "tako" ? "已同步当前账号密钥" : "使用独立账号登录");
  } else {
    parts.push(app.mode === "tako" ? "免官方登录直连 Tako 网关" : "官方原版直连");
  }
  if (app.id === "claude" || app.id === "antigravity") {
    if (app.uiPatch.status === "patched") {
      parts.push("完整简体中文已开启");
    } else if (app.uiPatch.status === "outdated_after_upgrade") {
      parts.push("升级后需重新应用汉化");
    }
  } else if (app.id === "codex" && app.chineseConfigured) {
    parts.push("中文回复已配置");
  }
  if (update?.hasUpdate && update.remoteVersion) {
    parts.push(`可升级至 ${formatVersion(update.remoteVersion)}`);
  }
  return parts.join(" · ");
}

export function OverviewPanel({
  dashboard,
  loading,
  updatesMap,
  updatableApps,
  batchUpdatingApps,
  busyApp,
  togglingChineseUi,
  mcpEnabledCount,
  mcpTotalCount,
  selfUpdateSummary,
  onNavigate,
  onToggleMainChineseUi,
  onOpenModelSquare,
  onBatchUpgradeApps,
  onInstallSelfUpdate,
  onOpenChangelog,
}: OverviewPanelProps) {
  const appsById = new Map((dashboard?.apps ?? []).map((app) => [app.id, app]));
  const orderedApps = CLIENT_ORDER.map((id) => appsById.get(id)).filter(
    (app): app is AppSnapshot => Boolean(app),
  );

  const takenOverCount = orderedApps.filter(
    (app) => app.install.installed && app.mode === "tako",
  ).length;
  const installedCount = orderedApps.filter((app) => app.install.installed).length;

  const preferZh = dashboard?.state.preferChineseUi ?? true;
  const anyMissingUiPatch = orderedApps.some(
    (a) =>
      (a.id === "antigravity" || a.id === "claude") &&
      a.install.installed &&
      (a.uiPatch.status === "unpatched" || a.uiPatch.status === "outdated_after_upgrade"),
  );
  const anyMissingConf = orderedApps.some(
    (a) => a.id !== "dsh-tako" && a.install.installed && !a.chineseConfigured,
  );
  const chineseFullyActive = preferZh && !anyMissingUiPatch && !anyMissingConf;

  const showSelfUpdateBar =
    selfUpdateSummary.status === "available" ||
    selfUpdateSummary.status === "downloading" ||
    selfUpdateSummary.status === "installing";

  return (
    <div className="max-w-[760px] mx-auto px-6 py-6 flex flex-col gap-5">
      {/* Page Head */}
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="text-[10.5px] font-bold tracking-[0.08em] uppercase text-[var(--accent)] mb-1">
            全局概览
          </div>
          <h1 className="text-[22px] font-semibold tracking-[-0.025em] text-[var(--text-primary)] leading-tight">
            Tako Switch 工作台
          </h1>
          <p className="text-[12.5px] text-[var(--text-secondary)] mt-1">
            查看全局运行状态；点击下方任一软件进入独立设置面板。
          </p>
        </div>

        <button
          type="button"
          data-testid="open-model-square-btn"
          onClick={onOpenModelSquare}
          title="免登录浏览全量模型人民币实付单价、官方折扣倍率与吞吐速度"
          className="h-[34px] px-3.5 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12.5px] font-medium text-[var(--text-primary)] inline-flex items-center gap-1.5 transition cursor-pointer shrink-0"
        >
          <LayoutGrid className="h-3.5 w-3.5 text-[var(--accent)]" />
          <span>模型广场</span>
        </button>
      </div>

      {/* Self-Update Banner (only when available or in progress) */}
      {showSelfUpdateBar && selfUpdateSummary.version && (
        <div className="rounded-[12px] border border-[var(--accent-border)] bg-[var(--accent-soft)]/70 px-4 py-3 flex flex-col gap-2">
          <div className="flex items-center justify-between gap-3">
            <div className="flex items-center gap-2 min-w-0 text-[12.5px]">
              <Download className="h-4 w-4 text-[var(--accent)] shrink-0" />
              <span className="font-semibold text-[var(--text-primary)] truncate">
                Tako Switch v{selfUpdateSummary.version} 已发布
              </span>
              <button
                type="button"
                onClick={onOpenChangelog}
                className="inline-flex items-center gap-1 text-[11.5px] text-[var(--accent)] hover:underline cursor-pointer shrink-0"
              >
                <FileText className="h-3 w-3" />
                <span>更新内容</span>
              </button>
            </div>

            {selfUpdateSummary.status === "downloading" ||
            selfUpdateSummary.status === "installing" ? (
              <span className="text-[12px] font-semibold text-[var(--accent)] tabular-nums shrink-0">
                {selfUpdateSummary.status === "installing"
                  ? "正在安装并重启..."
                  : `下载中 ${selfUpdateSummary.percent ?? 0}%`}
              </span>
            ) : (
              <button
                type="button"
                onClick={onInstallSelfUpdate}
                className="h-7 px-3 rounded-[8px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium cursor-pointer shrink-0"
              >
                立即更新
              </button>
            )}
          </div>

          {(selfUpdateSummary.status === "downloading" ||
            selfUpdateSummary.status === "installing") && (
            <div className="h-1 w-full rounded-full bg-[var(--bg-card)]/70 overflow-hidden">
              <div
                className="h-full rounded-full bg-[var(--accent)] transition-all duration-150"
                style={{ width: `${Math.max(4, selfUpdateSummary.percent ?? 4)}%` }}
              />
            </div>
          )}
        </div>
      )}

      {/* Batch Client Update Strip */}
      {updatableApps.length > 0 && (
        <div className="rounded-[12px] border border-[var(--border)] bg-[var(--bg-card)] px-4 py-3 flex items-center justify-between gap-3">
          <div className="min-w-0">
            <div className="text-[13px] font-medium text-[var(--text-primary)]">
              发现 {updatableApps.length} 款客户端有新版本
            </div>
            <div className="text-[11.5px] text-[var(--text-muted)] truncate mt-0.5">
              {updatableApps.map((a) => APP_DISPLAY_NAMES[a.id]).join(" · ")}
            </div>
          </div>
          <button
            type="button"
            disabled={batchUpdatingApps || busyApp !== null}
            onClick={onBatchUpgradeApps}
            className="h-8 px-3 rounded-[8px] bg-[var(--accent-soft)] hover:bg-[var(--accent-border)] text-[var(--accent)] text-[12px] font-semibold inline-flex items-center gap-1.5 transition cursor-pointer disabled:opacity-50 shrink-0"
          >
            <ArrowUpCircle className="h-3.5 w-3.5" />
            <span>{batchUpdatingApps ? "更新中..." : "一键全部更新"}</span>
          </button>
        </div>
      )}

      {/* Top Quick Controls: 2 clean cards */}
      <div className="grid grid-cols-1 min-[680px]:grid-cols-2 gap-3">
        {/* Card A: One-click Chinese UI */}
        <div className="rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] px-4 py-3.5 flex items-center justify-between gap-3">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <span className="text-[13px] font-semibold text-[var(--text-primary)]">
                一键三端汉化
              </span>
              <StatusPill tone={chineseFullyActive ? "ok" : "neutral"}>
                {togglingChineseUi
                  ? "同步中..."
                  : chineseFullyActive
                    ? "已生效"
                    : "待应用"}
              </StatusPill>
            </div>
            <p className="text-[11px] text-[var(--text-muted)] mt-1 leading-relaxed">
              Codex / Claude 界面与中文回复 · 反重力系统菜单与中文交互
            </p>
          </div>
          <ToggleSwitch
            checked={chineseFullyActive}
            disabled={togglingChineseUi || busyApp !== null || !dashboard}
            ariaLabel="一键三端汉化"
            data-testid="main-chinese-ui-toggle"
            onChange={() => onToggleMainChineseUi()}
          />
        </div>

        {/* Card B: Fleet & MCP Summary */}
        <div className="rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] px-4 py-3.5 flex items-center justify-between gap-3">
          <div className="flex items-center gap-5 min-w-0">
            <div>
              <div className="text-[11px] text-[var(--text-muted)]">已接管软件</div>
              <div className="text-[16px] font-semibold tabular-nums text-[var(--text-primary)] mt-0.5">
                {takenOverCount}
                <span className="text-[12px] font-normal text-[var(--text-muted)]">
                  {" "}
                  / {installedCount}
                </span>
              </div>
            </div>
            <div className="h-7 w-px bg-[var(--border)]" />
            <div>
              <div className="text-[11px] text-[var(--text-muted)]">MCP 能力</div>
              <div className="text-[16px] font-semibold tabular-nums text-[var(--text-primary)] mt-0.5">
                {mcpEnabledCount}
                <span className="text-[12px] font-normal text-[var(--text-muted)]">
                  {" "}
                  / {mcpTotalCount}
                </span>
              </div>
            </div>
          </div>

          <button
            type="button"
            onClick={() => onNavigate("mcp")}
            className="text-[12px] font-semibold text-[var(--accent)] hover:underline cursor-pointer shrink-0"
          >
            管理能力 →
          </button>
        </div>
      </div>

      {/* 4-Client Quick List */}
      <section className="rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] overflow-hidden">
        <div className="px-4 py-3.5 border-b border-[var(--border)] flex items-center justify-between">
          <div>
            <h2 className="text-[13.5px] font-semibold text-[var(--text-primary)]">
              桌面客户端
            </h2>
            <p className="text-[11px] text-[var(--text-muted)] mt-0.5">
              点击任一客户端进入该软件的专属模式、模型、汉化与维护设置。
            </p>
          </div>
        </div>

        {loading ? (
          <div className="px-4 py-8 text-center text-[12.5px] text-[var(--text-muted)]">
            正在读取本机客户端与配置...
          </div>
        ) : (
          <div className="divide-y divide-[var(--border)]">
            {orderedApps.map((app) => {
              const update = updatesMap[app.id];
              const isTako = app.install.installed && app.mode === "tako";
              return (
                <button
                  key={app.id}
                  type="button"
                  onClick={() => onNavigate(app.id)}
                  className="w-full px-4 py-3.5 flex items-center gap-3.5 text-left hover:bg-[var(--bg-subtle)]/70 transition cursor-pointer"
                >
                  <AppBrandIcon appId={app.id} className="h-7 w-7" />

                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 min-w-0">
                      <span className="text-[13.5px] font-semibold text-[var(--text-primary)] truncate">
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
                    </div>
                    <div className="text-[11.5px] text-[var(--text-muted)] truncate mt-0.5">
                      {briefClientSummary(app, update)}
                    </div>
                  </div>

                  <StatusPill
                    tone={
                      !app.install.installed
                        ? "neutral"
                        : isTako
                          ? app.id === "dsh-tako"
                            ? "ok"
                            : "accent"
                          : "neutral"
                    }
                  >
                    {!app.install.installed ? "未安装" : clientModeLabel(app)}
                  </StatusPill>

                  <ChevronRight className="h-4 w-4 text-[var(--text-muted)] shrink-0" />
                </button>
              );
            })}
          </div>
        )}
      </section>
    </div>
  );
}
