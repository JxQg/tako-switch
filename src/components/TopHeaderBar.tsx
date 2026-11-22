import { useEffect, useRef, useState } from "react";
import { KeyRound, LogOut, RefreshCw, Settings } from "lucide-react";
import type { AccountStatus } from "../lib/api";
import {
  APP_DISPLAY_NAMES,
  calcPercent,
  type NavRoute,
} from "./UiPrimitives";

interface TopHeaderBarProps {
  isMac: boolean;
  activeRoute: NavRoute;
  account?: AccountStatus;
  anyTakoMode: boolean;
  localGatewayHealthy: boolean;
  fixingGateway: boolean;
  refreshing: boolean;
  checkingUpdates: boolean;
  loggingIn: boolean;
  onEnsureGateway: () => void;
  onRefresh: () => void;
  onOAuthLogin: () => void;
  onOpenKeyModal: () => void;
  onLogout: () => void;
  onOpenGeneralSettings: () => void;
}

function routeBreadcrumb(route: NavRoute): { parent?: string; current: string } {
  if (route === "overview") return { current: "总览" };
  if (route === "general") return { current: "通用设置" };
  if (route === "mcp") return { current: "MCP 能力中心" };
  return { parent: "软件独立设置", current: APP_DISPLAY_NAMES[route] };
}

