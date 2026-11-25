import { useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  KeyRound,
  LogOut,
  Moon,
  RefreshCw,
  Settings,
  Sun,
} from "lucide-react";
import type { AccountStatus } from "../lib/api";
import { TakoBrandMark } from "./BrandIcons";
import {
  APP_DISPLAY_NAMES,
  calcPercent,
  type NavRoute,
} from "./UiPrimitives";
import { WindowsCaptionControls } from "./WindowChrome";

interface TopHeaderBarProps {
  isMac: boolean;
  isWindows: boolean;
  scrolled: boolean;
  windowFocused?: boolean;
  isMaximized?: boolean;
  appVersion: string;
  activeRoute: NavRoute;
  account?: AccountStatus;
  isDark: boolean;
  hasPendingUpdates: boolean;
  refreshing: boolean;
  checkingUpdates: boolean;
  loggingIn: boolean;
  onGoHome: () => void;
  onToggleTheme: () => void;
  onRefresh: () => void;
  onOAuthLogin: () => void;
  onOpenKeyModal: () => void;
  onLogout: () => void;
  onOpenGeneralSettings: () => void;
  onMinimizeWindow?: () => void;
  onToggleMaximizeWindow?: () => void;
  onCloseWindow?: () => void;
  onTitleBarContextMenu?: () => void;
}

function subpageTitle(route: NavRoute): string {
  if (route === "general") return "通用设置";
  if (route === "mcp") return "MCP 能力中心";
  if (route === "extensions") return "通用扩展中心";
  if (route === "overview") return "首页";
  return `${APP_DISPLAY_NAMES[route]} 设置`;
}

function isInteractiveTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return Boolean(
    target.closest(
      'button, a, input, select, textarea, [role="button"], [data-tauri-drag-region="false"]',
    ),
  );
}

