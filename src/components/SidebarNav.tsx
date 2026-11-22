import { Blocks, LayoutDashboard, Settings } from "lucide-react";
import type { AppId, AppSnapshot, AppUpdateStatus } from "../lib/api";
import { AppBrandIcon, TakoBrandMark } from "./BrandIcons";
import {
  APP_DISPLAY_NAMES,
  CLIENT_ORDER,
  sidebarShortStatus,
  type NavRoute,
} from "./UiPrimitives";

interface SidebarNavProps {
  activeRoute: NavRoute;
  onSelectRoute: (route: NavRoute) => void;
  appVersion: string;
  apps: AppSnapshot[];
  updatesMap: Partial<Record<AppId, AppUpdateStatus>>;
  mcpEnabledCount: number;
  mcpTotalCount: number;
  hasSelfUpdate: boolean;
  anyTakoMode: boolean;
  localGatewayHealthy: boolean;
}

export function SidebarNav({
  activeRoute,
  onSelectRoute,
  appVersion,
  apps,
  updatesMap,
  mcpEnabledCount,
  mcpTotalCount,
  hasSelfUpdate,
  anyTakoMode,
  localGatewayHealthy,
}: SidebarNavProps) {
  const appsById = new Map(apps.map((app) => [app.id, app]));

  const globalItems: {
    id: "overview" | "general" | "mcp";
    label: string;
    Icon: typeof LayoutDashboard;
    meta?: string;
    dot?: boolean;
  }[] = [
    { id: "overview", label: "总览", Icon: LayoutDashboard },
    { id: "general", label: "通用设置", Icon: Settings, dot: hasSelfUpdate },
    {
      id: "mcp",
      label: "MCP 能力",
      Icon: Blocks,
      meta: `${mcpEnabledCount}/${mcpTotalCount}`,
    },
  ];

  return (
    <aside className="w-[220px] shrink-0 border-r border-[var(--border)] bg-[var(--bg-sidebar)] px-3 pt-4 pb-3 flex flex-col select-none overflow-y-auto subtle-scroll">
      {/* Brand Block */}
      <div className="flex items-center gap-2.5 px-2 pb-4">
        <TakoBrandMark className="h-[28px] w-[28px] shrink-0" />
        <div className="min-w-0 leading-tight">
          <div className="text-[13.5px] font-semibold tracking-[-0.015em] text-[var(--text-primary)] truncate">
            Tako Switch
          </div>
          <div className="text-[10.5px] text-[var(--text-muted)] tabular-nums mt-0.5">
            v{appVersion}
          </div>
        </div>
      </div>

      {/* Group 1: Global */}
      <div className="px-2.5 mb-1.5 text-[10.5px] font-semibold tracking-[0.06em] text-[var(--text-muted)] uppercase">
        全局
      </div>
      <nav className="flex flex-col gap-0.5" aria-label="全局导航">
        {globalItems.map(({ id, label, Icon, meta, dot }) => {
          const active = activeRoute === id;
          return (
            <button
              key={id}
              type="button"
              data-testid={`nav-${id}`}
              onClick={() => onSelectRoute(id)}
              className={`h-[36px] w-full rounded-[9px] px-2.5 flex items-center gap-2.5 text-[13px] transition cursor-pointer whitespace-nowrap ${
                active
                  ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-semibold shadow-[var(--shadow-nav)]"
                  : "bg-transparent shadow-none text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]"
              }`}
            >
              <Icon
                className={`h-[16px] w-[16px] shrink-0 ${
                  active ? "text-[var(--accent)]" : "text-[var(--text-muted)]"
                }`}
                strokeWidth={1.8}
              />
              <span className="truncate">{label}</span>
              {dot && (
                <span
                  className="ml-auto h-1.5 w-1.5 rounded-full bg-[var(--accent)] shrink-0"
                  title="发现新版本"
                />
              )}
              {meta && (
                <span
                  className={`ml-auto text-[11px] tabular-nums shrink-0 ${
                    active ? "text-[var(--accent)] font-semibold" : "text-[var(--text-muted)]"
                  }`}
                >
                  {meta}
                </span>
              )}
            </button>
          );
        })}
      </nav>

      <div className="h-px bg-[var(--border)] my-3 mx-2" />

      {/* Group 2: Per-client Settings */}
      <div className="px-2.5 mb-1.5 text-[10.5px] font-semibold tracking-[0.06em] text-[var(--text-muted)] uppercase">
        软件独立设置
      </div>
      <nav className="flex flex-col gap-0.5" aria-label="软件独立设置导航">
        {CLIENT_ORDER.map((appId) => {
          const app = appsById.get(appId);
          const active = activeRoute === appId;
          const short = sidebarShortStatus(app);
          const hasUpdate = Boolean(updatesMap[appId]?.hasUpdate);

          const statusClass =
            short.tone === "accent"
              ? "text-[var(--accent)]"
              : short.tone === "ok"
                ? "text-[var(--success)]"
                : short.tone === "muted"
                  ? "text-[var(--text-muted)]/80"
                  : "text-[var(--text-muted)]";

          return (
            <button
              key={appId}
              type="button"
              data-testid={`nav-client-${appId}`}
              onClick={() => onSelectRoute(appId)}
              className={`h-[36px] w-full rounded-[9px] px-2.5 flex items-center gap-2.5 text-[13px] transition cursor-pointer whitespace-nowrap ${
                active
                  ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-semibold shadow-[var(--shadow-nav)]"
                  : "bg-transparent shadow-none text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]"
              }`}
            >
              <AppBrandIcon appId={appId} className="h-5 w-5" />
              <span className="truncate min-w-0">{APP_DISPLAY_NAMES[appId]}</span>
              <span className="ml-auto inline-flex items-center gap-1.5 shrink-0">
                {hasUpdate && (
                  <span
                    className="h-1.5 w-1.5 rounded-full bg-[var(--accent)] shrink-0"
                    title="有可用更新"
                  />
                )}
                <span className={`text-[11px] font-medium whitespace-nowrap ${statusClass}`}>
                  {short.text}
                </span>
              </span>
            </button>
          );
        })}
      </nav>

      {/* Footer status summary */}
      <div className="mt-auto pt-3 px-2 border-t border-[var(--border)] text-[11px] text-[var(--text-muted)] leading-snug">
        <div className="flex items-center gap-1.5 text-[var(--text-secondary)]">
          <span
            className={`h-1.5 w-1.5 rounded-full shrink-0 ${
              !anyTakoMode
                ? "bg-[var(--text-muted)]/60"
                : localGatewayHealthy
                  ? "bg-[var(--success)]"
                  : "bg-[var(--warning)]"
            }`}
          />
          <span className="truncate">
            {!anyTakoMode
              ? "本地网关待机"
              : localGatewayHealthy
                ? "网关 127.0.0.1:18419"
                : "本地网关待修复"}
          </span>
        </div>
      </div>
    </aside>
  );
}