export function TopHeaderBar({
  isMac,
  activeRoute,
  account,
  anyTakoMode,
  localGatewayHealthy,
  fixingGateway,
  refreshing,
  checkingUpdates,
  loggingIn,
  onEnsureGateway,
  onRefresh,
  onOAuthLogin,
  onOpenKeyModal,
  onLogout,
  onOpenGeneralSettings,
}: TopHeaderBarProps) {
  const [accountOpen, setAccountOpen] = useState(false);
  const popoverRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!accountOpen) return;
    const onDown = (e: MouseEvent) => {
      if (popoverRef.current && !popoverRef.current.contains(e.target as Node)) {
        setAccountOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setAccountOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [accountOpen]);

  const crumb = routeBreadcrumb(activeRoute);
  const remainUsd = account?.walletRemainUsd;
  const usedUsd = account?.walletUsedUsd;
  const pct5h = calcPercent(account?.quota?.windowUsed, account?.quota?.windowCostLimit);
  const pct7d = calcPercent(account?.quota?.weeklyUsed, account?.quota?.weeklyCostLimit);
  const hasSubQuota = Boolean(
    account?.quota &&
      (account.quota.windowCostLimit > 0 || account.quota.weeklyCostLimit > 0),
  );
  const avatarChar = (account?.userName?.trim()?.[0] ?? "T").toUpperCase();

  return (
    <header
      data-tauri-drag-region
      className={`h-[48px] shrink-0 border-b border-[var(--border)] bg-[var(--bg-titlebar)] flex items-center justify-between gap-3 pr-3.5 select-none ${
        isMac ? "pl-[82px]" : "pl-4"
      }`}
    >
      {/* Left Breadcrumb */}
      <div
        data-tauri-drag-region
        className="flex items-center gap-1.5 min-w-0 text-[12.5px] truncate"
      >
        {crumb.parent && (
          <>
            <span data-tauri-drag-region className="text-[var(--text-muted)] shrink-0">
              {crumb.parent}
            </span>
            <span data-tauri-drag-region className="text-[var(--border-strong)] shrink-0">
              /
            </span>
          </>
        )}
        <span
          data-tauri-drag-region
          className="font-semibold text-[var(--text-primary)] truncate"
        >
          {crumb.current}
        </span>
      </div>

      {/* Drag Spacer */}
      <div data-tauri-drag-region className="flex-1 h-full min-w-[16px]" />

      {/* Right Status & Account */}
      <div className="flex items-center gap-2 shrink-0">
        {/* Gateway status indicator */}
        {!anyTakoMode ? (
          <span
            className="inline-flex items-center gap-1.5 text-[11.5px] text-[var(--text-muted)] px-2 h-7 rounded-full"
            title="当前暂无客户端处于 Tako 模式，网关处于待机状态"
          >
            <span className="h-[7px] w-[7px] rounded-full bg-[var(--text-muted)]/60 shrink-0" />
            <span className="hidden sm:inline">网关待机</span>
          </span>
        ) : localGatewayHealthy ? (
          <span
            className="inline-flex items-center gap-1.5 text-[11.5px] text-[var(--text-secondary)] px-2 h-7 rounded-full"
            title="本地统一网关 127.0.0.1:18419 运行正常"
          >
            <span className="tako-live-dot" />
            <span className="hidden sm:inline">网关运行中</span>
          </span>
        ) : (
          <button
            type="button"
            disabled={fixingGateway}
            onClick={onEnsureGateway}
            className="inline-flex items-center gap-1.5 h-7 px-2.5 rounded-full bg-[var(--warning-soft)] text-[var(--warning)] text-[11.5px] font-medium cursor-pointer disabled:opacity-60"
            title="点击立即重启本地统一网关"
          >
            <span className="h-[7px] w-[7px] rounded-full bg-[var(--warning)] shrink-0" />
            <span>{fixingGateway ? "修复中..." : "网关未启动 · 修复"}</span>
          </button>
        )}

        {/* Quiet Refresh */}
        <button
          type="button"
          onClick={onRefresh}
          disabled={refreshing || checkingUpdates}
          title="刷新状态与版本"
          aria-label="刷新状态与版本"
          className="h-7 w-7 rounded-[8px] border border-transparent hover:border-[var(--border)] hover:bg-[var(--bg-subtle)] text-[var(--text-muted)] hover:text-[var(--text-primary)] inline-flex items-center justify-center transition cursor-pointer disabled:opacity-50"
        >
          <RefreshCw
            className={`h-3.5 w-3.5 ${refreshing || checkingUpdates ? "animate-spin" : ""}`}
          />
        </button>

        {/* Account Area */}
        {account?.loggedIn ? (
          <div className="relative" ref={popoverRef}>
            <button
              type="button"
              onClick={() => setAccountOpen((prev) => !prev)}
              className="h-7 pl-2 pr-1 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] inline-flex items-center gap-2 transition cursor-pointer"
              title="查看账号、余额与配额"
            >
              {remainUsd !== null && remainUsd !== undefined && (
                <span className="text-[12px] font-semibold tabular-nums text-[var(--text-primary)]">
                  ${remainUsd.toFixed(2)}
                </span>
              )}
              <span className="h-[22px] w-[22px] rounded-[6px] bg-gradient-to-br from-[#6a61eb] to-[#4539ca] text-white text-[10.5px] font-bold inline-flex items-center justify-center shrink-0">
                {avatarChar}
              </span>
            </button>

            {accountOpen && (
              <div className="absolute right-0 mt-1.5 w-[264px] rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] shadow-[var(--shadow-modal)] p-3.5 z-40 flex flex-col gap-3">
                <div className="flex items-center gap-2.5 min-w-0">
                  <span className="h-9 w-9 rounded-[10px] bg-gradient-to-br from-[#6a61eb] to-[#4539ca] text-white text-[13px] font-bold inline-flex items-center justify-center shrink-0">
                    {avatarChar}
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5 min-w-0">
                      <span className="text-[13px] font-semibold text-[var(--text-primary)] truncate">
                        {account.userName ?? "Tako 用户"}
                      </span>
                      {account.planName && (
                        <span className="text-[10.5px] font-semibold px-1.5 py-0.5 rounded bg-[var(--accent-soft)] text-[var(--accent)] shrink-0">
                          {account.planName}
                        </span>
                      )}
                    </div>
                    {account.maskedKey && (
                      <div className="text-[11px] font-mono text-[var(--text-muted)] truncate mt-0.5">
                        {account.maskedKey}
                      </div>
                    )}
                  </div>
                </div>

                {remainUsd !== null && remainUsd !== undefined && (
                  <div className="rounded-[9px] bg-[var(--bg-subtle)] px-3 py-2 flex items-center justify-between">
                    <span className="text-[11.5px] text-[var(--text-secondary)]">钱包余额</span>
                    <div className="text-right tabular-nums">
                      <div className="text-[13.5px] font-semibold text-[var(--text-primary)]">
                        ${remainUsd.toFixed(2)}
                      </div>
                      {usedUsd !== null && usedUsd !== undefined && (
                        <div className="text-[10.5px] text-[var(--text-muted)]">
                          已用 ${usedUsd.toFixed(2)}
                        </div>
                      )}
                    </div>
                  </div>
                )}

                {hasSubQuota && (
                  <div className="flex flex-col gap-1.5 px-0.5 text-[11px] text-[var(--text-secondary)]">
                    <div className="flex items-center justify-between tabular-nums">
                      <span>5 小时窗口已用</span>
                      <span className="font-medium text-[var(--text-primary)]">{pct5h}%</span>
                    </div>
                    <div className="h-1.5 rounded-full bg-[var(--bg-subtle)] overflow-hidden">
                      <div
                        className="h-full rounded-full bg-[var(--accent)]"
                        style={{ width: `${Math.max(2, pct5h)}%` }}
                      />
                    </div>
                    <div className="flex items-center justify-between tabular-nums mt-1">
                      <span>7 天周期已用</span>
                      <span className="font-medium text-[var(--text-primary)]">{pct7d}%</span>
                    </div>
                    <div className="h-1.5 rounded-full bg-[var(--bg-subtle)] overflow-hidden">
                      <div
                        className="h-full rounded-full bg-[var(--accent)]"
                        style={{ width: `${Math.max(2, pct7d)}%` }}
                      />
                    </div>
                  </div>
                )}

                <div className="pt-2 border-t border-[var(--border)] flex items-center justify-between gap-2">
                  <button
                    type="button"
                    onClick={() => {
                      setAccountOpen(false);
                      onOpenGeneralSettings();
                    }}
                    className="inline-flex items-center gap-1.5 text-[12px] text-[var(--text-secondary)] hover:text-[var(--text-primary)] cursor-pointer"
                  >
                    <Settings className="h-3.5 w-3.5" />
                    <span>账号与通用设置</span>
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      setAccountOpen(false);
                      onLogout();
                    }}
                    className="inline-flex items-center gap-1 text-[12px] text-[var(--danger)] hover:opacity-80 cursor-pointer"
                  >
                    <LogOut className="h-3.5 w-3.5" />
                    <span>退出</span>
                  </button>
                </div>
              </div>
            )}
          </div>
        ) : (
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              onClick={onOpenKeyModal}
              title="手动输入 API Key 登录"
              className="h-7 px-2 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[11.5px] font-medium text-[var(--text-secondary)] inline-flex items-center gap-1 transition cursor-pointer"
            >
              <KeyRound className="h-3 w-3" />
              <span className="hidden sm:inline">Key</span>
            </button>
            <button
              type="button"
              onClick={onOAuthLogin}
              disabled={loggingIn}
              className="h-7 px-3 rounded-[8px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium transition cursor-pointer disabled:opacity-60"
            >
              {loggingIn ? "登录中..." : "登录 Tako"}
            </button>
          </div>
        )}
      </div>
    </header>
  );
}
