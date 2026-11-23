import { useEffect, useState } from "react";
import {
  ChevronDown,
  ExternalLink,
  FileText,
  KeyRound,
  LogOut,
  MessageSquare,
  RefreshCw,
} from "lucide-react";
import {
  api,
  DEFAULT_FALLBACK_MANIFEST_URL,
  DEFAULT_MANIFEST_URL,
  TAKO_FEEDBACK_URL,
  TAKO_RELEASES_URL,
  type AppId,
  type AppSnapshot,
  type DashboardPayload,
  type SettingsPatch,
} from "../lib/api";
import {
  APP_DISPLAY_NAMES,
  SegmentedControl,
  StatusPill,
  ToggleSwitch,
  calcPercent,
  type NavRoute,
  type ThemePref,
} from "./UiPrimitives";

interface SelfUpdateViewState {
  status: "idle" | "checking" | "latest" | "available" | "downloading" | "installing" | "error";
  version?: string;
  source?: string;
  percent?: number;
  error?: string | null;
}

interface GeneralSettingsPanelProps {
  dashboard: DashboardPayload | null;
  themePref: ThemePref;
  onChangeTheme: (pref: ThemePref) => void;
  anyTakoMode: boolean;
  fixingGateway: boolean;
  loggingIn: boolean;
  savingSettings: boolean;
  checkingUpdates: boolean;
  lastCheckedAt: string | null;
  updatableApps: AppSnapshot[];
  batchUpdatingApps: boolean;
  busyApp: AppId | null;
  selfUpdateView: SelfUpdateViewState;
  mcpEnabledCount: number;
  mcpTotalCount: number;
  onEnsureGateway: () => void;
  onOAuthLogin: () => void;
  onOpenKeyModal: () => void;
  onLogout: () => void;
  onRefreshAccount: () => void;
  onSavePatch: (patch: SettingsPatch, note?: string) => Promise<void>;
  onCheckUpdates: () => void;
  onInstallSelfUpdate: () => void;
  onBatchUpgradeApps: () => void;
  onOpenChangelog: () => void;
  onNavigate: (route: NavRoute) => void;
}

const THEME_OPTIONS: { id: ThemePref; label: string }[] = [
  { id: "system", label: "跟随系统" },
  { id: "light", label: "浅色" },
  { id: "dark", label: "深色" },
];

