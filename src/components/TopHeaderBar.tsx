import { useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  Check,
  ChevronDown,
  ExternalLink,
  KeyRound,
  Loader2,
  LogOut,
  Moon,
  Plus,
  RefreshCw,
  Settings,
  Sun,
  Trash2,
  Wallet,
} from "lucide-react";
import { api, type AccountStatus } from "../lib/api";
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
  switchingKeyId: string | null;
  onGoHome: () => void;
  onToggleTheme: () => void;
  onRefresh: () => void;
  onOAuthLogin: () => void;
  onOpenKeyModal: () => void;
  onSwitchKey: (keyId: string) => void;
  onRemoveKey: (keyId: string) => void;
  onLogout: () => void;
  onOpenGeneralSettings: () => void;
  onMinimizeWindow?: () => void;
  onToggleMaximizeWindow?: () => void;
  onCloseWindow?: () => void;
  onTitleBarContextMenu?: () => void;
}

const DEFAULT_TAKO_ORIGIN = "https://tako.shiroha.tech";

function normalizeOrigin(baseUrl?: string | null): string {
  const trimmed = (baseUrl ?? "").trim().replace(/\/+$/, "");
  if (!trimmed) return DEFAULT_TAKO_ORIGIN;
  return trimmed.endsWith("/v1") ? trimmed.slice(0, -3) : trimmed;
}

