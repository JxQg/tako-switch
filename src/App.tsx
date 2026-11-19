import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import {
  Activity,
  Download,
  ExternalLink,
  KeyRound,
  LogIn,
  LogOut,
  RefreshCw,
  RotateCw,
  Settings,
  Sparkles,
  X,
} from "lucide-react";
import {
  api,
  type AppId,
  type AppMode,
  type AppSnapshot,
  type AppUpdateStatus,
  type DashboardPayload,
  type DiagnosticResult,
  type InstallProgressEvent,
  type McpToolsConfig,
} from "./lib/api";

const MODES: { id: AppMode; label: string; tip: string }[] = [
  { id: "official", label: "官方", tip: "纯官方账号与直连端点，无损恢复原始配置" },
  { id: "hybrid", label: "聚合", tip: "保留官方订阅主模型，注入 Tako 超级 MCP 工具" },
  { id: "tako", label: "Tako", tip: "主模型与 MCP 全量走 Tako 企业网关" },
];

const MCP_TOOLS: { key: keyof Pick<McpToolsConfig, "image" | "video" | "audio" | "search" | "consult">; label: string }[] = [
  { key: "image", label: "生图" },
  { key: "video", label: "生视频" },
  { key: "audio", label: "音频" },
  { key: "search", label: "联网搜索" },
  { key: "consult", label: "外脑顾问" },
];

function QuotaPill({
  label,
  used,
  limit,
}: {
  label: string;
  used: number;
  limit: number;
}) {
  if (limit <= 0) return null;
  const pct = Math.min(100, Math.max(0, Math.round((used / limit) * 100)));
  const barColor =
    pct >= 90 ? "bg-rose-500" : pct >= 70 ? "bg-amber-500" : "bg-emerald-500";
  return (
    <div className="flex items-center gap-1.5 text-[11px] text-slate-600 dark:text-slate-300">
      <span className="font-medium text-slate-500 dark:text-slate-400">{label}</span>
      <div className="h-1.5 w-12 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-700">
        <div className={`h-full ${barColor}`} style={{ width: `${pct}%` }} />
      </div>
      <span className="tabular-nums">{pct}%</span>
    </div>
  );
}