export function TopHeaderBar({
  isMac,
  isWindows,
  scrolled,
  windowFocused = true,
  isMaximized = false,
  appVersion,
  activeRoute,
  account,
  isDark,
  hasPendingUpdates,
  refreshing,
  checkingUpdates,
  loggingIn,
  onGoHome,
  onToggleTheme,
  onRefresh,
  onOAuthLogin,
  onOpenKeyModal,
  onLogout,
  onOpenGeneralSettings,
  onMinimizeWindow,
  onToggleMaximizeWindow,
  onCloseWindow,
  onTitleBarContextMenu,
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

  const remainUsd = account?.walletRemainUsd;
  const usedUsd = account?.walletUsedUsd;
  const pct5h = calcPercent(account?.quota?.windowUsed, account?.quota?.windowCostLimit);
  const pct7d = calcPercent(account?.quota?.weeklyUsed, account?.quota?.weeklyCostLimit);
  const hasSubQuota = Boolean(
    account?.quota &&
      (account.quota.windowCostLimit > 0 || account.quota.weeklyCostLimit > 0),
  );
  const avatarChar = (account?.userName?.trim()?.[0] ?? "T").toUpperCase();
  const balanceColor =
    remainUsd !== null && remainUsd !== undefined
      ? remainUsd < 0.5
        ? "var(--danger)"
        : remainUsd < 2.0
          ? "var(--warning)"
          : "var(--success)"
      : "var(--text-primary)";

  const isSubpage = activeRoute !== "overview";
  const paddingClass = isMac
    ? "tako-titlebar-mac pl-[82px] pr-3.5"
    : isWindows
      ? "pl-[var(--density-page-px)] pr-0"
      : "px-[var(--density-page-px)]";

  const handleContextMenu = (e: React.MouseEvent<HTMLElement>) => {
    if (!isWindows || !onTitleBarContextMenu) return;
    if (isInteractiveTarget(e.target)) return;
    e.preventDefault();
    onTitleBarContextMenu();
  };

  return (
    <header
      data-tauri-drag-region
      onContextMenu={handleContextMenu}
      className={`tako-titlebar ${paddingClass} ${
        scrolled ? "tako-titlebar-scrolled" : "tako-titlebar-merged"
      } shrink-0 flex items-center justify-between gap-2 select-none z-20`}
    >
      {/* Left: Brand on Home, Prominent Back + Title on Subpages */}
      <div
        data-tauri-drag-region="deep"
        className={`flex items-center gap-2.5 min-w-0 shrink-0 max-w-[54%] transition-opacity ${
          !windowFocused ? "opacity-80" : ""
        }`}
      >
        {isSubpage ? (
          <>
            <button
              type="button"
              data-testid="topbar-back-home"
              onClick={onGoHome}
              title="返回首页 (Esc)"
              className="h-8 px-3 -ml-1 rounded-[9px] border border-[var(--border-strong)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-primary)] text-[12.5px] font-semibold inline-flex items-center gap-1.5 transition cursor-pointer shrink-0 shadow-[var(--shadow-card)]"
            >
              <ArrowLeft className="h-3.5 w-3.5 text-[var(--text-primary)]" />
              <span>返回首页</span>
            </button>
            <span className="text-[var(--border-strong)] shrink-0">/</span>
            <span className="text-[13.5px] font-semibold tracking-[-0.015em] text-[var(--text-primary)] truncate">
              {subpageTitle(activeRoute)}
            </span>
          </>
        ) : (
          <>
            <div className="pointer-events-none shrink-0 flex items-center">
              <TakoBrandMark className="h-[24px] w-[24px]" />
            </div>
            <span className="text-[13.5px] font-semibold tracking-[-0.015em] text-[var(--text-primary)] truncate">
              Tako Switch
            </span>
            <span className="tabular-nums text-[11px] font-medium text-[var(--text-muted)] bg-[var(--bg-subtle)] border border-[var(--border)] rounded-full px-2 py-0.5 shrink-0">
              v{appVersion}
            </span>
          </>
        )}
      </div>

      {/* Flexible Drag Spacer (Drag + Double-Click Maximize) */}
      <div data-tauri-drag-region className="flex-1 h-full min-w-[16px]" />

      {/* Right App Actions + Native Windows Caption Controls */}
      <div className="flex items-center h-full shrink-0">
        <div
          data-tauri-drag-region="false"
          className={`flex items-center gap-1.5 shrink-0 ${
            isWindows ? "pr-1.5" : ""
          }`}
        >
          {account?.loggedIn ? (
            <div className="relative" ref={popoverRef}>
              <button
                type="button"
                onClick={() => setAccountOpen((prev) => !prev)}
                title="点击查看账号、余额与配额详情"
                className="h-8 px-2.5 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] shadow-[var(--shadow-card)] inline-flex items-center gap-2 transition cursor-pointer"
              >
                <span className="text-[12px] font-medium text-[var(--text-primary)] max-w-[84px] truncate">
                  {account.userName ?? "Tako 用户"}
                </span>
                {account.planName && (
                  <span className="hidden min-[900px]:inline text-[11.5px] font-medium text-[var(--accent)] shrink-0">
                    {account.planName}
                  </span>
                )}
                {remainUsd !== null && remainUsd !== undefined && (
                  <>
                    <span className="h-3.5 w-px bg-[var(--border)]" />
                    <span
                      className="tabular-nums text-[12px] font-semibold shrink-0"
                      style={{ color: balanceColor }}
                    >
                      ${remainUsd.toFixed(2)}
                    </span>
                  </>
                )}
              </button>

              {accountOpen && (
                <div className="absolute right-0 mt-1.5 w-[268px] rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] shadow-[var(--shadow-modal)] p-3.5 z-40 flex flex-col gap-3">
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
                      <span className="text-[11.5px] text-[var(--text-secondary)]">
                        钱包余额
                      </span>
                      <div className="text-right tabular-nums">
                        <div
                          className="text-[13.5px] font-semibold"
                          style={{ color: balanceColor }}
                        >
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
                        <span className="font-medium text-[var(--text-primary)]">
                          {pct5h}%
                        </span>
                      </div>
                      <div className="h-1.5 rounded-full bg-[var(--bg-subtle)] overflow-hidden">
                        <div
                          className="h-full rounded-full bg-[var(--accent)]"
                          style={{ width: `${Math.max(2, pct5h)}%` }}
                        />
                      </div>
                      <div className="flex items-center justify-between tabular-nums mt-1">
                        <span>7 天周期已用</span>
                        <span className="font-medium text-[var(--text-primary)]">
                          {pct7d}%
                        </span>
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
                className="h-8 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)] inline-flex items-center gap-1.5 transition cursor-pointer"
              >
                <KeyRound className="h-3.5 w-3.5" />
                <span>Key 登录</span>
              </button>
              <button
                type="button"
                onClick={onOAuthLogin}
                disabled={loggingIn}
                className="h-8 px-3 rounded-[8px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium transition cursor-pointer disabled:opacity-60"
              >
                {loggingIn ? "登录中..." : "登录 Tako"}
              </button>
            </div>
          )}

          <button
            type="button"
            onClick={onRefresh}
            disabled={refreshing || checkingUpdates}
            title="刷新状态与版本"
            aria-label="刷新状态与版本"
            className="h-8 w-8 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] inline-flex items-center justify-center transition cursor-pointer disabled:opacity-50"
          >
            <RefreshCw
              className={`h-3.5 w-3.5 ${
                refreshing || checkingUpdates ? "animate-spin" : ""
              }`}
            />
          </button>

          <button
            type="button"
            onClick={onToggleTheme}
            title={isDark ? "切换浅色模式" : "切换深色模式"}
            aria-label={isDark ? "切换浅色模式" : "切换深色模式"}
            className="h-8 w-8 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] inline-flex items-center justify-center transition cursor-pointer"
          >
            {isDark ? <Sun className="h-3.5 w-3.5" /> : <Moon className="h-3.5 w-3.5" />}
          </button>

          <button
            type="button"
            data-testid="topbar-general-settings"
            onClick={() =>
              activeRoute === "general" ? onGoHome() : onOpenGeneralSettings()
            }
            title={activeRoute === "general" ? "返回首页" : "通用设置与更新"}
            aria-label="通用设置与更新"
            className={`relative h-8 w-8 rounded-[8px] border inline-flex items-center justify-center transition cursor-pointer ${
              activeRoute === "general"
                ? "border-[var(--accent-border)] bg-[var(--accent-soft)] text-[var(--accent)]"
                : "border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
            }`}
          >
            <Settings className="h-3.5 w-3.5" />
            {hasPendingUpdates && (
              <span className="absolute -top-0.5 -right-0.5 h-2 w-2 rounded-full bg-[var(--accent)] ring-2 ring-[var(--bg-canvas)]" />
            )}
          </button>
        </div>

        {isWindows && onMinimizeWindow && onToggleMaximizeWindow && onCloseWindow && (
          <>
            <span
              aria-hidden="true"
              className="h-4 w-px bg-[var(--border)] shrink-0 ml-0.5"
            />
            <WindowsCaptionControls
              isMaximized={isMaximized}
              isFocused={windowFocused}
              onMinimize={onMinimizeWindow}
              onToggleMaximize={onToggleMaximizeWindow}
              onClose={onCloseWindow}
            />
          </>
        )}
      </div>
    </header>
  );
}