export function GeneralSettingsPanel({
  dashboard,
  themePref,
  onChangeTheme,
  anyTakoMode,
  fixingGateway,
  loggingIn,
  savingSettings,
  checkingUpdates,
  lastCheckedAt,
  updatableApps,
  batchUpdatingApps,
  busyApp,
  selfUpdateView,
  mcpEnabledCount,
  mcpTotalCount,
  onEnsureGateway,
  onOAuthLogin,
  onOpenKeyModal,
  onLogout,
  onRefreshAccount,
  onSavePatch,
  onCheckUpdates,
  onInstallSelfUpdate,
  onBatchUpgradeApps,
  onOpenChangelog,
  onNavigate,
}: GeneralSettingsPanelProps) {
  const [advancedOpen, setAdvancedOpen] = useState(false);

  const [gatewayDisplayName, setGatewayDisplayName] = useState("Tako");
  const [outputDir, setOutputDir] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [manifestUrl, setManifestUrl] = useState("");
  const [fallbackManifestUrl, setFallbackManifestUrl] = useState("");

  useEffect(() => {
    if (!dashboard) return;
    setGatewayDisplayName(dashboard.state.gatewayDisplayName ?? "Tako");
    setOutputDir(dashboard.state.mcp.outputDir ?? "");
    setBaseUrl(dashboard.state.baseUrl ?? "");
    setManifestUrl(dashboard.state.manifestUrl ?? "");
    setFallbackManifestUrl(dashboard.state.fallbackManifestUrl ?? "");
  }, [dashboard]);

  const account = dashboard?.account;
  const remainUsd = account?.walletRemainUsd;
  const usedUsd = account?.walletUsedUsd;
  const pct5h = calcPercent(account?.quota?.windowUsed, account?.quota?.windowCostLimit);
  const pct7d = calcPercent(account?.quota?.weeklyUsed, account?.quota?.weeklyCostLimit);
  const hasSubQuota = Boolean(
    account?.quota &&
      (account.quota.windowCostLimit > 0 || account.quota.weeklyCostLimit > 0),
  );
  const avatarChar = (account?.userName?.trim()?.[0] ?? "T").toUpperCase();

  const nameDirty =
    dashboard != null &&
    gatewayDisplayName.trim() !== (dashboard.state.gatewayDisplayName ?? "Tako").trim();
  const outputDirDirty =
    dashboard != null && outputDir.trim() !== (dashboard.state.mcp.outputDir ?? "").trim();
  const advancedDirty =
    dashboard != null &&
    (baseUrl.trim() !== dashboard.state.baseUrl.trim() ||
      manifestUrl.trim() !== dashboard.state.manifestUrl.trim() ||
      fallbackManifestUrl.trim() !== dashboard.state.fallbackManifestUrl.trim());

  return (
    <div className="max-w-[760px] mx-auto px-6 py-6 flex flex-col gap-5">
      {/* Page Head */}
      <div>
        <div className="text-[10.5px] font-bold tracking-[0.08em] uppercase text-[var(--accent)] mb-1">
          全局配置
        </div>
        <h1 className="text-[22px] font-semibold tracking-[-0.025em] text-[var(--text-primary)] leading-tight">
          通用设置
        </h1>
        <p className="text-[12.5px] text-[var(--text-secondary)] mt-1">
          管理所有桌面客户端共享的账号、网关、更新策略与外观偏好。
        </p>
      </div>

      {/* Account & Balance Card */}
      <section className="rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] px-4 py-4">
        {account?.loggedIn ? (
          <div className="flex flex-col gap-3.5">
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div className="flex items-center gap-3 min-w-0 flex-1">
                <span className="h-[42px] w-[42px] rounded-[12px] bg-gradient-to-br from-[#6a61eb] to-[#4539ca] text-white text-[15px] font-bold inline-flex items-center justify-center shrink-0">
                  {avatarChar}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-[14px] font-semibold text-[var(--text-primary)] truncate">
                      {account.userName ?? "Tako 用户"}
                    </span>
                    {account.planName && (
                      <StatusPill tone="accent">{account.planName}</StatusPill>
                    )}
                  </div>
                  <div className="text-[11.5px] text-[var(--text-muted)] font-mono truncate mt-0.5">
                    {account.maskedKey ?? "已通过 OAuth 授权登录"}
                  </div>
                </div>
              </div>

              {remainUsd !== null && remainUsd !== undefined && (
                <div className="pl-4 border-l border-[var(--border)] tabular-nums shrink-0">
                  <div className="text-[10.5px] text-[var(--text-muted)]">钱包余额</div>
                  <div className="text-[16px] font-semibold text-[var(--text-primary)] mt-0.5">
                    ${remainUsd.toFixed(2)}
                  </div>
                  {usedUsd !== null && usedUsd !== undefined && (
                    <div className="text-[10.5px] text-[var(--text-muted)]">
                      累计已用 ${usedUsd.toFixed(2)}
                    </div>
                  )}
                </div>
              )}

              <div className="flex items-center gap-2 shrink-0">
                <button
                  type="button"
                  onClick={onRefreshAccount}
                  className="h-8 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] text-[var(--text-secondary)] hover:text-[var(--text-primary)] inline-flex items-center gap-1.5 cursor-pointer"
                >
                  <RefreshCw className="h-3.5 w-3.5" />
                  <span>刷新</span>
                </button>
                <button
                  type="button"
                  onClick={onLogout}
                  className="h-8 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--danger-soft)] text-[12px] text-[var(--danger)] inline-flex items-center gap-1.5 cursor-pointer"
                >
                  <LogOut className="h-3.5 w-3.5" />
                  <span>退出登录</span>
                </button>
              </div>
            </div>

            {hasSubQuota && (
              <div className="pt-3 border-t border-[var(--border)] grid grid-cols-2 gap-4 text-[11.5px]">
                <div>
                  <div className="flex items-center justify-between text-[var(--text-secondary)] tabular-nums mb-1">
                    <span>5 小时窗口配额</span>
                    <span className="font-semibold text-[var(--text-primary)]">{pct5h}%</span>
                  </div>
                  <div className="h-1.5 rounded-full bg-[var(--bg-subtle)] overflow-hidden">
                    <div
                      className="h-full rounded-full bg-[var(--accent)]"
                      style={{ width: `${Math.max(2, pct5h)}%` }}
                    />
                  </div>
                </div>
                <div>
                  <div className="flex items-center justify-between text-[var(--text-secondary)] tabular-nums mb-1">
                    <span>7 天周期配额</span>
                    <span className="font-semibold text-[var(--text-primary)]">{pct7d}%</span>
                  </div>
                  <div className="h-1.5 rounded-full bg-[var(--bg-subtle)] overflow-hidden">
                    <div
                      className="h-full rounded-full bg-[var(--accent)]"
                      style={{ width: `${Math.max(2, pct7d)}%` }}
                    />
                  </div>
                </div>
              </div>
            )}
          </div>
        ) : (
          <div className="flex flex-wrap items-center justify-between gap-4">
            <div className="min-w-0 flex-1">
              <div className="text-[14px] font-semibold text-[var(--text-primary)]">
                尚未登录 Tako 账号
              </div>
              <p className="text-[12px] text-[var(--text-muted)] mt-0.5">
                登录后统一同步全部客户端与 MCP 能力；无需登录也可先浏览模型广场。
              </p>
            </div>
            <div className="flex items-center gap-2 shrink-0">
              <button
                type="button"
                onClick={onOpenKeyModal}
                className="h-8 px-3 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-secondary)] inline-flex items-center gap-1.5 cursor-pointer"
              >
                <KeyRound className="h-3.5 w-3.5" />
                <span>手动填入 Key</span>
              </button>
              <button
                type="button"
                disabled={loggingIn}
                onClick={onOAuthLogin}
                className="h-8 px-3.5 rounded-[8px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-semibold cursor-pointer disabled:opacity-60"
              >
                {loggingIn ? "登录中..." : "OAuth 一键登录"}
              </button>
            </div>
          </div>
        )}
      </section>

      {/* Two-Column Global Cards (stacks cleanly on narrow windows) */}
      <div className="grid grid-cols-1 min-[700px]:grid-cols-2 gap-3.5">
        {/* Card 1: Gateway & Network */}
        <section className="rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] overflow-hidden flex flex-col">
          <div className="px-4 py-3 border-b border-[var(--border)]">
            <h2 className="text-[13.5px] font-semibold text-[var(--text-primary)]">
              网关与网络
            </h2>
          </div>
          <div className="divide-y divide-[var(--border)] px-4 flex-1">
            <div className="py-3 flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[12.5px] font-medium text-[var(--text-primary)]">
                  本地统一网关
                </div>
                <div className="text-[11px] font-mono text-[var(--text-muted)] mt-0.5 truncate">
                  {dashboard?.localGatewayUrl ?? "http://127.0.0.1:18419"}
                </div>
              </div>
              {!anyTakoMode ? (
                <StatusPill tone="neutral">待机</StatusPill>
              ) : dashboard?.localGatewayHealthy ? (
                <StatusPill tone="ok">运行中</StatusPill>
              ) : (
                <button
                  type="button"
                  disabled={fixingGateway}
                  onClick={onEnsureGateway}
                  className="h-7 px-2.5 rounded-[7px] bg-[var(--warning-soft)] text-[var(--warning)] text-[11.5px] font-semibold cursor-pointer disabled:opacity-50 shrink-0"
                >
                  {fixingGateway ? "修复中..." : "立即启动"}
                </button>
              )}
            </div>

            <div className="py-3 flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[12.5px] font-medium text-[var(--text-primary)]">
                  直连 Tako 网关（绕过系统代理）
                </div>
                <div className="text-[11px] text-[var(--text-muted)] mt-0.5 leading-snug">
                  避免全局代理拖慢大请求上传与流式输出。
                </div>
              </div>
              <ToggleSwitch
                checked={dashboard?.state.bypassSystemProxy ?? true}
                disabled={savingSettings || !dashboard}
                ariaLabel="直连 Tako 网关（绕过系统代理）"
                data-testid="settings-bypass-system-proxy-toggle"
                onChange={(next) =>
                  void onSavePatch(
                    { bypassSystemProxy: next },
                    next ? "已开启绕过系统代理直连网关" : "已切换为遵循系统代理",
                  )
                }
              />
            </div>

            <div className="py-3 flex flex-col gap-1.5">
              <div className="text-[12.5px] font-medium text-[var(--text-primary)]">
                网关显示名称
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  maxLength={60}
                  value={gatewayDisplayName}
                  placeholder="Tako"
                  onChange={(e) => setGatewayDisplayName(e.target.value)}
                  className="flex-1 h-8 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-canvas)] text-[12px] text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
                />
                <button
                  type="button"
                  disabled={!nameDirty || savingSettings}
                  onClick={() =>
                    void onSavePatch(
                      { gatewayDisplayName: gatewayDisplayName.trim() || "Tako" },
                      "网关显示名称已保存",
                    )
                  }
                  className="h-8 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[11.5px] font-medium text-[var(--text-primary)] cursor-pointer disabled:opacity-45 shrink-0"
                >
                  保存
                </button>
              </div>
            </div>
          </div>
        </section>

        {/* Card 2: Update Strategy */}
        <section className="rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] overflow-hidden flex flex-col">
          <div className="px-4 py-3 border-b border-[var(--border)] flex items-center justify-between gap-2">
            <h2 className="text-[13.5px] font-semibold text-[var(--text-primary)]">
              更新策略
            </h2>
            {lastCheckedAt && (
              <span className="text-[11px] text-[var(--text-muted)] tabular-nums">
                上次检查 {lastCheckedAt}
              </span>
            )}
          </div>
          <div className="divide-y divide-[var(--border)] px-4 flex-1">
            <div className="py-3 flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[12.5px] font-medium text-[var(--text-primary)]">
                  启动时自动检查更新
                </div>
                <div className="text-[11px] text-[var(--text-muted)] mt-0.5">
                  同时检查 Tako Switch 与已安装桌面端版本。
                </div>
              </div>
              <ToggleSwitch
                checked={dashboard?.state.autoCheckUpdates ?? true}
                disabled={savingSettings || !dashboard}
                ariaLabel="启动时自动检查更新"
                onChange={(next) =>
                  void onSavePatch(
                    { autoCheckUpdates: next },
                    next ? "已开启启动自动检查更新" : "已关闭启动自动检查更新",
                  )
                }
              />
            </div>

            <div className="py-3 flex flex-col gap-2">
              <div className="flex items-center justify-between gap-2">
                <div className="min-w-0">
                  <div className="flex items-center gap-1.5 flex-wrap">
                    <span className="text-[12.5px] font-medium text-[var(--text-primary)]">
                      Tako Switch v{dashboard?.appVersion ?? "2.2.1"}
                    </span>
                    <button
                      type="button"
                      onClick={onOpenChangelog}
                      className="inline-flex items-center gap-0.5 text-[11px] text-[var(--accent)] hover:underline cursor-pointer"
                    >
                      <FileText className="h-3 w-3" />
                      <span>更新日志</span>
                    </button>
                  </div>
                  <div className="text-[11px] text-[var(--text-muted)] mt-0.5">
                    {selfUpdateView.status === "available" && selfUpdateView.version
                      ? `发现新版 v${selfUpdateView.version}`
                      : selfUpdateView.status === "latest"
                        ? "当前已是最新版本"
                        : selfUpdateView.error
                          ? selfUpdateView.error
                          : "支持国内加速源与 GitHub 自动回退"}
                  </div>
                </div>

                {selfUpdateView.status === "downloading" ||
                selfUpdateView.status === "installing" ? (
                  <span className="text-[11.5px] font-semibold text-[var(--accent)] tabular-nums shrink-0">
                    {selfUpdateView.status === "installing"
                      ? "安装中..."
                      : `${selfUpdateView.percent ?? 0}%`}
                  </span>
                ) : selfUpdateView.status === "available" ||
                  selfUpdateView.status === "error" ? (
                  <button
                    type="button"
                    onClick={onInstallSelfUpdate}
                    className="h-7 px-2.5 rounded-[7px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[11.5px] font-medium cursor-pointer shrink-0"
                  >
                    {selfUpdateView.status === "error" ? "重试" : "立即更新"}
                  </button>
                ) : (
                  <button
                    type="button"
                    disabled={checkingUpdates}
                    onClick={onCheckUpdates}
                    className="h-7 px-2.5 rounded-[7px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[11.5px] font-medium text-[var(--text-primary)] cursor-pointer disabled:opacity-50 shrink-0"
                  >
                    {checkingUpdates ? "检查中..." : "检查更新"}
                  </button>
                )}
              </div>

              {(selfUpdateView.status === "downloading" ||
                selfUpdateView.status === "installing") && (
                <div className="h-1 w-full rounded-full bg-[var(--bg-subtle)] overflow-hidden">
                  <div
                    className="h-full rounded-full bg-[var(--accent)] transition-all duration-150"
                    style={{ width: `${Math.max(4, selfUpdateView.percent ?? 4)}%` }}
                  />
                </div>
              )}
            </div>

            <div className="py-3 flex items-center justify-between gap-2">
              <div className="min-w-0">
                <div className="text-[12.5px] font-medium text-[var(--text-primary)]">
                  桌面客户端集中更新
                </div>
                <div className="text-[11px] text-[var(--text-muted)] truncate mt-0.5">
                  {updatableApps.length > 0
                    ? `${updatableApps.map((a) => APP_DISPLAY_NAMES[a.id]).join(" · ")} 可升级`
                    : "已安装客户端均为最新版本"}
                </div>
              </div>
              {updatableApps.length > 0 ? (
                <button
                  type="button"
                  disabled={batchUpdatingApps || busyApp !== null}
                  onClick={onBatchUpgradeApps}
                  className="h-7 px-2.5 rounded-[7px] bg-[var(--accent-soft)] text-[var(--accent)] text-[11.5px] font-semibold cursor-pointer disabled:opacity-50 shrink-0"
                >
                  {batchUpdatingApps ? "更新中..." : `全部更新 (${updatableApps.length})`}
                </button>
              ) : (
                <StatusPill tone="ok">最新</StatusPill>
              )}
            </div>
          </div>
        </section>

        {/* Card 3: Appearance & Behavior */}
        <section className="rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] overflow-hidden flex flex-col">
          <div className="px-4 py-3 border-b border-[var(--border)]">
            <h2 className="text-[13.5px] font-semibold text-[var(--text-primary)]">
              外观与行为
            </h2>
          </div>
          <div className="divide-y divide-[var(--border)] px-4 flex-1">
            <div className="py-3 flex flex-wrap items-center justify-between gap-3">
              <div className="text-[12.5px] font-medium text-[var(--text-primary)]">
                界面主题
              </div>
              <SegmentedControl
                options={THEME_OPTIONS}
                value={themePref}
                onChange={onChangeTheme}
              />
            </div>

            <div className="py-3 flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[12.5px] font-medium text-[var(--text-primary)]">
                  菜单栏显示实时余额
                </div>
                <div className="text-[11px] text-[var(--text-muted)] mt-0.5">
                  在系统状态栏轻量化展示当前余额。
                </div>
              </div>
              <ToggleSwitch
                checked={dashboard?.state.showTrayBalance ?? true}
                disabled={savingSettings || !dashboard}
                ariaLabel="菜单栏显示实时余额"
                onChange={(next) =>
                  void onSavePatch(
                    { showTrayBalance: next },
                    next ? "已开启菜单栏实时余额" : "已关闭菜单栏实时余额",
                  )
                }
              />
            </div>

            <div className="py-3 flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[12.5px] font-medium text-[var(--text-primary)]">
                  客户端优先简体中文
                </div>
                <div className="text-[11px] text-[var(--text-muted)] mt-0.5 leading-snug">
                  Codex / Claude 界面与中文回复 · 反重力系统菜单与中文交互
                </div>
              </div>
              <ToggleSwitch
                checked={dashboard?.state.preferChineseUi ?? true}
                disabled={savingSettings || !dashboard}
                ariaLabel="客户端优先简体中文"
                onChange={(next) =>
                  void onSavePatch(
                    { preferChineseUi: next },
                    next ? "已开启客户端简体中文偏好" : "已关闭客户端简体中文偏好",
                  )
                }
              />
            </div>
          </div>
        </section>

        {/* Card 4: MCP & Output */}
        <section className="rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] overflow-hidden flex flex-col">
          <div className="px-4 py-3 border-b border-[var(--border)]">
            <h2 className="text-[13.5px] font-semibold text-[var(--text-primary)]">
              MCP 与输出目录
            </h2>
          </div>
          <div className="divide-y divide-[var(--border)] px-4 flex-1">
            <div className="py-3 flex items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[12.5px] font-medium text-[var(--text-primary)]">
                  全局 MCP 能力
                </div>
                <div className="text-[11px] text-[var(--text-muted)] mt-0.5">
                  当前已开启 {mcpEnabledCount} / {mcpTotalCount} 项能力，自动同步至 Tako 模式客户端。
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

            <div className="py-3 flex flex-col gap-1.5">
              <div className="text-[12.5px] font-medium text-[var(--text-primary)]">
                媒体文件输出目录
              </div>
              <div className="text-[11px] text-[var(--text-muted)]">
                生图、视频与音频默认落盘位置（留空使用系统下载目录）。
              </div>
              <div className="flex items-center gap-2 mt-1">
                <input
                  type="text"
                  value={outputDir}
                  placeholder="~/Downloads/Tako"
                  onChange={(e) => setOutputDir(e.target.value)}
                  className="flex-1 h-8 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-canvas)] text-[12px] font-mono text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
                />
                <button
                  type="button"
                  disabled={!outputDirDirty || savingSettings || !dashboard}
                  onClick={() => {
                    if (!dashboard) return;
                    void onSavePatch(
                      { mcp: { ...dashboard.state.mcp, outputDir: outputDir.trim() } },
                      "媒体输出目录已保存",
                    );
                  }}
                  className="h-8 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[11.5px] font-medium text-[var(--text-primary)] cursor-pointer disabled:opacity-45 shrink-0"
                >
                  保存
                </button>
              </div>
            </div>
          </div>
        </section>
      </div>

      {/* Collapsible Advanced Configuration */}
      <div>
        <button
          type="button"
          onClick={() => setAdvancedOpen((prev) => !prev)}
          className="w-full h-10 rounded-[10px] border border-dashed border-[var(--border-strong)] hover:border-[var(--accent)]/60 text-[12px] text-[var(--text-secondary)] hover:text-[var(--text-primary)] inline-flex items-center justify-center gap-1.5 transition cursor-pointer"
        >
          <ChevronDown
            className={`h-3.5 w-3.5 transition-transform ${advancedOpen ? "rotate-180" : ""}`}
          />
          <span>
            {advancedOpen
              ? "收起高级配置"
              : "展开高级配置（服务端地址、Manifest 源与反馈）"}
          </span>
        </button>

        {advancedOpen && (
          <div className="mt-3 rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] p-4 flex flex-col gap-3.5">
            <div className="flex flex-col gap-1.5">
              <label className="text-[12px] font-medium text-[var(--text-primary)]">
                Tako 服务端地址 (baseUrl)
              </label>
              <input
                type="url"
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                placeholder="https://api.tako.example"
                className="h-[34px] px-3 rounded-[8px] border border-[var(--border)] bg-[var(--bg-canvas)] text-[12px] font-mono text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
              />
            </div>

            <div className="flex flex-col gap-1.5">
              <div className="flex items-center justify-between">
                <label className="text-[12px] font-medium text-[var(--text-primary)]">
                  客户端 Manifest 更新源
                </label>
                <button
                  type="button"
                  onClick={() => {
                    setManifestUrl(DEFAULT_MANIFEST_URL);
                    setFallbackManifestUrl(DEFAULT_FALLBACK_MANIFEST_URL);
                  }}
                  className="text-[11px] text-[var(--accent)] hover:underline cursor-pointer"
                >
                  恢复默认源
                </button>
              </div>
              <input
                type="url"
                value={manifestUrl}
                onChange={(e) => setManifestUrl(e.target.value)}
                placeholder="主更新源 URL"
                className="h-[34px] px-3 rounded-[8px] border border-[var(--border)] bg-[var(--bg-canvas)] text-[11.5px] font-mono text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
              />
              <input
                type="url"
                value={fallbackManifestUrl}
                onChange={(e) => setFallbackManifestUrl(e.target.value)}
                placeholder="备用更新源 URL"
                className="h-[34px] px-3 rounded-[8px] border border-[var(--border)] bg-[var(--bg-canvas)] text-[11.5px] font-mono text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
              />
            </div>

            <div className="pt-2 border-t border-[var(--border)] flex flex-wrap items-center justify-between gap-2">
              <div className="flex items-center gap-4 text-[12px]">
                <button
                  type="button"
                  onClick={() => void api.openUrl(TAKO_FEEDBACK_URL)}
                  className="inline-flex items-center gap-1.5 text-[var(--text-secondary)] hover:text-[var(--text-primary)] cursor-pointer"
                >
                  <MessageSquare className="h-3.5 w-3.5" />
                  <span>GitHub 问题反馈</span>
                </button>
                <button
                  type="button"
                  onClick={() => void api.openUrl(TAKO_RELEASES_URL)}
                  className="inline-flex items-center gap-1.5 text-[var(--text-secondary)] hover:text-[var(--text-primary)] cursor-pointer"
                >
                  <ExternalLink className="h-3.5 w-3.5" />
                  <span>下载页 Releases</span>
                </button>
              </div>

              <button
                type="button"
                disabled={!advancedDirty || savingSettings}
                onClick={() =>
                  void onSavePatch(
                    {
                      baseUrl: baseUrl.trim(),
                      manifestUrl: manifestUrl.trim(),
                      fallbackManifestUrl: fallbackManifestUrl.trim(),
                    },
                    "高级连接配置已保存",
                  )
                }
                className="h-8 px-3.5 rounded-[8px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-semibold cursor-pointer disabled:opacity-45"
              >
                {savingSettings ? "保存中..." : "保存高级配置"}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