export default function App() {
  const [dashboard, setDashboard] = useState<DashboardPayload | null>(null);
  const [updates, setUpdates] = useState<Record<AppId, AppUpdateStatus | undefined>>({
    codex: undefined,
    claude: undefined,
    antigravity: undefined,
  });
  const [busyApp, setBusyApp] = useState<AppId | null>(null);
  const [progressByApp, setProgressByApp] = useState<
    Partial<Record<AppId, InstallProgressEvent>>
  >({});
  const [authBusy, setAuthBusy] = useState(false);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const [selfUpdate, setSelfUpdate] = useState<Update | null>(null);
  const [selfUpdating, setSelfUpdating] = useState(false);

  const [banner, setBanner] = useState<{ kind: "err" | "ok"; text: string } | null>(
    null
  );
  const [showKeyModal, setShowKeyModal] = useState(false);
  const [keyInput, setKeyInput] = useState("");
  const [showSettingsModal, setShowSettingsModal] = useState(false);
  const [diagModal, setDiagModal] = useState<{
    name: string;
    hasBackup: boolean;
    result: DiagnosticResult;
  } | null>(null);

  const [settingsForm, setSettingsForm] = useState({
    baseUrl: "",
    agyApiKey: "",
    manifestUrl: "",
    fallbackManifestUrl: "",
    autoCheckUpdates: true,
    outputDir: "",
    consultModel: "",
  });

  const loadDashboard = useCallback(async (refreshAccount: boolean) => {
    try {
      const data = await api.getDashboard(refreshAccount);
      setDashboard(data);
      setSettingsForm({
        baseUrl: data.state.baseUrl,
        agyApiKey: data.state.agyApiKey ?? "",
        manifestUrl: data.state.manifestUrl,
        fallbackManifestUrl: data.state.fallbackManifestUrl,
        autoCheckUpdates: data.state.autoCheckUpdates,
        outputDir: data.state.mcp.outputDir,
        consultModel: data.state.mcp.consultModel,
      });
    } catch (err) {
      setBanner({ kind: "err", text: String(err) });
    }
  }, []);

  const runUpdateChecks = useCallback(async () => {
    setCheckingUpdates(true);
    try {
      const [manifestRes, appUpd] = await Promise.allSettled([
        api.checkAppUpdates(),
        check(),
      ]);
      if (manifestRes.status === "fulfilled") {
        const next: Record<AppId, AppUpdateStatus | undefined> = {
          codex: undefined,
          claude: undefined,
          antigravity: undefined,
        };
        for (const u of manifestRes.value.updates) {
          next[u.appId] = u;
        }
        setUpdates(next);
      }
      if (appUpd.status === "fulfilled" && appUpd.value) {
        setSelfUpdate(appUpd.value);
      }
    } finally {
      setCheckingUpdates(false);
    }
  }, []);

  useEffect(() => {
    void loadDashboard(true);
    void runUpdateChecks();

    const unlistenProgress = listen<InstallProgressEvent>("install-progress", (e) => {
      setProgressByApp((prev) => ({ ...prev, [e.payload.appId]: e.payload }));
    });
    const unlistenState = listen("state-changed", () => {
      void loadDashboard(false);
    });
    const unlistenUpdates = listen("request-check-updates", () => {
      void runUpdateChecks();
    });

    return () => {
      void unlistenProgress.then((fn) => fn());
      void unlistenState.then((fn) => fn());
      void unlistenUpdates.then((fn) => fn());
    };
  }, [loadDashboard, runUpdateChecks]);

  const handleOAuthLogin = async () => {
    setAuthBusy(true);
    setBanner(null);
    try {
      const account = await api.loginOAuth();
      setDashboard((prev) => (prev ? { ...prev, account } : prev));
      setBanner({ kind: "ok", text: "Tako 账号登录成功" });
    } catch (err) {
      setBanner({ kind: "err", text: String(err) });
    } finally {
      setAuthBusy(false);
    }
  };

  const handleKeySubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!keyInput.trim()) return;
    setAuthBusy(true);
    setBanner(null);
    try {
      const account = await api.loginWithKey(keyInput.trim());
      setDashboard((prev) => (prev ? { ...prev, account } : prev));
      setKeyInput("");
      setShowKeyModal(false);
      setBanner({ kind: "ok", text: "Key 验证成功" });
    } catch (err) {
      setBanner({ kind: "err", text: String(err) });
    } finally {
      setAuthBusy(false);
    }
  };

  const handleLogout = async () => {
    setAuthBusy(true);
    try {
      const account = await api.logoutAccount();
      setDashboard((prev) => (prev ? { ...prev, account } : prev));
    } catch (err) {
      setBanner({ kind: "err", text: String(err) });
    } finally {
      setAuthBusy(false);
    }
  };

  const handleModeSwitch = async (appId: AppId, mode: AppMode) => {
    setBusyApp(appId);
    setBanner(null);
    try {
      const updated = await api.setAppMode(appId, mode, false);
      setDashboard((prev) =>
        prev
          ? {
              ...prev,
              apps: prev.apps.map((a) => (a.id === appId ? updated : a)),
            }
          : prev
      );
      if (updated.running) {
        setBanner({
          kind: "ok",
          text: `${updated.name} 配置已切换，点击“重启生效”使其应用新模式`,
        });
      }
    } catch (err) {
      setBanner({ kind: "err", text: String(err) });
    } finally {
      setBusyApp(null);
    }
  };

  const handleRestartOrLaunch = async (snap: AppSnapshot) => {
    setBusyApp(snap.id);
    setBanner(null);
    try {
      const updated = snap.running
        ? await api.restartApp(snap.id)
        : await api.launchApp(snap.id);
      setDashboard((prev) =>
        prev
          ? {
              ...prev,
              apps: prev.apps.map((a) => (a.id === snap.id ? updated : a)),
            }
          : prev
      );
    } catch (err) {
      setBanner({ kind: "err", text: String(err) });
    } finally {
      setBusyApp(null);
    }
  };

  const handleInstallOrUpgrade = async (appId: AppId) => {
    setBusyApp(appId);
    setBanner(null);
    try {
      const outcome = await api.installOrUpgradeApp(appId);
      setDashboard((prev) =>
        prev
          ? {
              ...prev,
              apps: prev.apps.map((a) => (a.id === appId ? outcome.snapshot : a)),
            }
          : prev
      );
      setUpdates((prev) => ({
        ...prev,
        [appId]: prev[appId] ? { ...prev[appId]!, hasUpdate: false, installed: true } : undefined,
      }));
      setBanner({
        kind: "ok",
        text: `${outcome.snapshot.name} 安装/更新完成`,
      });
    } catch (err) {
      setBanner({ kind: "err", text: String(err) });
    } finally {
      setBusyApp(null);
      setProgressByApp((prev) => {
        const next = { ...prev };
        delete next[appId];
        return next;
      });
    }
  };

  const handleToggleMcpTool = async (
    toolKey: keyof Pick<McpToolsConfig, "image" | "video" | "audio" | "search" | "consult">
  ) => {
    if (!dashboard) return;
    const nextMcp: McpToolsConfig = {
      ...dashboard.state.mcp,
      [toolKey]: !dashboard.state.mcp[toolKey],
    };
    try {
      const nextState = await api.saveSettings({ mcp: nextMcp });
      setDashboard((prev) => (prev ? { ...prev, state: nextState } : prev));
    } catch (err) {
      setBanner({ kind: "err", text: String(err) });
    }
  };

  const handleSaveSettings = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!dashboard) return;
    try {
      const nextState = await api.saveSettings({
        baseUrl: settingsForm.baseUrl,
        agyApiKey: settingsForm.agyApiKey.trim(),
        manifestUrl: settingsForm.manifestUrl,
        fallbackManifestUrl: settingsForm.fallbackManifestUrl,
        autoCheckUpdates: settingsForm.autoCheckUpdates,
        mcp: {
          ...dashboard.state.mcp,
          outputDir: settingsForm.outputDir,
          consultModel: settingsForm.consultModel,
        },
      });
      setDashboard((prev) => (prev ? { ...prev, state: nextState } : prev));
      setShowSettingsModal(false);
      setBanner({ kind: "ok", text: "设置已保存" });
    } catch (err) {
      setBanner({ kind: "err", text: String(err) });
    }
  };

  const handleSelfUpdate = async () => {
    if (!selfUpdate) return;
    setSelfUpdating(true);
    try {
      await selfUpdate.downloadAndInstall();
      await relaunch();
    } catch (err) {
      setBanner({ kind: "err", text: `应用自更新失败: ${String(err)}` });
      setSelfUpdating(false);
    }
  };

  const handleDiagnose = async (snap: AppSnapshot) => {
    try {
      const result = await api.diagnoseApp(snap.id);
      setDiagModal({ name: snap.name, hasBackup: snap.hasBackup, result });
    } catch (err) {
      setBanner({ kind: "err", text: String(err) });
    }
  };

  const acct = dashboard?.account;

  return (
    <div className="flex h-screen flex-col justify-between bg-slate-50 px-4 py-3 text-slate-900 dark:bg-slate-950 dark:text-slate-100">
      {/* Header */}
      <header className="flex items-center justify-between gap-2 border-b border-slate-200/80 pb-2.5 dark:border-slate-800">
        <div className="flex items-center gap-2">
          <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-gradient-to-br from-sky-500 to-cyan-500 text-xs font-bold text-white shadow-sm">
            T
          </div>
          <div>
            <div className="flex items-center gap-1.5">
              <span className="text-sm font-semibold tracking-tight">Tako Switch</span>
              <span className="rounded bg-slate-200/80 px-1.5 py-0.5 text-[10px] font-medium text-slate-600 dark:bg-slate-800 dark:text-slate-400">
                v{dashboard?.appVersion ?? "2.0.0"}
              </span>
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {acct?.loggedIn ? (
            <div className="flex items-center gap-2.5 rounded-lg border border-slate-200 bg-white px-2.5 py-1 shadow-xs dark:border-slate-800 dark:bg-slate-900">
              <div className="flex flex-col leading-tight">
                <span className="text-xs font-medium">
                  {acct.userName || acct.maskedKey || "已登录"}
                </span>
                {acct.planName && (
                  <span className="text-[10px] text-sky-600 dark:text-sky-400">
                    {acct.planName}
                  </span>
                )}
              </div>
              {acct.quota && (
                <div className="flex items-center gap-2 border-l border-slate-200 pl-2 dark:border-slate-700">
                  <QuotaPill
                    label="5h"
                    used={acct.quota.windowUsed}
                    limit={acct.quota.windowCostLimit}
                  />
                  <QuotaPill
                    label="7d"
                    used={acct.quota.weeklyUsed}
                    limit={acct.quota.weeklyCostLimit}
                  />
                </div>
              )}
              <button
                onClick={handleLogout}
                disabled={authBusy}
                title="退出登录"
                className="rounded p-1 text-slate-400 transition hover:bg-slate-100 hover:text-rose-500 dark:hover:bg-slate-800"
              >
                <LogOut className="h-3.5 w-3.5" />
              </button>
            </div>
          ) : (
            <div className="flex items-center gap-1.5">
              <button
                onClick={() => setShowKeyModal(true)}
                disabled={authBusy}
                className="flex items-center gap-1 rounded-md border border-slate-200 bg-white px-2.5 py-1 text-xs font-medium text-slate-700 transition hover:bg-slate-100 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200 dark:hover:bg-slate-800"
              >
                <KeyRound className="h-3.5 w-3.5" />
                填入 Key
              </button>
              <button
                onClick={handleOAuthLogin}
                disabled={authBusy}
                className="flex items-center gap-1 rounded-md bg-sky-600 px-2.5 py-1 text-xs font-medium text-white shadow-xs transition hover:bg-sky-500 disabled:opacity-60"
              >
                <LogIn className="h-3.5 w-3.5" />
                {authBusy ? "授权中…" : "登录 Tako"}
              </button>
            </div>
          )}

          <button
            onClick={runUpdateChecks}
            disabled={checkingUpdates}
            title="检查应用更新"
            className="rounded-md border border-slate-200 bg-white p-1.5 text-slate-600 transition hover:bg-slate-100 dark:border-slate-800 dark:bg-slate-900 dark:text-slate-300 dark:hover:bg-slate-800"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${checkingUpdates ? "animate-spin" : ""}`} />
          </button>
          <button
            onClick={() => setShowSettingsModal(true)}
            title="高级设置"
            className="rounded-md border border-slate-200 bg-white p-1.5 text-slate-600 transition hover:bg-slate-100 dark:border-slate-800 dark:bg-slate-900 dark:text-slate-300 dark:hover:bg-slate-800"
          >
            <Settings className="h-3.5 w-3.5" />
          </button>
        </div>
      </header>

      {/* Self-update or status banner */}
      {selfUpdate && (
        <div className="mt-2 flex items-center justify-between rounded-lg border border-sky-200 bg-sky-50 px-3 py-1.5 text-xs text-sky-900 dark:border-sky-800/70 dark:bg-sky-950/60 dark:text-sky-200">
          <span>
            发现新版本 <strong>v{selfUpdate.version}</strong> 可用
          </span>
          <button
            onClick={handleSelfUpdate}
            disabled={selfUpdating}
            className="rounded bg-sky-600 px-2.5 py-0.5 font-medium text-white hover:bg-sky-500 disabled:opacity-60"
          >
            {selfUpdating ? "更新中…" : "立即更新并重启"}
          </button>
        </div>
      )}

      {banner && (
        <div
          className={`mt-2 flex items-center justify-between rounded-lg border px-3 py-1.5 text-xs ${
            banner.kind === "err"
              ? "border-rose-200 bg-rose-50 text-rose-800 dark:border-rose-800/70 dark:bg-rose-950/50 dark:text-rose-200"
              : "border-emerald-200 bg-emerald-50 text-emerald-800 dark:border-emerald-800/70 dark:bg-emerald-950/50 dark:text-emerald-200"
          }`}
        >
          <span className="truncate pr-2">{banner.text}</span>
          <button onClick={() => setBanner(null)} className="shrink-0 opacity-70 hover:opacity-100">
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      )}

      {/* Main 3 Desktop App Cards */}
      <main className="my-2.5 flex flex-1 flex-col justify-center gap-2.5">
        {(dashboard?.apps ?? []).map((snap) => {
          const upd = updates[snap.id];
          const progress = progressByApp[snap.id];
          const isBusy = busyApp === snap.id;

          return (
            <section
              key={snap.id}
              className="rounded-xl border border-slate-200/90 bg-white px-3.5 py-2.5 shadow-xs transition dark:border-slate-800 dark:bg-slate-900/90"
            >
              <div className="flex items-center justify-between gap-3">
                {/* Left metadata */}
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <h2 className="text-sm font-semibold">{snap.name}</h2>
                    {snap.install.installed ? (
                      <span className="rounded bg-slate-100 px-1.5 py-0.5 text-[10px] font-medium text-slate-600 dark:bg-slate-800 dark:text-slate-300">
                        {snap.install.version ? `v${snap.install.version}` : "已安装"}
                      </span>
                    ) : (
                      <span className="rounded bg-amber-100 px-1.5 py-0.5 text-[10px] font-medium text-amber-800 dark:bg-amber-900/50 dark:text-amber-200">
                        未安装
                      </span>
                    )}
                    {snap.running && (
                      <span className="flex items-center gap-1 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
                        <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
                        运行中
                      </span>
                    )}
                  </div>

                  <div className="mt-1 flex items-center gap-2 text-[11px] text-slate-500 dark:text-slate-400">
                    <span>
                      MCP:{" "}
                      <strong className="font-medium text-slate-700 dark:text-slate-200">
                        {snap.mcpEnabled ? "已注入" : "未开启"}
                      </strong>
                    </span>
                    <span>·</span>
                    <button
                      onClick={() => handleDiagnose(snap)}
                      className="inline-flex items-center gap-0.5 text-slate-500 underline-offset-2 hover:text-sky-600 hover:underline dark:text-slate-400 dark:hover:text-sky-400"
                    >
                      <Activity className="h-3 w-3" />
                      {snap.diagnostic.ok ? "配置正常" : "需检查"}
                    </button>
                  </div>
                </div>

                {/* Right controls: 3-mode segmented pill + action buttons */}
                <div className="flex shrink-0 items-center gap-2">
                  <div className="inline-flex rounded-lg bg-slate-100 p-0.5 dark:bg-slate-800">
                    {MODES.map((m) => {
                      const active = snap.mode === m.id;
                      return (
                        <button
                          key={m.id}
                          title={m.tip}
                          disabled={isBusy}
                          onClick={() => handleModeSwitch(snap.id, m.id)}
                          className={`rounded-md px-2.5 py-1 text-xs font-medium transition ${
                            active
                              ? m.id === "tako"
                                ? "bg-sky-600 text-white shadow-xs"
                                : m.id === "hybrid"
                                  ? "bg-white text-sky-700 shadow-xs dark:bg-slate-700 dark:text-sky-300"
                                  : "bg-white text-slate-900 shadow-xs dark:bg-slate-700 dark:text-white"
                              : "text-slate-600 hover:text-slate-900 dark:text-slate-400 dark:hover:text-slate-200"
                          }`}
                        >
                          {m.label}
                        </button>
                      );
                    })}
                  </div>

                  {/* Install / Upgrade */}
                  {!snap.install.installed && upd?.platformSupported && (
                    <button
                      disabled={isBusy}
                      onClick={() => handleInstallOrUpgrade(snap.id)}
                      className="inline-flex items-center gap-1 rounded-md bg-emerald-600 px-2.5 py-1 text-xs font-medium text-white transition hover:bg-emerald-500 disabled:opacity-60"
                    >
                      <Download className="h-3.5 w-3.5" />
                      一键安装
                    </button>
                  )}

                  {snap.install.installed && upd?.hasUpdate && upd.remoteVersion && (
                    <button
                      disabled={isBusy}
                      onClick={() => handleInstallOrUpgrade(snap.id)}
                      className="inline-flex items-center gap-1 rounded-md bg-amber-500 px-2 py-1 text-xs font-medium text-white transition hover:bg-amber-400 disabled:opacity-60"
                    >
                      <Download className="h-3 w-3" />
                      升级 v{upd.remoteVersion}
                    </button>
                  )}

                  {snap.install.installed && (
                    <button
                      disabled={isBusy}
                      onClick={() => handleRestartOrLaunch(snap)}
                      className="inline-flex items-center gap-1 rounded-md border border-slate-200 bg-slate-50 px-2 py-1 text-xs font-medium text-slate-700 transition hover:bg-slate-100 disabled:opacity-60 dark:border-slate-700 dark:bg-slate-800 dark:text-slate-200 dark:hover:bg-slate-700"
                    >
                      {snap.running ? (
                        <>
                          <RotateCw className="h-3 w-3" />
                          重启生效
                        </>
                      ) : (
                        <>
                          <ExternalLink className="h-3 w-3" />
                          打开
                        </>
                      )}
                    </button>
                  )}
                </div>
              </div>

              {progress && (
                <div className="mt-2">
                  <div className="mb-0.5 flex justify-between text-[10px] text-slate-500 dark:text-slate-400">
                    <span>{progress.message}</span>
                    <span className="tabular-nums">{progress.percent}%</span>
                  </div>
                  <div className="h-1.5 w-full overflow-hidden rounded-full bg-slate-200 dark:bg-slate-800">
                    <div
                      className="h-full bg-sky-500 transition-all duration-200"
                      style={{ width: `${progress.percent}%` }}
                    />
                  </div>
                </div>
              )}
            </section>
          );
        })}
      </main>

      {/* Bottom Compact Tako Super MCP Bar */}
      <footer className="rounded-xl border border-slate-200/90 bg-white px-3.5 py-2 shadow-xs dark:border-slate-800 dark:bg-slate-900/90">
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-1.5">
            <Sparkles className="h-3.5 w-3.5 text-sky-500" />
            <span className="text-xs font-semibold">Tako 超级 MCP</span>
            <span className="text-[11px] text-slate-400">
              “聚合 / Tako”自动接入，开关即时生效
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            {MCP_TOOLS.map((tool) => {
              const on = dashboard?.state.mcp[tool.key] ?? true;
              return (
                <button
                  key={tool.key}
                  onClick={() => handleToggleMcpTool(tool.key)}
                  className={`rounded-md border px-2 py-0.5 text-[11px] font-medium transition ${
                    on
                      ? "border-sky-200 bg-sky-50 text-sky-700 dark:border-sky-800 dark:bg-sky-950/70 dark:text-sky-300"
                      : "border-slate-200 bg-slate-50 text-slate-400 line-through dark:border-slate-800 dark:bg-slate-950 dark:text-slate-500"
                  }`}
                >
                  {tool.label}
                </button>
              );
            })}
          </div>
        </div>
      </footer>

      {/* Manual Key Modal */}
      {showKeyModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 backdrop-blur-xs">
          <form
            onSubmit={handleKeySubmit}
            className="w-full max-w-sm rounded-xl border border-slate-200 bg-white p-4 shadow-xl dark:border-slate-800 dark:bg-slate-900"
          >
            <div className="mb-3 flex items-center justify-between">
              <h3 className="text-sm font-semibold">手动填入 Tako Key</h3>
              <button
                type="button"
                onClick={() => setShowKeyModal(false)}
                className="text-slate-400 hover:text-slate-600"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <input
              type="password"
              value={keyInput}
              onChange={(e) => setKeyInput(e.target.value)}
              placeholder="cr_..."
              autoFocus
              className="w-full rounded-lg border border-slate-300 bg-slate-50 px-3 py-2 text-xs outline-none focus:border-sky-500 dark:border-slate-700 dark:bg-slate-950"
            />
            <div className="mt-3 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setShowKeyModal(false)}
                className="rounded-md px-3 py-1.5 text-xs text-slate-600 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800"
              >
                取消
              </button>
              <button
                type="submit"
                disabled={authBusy}
                className="rounded-md bg-sky-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-500 disabled:opacity-60"
              >
                {authBusy ? "验证中…" : "验证并保存"}
              </button>
            </div>
          </form>
        </div>
      )}

      {/* Settings Modal */}
      {showSettingsModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 backdrop-blur-xs">
          <form
            onSubmit={handleSaveSettings}
            className="w-full max-w-md rounded-xl border border-slate-200 bg-white p-4 shadow-xl dark:border-slate-800 dark:bg-slate-900"
          >
            <div className="mb-3 flex items-center justify-between">
              <h3 className="text-sm font-semibold">高级设置</h3>
              <button
                type="button"
                onClick={() => setShowSettingsModal(false)}
                className="text-slate-400 hover:text-slate-600"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="space-y-2.5 text-xs">
              <label className="block">
                <span className="mb-1 block font-medium text-slate-600 dark:text-slate-300">
                  Tako 网关地址
                </span>
                <input
                  value={settingsForm.baseUrl}
                  onChange={(e) =>
                    setSettingsForm((s) => ({ ...s, baseUrl: e.target.value }))
                  }
                  className="w-full rounded-lg border border-slate-300 bg-slate-50 px-2.5 py-1.5 outline-none focus:border-sky-500 dark:border-slate-700 dark:bg-slate-950"
                />
              </label>

              <label className="block">
                <span className="mb-1 block font-medium text-slate-600 dark:text-slate-300">
                  专用反重力 Key (留空则复用主 Key)
                </span>
                <input
                  type="password"
                  value={settingsForm.agyApiKey}
                  placeholder="可选，例如 cr_..."
                  onChange={(e) =>
                    setSettingsForm((s) => ({ ...s, agyApiKey: e.target.value }))
                  }
                  className="w-full rounded-lg border border-slate-300 bg-slate-50 px-2.5 py-1.5 outline-none focus:border-sky-500 dark:border-slate-700 dark:bg-slate-950"
                />
              </label>

              <div className="grid grid-cols-2 gap-2">
                <label className="block">
                  <span className="mb-1 block font-medium text-slate-600 dark:text-slate-300">
                    生图/视频输出目录
                  </span>
                  <input
                    value={settingsForm.outputDir}
                    onChange={(e) =>
                      setSettingsForm((s) => ({ ...s, outputDir: e.target.value }))
                    }
                    className="w-full rounded-lg border border-slate-300 bg-slate-50 px-2.5 py-1.5 outline-none focus:border-sky-500 dark:border-slate-700 dark:bg-slate-950"
                  />
                </label>
                <label className="block">
                  <span className="mb-1 block font-medium text-slate-600 dark:text-slate-300">
                    外脑默认模型
                  </span>
                  <input
                    value={settingsForm.consultModel}
                    onChange={(e) =>
                      setSettingsForm((s) => ({ ...s, consultModel: e.target.value }))
                    }
                    className="w-full rounded-lg border border-slate-300 bg-slate-50 px-2.5 py-1.5 outline-none focus:border-sky-500 dark:border-slate-700 dark:bg-slate-950"
                  />
                </label>
              </div>

              <label className="block">
                <span className="mb-1 block font-medium text-slate-600 dark:text-slate-300">
                  国内镜像 Manifest URL
                </span>
                <input
                  value={settingsForm.manifestUrl}
                  onChange={(e) =>
                    setSettingsForm((s) => ({ ...s, manifestUrl: e.target.value }))
                  }
                  className="w-full rounded-lg border border-slate-300 bg-slate-50 px-2.5 py-1.5 outline-none focus:border-sky-500 dark:border-slate-700 dark:bg-slate-950"
                />
              </label>
            </div>

            <div className="mt-4 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setShowSettingsModal(false)}
                className="rounded-md px-3 py-1.5 text-xs text-slate-600 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800"
              >
                取消
              </button>
              <button
                type="submit"
                className="rounded-md bg-sky-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-500"
              >
                保存设置
              </button>
            </div>
          </form>
        </div>
      )}

      {/* Diagnostic Modal */}
      {diagModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 backdrop-blur-xs">
          <div className="w-full max-w-md rounded-xl border border-slate-200 bg-white p-4 shadow-xl dark:border-slate-800 dark:bg-slate-900">
            <div className="mb-2 flex items-center justify-between">
              <h3 className="text-sm font-semibold">{diagModal.name} 配置诊断</h3>
              <button
                onClick={() => setDiagModal(null)}
                className="text-slate-400 hover:text-slate-600"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="space-y-2 text-xs">
              <div className="flex items-center gap-2">
                <span
                  className={`rounded px-2 py-0.5 font-medium ${
                    diagModal.result.ok
                      ? "bg-emerald-100 text-emerald-800 dark:bg-emerald-900/50 dark:text-emerald-200"
                      : "bg-rose-100 text-rose-800 dark:bg-rose-900/50 dark:text-rose-200"
                  }`}
                >
                  {diagModal.result.ok ? "检查通过" : "发现异常"}
                </span>
                <span className="text-slate-500">
                  {diagModal.hasBackup ? "已保留首次无损备份" : "尚未写入过修改"}
                </span>
              </div>
              {diagModal.result.issues.length > 0 && (
                <ul className="list-disc space-y-1 pl-4 text-rose-600 dark:text-rose-400">
                  {diagModal.result.issues.map((item, idx) => (
                    <li key={idx}>{item}</li>
                  ))}
                </ul>
              )}
              {diagModal.result.notes.length > 0 && (
                <ul className="space-y-1 rounded-lg bg-slate-50 p-2.5 text-[11px] text-slate-600 dark:bg-slate-950 dark:text-slate-400">
                  {diagModal.result.notes.map((note, idx) => (
                    <li key={idx} className="break-all">
                      {note}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