function resolvePrimaryKeyLabel(account?: AccountStatus): string {
  if (!account) return "Tako 用户";
  const direct = account.displayLabel?.trim();
  if (direct) return direct;
  const user = account.userName?.trim();
  const key = account.keyName?.trim();
  if (user && key) return `${user} - ${key}`;
  if (user && account.maskedKey) return `${user} - ${account.maskedKey}`;
  return user || key || account.maskedKey || "Tako 用户";
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
  switchingKeyId,
  onGoHome,
  onToggleTheme,
  onRefresh,
  onOAuthLogin,
  onOpenKeyModal,
  onSwitchKey,
  onRemoveKey,
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

  const takoOrigin = normalizeOrigin(account?.baseUrl);
  const walletUrl = `${takoOrigin}/wallet`;
  const keysManageUrl = `${takoOrigin}/keys`;
  const savedKeys = account?.savedKeys ?? [];
  const primaryKeyLabel = resolvePrimaryKeyLabel(account);
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
        className={`flex items-center gap-2.5 min-w-0 max-w-[54%] transition-opacity ${
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
      <div className="flex items-center h-full min-w-0">
        <div
          data-tauri-drag-region="false"
          className={`flex items-center gap-1.5 min-w-0 ${
            isWindows ? "pr-1.5" : ""
          }`}
        >
          {account?.loggedIn ? (
            <>
              <button
                type="button"
                data-testid="topbar-recharge"
                onClick={() => void api.openUrl(walletUrl)}
                title="前往 Tako 钱包充值"
                className="h-8 px-2.5 rounded-[9px] border border-[var(--accent)]/30 bg-[var(--accent-soft)] hover:bg-[var(--accent)] hover:text-white text-[var(--accent)] text-[12px] font-medium inline-flex items-center gap-1.5 transition cursor-pointer shrink-0"
              >
                <Wallet className="h-3.5 w-3.5" />
                <span>充值</span>
              </button>

              <div className="relative min-w-0" ref={popoverRef}>
                <button
                  type="button"
                  data-testid="topbar-key-switcher"
                  onClick={() => setAccountOpen((prev) => !prev)}
                  title={`${primaryKeyLabel}${account.activeKeyAlias ? ` [${account.activeKeyAlias}]` : ""}${account.maskedKey ? ` · ${account.maskedKey}` : ""}（点击切换密钥或查看余额）`}
                  className="h-8 w-full px-2.5 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] shadow-[var(--shadow-card)] inline-flex items-center gap-1.5 transition cursor-pointer"
                >
                  {account.activeKeyAlias && (
                    <span className="text-[10.5px] font-semibold px-1.5 py-0.5 rounded bg-[var(--accent-soft)] text-[var(--accent)] shrink-0 max-w-[54px] truncate">
                      {account.activeKeyAlias}
                    </span>
                  )}
                  <span
                    className={`text-[12px] font-medium text-[var(--text-primary)] truncate ${
                      isWindows ? "max-w-[136px]" : "max-w-[168px]"
                    }`}
                  >
                    {primaryKeyLabel}
                  </span>
                  {account.planName && (
                    <span className="hidden min-[900px]:inline max-w-[120px] truncate text-[11px] font-medium text-[var(--accent)]">
                      {account.planName}
                    </span>
                  )}
                  {remainUsd !== null && remainUsd !== undefined && (
                    <>
                      <span className="h-3.5 w-px bg-[var(--border)] shrink-0" />
                      <span
                        className="tabular-nums text-[12px] font-semibold shrink-0"
                        style={{ color: balanceColor }}
                      >
                        ${remainUsd.toFixed(2)}
                      </span>
                    </>
                  )}
                  <ChevronDown
                    className={`h-3.5 w-3.5 text-[var(--text-muted)] shrink-0 transition-transform ${
                      accountOpen ? "rotate-180" : ""
                    }`}
                  />
                </button>

                {accountOpen && (
                  <div className="absolute right-0 mt-1.5 w-[320px] rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] shadow-[var(--shadow-modal)] p-3.5 z-40 flex flex-col gap-3">
                    <div className="flex items-start gap-2.5 min-w-0">
                      <span className="h-9 w-9 rounded-[10px] bg-gradient-to-br from-[#6a61eb] to-[#4539ca] text-white text-[13px] font-bold inline-flex items-center justify-center shrink-0">
                        {avatarChar}
                      </span>
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-1.5 flex-wrap">
                          <span className="text-[12.5px] font-semibold text-[var(--text-primary)] break-all leading-snug">
                            {primaryKeyLabel}
                          </span>
                          {account.activeKeyAlias && (
                            <span className="text-[10px] font-semibold px-1.5 py-0.5 rounded bg-[var(--accent-soft)] text-[var(--accent)] shrink-0">
                              {account.activeKeyAlias}
                            </span>
                          )}
                          {account.planName && (
                            <span className="text-[10px] font-semibold px-1.5 py-0.5 rounded bg-[var(--bg-subtle)] text-[var(--text-secondary)] shrink-0">
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

                    {/* Local Saved Keys Switcher */}
                    <div className="pt-2 border-t border-[var(--border)] flex flex-col gap-1.5">
                      <div className="flex items-center justify-between gap-2">
                        <span className="text-[11px] font-semibold text-[var(--text-secondary)]">
                          切换密钥 ({savedKeys.length})
                        </span>
                        <button
                          type="button"
                          data-testid="popover-add-key"
                          onClick={() => {
                            setAccountOpen(false);
                            onOpenKeyModal();
                          }}
                          className="inline-flex items-center gap-1 text-[11px] font-medium text-[var(--accent)] hover:underline cursor-pointer"
                        >
                          <Plus className="h-3 w-3" />
                          <span>添加密钥</span>
                        </button>
                      </div>

                      <div className="flex flex-col gap-1 max-h-[196px] overflow-y-auto subtle-scroll pr-0.5">
                        {savedKeys.map((item) => {
                          const isSwitching = switchingKeyId === item.id;
                          return (
                            <div
                              key={item.id}
                              className={`group rounded-[8px] border px-2 py-1.5 flex items-center gap-2 transition ${
                                item.active
                                  ? "border-[var(--accent)]/40 bg-[var(--accent-soft)]/70"
                                  : "border-transparent hover:border-[var(--border)] hover:bg-[var(--bg-subtle)]"
                              }`}
                            >
                              <button
                                type="button"
                                disabled={Boolean(switchingKeyId) || item.active}
                                onClick={() => {
                                  onSwitchKey(item.id);
                                  setAccountOpen(false);
                                }}
                                title={
                                  item.active
                                    ? "当前正在使用的密钥"
                                    : `切换到 ${item.displayLabel}`
                                }
                                className={`flex-1 min-w-0 flex items-center gap-2 text-left ${
                                  item.active ? "cursor-default" : "cursor-pointer"
                                } disabled:opacity-70`}
                              >
                                <span className="w-3.5 shrink-0 flex items-center justify-center">
                                  {isSwitching ? (
                                    <Loader2 className="h-3.5 w-3.5 animate-spin text-[var(--accent)]" />
                                  ) : item.active ? (
                                    <Check className="h-3.5 w-3.5 text-[var(--accent)]" />
                                  ) : (
                                    <span className="h-1.5 w-1.5 rounded-full bg-[var(--border-strong)] group-hover:bg-[var(--accent)]" />
                                  )}
                                </span>
                                <div className="min-w-0 flex-1">
                                  <div className="flex items-center gap-1 min-w-0">
                                    <span
                                      className={`text-[12px] truncate ${
                                        item.active
                                          ? "font-semibold text-[var(--text-primary)]"
                                          : "font-medium text-[var(--text-primary)]"
                                      }`}
                                    >
                                      {item.displayLabel}
                                    </span>
                                    {item.alias && (
                                      <span className="text-[10px] px-1.5 py-0 rounded bg-[var(--bg-card)] border border-[var(--border)] text-[var(--text-secondary)] shrink-0">
                                        {item.alias}
                                      </span>
                                    )}
                                  </div>
                                  <div className="text-[10.5px] font-mono text-[var(--text-muted)] truncate flex items-center gap-1.5">
                                    <span>{item.maskedKey}</span>
                                    {item.authSource === "oauth" && (
                                      <span className="font-sans text-[10px] text-[var(--text-muted)]">
                                        · OAuth
                                      </span>
                                    )}
                                  </div>
                                </div>
                              </button>
                              <button
                                type="button"
                                disabled={Boolean(switchingKeyId)}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  setAccountOpen(false);
                                  onRemoveKey(item.id);
                                }}
                                title="从本地列表移除此密钥（不会删除平台密钥）"
                                aria-label={`移除密钥 ${item.displayLabel}`}
                                className="p-1 rounded text-[var(--text-muted)] hover:text-[var(--danger)] hover:bg-[var(--danger-soft)] opacity-0 group-hover:opacity-100 focus:opacity-100 transition cursor-pointer shrink-0 disabled:opacity-40"
                              >
                                <Trash2 className="h-3 w-3" />
                              </button>
                            </div>
                          );
                        })}
                      </div>
                    </div>

                    <div className="pt-2 border-t border-[var(--border)] flex items-center justify-between gap-2">
                      <div className="flex items-center gap-2.5">
                        <button
                          type="button"
                          onClick={() => {
                            setAccountOpen(false);
                            void api.openUrl(keysManageUrl);
                          }}
                          title="前往 Tako 平台管理密钥"
                          className="inline-flex items-center gap-1 text-[11.5px] text-[var(--text-secondary)] hover:text-[var(--accent)] cursor-pointer"
                        >
                          <ExternalLink className="h-3 w-3" />
                          <span>平台管理</span>
                        </button>
                        <button
                          type="button"
                          onClick={() => {
                            setAccountOpen(false);
                            onOpenGeneralSettings();
                          }}
                          className="inline-flex items-center gap-1 text-[11.5px] text-[var(--text-secondary)] hover:text-[var(--text-primary)] cursor-pointer"
                        >
                          <Settings className="h-3 w-3" />
                          <span>设置</span>
                        </button>
                      </div>
                      <button
                        type="button"
                        onClick={() => {
                          setAccountOpen(false);
                          onLogout();
                        }}
                        className="inline-flex items-center gap-1 text-[11.5px] text-[var(--danger)] hover:opacity-80 cursor-pointer"
                      >
                        <LogOut className="h-3 w-3" />
                        <span>退出</span>
                      </button>
                    </div>
                  </div>
                )}
              </div>
            </>
          ) : (
            <div className="flex items-center gap-1.5">
              {savedKeys.length > 0 && (
                <div className="relative" ref={popoverRef}>
                  <button
                    type="button"
                    data-testid="topbar-saved-keys-logged-out"
                    onClick={() => setAccountOpen((prev) => !prev)}
                    className="h-8 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-primary)] inline-flex items-center gap-1.5 transition cursor-pointer"
                  >
                    <KeyRound className="h-3.5 w-3.5 text-[var(--accent)]" />
                    <span>已存密钥 ({savedKeys.length})</span>
                    <ChevronDown className="h-3 w-3 text-[var(--text-muted)]" />
                  </button>
                  {accountOpen && (
                    <div className="absolute right-0 mt-1.5 w-[296px] rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] shadow-[var(--shadow-modal)] p-3 z-40 flex flex-col gap-2">
                      <div className="flex items-center justify-between">
                        <span className="text-[11.5px] font-semibold text-[var(--text-secondary)]">
                          选择本地密钥重新激活
                        </span>
                        <button
                          type="button"
                          onClick={() => {
                            setAccountOpen(false);
                            onOpenKeyModal();
                          }}
                          className="inline-flex items-center gap-1 text-[11px] font-medium text-[var(--accent)] hover:underline cursor-pointer"
                        >
                          <Plus className="h-3 w-3" />
                          <span>添加密钥</span>
                        </button>
                      </div>
                      <div className="flex flex-col gap-1 max-h-[196px] overflow-y-auto subtle-scroll">
                        {savedKeys.map((item) => (
                          <div
                            key={item.id}
                            className="group rounded-[8px] border border-transparent hover:border-[var(--border)] hover:bg-[var(--bg-subtle)] px-2 py-1.5 flex items-center gap-2"
                          >
                            <button
                              type="button"
                              disabled={Boolean(switchingKeyId)}
                              onClick={() => {
                                onSwitchKey(item.id);
                                setAccountOpen(false);
                              }}
                              className="flex-1 min-w-0 text-left cursor-pointer disabled:opacity-60"
                            >
                              <div className="flex items-center gap-1.5">
                                <span className="text-[12px] font-medium text-[var(--text-primary)] truncate">
                                  {item.displayLabel}
                                </span>
                                {item.alias && (
                                  <span className="text-[10px] px-1.5 rounded bg-[var(--accent-soft)] text-[var(--accent)] shrink-0">
                                    {item.alias}
                                  </span>
                                )}
                              </div>
                              <div className="text-[10.5px] font-mono text-[var(--text-muted)] truncate">
                                {item.maskedKey}
                              </div>
                            </button>
                            <button
                              type="button"
                              disabled={Boolean(switchingKeyId)}
                              onClick={(e) => {
                                e.stopPropagation();
                                setAccountOpen(false);
                                onRemoveKey(item.id);
                              }}
                              title="从本地列表移除"
                              className="p-1 rounded text-[var(--text-muted)] hover:text-[var(--danger)] hover:bg-[var(--danger-soft)] opacity-0 group-hover:opacity-100 transition cursor-pointer shrink-0"
                            >
                              <Trash2 className="h-3 w-3" />
                            </button>
                          </div>
                        ))}
                      </div>
                    </div>
                  )}
                </div>
              )}
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
            className="h-8 w-8 shrink-0 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] inline-flex items-center justify-center transition cursor-pointer disabled:opacity-50"
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
            className="h-8 w-8 shrink-0 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] inline-flex items-center justify-center transition cursor-pointer"
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
            className={`relative h-8 w-8 shrink-0 rounded-[8px] border inline-flex items-center justify-center transition cursor-pointer ${
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
