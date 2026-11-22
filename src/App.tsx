import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  AlertCircle,
  ArrowUpCircle,
  Brain,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Download,
  ExternalLink,
  FileText,
  Film,
  Globe,
  Headphones,
  Image as ImageIcon,
  KeyRound,
  LayoutGrid,
  LogOut,
  MessageSquare,
  Moon,
  Power,
  RefreshCw,
  RotateCcw,
  RotateCw,
  Settings as SettingsIcon,
  SlidersHorizontal,
  Sparkles,
  Sun,
  X,
} from "lucide-react";
import {
  api,
  DEFAULT_FALLBACK_MANIFEST_URL,
  DEFAULT_MANIFEST_URL,
  parseUiPatchBlockError,
  TAKO_FEEDBACK_URL,
  TAKO_RELEASES_URL,
  type AccountStatus,
  type AppId,
  type AppMode,
  type AppSnapshot,
  type AppUpdateStatus,
  type ClientModelFilter,
  type DashboardPayload,
  type InstallProgressEvent,
  type McpToolsConfig,
  type SelfUpdateCheckResult,
  type SelfUpdateProgressEvent,
  type UiPatchPermissionPrompt,
} from "./lib/api";
import { AppBrandIcon, TakoBrandMark } from "./components/BrandIcons";
import { McpInlineTheater, type McpToolKey } from "./components/McpInlineTheater";
import { ModelPickerModal } from "./components/ModelPickerModal";
import { UiPatchPermissionModal } from "./components/UiPatchPermissionModal";
import { CHANGELOG_RELEASES, parseReleaseNotesLines } from "./lib/changelog";

type ThemePref = "system" | "light" | "dark";
type SelfUpdateStatus =
  | "idle"
  | "checking"
  | "latest"
  | "available"
  | "downloading"
  | "installing"
  | "error";

interface SelfUpdateInfo {
  version: string;
  notes: string | null;
  source: string;
}

interface SelfUpdateProgress {
  downloaded: number;
  total: number | null;
  percent: number;
}

const SELF_UPDATE_INTERVAL_MS = 30 * 60 * 1000;
const FOCUS_RECHECK_THRESHOLD_MS = 10 * 60 * 1000;

interface McpToolItem {
  key: McpToolKey;
  toolId: string;
  label: string;
  subtitle: string;
  Icon: typeof ImageIcon;
}

const MCP_TOOLS: McpToolItem[] = [
  { key: "image", toolId: "tako_generate_image", label: "生图", subtitle: "改图 · 多图融合", Icon: ImageIcon },
  { key: "video", toolId: "tako_generate_video", label: "生视频", subtitle: "首尾帧 · 动态成片", Icon: Film },
  { key: "audio", toolId: "tako_generate_audio", label: "声音", subtitle: "冰糖多情绪真声", Icon: Headphones },
  { key: "search", toolId: "tako_web_search", label: "联网", subtitle: "实时信源溯源", Icon: Globe },
  { key: "consult", toolId: "tako_consult", label: "外脑", subtitle: "双模型交叉会诊", Icon: Brain },
];

const MODE_OPTIONS: { id: AppMode; label: string }[] = [
  { id: "official", label: "官方" },
  { id: "tako", label: "Tako" },
];

const THEME_OPTIONS: { id: ThemePref; label: string }[] = [
  { id: "system", label: "跟随系统" },
  { id: "light", label: "浅色" },
  { id: "dark", label: "深色" },
];

const APP_DISPLAY_NAMES: Record<AppId, string> = {
  "dsh-tako": "DSH Tako",
  codex: "Codex Desktop",
  claude: "Claude Desktop",
  antigravity: "Google Antigravity",
};

const CONSULT_MODEL_PRESETS = [
  "gpt-6.2-sol",
  "gpt-6.2-astra",
  "deepseek-v4.1-flash",
  "swe-2",
  "claude-opus-5-5",
] as const;

function modelFilterPillLabel(filter?: ClientModelFilter): string {
  if (!filter) return "模型";
  if (filter.mode === "recommended") {
    const n = filter.enabledIds.length;
    return n > 0 ? `模型 ${n}` : "模型 推荐";
  }
  if (filter.mode === "custom") {
    const n = filter.enabledIds.length;
    return n > 0 ? `模型 自选(${n})` : "模型 自选";
  }
  return "模型 全部";
}

function modelFilterPillTitle(filter?: ClientModelFilter): string {
  if (!filter) return "管理该客户端在 Tako 模式下展示的模型列表";
  if (filter.mode === "recommended") {
    const n = filter.enabledIds.length;
    return `推荐精简（${n > 0 ? `${n} 个主力模型` : "默认精简"}）· 点击管理模型列表`;
  }
  if (filter.mode === "custom") {
    const n = filter.enabledIds.length;
    return `自定义筛选（已选 ${n} 个模型）· 点击管理模型列表`;
  }
  return "全部显示 · 点击管理或精简模型列表";
}

function calcPercent(used?: number | null, limit?: number | null): number {
  if (!limit || limit <= 0 || !used || used <= 0) return 0;
  return Math.min(100, Math.round((used / limit) * 100));
}

function formatBytes(bytes?: number | null): string {
  if (!bytes || bytes <= 0) return "";
  if (bytes >= 1_073_741_824) return `${(bytes / 1_073_741_824).toFixed(2)} GB`;
  return `${(bytes / 1_048_576).toFixed(0)} MB`;
}

function formatTimeHHMM(date = new Date()): string {
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

function installStageLabel(stage: string): string {
  switch (stage) {
    case "downloading":
      return "下载中";
    case "verifying":
      return "校验中";
    case "closing":
      return "退出旧版";
    case "installing":
      return "安装中";
    case "configuring":
      return "恢复配置";
    case "done":
      return "已完成";
    case "cancelled":
      return "已取消";
    case "error":
      return "失败";
    default:
      return "处理中";
  }
}

function modeSummary(app: AppSnapshot): string {
  if (app.id === "dsh-tako") {
    return app.mode === "tako"
      ? "Tako 原生桌面端 · 已同步密钥（开箱即用）"
      : "Tako 原生桌面端 · 独立登录";
  }
  if (app.mode === "official") return "官方直连（需官方登录）";
  return app.mcpEnabled ? "Tako 全接管（免官方登录）· Tako MCP" : "Tako 全接管（免官方登录）";
}

function loadInitialTheme(): ThemePref {
  try {
    const saved = localStorage.getItem("tako-switch-theme");
    if (saved === "light" || saved === "dark" || saved === "system") return saved;
  } catch {
    // ignore storage errors
  }
  return "system";
}

function ToggleSwitch({
  checked,
  onChange,
  disabled = false,
  ariaLabel,
  testId,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  ariaLabel?: string;
  testId?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      data-testid={testId}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative inline-flex h-[20px] w-[36px] shrink-0 items-center rounded-full transition-colors duration-150 cursor-pointer disabled:opacity-50 ${
        checked ? "bg-[var(--accent)]" : "bg-[var(--border-strong)]"
      }`}
    >
      <span
        className={`inline-block h-[16px] w-[16px] rounded-full bg-white shadow-sm transition-transform duration-150 ${
          checked ? "translate-x-[18px]" : "translate-x-[2px]"
        }`}
      />
    </button>
  );
}

export default function App() {
  const [dashboard, setDashboard] = useState<DashboardPayload | null>(null);
  const [updatesMap, setUpdatesMap] = useState<Partial<Record<AppId, AppUpdateStatus>>>({});
  const [installProgress, setInstallProgress] = useState<
    Partial<Record<AppId, InstallProgressEvent>>
  >({});
  const [needsRestart, setNeedsRestart] = useState<Partial<Record<AppId, boolean>>>({});
  const [expandedApp, setExpandedApp] = useState<AppId | null>(null);
  const [modelModalApp, setModelModalApp] = useState<AppId | null>(null);
  const [theaterOpen, setTheaterOpen] = useState(false);
  const [theaterTab, setTheaterTab] = useState<McpToolKey>("image");
  const theaterRef = useRef<HTMLDivElement | null>(null);
  const hoverDebounceRef = useRef<number | null>(null);

  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const [busyApp, setBusyApp] = useState<AppId | null>(null);
  const [batchUpdatingApps, setBatchUpdatingApps] = useState(false);
  const [cancellingApp, setCancellingApp] = useState<AppId | null>(null);
  const [fixingGateway, setFixingGateway] = useState(false);
  const [savingMcp, setSavingMcp] = useState(false);
  const [loggingIn, setLoggingIn] = useState(false);

  const [selfUpdate, setSelfUpdate] = useState<SelfUpdateInfo | null>(null);
  const [selfUpdateStatus, setSelfUpdateStatus] = useState<SelfUpdateStatus>("idle");
  const [selfUpdateProgress, setSelfUpdateProgress] = useState<SelfUpdateProgress | null>(null);
  const [selfUpdateError, setSelfUpdateError] = useState<string | null>(null);
  const [lastCheckedAt, setLastCheckedAt] = useState<string | null>(null);
  const lastCheckTsRef = useRef<number>(0);

  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [statusNote, setStatusNote] = useState<string | null>(null);

  const [showKeyModal, setShowKeyModal] = useState(false);
  const [apiKeyInput, setApiKeyInput] = useState("");

  const [showSettingsModal, setShowSettingsModal] = useState(false);
  const [showChangelogModal, setShowChangelogModal] = useState(false);
  const [showAdvancedSettings, setShowAdvancedSettings] = useState(false);
  const selfUpdateNoteLines = useMemo(
    () => parseReleaseNotesLines(selfUpdate?.notes),
    [selfUpdate?.notes],
  );
  const [settingsForm, setSettingsForm] = useState({
    baseUrl: "",
    agyApiKey: "",
    manifestUrl: "",
    fallbackManifestUrl: "",
    outputDir: "",
    consultModel: "",
    autoCheckUpdates: true,
    codexPreserveOfficialAuth: false,
    gatewayDisplayName: "Tako",
    gatewayDisplaySubtitle: "Tako AI Gateway",
    preferChineseUi: true,
    showTrayBalance: true,
    bypassSystemProxy: true,
  });
  const [savingSettings, setSavingSettings] = useState(false);

  const [themePref, setThemePref] = useState<ThemePref>(loadInitialTheme);
  const [systemDark, setSystemDark] = useState<boolean>(() =>
    typeof window !== "undefined"
      ? window.matchMedia("(prefers-color-scheme: dark)").matches
      : false,
  );

  const initialAutoCheckDone = useRef(false);
  const runningSyncInFlight = useRef(false);

  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  const isDark = themePref === "dark" || (themePref === "system" && systemDark);

  useEffect(() => {
    const root = document.documentElement;
    if (themePref === "system") {
      root.removeAttribute("data-theme");
    } else {
      root.setAttribute("data-theme", themePref);
    }
    try {
      localStorage.setItem("tako-switch-theme", themePref);
    } catch {
      // ignore storage errors
    }
  }, [themePref]);

  const toggleTheme = () => {
    setThemePref(isDark ? "light" : "dark");
  };

  const showTransientNote = useCallback((msg: string) => {
    setStatusNote(msg);
    setTimeout(() => {
      setStatusNote((prev) => (prev === msg ? null : prev));
    }, 3200);
  }, []);

  const loadDashboard = useCallback(async (refreshAccount: boolean) => {
    try {
      const data = await api.getDashboard(refreshAccount);
      setDashboard((prev) => {
        if (
          !refreshAccount &&
          prev?.account.loggedIn &&
          data.account.loggedIn &&
          prev.account.maskedKey === data.account.maskedKey &&
          !data.account.userName
        ) {
          return {
            ...data,
            account: {
              ...data.account,
              userName: prev.account.userName,
              planName: prev.account.planName,
              walletRemain: prev.account.walletRemain,
              walletUsed: prev.account.walletUsed,
              walletRemainUsd: prev.account.walletRemainUsd,
              walletUsedUsd: prev.account.walletUsedUsd,
              quota: prev.account.quota,
            },
          };
        }
        return data;
      });
      setErrorMsg(null);
      return data;
    } catch (err) {
      setErrorMsg(String(err));
      return null;
    } finally {
      setLoading(false);
    }
  }, []);

  const applySelfUpdateCheckResult = useCallback((res: SelfUpdateCheckResult) => {
    if (res.available && res.version) {
      setSelfUpdate({
        version: res.version,
        notes: res.notes,
        source: res.source,
      });
      setSelfUpdateStatus((prev) =>
        prev === "downloading" || prev === "installing" ? prev : "available",
      );
    } else {
      setSelfUpdate(null);
      setSelfUpdateStatus((prev) =>
        prev === "downloading" || prev === "installing" ? prev : "latest",
      );
    }
  }, []);

  const runUpdateChecks = useCallback(
    async (silent = false) => {
      setCheckingUpdates(true);
      setSelfUpdateError(null);
      setSelfUpdateStatus((prev) =>
        prev === "downloading" || prev === "installing" ? prev : "checking",
      );

      try {
        const [manifestRes, selfRes] = await Promise.allSettled([
          api.checkAppUpdates(),
          api.checkSelfUpdateSmart(),
        ]);

        if (manifestRes.status === "fulfilled") {
          const nextMap: Partial<Record<AppId, AppUpdateStatus>> = {};
          for (const item of manifestRes.value.updates) {
            nextMap[item.appId] = item;
          }
          setUpdatesMap(nextMap);
        } else if (!silent) {
          setErrorMsg(`检查客户端更新失败：${String(manifestRes.reason)}`);
        }

        if (selfRes.status === "fulfilled") {
          applySelfUpdateCheckResult(selfRes.value);
        } else {
          setSelfUpdateStatus((prev) => {
            if (prev === "downloading" || prev === "installing") return prev;
            return silent ? "idle" : "error";
          });
          if (!silent) {
            setSelfUpdateError(String(selfRes.reason));
          }
        }

        lastCheckTsRef.current = Date.now();
        setLastCheckedAt(formatTimeHHMM());
      } finally {
        setCheckingUpdates(false);
      }
    },
    [applySelfUpdateCheckResult],
  );

  useEffect(() => {
    let disposed = false;

    loadDashboard(true).then((data) => {
      if (disposed || !data) return;
      if (data.state.autoCheckUpdates && !initialAutoCheckDone.current) {
        initialAutoCheckDone.current = true;
        void runUpdateChecks(true);
      }
    });

    const unlistenProgressPromise = listen<InstallProgressEvent>("install-progress", (event) => {
      const payload = event.payload;
      setInstallProgress((prev) => ({ ...prev, [payload.appId]: payload }));
      if (
        payload.stage === "done" ||
        payload.stage === "error" ||
        payload.stage === "cancelled"
      ) {
        setTimeout(() => {
          setInstallProgress((prev) => {
            const copy = { ...prev };
            delete copy[payload.appId];
            return copy;
          });
        }, payload.stage === "cancelled" ? 500 : 1200);
        void loadDashboard(false);
      }
    });

    const unlistenSelfProgressPromise = listen<SelfUpdateProgressEvent>(
      "self-update-progress",
      (event) => {
        const { stage, downloaded, total } = event.payload;
        if (stage === "started") {
          setSelfUpdateStatus("downloading");
          setSelfUpdateProgress({ downloaded: 0, total, percent: 4 });
        } else if (stage === "progress") {
          const pct =
            total && total > 0
              ? Math.min(96, Math.max(4, Math.round((downloaded / total) * 96)))
              : Math.min(90, Math.round(4 + downloaded / 524_288));
          setSelfUpdateStatus("downloading");
          setSelfUpdateProgress({ downloaded, total, percent: pct });
        } else if (stage === "finished") {
          setSelfUpdateStatus("installing");
          setSelfUpdateProgress({
            downloaded,
            total: total ?? downloaded,
            percent: 100,
          });
        }
      },
    );

    const unlistenSelfStatusPromise = listen<SelfUpdateCheckResult>(
      "self-update-status",
      (event) => {
        applySelfUpdateCheckResult(event.payload);
        lastCheckTsRef.current = Date.now();
        setLastCheckedAt(formatTimeHHMM());
      },
    );

    const unlistenStatePromise = listen("state-changed", () => {
      void loadDashboard(false);
    });

    const unlistenUpdatesPromise = listen("request-check-updates", () => {
      void runUpdateChecks(false);
    });

    const unlistenAccountPromise = listen<AccountStatus>("account-updated", (event) => {
      const nextAccount = event.payload;
      setDashboard((prev) => (prev ? { ...prev, account: nextAccount } : prev));
    });

    return () => {
      disposed = true;
      void unlistenProgressPromise.then((fn) => fn());
      void unlistenSelfProgressPromise.then((fn) => fn());
      void unlistenSelfStatusPromise.then((fn) => fn());
      void unlistenStatePromise.then((fn) => fn());
      void unlistenUpdatesPromise.then((fn) => fn());
      void unlistenAccountPromise.then((fn) => fn());
    };
  }, [applySelfUpdateCheckResult, loadDashboard, runUpdateChecks]);

  const syncRunningStates = useCallback(async () => {
    if (runningSyncInFlight.current) return;
    runningSyncInFlight.current = true;
    try {
      const states = await api.getRunningStates();
      setDashboard((prev) => {
        if (!prev) return prev;
        let changed = false;
        const nextApps = prev.apps.map((app) => {
          const nextRunning = Boolean(states[app.id]);
          if (nextRunning !== app.running) {
            changed = true;
            return { ...app, running: nextRunning };
          }
          return app;
        });
        return changed ? { ...prev, apps: nextApps } : prev;
      });
      setNeedsRestart((prev) => {
        let changed = false;
        const next = { ...prev };
        for (const id of Object.keys(prev) as AppId[]) {
          if (prev[id] && !states[id]) {
            next[id] = false;
            changed = true;
          }
        }
        return changed ? next : prev;
      });
    } catch {
      // ignore transient polling errors
    } finally {
      runningSyncInFlight.current = false;
    }
  }, []);

  useEffect(() => {
    const autoCheck = Boolean(dashboard?.state.autoCheckUpdates);
    const maybeRecheckOnFocus = () => {
      if (!autoCheck || document.visibilityState !== "visible") return;
      if (Date.now() - lastCheckTsRef.current >= FOCUS_RECHECK_THRESHOLD_MS) {
        void runUpdateChecks(true);
      }
    };
    const onFocusOrVisible = () => {
      if (document.visibilityState === "visible") {
        void syncRunningStates();
        maybeRecheckOnFocus();
      }
    };
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") {
        void syncRunningStates();
      }
    }, 2500);
    const updateTimer = autoCheck
      ? window.setInterval(() => {
          void runUpdateChecks(true);
        }, SELF_UPDATE_INTERVAL_MS)
      : null;
    window.addEventListener("focus", onFocusOrVisible);
    document.addEventListener("visibilitychange", onFocusOrVisible);
    return () => {
      window.clearInterval(timer);
      if (updateTimer !== null) {
        window.clearInterval(updateTimer);
      }
      window.removeEventListener("focus", onFocusOrVisible);
      document.removeEventListener("visibilitychange", onFocusOrVisible);
    };
  }, [dashboard?.state.autoCheckUpdates, runUpdateChecks, syncRunningStates]);

  const handleRefresh = async () => {
    setRefreshing(true);
    await Promise.all([loadDashboard(true), runUpdateChecks(false)]);
    setRefreshing(false);
    showTransientNote("状态与版本信息已刷新");
  };

  const handleEnsureGateway = async () => {
    if (fixingGateway) return;
    setFixingGateway(true);
    try {
      const ok = await api.ensureLocalGateway();
      await loadDashboard(false);
      if (ok) {
        showTransientNote("本地统一网关已就绪 (127.0.0.1:18419)");
      } else {
        setErrorMsg("本地统一网关未能启动，请检查 18419 端口是否被占用");
      }
    } catch (err) {
      setErrorMsg(`拉起本地网关失败：${String(err)}`);
    } finally {
      setFixingGateway(false);
    }
  };

  const handleOAuthLogin = async () => {
    setLoggingIn(true);
    setErrorMsg(null);
    try {
      await api.loginOAuth();
      await loadDashboard(true);
      showTransientNote("Tako 账号已登录");
    } catch (err) {
      setErrorMsg(`登录失败：${String(err)}`);
    } finally {
      setLoggingIn(false);
    }
  };

  const handleKeyLogin = async (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = apiKeyInput.trim();
    if (!trimmed) return;
    setLoggingIn(true);
    setErrorMsg(null);
    try {
      await api.loginWithKey(trimmed);
      setApiKeyInput("");
      setShowKeyModal(false);
      await loadDashboard(true);
      showTransientNote("API Key 验证成功");
    } catch (err) {
      setErrorMsg(`Key 验证失败：${String(err)}`);
    } finally {
      setLoggingIn(false);
    }
  };

  const handleLogout = async () => {
    try {
      await api.logoutAccount();
      await loadDashboard(false);
      showTransientNote("已退出当前账号");
    } catch (err) {
      setErrorMsg(String(err));
    }
  };

  const handleSetMode = async (app: AppSnapshot, mode: AppMode) => {
    if (app.mode === mode || busyApp === app.id) return;
    setBusyApp(app.id);
    setErrorMsg(null);
    try {
      const updated = await api.setAppMode(app.id, mode, false);
      const gwOk =
        app.id !== "dsh-tako" && updated.mode === "tako"
          ? await api.ensureLocalGateway().catch(() => true)
          : undefined;
      setDashboard((prev) =>
        prev
          ? {
              ...prev,
              localGatewayHealthy: gwOk ?? prev.localGatewayHealthy,
              apps: prev.apps.map((item) => (item.id === app.id ? updated : item)),
            }
          : prev,
      );
      if (updated.running) {
        setNeedsRestart((prev) => ({ ...prev, [app.id]: true }));
        showTransientNote(`${APP_DISPLAY_NAMES[app.id]} 已切换模式，重启应用后生效`);
      }
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusyApp(null);
    }
  };

  const handleCloseApp = async (appId: AppId) => {
    setBusyApp(appId);
    setErrorMsg(null);
    try {
      const updated = await api.closeApp(appId);
      setNeedsRestart((prev) => ({ ...prev, [appId]: false }));
      setDashboard((prev) =>
        prev
          ? {
              ...prev,
              apps: prev.apps.map((item) => (item.id === appId ? updated : item)),
            }
          : prev,
      );
      showTransientNote(`${APP_DISPLAY_NAMES[appId]} 已关闭`);
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusyApp(null);
    }
  };

  const handleRestartApp = async (appId: AppId) => {
    setBusyApp(appId);
    setErrorMsg(null);
    try {
      const updated = await api.restartApp(appId);
      setNeedsRestart((prev) => ({ ...prev, [appId]: false }));
      setDashboard((prev) =>
        prev
          ? {
              ...prev,
              apps: prev.apps.map((item) => (item.id === appId ? updated : item)),
            }
          : prev,
      );
      showTransientNote(`${APP_DISPLAY_NAMES[appId]} 已重启`);
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusyApp(null);
    }
  };

  const handleLaunchApp = async (appId: AppId) => {
    setBusyApp(appId);
    setErrorMsg(null);
    try {
      const updated = await api.launchApp(appId);
      setNeedsRestart((prev) => ({ ...prev, [appId]: false }));
      setDashboard((prev) =>
        prev
          ? {
              ...prev,
              apps: prev.apps.map((item) => (item.id === appId ? updated : item)),
            }
          : prev,
      );
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusyApp(null);
    }
  };

  const handleInstallOrUpgrade = async (appId: AppId): Promise<boolean> => {
    setBusyApp(appId);
    setErrorMsg(null);
    try {
      const outcome = await api.installOrUpgradeApp(appId);
      setNeedsRestart((prev) => ({ ...prev, [appId]: false }));
      setDashboard((prev) =>
        prev
          ? {
              ...prev,
              apps: prev.apps.map((item) => (item.id === appId ? outcome.snapshot : item)),
            }
          : prev,
      );
      await runUpdateChecks(true);
      showTransientNote(`${APP_DISPLAY_NAMES[appId]} 安装更新完成`);
      return true;
    } catch (err) {
      const msg = String(err);
      if (msg.includes("已取消下载")) {
        showTransientNote(`已取消 ${APP_DISPLAY_NAMES[appId]} 下载`);
      } else {
        setErrorMsg(msg);
      }
      return false;
    } finally {
      setCancellingApp((prev) => (prev === appId ? null : prev));
      setBusyApp(null);
    }
  };

  const handleCancelAppInstall = async (appId: AppId) => {
    setCancellingApp(appId);
    try {
      await api.cancelAppInstall(appId);
    } catch {
      // ignore cancel race if install already moved past downloading
    }
  };

  const updatableApps = useMemo(() => {
    if (!dashboard) return [];
    return dashboard.apps.filter((app) => Boolean(updatesMap[app.id]?.hasUpdate));
  }, [dashboard, updatesMap]);

  const handleBatchUpgradeApps = async () => {
    if (batchUpdatingApps || busyApp !== null || updatableApps.length === 0) return;
    setBatchUpdatingApps(true);
    try {
      for (const app of updatableApps) {
        const ok = await handleInstallOrUpgrade(app.id);
        if (!ok) break;
      }
    } finally {
      setBatchUpdatingApps(false);
    }
  };

  const handleRediagnose = async (appId: AppId) => {
    try {
      const result = await api.diagnoseApp(appId);
      setDashboard((prev) =>
        prev
          ? {
              ...prev,
              apps: prev.apps.map((item) =>
                item.id === appId ? { ...item, diagnostic: result } : item,
              ),
            }
          : prev,
      );
      showTransientNote(`${APP_DISPLAY_NAMES[appId]} 诊断已完成`);
    } catch (err) {
      setErrorMsg(String(err));
    }
  };

  const [togglingChineseUi, setTogglingChineseUi] = useState(false);
  const [permissionPrompts, setPermissionPrompts] = useState<UiPatchPermissionPrompt[]>([]);
  const [retryingPermission, setRetryingPermission] = useState(false);
  const permissionRetryRef = useRef<(() => Promise<void>) | null>(null);

  const runSetPreferChineseUi = async (nextEnable: boolean) => {
    setTogglingChineseUi(true);
    setErrorMsg(null);
    try {
      const nextDashboard = await api.setPreferChineseUi(nextEnable, true, false);
      setDashboard(nextDashboard);
      setSettingsForm((prev) => ({ ...prev, preferChineseUi: nextEnable }));
      const prompts = nextDashboard.uiPatchPrompts ?? [];
      if (prompts.length > 0) {
        permissionRetryRef.current = () => runSetPreferChineseUi(nextEnable);
        setPermissionPrompts(prompts);
      } else {
        setPermissionPrompts([]);
        permissionRetryRef.current = null;
      }
      const runningTargets = nextDashboard.apps.filter(
        (a) => a.id !== "dsh-tako" && a.running,
      );
      if (runningTargets.length > 0) {
        setNeedsRestart((prev) => {
          const copy = { ...prev };
          for (const a of runningTargets) copy[a.id] = true;
          return copy;
        });
      }
      if (prompts.length > 0) {
        showTransientNote(
          nextEnable
            ? "配置文件级简体中文已生效，界面资源包需授权后完成注入"
            : "配置文件级中文增强已还原，界面资源包需授权后完成还原",
        );
      } else if (nextEnable) {
        showTransientNote(
          runningTargets.length > 0
            ? "三端简体中文汉化已写入，运行中客户端点击「重启生效」即可应用"
            : "三端简体中文汉化已一步到位",
        );
      } else {
        showTransientNote("已关闭客户端中文增强并无损恢复原配置");
      }
    } catch (err) {
      const block = parseUiPatchBlockError(err);
      if (block) {
        permissionRetryRef.current = () => runSetPreferChineseUi(nextEnable);
        setPermissionPrompts([block]);
      } else {
        setErrorMsg(`切换三端汉化失败：${String(err)}`);
      }
    } finally {
      setTogglingChineseUi(false);
    }
  };

  const handleToggleMainChineseUi = async () => {
    if (!dashboard || togglingChineseUi || busyApp !== null) return;
    const currentEnabled = dashboard.state.preferChineseUi ?? true;
    const needsUiPatch = dashboard.apps.some(
      (a) =>
        (a.id === "antigravity" || a.id === "claude") &&
        a.install.installed &&
        (a.uiPatch.status === "unpatched" ||
          a.uiPatch.status === "outdated_after_upgrade"),
    );
    const anyInstalledMissingZh = dashboard.apps.some(
      (a) => a.id !== "dsh-tako" && a.install.installed && !a.chineseConfigured,
    );
    // If preferChineseUi is already true in state, but some installed client or Claude/Antigravity UI isn't localized yet,
    // clicking applies full 3-client localization + UI patch first instead of unexpectedly disabling!
    const nextEnable =
      currentEnabled && (needsUiPatch || anyInstalledMissingZh) ? true : !currentEnabled;

    await runSetPreferChineseUi(nextEnable);
  };

  const handlePatchAppUi = async (app: AppSnapshot) => {
    if (busyApp !== null) return;
    const label = APP_DISPLAY_NAMES[app.id] ?? app.name;
    setBusyApp(app.id);
    setErrorMsg(null);
    try {
      const updated = await api.patchAppUi(app.id, false);
      setPermissionPrompts([]);
      permissionRetryRef.current = null;
      setDashboard((prev) =>
        prev
          ? {
              ...prev,
              apps: prev.apps.map((item) => (item.id === app.id ? updated : item)),
            }
          : prev,
      );
      if (app.running || updated.running) {
        setNeedsRestart((prev) => ({ ...prev, [app.id]: true }));
        showTransientNote(`${label} 界面汉化补丁已注入，点击「重启生效」即可应用`);
      } else {
        showTransientNote(`${label} 简体中文界面汉化已完成`);
      }
    } catch (err) {
      const block = parseUiPatchBlockError(err);
      if (block) {
        permissionRetryRef.current = () => handlePatchAppUi(app);
        setPermissionPrompts([block]);
      } else {
        setErrorMsg(`${label} 界面汉化失败：${String(err)}`);
      }
    } finally {
      setBusyApp(null);
    }
  };

  const handleRestoreAppUi = async (app: AppSnapshot) => {
    if (busyApp !== null) return;
    const label = APP_DISPLAY_NAMES[app.id] ?? app.name;
    setBusyApp(app.id);
    setErrorMsg(null);
    try {
      const updated = await api.restoreAppUi(app.id, false);
      setPermissionPrompts([]);
      permissionRetryRef.current = null;
      setDashboard((prev) =>
        prev
          ? {
              ...prev,
              apps: prev.apps.map((item) => (item.id === app.id ? updated : item)),
            }
          : prev,
      );
      if (app.running || updated.running) {
        setNeedsRestart((prev) => ({ ...prev, [app.id]: true }));
        showTransientNote(`已无损还原原版 ${label}，重启应用后生效`);
      } else {
        showTransientNote(`已字节级无损还原原版 ${label}`);
      }
    } catch (err) {
      const block = parseUiPatchBlockError(err);
      if (block) {
        permissionRetryRef.current = () => handleRestoreAppUi(app);
        setPermissionPrompts([block]);
      } else {
        setErrorMsg(`还原 ${label} 原版失败：${String(err)}`);
      }
    } finally {
      setBusyApp(null);
    }
  };

  const handleOpenMacosAppManagementSettings = async () => {
    try {
      await api.openMacosAppManagementSettings();
    } catch (err) {
      setErrorMsg(`打开系统设置失败：${String(err)}`);
    }
  };

  const handleRetryPermissionAction = async () => {
    const retryFn = permissionRetryRef.current;
    if (!retryFn || retryingPermission) return;
    setRetryingPermission(true);
    try {
      await retryFn();
    } finally {
      setRetryingPermission(false);
    }
  };

  const handleToggleMcpTool = async (key: McpToolKey) => {
    if (!dashboard || savingMcp) return;
    const currentMcp = dashboard.state.mcp;
    const nextMcp: McpToolsConfig = { ...currentMcp, [key]: !currentMcp[key] };

    setSavingMcp(true);
    setDashboard((prev) =>
      prev ? { ...prev, state: { ...prev.state, mcp: nextMcp } } : prev,
    );

    try {
      const savedState = await api.saveSettings({ mcp: nextMcp });
      setDashboard((prev) => (prev ? { ...prev, state: savedState } : prev));
    } catch (err) {
      setDashboard((prev) =>
        prev ? { ...prev, state: { ...prev.state, mcp: currentMcp } } : prev,
      );
      setErrorMsg(`更新 MCP 开关失败：${String(err)}`);
    } finally {
      setSavingMcp(false);
    }
  };

  const handleSelectConsultModel = async (model: string) => {
    if (!dashboard || savingMcp) return;
    const currentMcp = dashboard.state.mcp;
    if (currentMcp.consultModel === model) return;
    const nextMcp: McpToolsConfig = { ...currentMcp, consultModel: model };

    setSavingMcp(true);
    setDashboard((prev) =>
      prev ? { ...prev, state: { ...prev.state, mcp: nextMcp } } : prev,
    );

    try {
      const savedState = await api.saveSettings({ mcp: nextMcp });
      setDashboard((prev) => (prev ? { ...prev, state: savedState } : prev));
      setSettingsForm((prev) => ({ ...prev, consultModel: savedState.mcp.consultModel }));
      showTransientNote(`默认外脑参谋模型已切换为 ${model}`);
    } catch (err) {
      setDashboard((prev) =>
        prev ? { ...prev, state: { ...prev.state, mcp: currentMcp } } : prev,
      );
      setErrorMsg(`更新外脑模型失败：${String(err)}`);
    } finally {
      setSavingMcp(false);
    }
  };

  const openMcpDemo = (tab: McpToolKey = "image") => {
    setTheaterTab(tab);
    setTheaterOpen(true);
  };

  const toggleMcpTheater = () => {
    setTheaterOpen((prev) => !prev);
  };

  const handleMcpCardHoverEnter = (key: McpToolKey) => {
    if (!theaterOpen) return;
    if (hoverDebounceRef.current !== null) {
      window.clearTimeout(hoverDebounceRef.current);
    }
    hoverDebounceRef.current = window.setTimeout(() => {
      setTheaterTab(key);
      hoverDebounceRef.current = null;
    }, 150);
  };

  const handleMcpCardHoverLeave = () => {
    if (hoverDebounceRef.current !== null) {
      window.clearTimeout(hoverDebounceRef.current);
      hoverDebounceRef.current = null;
    }
  };

  useEffect(() => {
    if (!theaterOpen) return;
    const id = window.setTimeout(() => {
      theaterRef.current?.scrollIntoView({ behavior: "smooth", block: "nearest" });
    }, 160);
    return () => window.clearTimeout(id);
  }, [theaterOpen]);

  useEffect(() => {
    return () => {
      if (hoverDebounceRef.current !== null) {
        window.clearTimeout(hoverDebounceRef.current);
      }
    };
  }, []);

  const openSettings = () => {
    if (!dashboard) return;
    setSettingsForm({
      baseUrl: dashboard.state.baseUrl,
      agyApiKey: dashboard.state.agyApiKey ?? "",
      manifestUrl: dashboard.state.manifestUrl,
      fallbackManifestUrl: dashboard.state.fallbackManifestUrl,
      outputDir: dashboard.state.mcp.outputDir,
      consultModel: dashboard.state.mcp.consultModel,
      autoCheckUpdates: dashboard.state.autoCheckUpdates,
      codexPreserveOfficialAuth: Boolean(dashboard.state.codexPreserveOfficialAuth),
      gatewayDisplayName: dashboard.state.gatewayDisplayName ?? "Tako",
      gatewayDisplaySubtitle: dashboard.state.gatewayDisplaySubtitle ?? "Tako AI Gateway",
      preferChineseUi: dashboard.state.preferChineseUi ?? true,
      showTrayBalance: dashboard.state.showTrayBalance ?? true,
      bypassSystemProxy: dashboard.state.bypassSystemProxy ?? true,
    });
    setShowAdvancedSettings(false);
    setShowSettingsModal(true);
  };

  const handleSaveSettings = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!dashboard) return;
    setSavingSettings(true);
    setErrorMsg(null);
    try {
      const saved = await api.saveSettings({
        baseUrl: settingsForm.baseUrl.trim(),
        agyApiKey: settingsForm.agyApiKey.trim(),
        manifestUrl: settingsForm.manifestUrl.trim(),
        fallbackManifestUrl: settingsForm.fallbackManifestUrl.trim(),
        autoCheckUpdates: settingsForm.autoCheckUpdates,
        codexPreserveOfficialAuth: settingsForm.codexPreserveOfficialAuth,
        gatewayDisplayName: settingsForm.gatewayDisplayName.trim(),
        gatewayDisplaySubtitle: settingsForm.gatewayDisplaySubtitle.trim(),
        preferChineseUi: settingsForm.preferChineseUi,
        showTrayBalance: settingsForm.showTrayBalance,
        bypassSystemProxy: settingsForm.bypassSystemProxy,
        mcp: {
          ...dashboard.state.mcp,
          outputDir: settingsForm.outputDir.trim(),
          consultModel: settingsForm.consultModel.trim(),
        },
      });
      setDashboard((prev) => (prev ? { ...prev, state: saved } : prev));
      await loadDashboard(false);
      setShowSettingsModal(false);
      showTransientNote("设置已保存");
    } catch (err) {
      setErrorMsg(`保存设置失败：${String(err)}`);
    } finally {
      setSavingSettings(false);
    }
  };

  const handleInstallSelfUpdate = async () => {
    if (
      !selfUpdate ||
      selfUpdateStatus === "downloading" ||
      selfUpdateStatus === "installing"
    ) {
      return;
    }
    setSelfUpdateStatus("downloading");
    setSelfUpdateError(null);
    setSelfUpdateProgress({ downloaded: 0, total: null, percent: 2 });

    try {
      await api.installSelfUpdate();
      setSelfUpdateStatus("installing");
    } catch (err) {
      const msg = String(err);
      setSelfUpdateStatus("error");
      setSelfUpdateError(msg);
      setErrorMsg(`Tako Switch 更新失败：${msg}`);
    }
  };

  const isMac = useMemo(() => {
    if (dashboard?.os) return dashboard.os === "macos";
    return typeof navigator !== "undefined" && navigator.platform.toLowerCase().includes("mac");
  }, [dashboard?.os]);

  const mcpEnabledCount = useMemo(() => {
    if (!dashboard) return 0;
    const mcp = dashboard.state.mcp;
    return [mcp.image, mcp.video, mcp.audio, mcp.search, mcp.consult].filter(Boolean).length;
  }, [dashboard]);

  const anyTakoMode = useMemo(
    () => Boolean(dashboard?.apps.some((a) => a.id !== "dsh-tako" && a.mode === "tako")),
    [dashboard?.apps],
  );

  const hasPendingUpdates =
    selfUpdateStatus === "available" || updatableApps.length > 0;

  const account = dashboard?.account;
  const pct5h = calcPercent(account?.quota?.windowUsed, account?.quota?.windowCostLimit);
  const pct7d = calcPercent(account?.quota?.weeklyUsed, account?.quota?.weeklyCostLimit);
  const hasSubQuota = Boolean(
    account?.quota &&
      (account.quota.windowCostLimit > 0 || account.quota.weeklyCostLimit > 0),
  );
  const remainUsd = account?.walletRemainUsd;
  const usedUsd = account?.walletUsedUsd;
  const balanceColor =
    remainUsd !== null && remainUsd !== undefined
      ? remainUsd < 0.5
        ? "var(--danger)"
        : remainUsd < 2.0
          ? "var(--warning)"
          : "var(--success)"
      : "var(--text-primary)";

  return (
    <div className="h-screen w-full flex flex-col app-surface overflow-x-hidden overflow-y-hidden">
      {/* Unified Native-Integrated Header (52px on desktop, wraps cleanly on <640px) */}
      <header
        data-tauri-drag-region
        className={`min-h-[52px] sm:h-[52px] shrink-0 flex flex-wrap sm:flex-nowrap items-center justify-between gap-x-2 gap-y-1.5 border-b border-[var(--border)] py-1.5 sm:py-0 pr-3.5 ${
          isMac ? "pl-[82px]" : "pl-4"
        }`}
      >
        {/* Left Brand */}
        <div data-tauri-drag-region className="flex items-center gap-2 sm:gap-2.5 min-w-0 shrink-0 max-w-full">
          <div data-tauri-drag-region className="pointer-events-none shrink-0">
            <TakoBrandMark className="h-[26px] w-[26px]" />
          </div>
          <span
            data-tauri-drag-region
            className="text-[14px] font-semibold tracking-[-0.015em] text-[var(--text-primary)] truncate"
          >
            Tako Switch
          </span>
          <span
            data-tauri-drag-region
            className="tabular-nums text-[12px] font-medium text-[var(--text-muted)] bg-[var(--bg-subtle)] border border-[var(--border)] rounded-full px-2 py-0.5 shrink-0"
          >
            v{dashboard?.appVersion ?? "2.0.1"}
          </span>
        </div>

        {/* Flexible drag region spacer */}
        <div data-tauri-drag-region className="hidden sm:block flex-1 h-full min-w-[16px]" />

        {/* Right Controls */}
        <div className="flex items-center flex-wrap sm:flex-nowrap justify-end gap-1.5 min-w-0 max-w-full">
          {account?.loggedIn ? (
            <div className="min-h-8 sm:h-8 px-2 sm:px-2.5 py-1 sm:py-0 rounded-lg border border-[var(--border)] bg-[var(--bg-card)] flex flex-wrap sm:flex-nowrap items-center gap-x-2 sm:gap-x-2.5 gap-y-1 shadow-[var(--shadow-card)] max-w-full min-w-0">
              <div className="flex items-center gap-1.5 min-w-0">
                <span className="text-[12px] font-medium text-[var(--text-primary)] max-w-[84px] truncate">
                  {account.userName ?? "Tako 用户"}
                </span>
                {account.planName && (
                  <span className="text-[12px] font-medium text-[var(--accent)] shrink-0">
                    {account.planName}
                  </span>
                )}
              </div>

              {remainUsd !== null && remainUsd !== undefined && (
                <>
                  <span className="hidden sm:inline-block h-3.5 w-px bg-[var(--border)]" />
                  <div
                    className="flex items-center gap-1 tabular-nums text-[12px] shrink-0"
                    title={`钱包余额 $${remainUsd.toFixed(2)}（累计已用 $${(usedUsd ?? 0).toFixed(2)}）`}
                  >
                    <span className="text-[var(--text-muted)]">余额</span>
                    <span className="font-semibold" style={{ color: balanceColor }}>
                      ${remainUsd.toFixed(2)}
                    </span>
                  </div>
                </>
              )}

              {hasSubQuota && (
                <>
                  <span className="hidden sm:inline-block h-3.5 w-px bg-[var(--border)]" />
                  <div className="flex items-center gap-2 tabular-nums text-[12px] text-[var(--text-secondary)] shrink-0">
                    <div className="flex items-center gap-1" title={`5 小时窗口已用 ${pct5h}%`}>
                      <span className="text-[var(--text-muted)]">5h</span>
                      <span>{pct5h}%</span>
                    </div>
                    <div className="flex items-center gap-1" title={`7 天周期已用 ${pct7d}%`}>
                      <span className="text-[var(--text-muted)]">7d</span>
                      <span>{pct7d}%</span>
                    </div>
                  </div>
                </>
              )}

              <button
                type="button"
                onClick={handleLogout}
                title="退出登录"
                className="p-1 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)] transition cursor-pointer shrink-0"
              >
                <LogOut className="h-3.5 w-3.5" />
              </button>
            </div>
          ) : (
            <div className="flex items-center gap-1.5 shrink-0">
              <button
                type="button"
                onClick={() => setShowKeyModal(true)}
                className="h-8 px-2.5 rounded-lg border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)] flex items-center gap-1.5 transition cursor-pointer"
              >
                <KeyRound className="h-3.5 w-3.5" />
                <span>Key 登录</span>
              </button>
              <button
                type="button"
                onClick={handleOAuthLogin}
                disabled={loggingIn}
                className="h-8 px-3 rounded-lg bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium transition cursor-pointer disabled:opacity-60"
              >
                {loggingIn ? "登录中..." : "登录 Tako"}
              </button>
            </div>
          )}

          <button
            type="button"
            onClick={handleRefresh}
            disabled={refreshing || checkingUpdates}
            title="刷新状态与版本"
            className="h-8 w-8 rounded-lg border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] flex items-center justify-center transition cursor-pointer disabled:opacity-60"
          >
            <RefreshCw
              className={`h-3.5 w-3.5 ${refreshing || checkingUpdates ? "animate-spin" : ""}`}
            />
          </button>

          <button
            type="button"
            onClick={toggleTheme}
            title={isDark ? "切换浅色模式" : "切换深色模式"}
            className="h-8 w-8 rounded-lg border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] flex items-center justify-center transition cursor-pointer"
          >
            {isDark ? <Sun className="h-3.5 w-3.5" /> : <Moon className="h-3.5 w-3.5" />}
          </button>

          <button
            type="button"
            onClick={openSettings}
            title="设置与更新"
            className="relative h-8 w-8 rounded-lg border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] flex items-center justify-center transition cursor-pointer"
          >
            <SettingsIcon className="h-3.5 w-3.5" />
            {hasPendingUpdates && (
              <span className="absolute top-1.5 right-1.5 h-1.5 w-1.5 rounded-full bg-[var(--accent)]" />
            )}
          </button>
        </div>
      </header>

      {/* Main Content Container */}
      <main className="flex-1 min-h-0 overflow-y-auto overflow-x-hidden subtle-scroll px-4 pt-3 pb-2.5 flex flex-col gap-3">
        {/* Self-Update Banner */}
        {(selfUpdate ||
          selfUpdateStatus === "downloading" ||
          selfUpdateStatus === "installing") && (
          <div className="shrink-0 rounded-[12px] px-3.5 py-2.5 text-[12px] flex flex-col gap-2 border bg-[var(--accent-soft)] border-[var(--accent-border)] text-[var(--accent)]">
            <div className="flex items-center justify-between gap-2">
              <div className="flex items-center gap-2 min-w-0">
                <ArrowUpCircle className="h-4 w-4 shrink-0" />
                <div className="min-w-0">
                  <div className="font-medium truncate">
                    {selfUpdateStatus === "installing"
                      ? `正在安装 Tako Switch v${selfUpdate?.version ?? ""}，即将重启...`
                      : selfUpdateStatus === "downloading" && selfUpdateProgress
                        ? `正在下载 Tako Switch v${selfUpdate?.version ?? ""} · ${formatBytes(selfUpdateProgress.downloaded)}${selfUpdateProgress.total ? ` / ${formatBytes(selfUpdateProgress.total)}` : ""}`
                        : `Tako Switch v${selfUpdate?.version ?? ""} 已发布${selfUpdate?.source === "github" ? "（GitHub 同步源）" : ""}，可一键更新并重启`}
                  </div>
                </div>
              </div>

              {selfUpdateStatus === "downloading" || selfUpdateStatus === "installing" ? (
                <span className="tabular-nums font-semibold shrink-0">
                  {selfUpdateProgress?.percent ?? 0}%
                </span>
              ) : (
                <div className="flex items-center gap-1.5 shrink-0">
                  {selfUpdateStatus === "error" && (
                    <button
                      type="button"
                      onClick={() => void api.openUrl(TAKO_RELEASES_URL)}
                      className="px-2 py-1 rounded-md border border-[var(--accent-border)] text-[12px] font-medium hover:bg-[var(--bg-card)]/40 cursor-pointer"
                    >
                      前往下载页
                    </button>
                  )}
                  <button
                    type="button"
                    onClick={() => setShowChangelogModal(true)}
                    className="px-2 py-1 rounded-md border border-[var(--accent-border)] text-[12px] font-medium hover:bg-[var(--bg-card)]/40 cursor-pointer"
                  >
                    更新说明
                  </button>
                  <button
                    type="button"
                    onClick={handleInstallSelfUpdate}
                    className="px-2.5 py-1 rounded-md bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium cursor-pointer"
                  >
                    {selfUpdateStatus === "error" ? "重试更新" : "立即更新"}
                  </button>
                </div>
              )}
            </div>

            {selfUpdateStatus !== "downloading" &&
              selfUpdateStatus !== "installing" &&
              selfUpdateNoteLines.length > 0 && (
                <div className="rounded-[8px] bg-[var(--bg-card)]/65 border border-[var(--accent-border)]/70 px-2.5 py-2 text-[11px] text-[var(--text-primary)] flex flex-col gap-1">
                  <ul className="space-y-1 leading-relaxed">
                    {selfUpdateNoteLines.slice(0, 3).map((line, idx) => (
                      <li key={idx} className="flex items-start gap-1.5">
                        <span className="text-[var(--accent)] select-none shrink-0">•</span>
                        <span>{line}</span>
                      </li>
                    ))}
                  </ul>
                  {selfUpdateNoteLines.length > 3 && (
                    <button
                      type="button"
                      onClick={() => setShowChangelogModal(true)}
                      className="self-start text-[11px] font-medium text-[var(--accent)] hover:underline cursor-pointer mt-0.5"
                    >
                      查看全部 {selfUpdateNoteLines.length} 项更新 →
                    </button>
                  )}
                </div>
              )}

            {(selfUpdateStatus === "downloading" || selfUpdateStatus === "installing") &&
              selfUpdateProgress && (
                <div className="h-1 w-full rounded-full bg-[var(--bg-card)]/60 overflow-hidden">
                  <div
                    className="h-full rounded-full bg-[var(--accent)] transition-all duration-150"
                    style={{ width: `${Math.max(4, selfUpdateProgress.percent)}%` }}
                  />
                </div>
              )}
          </div>
        )}

        {/* Dismissible Error / Transient Status Bar */}
        {(errorMsg || statusNote || account?.error) && (
          <div
            className="shrink-0 rounded-[11px] px-3 py-2 text-[12px] flex items-center justify-between gap-2 border"
            style={{
              backgroundColor:
                errorMsg || account?.error ? "var(--danger-soft)" : "var(--accent-soft)",
              borderColor:
                errorMsg || account?.error
                  ? "rgba(220, 38, 38, 0.24)"
                  : "var(--accent-border)",
              color: errorMsg || account?.error ? "var(--danger)" : "var(--accent)",
            }}
          >
            <div className="flex items-center gap-2 min-w-0">
              <AlertCircle className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{errorMsg ?? account?.error ?? statusNote}</span>
            </div>
            <button
              type="button"
              onClick={() => {
                setErrorMsg(null);
                setStatusNote(null);
              }}
              className="p-0.5 rounded hover:opacity-75 cursor-pointer shrink-0"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        )}

        {/* Client Rows Section */}
        <section className="flex flex-col gap-2 min-w-0">
          <div className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1.5 px-0.5 min-w-0">
            <div className="flex items-center flex-wrap gap-1.5 min-w-0">
              <h2 className="text-[12px] font-medium text-[var(--text-secondary)] shrink-0">
                桌面客户端
              </h2>
              {(() => {
                const zhPrefOn = dashboard?.state.preferChineseUi ?? true;
                const uiNeedsPatch = Boolean(
                  dashboard?.apps.some(
                    (a) =>
                      (a.id === "antigravity" || a.id === "claude") &&
                      a.install.installed &&
                      (a.uiPatch.status === "unpatched" ||
                        a.uiPatch.status === "outdated_after_upgrade"),
                  ),
                );
                const anyInstalledMissingZh = Boolean(
                  dashboard?.apps.some(
                    (a) => a.id !== "dsh-tako" && a.install.installed && !a.chineseConfigured,
                  ),
                );
                const fullyActive = zhPrefOn && !uiNeedsPatch && !anyInstalledMissingZh;
                return (
                  <button
                    type="button"
                    data-testid="main-chinese-ui-toggle"
                    disabled={!dashboard || togglingChineseUi || busyApp !== null}
                    onClick={() => void handleToggleMainChineseUi()}
                    title={
                      fullyActive
                        ? "三端简体中文配置与界面汉化已生效 · 点击可一键关闭并恢复原配置"
                        : "一键同步 Codex / Claude / Antigravity 简体中文配置并注入 Claude 与反重力界面汉化"
                    }
                    className={`h-6 px-2 rounded-full border text-[11px] font-medium inline-flex items-center gap-1 transition cursor-pointer disabled:opacity-60 shrink-0 ${
                      fullyActive
                        ? "bg-[var(--success-soft)] border-[var(--success-border)] text-[var(--success)] hover:opacity-90"
                        : "bg-[var(--accent-soft)] border-[var(--accent-border)] text-[var(--accent)] hover:bg-[var(--accent-border)]"
                    }`}
                  >
                    <Globe className="h-3 w-3 shrink-0" />
                    <span>
                      {togglingChineseUi
                        ? "汉化同步中..."
                        : fullyActive
                          ? "三端汉化：已开启"
                          : "一键三端汉化"}
                    </span>
                  </button>
                );
              })()}

              <button
                type="button"
                data-testid="open-model-square-btn"
                onClick={() => setModelModalApp("dsh-tako")}
                title="免登录浏览全量模型人民币实付单价、官方折扣倍率与吞吐速度"
                className="h-6 px-2 rounded-full border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] hover:border-[var(--border-strong)] text-[11px] font-medium text-[var(--text-primary)] inline-flex items-center gap-1 transition cursor-pointer shrink-0"
              >
                <LayoutGrid className="h-3 w-3 text-[var(--accent)] shrink-0" />
                <span>模型广场</span>
              </button>
            </div>

            <div className="flex items-center flex-wrap gap-2 min-w-0 max-w-full">
              {updatableApps.length >= 2 && (
                <button
                  type="button"
                  disabled={batchUpdatingApps || busyApp !== null}
                  onClick={handleBatchUpgradeApps}
                  className="px-2 py-0.5 rounded-md bg-[var(--accent-soft)] hover:bg-[var(--accent-border)] text-[var(--accent)] border border-[var(--accent-border)] text-[11px] font-medium flex items-center gap-1 transition cursor-pointer disabled:opacity-60 shrink-0"
                >
                  <ArrowUpCircle className="h-3 w-3" />
                  <span>
                    {batchUpdatingApps
                      ? "批量更新中..."
                      : `全部更新 (${updatableApps.length})`}
                  </span>
                </button>
              )}
              <span className="hidden sm:inline text-[12px] text-[var(--text-muted)] truncate max-w-full">
                官方直连 · Tako 全接管（免官方登录）
              </span>
            </div>
          </div>

          {loading ? (
            <div className="rounded-[14px] border border-[var(--border)] bg-[var(--bg-card)] p-6 text-center text-[13px] text-[var(--text-secondary)]">
              正在读取本机客户端与配置...
            </div>
          ) : (
            <div className="flex flex-col gap-2">
              {dashboard?.apps.map((app) => {
                const update = updatesMap[app.id];
                const progress = installProgress[app.id];
                const isBusy = busyApp === app.id;
                const isExpanded = expandedApp === app.id;
                const restartRequired = Boolean(needsRestart[app.id]);
                const artifactSizeLabel = formatBytes(update?.artifact?.size);

                return (
                  <div
                    key={app.id}
                    className={`rounded-[14px] border bg-[var(--bg-card)] shadow-[var(--shadow-card)] transition-colors overflow-hidden ${
                      app.running
                        ? "border-[var(--success-border)]"
                        : "border-[var(--border)] hover:border-[var(--border-strong)]"
                    }`}
                  >
                    <div
                      className={`px-3.5 py-3 flex flex-wrap sm:flex-nowrap items-center gap-x-3.5 gap-y-2.5 ${
                        app.running ? "tako-running-row" : ""
                      }`}
                    >
                      {/* App Brand & Info */}
                      <div className="w-full sm:w-auto sm:flex-1 min-w-0 flex items-center gap-3">
                        <AppBrandIcon appId={app.id} />
                        <div className="min-w-0 flex-1">
                          <div className="flex flex-wrap sm:flex-nowrap items-center gap-1.5 sm:gap-2 min-w-0">
                            <span className="text-[14px] font-semibold tracking-[-0.01em] text-[var(--text-primary)] truncate">
                              {APP_DISPLAY_NAMES[app.id] ?? app.name}
                            </span>

                            {app.install.installed ? (
                              <span className="tabular-nums text-[12px] font-medium px-2 py-0.5 rounded-md bg-[var(--bg-subtle)] text-[var(--text-secondary)] shrink-0">
                                {app.install.version
                                  ? `v${app.install.version.replace(/^v/i, "")}`
                                  : "已安装"}
                              </span>
                            ) : (
                              <span className="text-[12px] font-medium px-2 py-0.5 rounded-md bg-[var(--bg-subtle)] text-[var(--text-muted)] shrink-0">
                                未安装
                              </span>
                            )}

                            {app.running && (
                              <span
                                className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full bg-[var(--success-soft)] border border-[var(--success-border)] text-[11px] leading-[16px] font-medium text-[var(--success)] shrink-0"
                                title="应用正在运行"
                              >
                                <span className="tako-live-dot" aria-hidden="true" />
                                运行中
                              </span>
                            )}

                            {app.id !== "dsh-tako" && app.chineseConfigured && (
                              <span
                                className="inline-flex items-center px-1.5 py-0.5 rounded-md bg-[var(--bg-subtle)] text-[11px] font-medium text-[var(--text-secondary)] shrink-0"
                                title="简体中文配置与中文回复规则已写入本地"
                              >
                                中文已配
                              </span>
                            )}

                            {update?.hasUpdate && update.remoteVersion && (
                              <span
                                className="tabular-nums text-[12px] font-medium px-2 py-0.5 rounded-md bg-[var(--accent-soft)] text-[var(--accent)] shrink-0"
                                title={
                                  artifactSizeLabel
                                    ? `可更新至 v${update.remoteVersion.replace(/^v/i, "")}（约 ${artifactSizeLabel}）`
                                    : undefined
                                }
                              >
                                新版 v{update.remoteVersion.replace(/^v/i, "")}
                              </span>
                            )}
                          </div>

                          <div className="mt-0.5 flex flex-wrap sm:flex-nowrap items-center gap-x-2 gap-y-1 text-[12px] text-[var(--text-secondary)] min-w-0">
                            <span className="truncate">{modeSummary(app)}</span>
                            <span className="text-[var(--text-muted)]">·</span>
                            <button
                              type="button"
                              onClick={() => setExpandedApp(isExpanded ? null : app.id)}
                              className="inline-flex items-center gap-1 hover:text-[var(--text-primary)] transition cursor-pointer shrink-0"
                              style={{
                                color: app.diagnostic.ok ? undefined : "var(--warning)",
                              }}
                            >
                              <span>{app.diagnostic.ok ? "配置正常" : "需检查"}</span>
                              <ChevronDown
                                className={`h-3.5 w-3.5 transition-transform duration-150 ${
                                  isExpanded ? "rotate-180" : ""
                                }`}
                              />
                            </button>
                            {app.id !== "dsh-tako" && app.mode === "tako" && (
                              <>
                                <span className="text-[var(--text-muted)]">·</span>
                                <button
                                  type="button"
                                  onClick={() => setModelModalApp(app.id)}
                                  title={modelFilterPillTitle(dashboard?.state.modelFilters?.[app.id])}
                                  className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md bg-[var(--accent-soft)] hover:bg-[var(--accent-border)] text-[var(--accent)] border border-[var(--accent-border)] text-[11px] font-medium transition cursor-pointer shrink-0 tabular-nums"
                                >
                                  <SlidersHorizontal className="h-3 w-3" />
                                  <span>
                                    {modelFilterPillLabel(dashboard?.state.modelFilters?.[app.id])}
                                  </span>
                                </button>
                              </>
                            )}
                            {app.id === "claude" && (
                              <>
                                <span className="text-[var(--text-muted)]">·</span>
                                {app.uiPatch.status === "patched" ? (
                                  <span className="inline-flex items-center flex-wrap gap-1 shrink-0">
                                    <button
                                      type="button"
                                      data-testid="claude-ui-patch-btn"
                                      disabled={isBusy}
                                      onClick={() => void handlePatchAppUi(app)}
                                      title={
                                        app.uiPatch.note ??
                                        "Claude Desktop 全量简体中文界面补丁已生效"
                                      }
                                      className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md bg-[var(--success-soft)] border border-[var(--success-border)] text-[var(--success)] text-[11px] font-medium transition cursor-pointer disabled:opacity-60 shrink-0"
                                    >
                                      <CheckCircle2 className="h-3 w-3" />
                                      <span>界面已汉化</span>
                                    </button>
                                    <button
                                      type="button"
                                      data-testid="claude-ui-restore-btn"
                                      disabled={isBusy}
                                      onClick={() => void handleRestoreAppUi(app)}
                                      title="从受控备份字节级无损还原 Claude Desktop 原版安装包与签名"
                                      className="inline-flex items-center gap-0.5 px-1.5 py-0.5 rounded-md border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] text-[11px] font-medium transition cursor-pointer disabled:opacity-60 shrink-0"
                                    >
                                      <RotateCcw className="h-2.5 w-2.5" />
                                      <span>还原原版</span>
                                    </button>
                                  </span>
                                ) : app.uiPatch.status === "outdated_after_upgrade" ? (
                                  <button
                                    type="button"
                                    data-testid="claude-ui-patch-btn"
                                    disabled={isBusy}
                                    onClick={() => void handlePatchAppUi(app)}
                                    title={
                                      app.uiPatch.note ??
                                      "检测到 Claude Desktop 已升级，点击基于新版重新注入简体中文界面补丁"
                                    }
                                    className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md text-[11px] font-medium transition cursor-pointer disabled:opacity-60 shrink-0"
                                    style={{
                                      backgroundColor: "var(--warning-soft)",
                                      color: "var(--warning)",
                                    }}
                                  >
                                    <Globe className="h-3 w-3" />
                                    <span>升级后重打汉化</span>
                                  </button>
                                ) : (
                                  <button
                                    type="button"
                                    data-testid="claude-ui-patch-btn"
                                    disabled={
                                      isBusy ||
                                      !app.install.installed ||
                                      app.uiPatch.status === "not_installed" ||
                                      app.uiPatch.status === "unsupported"
                                    }
                                    onClick={() => void handlePatchAppUi(app)}
                                    title={
                                      !app.install.installed
                                        ? "安装 Claude Desktop 后可一键注入全量简体中文界面补丁"
                                        : (app.uiPatch.note ??
                                          "自动备份原版包并注入 Claude Desktop Chat / Cowork / Claude Code 全量简体中文界面")
                                    }
                                    className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md border border-[var(--border)] bg-[var(--bg-subtle)]/80 hover:bg-[var(--bg-subtle)] text-[var(--text-primary)] text-[11px] font-medium transition cursor-pointer disabled:opacity-50 shrink-0"
                                  >
                                    <Globe className="h-3 w-3 text-[var(--accent)]" />
                                    <span>
                                      {app.uiPatch.status === "unsupported"
                                        ? "暂不支持汉化"
                                        : "界面汉化"}
                                    </span>
                                  </button>
                                )}
                              </>
                            )}
                            {app.id === "antigravity" && (
                              <>
                                <span className="text-[var(--text-muted)]">·</span>
                                {app.uiPatch.status === "patched" ? (
                                  <span className="inline-flex items-center flex-wrap gap-1 shrink-0">
                                    <button
                                      type="button"
                                      data-testid="agy-ui-patch-btn"
                                      disabled={isBusy}
                                      onClick={() => void handlePatchAppUi(app)}
                                      title={
                                        app.uiPatch.note ??
                                        "Antigravity 菜单与 Hub 界面简体中文补丁已生效"
                                      }
                                      className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md bg-[var(--success-soft)] border border-[var(--success-border)] text-[var(--success)] text-[11px] font-medium transition cursor-pointer disabled:opacity-60 shrink-0"
                                    >
                                      <CheckCircle2 className="h-3 w-3" />
                                      <span>界面已汉化</span>
                                    </button>
                                    <button
                                      type="button"
                                      data-testid="agy-ui-restore-btn"
                                      disabled={isBusy}
                                      onClick={() => void handleRestoreAppUi(app)}
                                      title="从受控备份字节级无损还原 Antigravity 原版安装包与签名"
                                      className="inline-flex items-center gap-0.5 px-1.5 py-0.5 rounded-md border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] text-[11px] font-medium transition cursor-pointer disabled:opacity-60 shrink-0"
                                    >
                                      <RotateCcw className="h-2.5 w-2.5" />
                                      <span>还原原版</span>
                                    </button>
                                  </span>
                                ) : app.uiPatch.status === "outdated_after_upgrade" ? (
                                  <button
                                    type="button"
                                    data-testid="agy-ui-patch-btn"
                                    disabled={isBusy}
                                    onClick={() => void handlePatchAppUi(app)}
                                    title={
                                      app.uiPatch.note ??
                                      "检测到 Antigravity 已升级，点击基于新版重新注入简体中文界面补丁"
                                    }
                                    className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md text-[11px] font-medium transition cursor-pointer disabled:opacity-60 shrink-0"
                                    style={{
                                      backgroundColor: "var(--warning-soft)",
                                      color: "var(--warning)",
                                    }}
                                  >
                                    <Globe className="h-3 w-3" />
                                    <span>升级后重打汉化</span>
                                  </button>
                                ) : (
                                  <button
                                    type="button"
                                    data-testid="agy-ui-patch-btn"
                                    disabled={
                                      isBusy ||
                                      !app.install.installed ||
                                      app.uiPatch.status === "not_installed" ||
                                      app.uiPatch.status === "unsupported"
                                    }
                                    onClick={() => void handlePatchAppUi(app)}
                                    title={
                                      !app.install.installed
                                        ? "安装 Google Antigravity 后可一键注入菜单与界面汉化"
                                        : (app.uiPatch.note ??
                                          "自动备份原版包并注入系统菜单、托盘与 Hub 界面简体中文")
                                    }
                                    className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md border border-[var(--border)] bg-[var(--bg-subtle)]/80 hover:bg-[var(--bg-subtle)] text-[var(--text-primary)] text-[11px] font-medium transition cursor-pointer disabled:opacity-50 shrink-0"
                                  >
                                    <Globe className="h-3 w-3 text-[var(--accent)]" />
                                    <span>
                                      {app.uiPatch.status === "unsupported"
                                        ? "暂不支持汉化"
                                        : "界面汉化"}
                                    </span>
                                  </button>
                                )}
                              </>
                            )}
                          </div>
                        </div>
                      </div>

                      {/* Fixed-Width Mode Switcher (Aligned Across All Rows) */}
                      <div className="flex-1 sm:flex-none sm:w-[132px] shrink-0 grid grid-cols-2 p-0.5 rounded-[10px] bg-[var(--bg-pill)] border border-[var(--border)]">
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
                              onClick={() => handleSetMode(app, option.id)}
                              className={`h-7 rounded-[8px] text-[12px] transition cursor-pointer disabled:opacity-60 ${
                                active
                                  ? "bg-[var(--bg-card)] font-medium shadow-[var(--shadow-segment)]"
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

                      {/* Fixed-Width Primary Action Slot (Aligned Across All 3 Rows) */}
                      <div className="flex-1 sm:flex-none sm:w-[126px] shrink-0 flex items-center justify-end gap-1.5">
                        {progress ? (
                          progress.stage === "downloading" ? (
                            <div className="w-full h-8 px-2 rounded-[9px] bg-[var(--accent-soft)] border border-[var(--accent-border)] flex items-center justify-between gap-1 text-[12px] font-medium text-[var(--accent)] tabular-nums">
                              <span className="truncate">
                                {installStageLabel(progress.stage)} {Math.round(progress.percent)}%
                              </span>
                              <button
                                type="button"
                                disabled={cancellingApp === app.id}
                                onClick={() => handleCancelAppInstall(app.id)}
                                className="px-1.5 py-0.5 rounded bg-[var(--bg-card)]/70 hover:bg-[var(--bg-card)] text-[var(--text-secondary)] hover:text-[var(--danger)] text-[11px] shrink-0 cursor-pointer disabled:opacity-50"
                              >
                                {cancellingApp === app.id ? "..." : "取消"}
                              </button>
                            </div>
                          ) : (
                            <div className="w-full h-8 px-2.5 rounded-[9px] bg-[var(--accent-soft)] border border-[var(--accent-border)] flex items-center justify-between text-[12px] font-medium text-[var(--accent)] tabular-nums">
                              <span className="truncate">{installStageLabel(progress.stage)}</span>
                              <span>{Math.round(progress.percent)}%</span>
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
                              disabled={isBusy}
                              onClick={() => void handleInstallOrUpgrade(app.id)}
                              className="w-full h-8 rounded-[9px] text-[12px] font-medium flex items-center justify-center gap-1.5 transition cursor-pointer disabled:opacity-60"
                              style={{
                                backgroundColor: "var(--btn-dark-bg)",
                                color: "var(--btn-dark-fg)",
                              }}
                            >
                              <Download className="h-3.5 w-3.5" />
                              <span>一键安装</span>
                            </button>
                          )
                        ) : update?.hasUpdate ? (
                          <>
                            <button
                              type="button"
                              disabled={isBusy}
                              onClick={() => void handleInstallOrUpgrade(app.id)}
                              className="flex-1 h-8 rounded-[9px] bg-[var(--accent-soft)] hover:bg-[var(--accent-border)] text-[var(--accent)] border border-[var(--accent-border)] text-[12px] font-medium flex items-center justify-center gap-1 transition cursor-pointer disabled:opacity-60"
                            >
                              <ArrowUpCircle className="h-3.5 w-3.5" />
                              <span>升级</span>
                            </button>
                            <button
                              type="button"
                              disabled={isBusy}
                              onClick={() =>
                                restartRequired
                                  ? handleRestartApp(app.id)
                                  : app.running
                                    ? handleCloseApp(app.id)
                                    : handleLaunchApp(app.id)
                              }
                              title={
                                restartRequired
                                  ? "重启生效"
                                  : app.running
                                    ? "关闭应用"
                                    : "打开应用"
                              }
                              className="h-8 px-2.5 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-primary)] text-[12px] font-medium flex items-center justify-center transition cursor-pointer disabled:opacity-60"
                            >
                              {restartRequired
                                ? "重启"
                                : app.running
                                  ? "关闭"
                                  : "打开"}
                            </button>
                          </>
                        ) : restartRequired ? (
                          <button
                            type="button"
                            disabled={isBusy}
                            onClick={() => handleRestartApp(app.id)}
                            className="w-full h-8 rounded-[9px] text-[12px] font-medium flex items-center justify-center gap-1.5 transition cursor-pointer disabled:opacity-60"
                            style={{
                              backgroundColor: "var(--warning-soft)",
                              color: "var(--warning)",
                            }}
                          >
                            <RotateCw className="h-3.5 w-3.5" />
                            <span>重启生效</span>
                          </button>
                        ) : (
                          <button
                            type="button"
                            disabled={isBusy}
                            onClick={() =>
                              app.running
                                ? handleCloseApp(app.id)
                                : handleLaunchApp(app.id)
                            }
                            title={app.running ? "关闭应用" : "打开应用"}
                            className="w-full h-8 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[var(--text-primary)] text-[12px] font-medium flex items-center justify-center gap-1.5 transition cursor-pointer disabled:opacity-60"
                          >
                            {app.running ? (
                              <Power className="h-3.5 w-3.5 text-[var(--text-secondary)]" />
                            ) : (
                              <ExternalLink className="h-3.5 w-3.5 text-[var(--text-secondary)]" />
                            )}
                            <span>{app.running ? "关闭" : "打开"}</span>
                          </button>
                        )}
                      </div>
                    </div>

                    {/* Inline Progress Bar */}
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
                        <span className="tabular-nums text-[12px] text-[var(--text-secondary)] truncate">
                          {progress.message}
                        </span>
                      </div>
                    )}

                    {/* Inline Expandable Detail Strip */}
                    {isExpanded && (
                      <div className="border-t border-[var(--border)] bg-[var(--bg-subtle)]/55 px-3.5 py-2.5 text-[12px] flex flex-col gap-1.5">
                        <div className="flex flex-wrap sm:flex-nowrap items-center justify-between gap-2 min-w-0">
                          <div className="min-w-0 flex items-center gap-2 text-[var(--text-secondary)] max-w-full">
                            <span className="text-[var(--text-muted)] shrink-0">安装位置:</span>
                            <code className="truncate font-mono text-[var(--text-primary)]">
                              {app.install.installPath ?? "未检测到应用路径"}
                            </code>
                          </div>
                          <div className="flex flex-wrap items-center gap-2 shrink-0">
                            {update?.hasUpdate && artifactSizeLabel && (
                              <span className="tabular-nums text-[var(--accent)]">
                                更新包 {artifactSizeLabel}
                              </span>
                            )}
                            <span className="text-[var(--text-muted)]">
                              {app.hasBackup ? "已保留配置备份" : "尚无备份"}
                            </span>
                            <button
                              type="button"
                              onClick={() => handleRediagnose(app.id)}
                              className="text-[var(--accent)] hover:underline cursor-pointer font-medium"
                            >
                              重新检测
                            </button>
                          </div>
                        </div>

                        {app.diagnostic.issues.length > 0 && (
                          <div className="flex flex-col gap-0.5 text-[var(--warning)]">
                            {app.diagnostic.issues.map((issue, idx) => (
                              <div key={idx}>• {issue}</div>
                            ))}
                          </div>
                        )}

                        {app.diagnostic.notes.length > 0 && (
                          <div className="flex flex-col gap-0.5 text-[var(--text-secondary)]">
                            {app.diagnostic.notes.map((note, idx) => (
                              <div key={idx} className="truncate">
                                • {note}
                              </div>
                            ))}
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </section>

        {/* Super MCP Card */}
        <section className="rounded-[14px] border border-[var(--border)] bg-[var(--bg-card)] shadow-[var(--shadow-card)] p-3.5 min-w-0">
          <div className="flex flex-wrap items-center justify-between gap-2 mb-2.5">
            <div className="flex items-center gap-2 min-w-0 flex-wrap sm:flex-nowrap">
              <Sparkles className="h-4 w-4 text-[var(--accent)] shrink-0" />
              <span className="text-[13px] font-semibold text-[var(--text-primary)] shrink-0">
                Tako 超级 MCP
              </span>
              <span className="text-[12px] text-[var(--text-muted)] truncate">
                Tako 模式下自动注入，开关即时生效
              </span>
            </div>

            <div className="flex items-center gap-2 shrink-0">
              <span className="tabular-nums text-[12px] font-medium text-[var(--text-secondary)]">
                {mcpEnabledCount}/5 已开启
              </span>
              <button
                type="button"
                data-testid="open-mcp-demo"
                onClick={toggleMcpTheater}
                className={`h-7 px-2.5 rounded-full border text-[11.5px] font-medium inline-flex items-center gap-1 transition cursor-pointer ${
                  theaterOpen
                    ? "bg-[var(--bg-subtle)] hover:bg-[var(--bg-hover)] border-[var(--border-strong)] text-[var(--text-primary)]"
                    : "bg-[var(--accent-soft)] hover:bg-[var(--accent-border)] border-[var(--accent-border)] text-[var(--accent)]"
                }`}
              >
                {theaterOpen ? (
                  <>
                    <ChevronUp className="h-3.5 w-3.5 shrink-0" />
                    <span>收起剧场</span>
                  </>
                ) : (
                  <>
                    <Sparkles className="h-3.5 w-3.5 shrink-0" />
                    <span>实景体验</span>
                  </>
                )}
              </button>
            </div>
          </div>

          <div className="grid grid-cols-2 sm:grid-cols-5 gap-2">
            {MCP_TOOLS.map(({ key, toolId, label, subtitle, Icon }) => {
              const enabled = Boolean(dashboard?.state.mcp[key]);
              const interactive = Boolean(dashboard) && !savingMcp;
              const isSelected = theaterOpen && theaterTab === key;
              return (
                <div
                  key={key}
                  role="button"
                  tabIndex={0}
                  data-mcp-tool={toolId}
                  title={`${label} · ${subtitle}`}
                  onClick={() => openMcpDemo(key)}
                  onMouseEnter={() => handleMcpCardHoverEnter(key)}
                  onMouseLeave={handleMcpCardHoverLeave}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      openMcpDemo(key);
                    }
                  }}
                  className={`relative rounded-[10px] border px-2.5 py-2 flex flex-col justify-between gap-1.5 transition-all duration-200 select-none min-w-0 cursor-pointer ${
                    isSelected
                      ? "-translate-y-[1.5px] border-[var(--accent)] bg-[var(--accent-soft)] text-[var(--accent)] shadow-[0_6px_16px_-8px_var(--accent)]"
                      : enabled
                        ? "bg-[var(--accent-soft)]/55 border-[var(--accent-border)] text-[var(--accent)] hover:bg-[var(--accent-soft)]"
                        : "bg-[var(--bg-subtle)]/60 border-[var(--border)] text-[var(--text-secondary)] hover:bg-[var(--bg-subtle)] hover:text-[var(--text-primary)]"
                  }`}
                >
                  {isSelected && (
                    <span
                      aria-hidden="true"
                      className="pointer-events-none absolute inset-x-2 bottom-0 h-[2px] rounded-full bg-gradient-to-r from-transparent via-[var(--accent)] to-transparent"
                    />
                  )}

                  <div className="flex items-center justify-between gap-1.5 min-w-0">
                    <span className="flex items-center gap-1.5 min-w-0 text-[12px] font-semibold">
                      <Icon className="h-3.5 w-3.5 shrink-0" />
                      <span className="truncate">{label}</span>
                    </span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={enabled}
                      aria-label={`切换${label}开关`}
                      data-testid={`toggle-mcp-${key}`}
                      disabled={!interactive}
                      onClick={(e) => {
                        e.stopPropagation();
                        if (interactive) void handleToggleMcpTool(key);
                      }}
                      title={enabled ? "已开启 · 点击关闭" : "已关闭 · 点击开启"}
                      className={`h-[18px] px-1.5 rounded-full border inline-flex items-center gap-1 shrink-0 transition cursor-pointer disabled:opacity-50 ${
                        enabled
                          ? "bg-[var(--success-soft)] border-[var(--success-border)]"
                          : "bg-[var(--bg-card)]/80 border-[var(--border-strong)]"
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
                      {subtitle}
                    </span>
                    <button
                      type="button"
                      data-testid={`open-mcp-demo-${key}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        openMcpDemo(key);
                      }}
                      className={`px-1.5 py-0.5 rounded border text-[10px] font-medium shrink-0 transition cursor-pointer ${
                        isSelected
                          ? "bg-[var(--accent)] border-transparent text-white"
                          : "bg-[var(--bg-card)]/90 hover:bg-[var(--bg-card)] border-[var(--border)] text-[var(--text-secondary)] hover:text-[var(--accent)]"
                      }`}
                    >
                      体验
                    </button>
                  </div>
                </div>
              );
            })}
          </div>

          <div
            ref={theaterRef}
            data-open={theaterOpen ? "true" : "false"}
            aria-hidden={!theaterOpen}
            className="mcp-theater-collapse"
          >
            <div className="mcp-theater-inner">
              <McpInlineTheater
                open={theaterOpen}
                activeTab={theaterTab}
                mcp={dashboard?.state.mcp ?? null}
                savingMcp={savingMcp}
                loggedIn={Boolean(dashboard?.account?.loggedIn)}
                loggingIn={loggingIn}
                onSelectTab={(tab) => setTheaterTab(tab)}
                onToggleMcp={(key) => void handleToggleMcpTool(key)}
                onSelectConsultModel={(model) => void handleSelectConsultModel(model)}
                onRequestLogin={() => void handleOAuthLogin()}
              />
            </div>
          </div>
        </section>
      </main>

      {/* Slim Bottom Status Bar */}
      <footer className="min-h-8 sm:h-8 shrink-0 px-4 py-1.5 sm:py-0 border-t border-[var(--border)] bg-[var(--bg-card)]/75 flex flex-wrap sm:flex-nowrap items-center justify-between gap-x-3 gap-y-1 text-[12px] text-[var(--text-secondary)]">
        <div className="flex items-center gap-2 min-w-0 max-w-full">
          {!anyTakoMode ? (
            <span className="inline-flex items-center gap-1.5 text-[var(--text-muted)] truncate">
              <span className="h-1.5 w-1.5 rounded-full bg-[var(--text-muted)] shrink-0" />
              <span className="truncate">本地网关待机</span>
            </span>
          ) : dashboard?.localGatewayHealthy ? (
            <span className="inline-flex items-center gap-1.5 text-[var(--text-secondary)] tabular-nums min-w-0">
              <span className="h-1.5 w-1.5 rounded-full bg-[var(--success)] shrink-0" />
              <span className="truncate">本地网关 127.0.0.1:18419</span>
              <span className="text-[var(--success)] font-medium shrink-0">· 运行中</span>
            </span>
          ) : (
            <button
              type="button"
              disabled={fixingGateway}
              onClick={handleEnsureGateway}
              className="inline-flex items-center gap-1.5 text-[var(--warning)] hover:underline cursor-pointer disabled:opacity-60 min-w-0"
            >
              <span className="h-1.5 w-1.5 rounded-full bg-[var(--warning)] shrink-0" />
              <span className="truncate">
                {fixingGateway ? "正在修复网关..." : "本地网关未运行 · 点击修复"}
              </span>
            </button>
          )}
        </div>

        <div className="flex items-center gap-3 shrink-0">
          <button
            type="button"
            onClick={openSettings}
            className="text-[var(--text-muted)] hover:text-[var(--text-primary)] tabular-nums transition cursor-pointer"
          >
            {checkingUpdates
              ? "检查更新中..."
              : selfUpdateStatus === "available" && selfUpdate
                ? `Tako Switch v${selfUpdate.version} 可更新`
                : updatableApps.length > 0
                  ? `${updatableApps.length} 款客户端可更新`
                  : lastCheckedAt
                    ? `${lastCheckedAt} 已检查更新`
                    : "检查更新"}
          </button>
          <span className="h-3 w-px bg-[var(--border)]" />
          <button
            type="button"
            onClick={() => void api.openUrl(TAKO_FEEDBACK_URL)}
            className="inline-flex items-center gap-1 text-[var(--text-muted)] hover:text-[var(--text-primary)] transition cursor-pointer"
          >
            <MessageSquare className="h-3 w-3" />
            <span>问题反馈</span>
          </button>
        </div>
      </footer>

      {/* API Key Login Modal */}
      {showKeyModal && (
        <div
          className="fixed inset-0 z-50 modal-backdrop flex items-center justify-center p-4"
          onClick={() => setShowKeyModal(false)}
        >
          <form
            onSubmit={handleKeyLogin}
            onClick={(e) => e.stopPropagation()}
            className="w-full max-w-[380px] rounded-[16px] border border-[var(--border)] bg-[var(--bg-card)] p-5 shadow-[var(--shadow-modal)] flex flex-col gap-3.5"
          >
            <div className="flex items-center justify-between">
              <h3 className="text-[14px] font-semibold text-[var(--text-primary)]">
                使用 Tako API Key 登录
              </h3>
              <button
                type="button"
                onClick={() => setShowKeyModal(false)}
                className="p-1 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] cursor-pointer"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="flex flex-col gap-1.5">
              <label className="text-[12px] text-[var(--text-secondary)]">API Key</label>
              <input
                type="password"
                autoFocus
                placeholder="sk-..."
                value={apiKeyInput}
                onChange={(e) => setApiKeyInput(e.target.value)}
                className="h-9 px-3 rounded-[10px] border border-[var(--border)] bg-[var(--bg-canvas)] text-[13px] text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
              />
            </div>

            <div className="flex items-center justify-end gap-2 pt-1">
              <button
                type="button"
                onClick={() => setShowKeyModal(false)}
                className="h-8 px-3 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-secondary)] cursor-pointer"
              >
                取消
              </button>
              <button
                type="submit"
                disabled={loggingIn || !apiKeyInput.trim()}
                className="h-8 px-3.5 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium cursor-pointer disabled:opacity-60"
              >
                {loggingIn ? "验证中..." : "确认登录"}
              </button>
            </div>
          </form>
        </div>
      )}

      {/* Clean macOS-Style Grouped Settings Modal */}
      {showSettingsModal && (
        <div
          className="fixed inset-0 z-50 modal-backdrop flex items-center justify-center p-4"
          onClick={() => setShowSettingsModal(false)}
        >
          <form
            onSubmit={handleSaveSettings}
            onClick={(e) => e.stopPropagation()}
            className="w-full max-w-[448px] max-h-[88vh] rounded-[16px] border border-[var(--border)] bg-[var(--bg-card)] shadow-[var(--shadow-modal)] flex flex-col overflow-hidden"
          >
            {/* Modal Header */}
            <div className="px-5 py-3.5 border-b border-[var(--border)] flex items-center justify-between shrink-0">
              <h3 className="text-[14px] font-semibold text-[var(--text-primary)]">偏好设置</h3>
              <button
                type="button"
                onClick={() => setShowSettingsModal(false)}
                className="p-1 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)] cursor-pointer"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            {/* Scrollable Settings Body */}
            <div className="flex-1 min-h-0 overflow-y-auto subtle-scroll p-4 flex flex-col gap-4 text-[12px]">
              {/* Section 1: General & Appearance (Super Simple) */}
              <div className="flex flex-col gap-1.5">
                <div className="px-1 text-[11px] font-semibold uppercase tracking-wider text-[var(--text-muted)]">
                  外观与常用
                </div>
                <div className="settings-group">
                  <div className="settings-row px-3.5 py-2.5 flex items-center justify-between gap-3">
                    <span className="text-[13px] text-[var(--text-primary)]">主题外观</span>
                    <div className="grid grid-cols-3 p-0.5 rounded-[9px] bg-[var(--bg-pill)] border border-[var(--border)]">
                      {THEME_OPTIONS.map((opt) => {
                        const active = themePref === opt.id;
                        return (
                          <button
                            key={opt.id}
                            type="button"
                            onClick={() => setThemePref(opt.id)}
                            className={`px-2.5 py-1 rounded-[7px] text-[12px] transition cursor-pointer ${
                              active
                                ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-medium shadow-[var(--shadow-segment)]"
                                : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                            }`}
                          >
                            {opt.label}
                          </button>
                        );
                      })}
                    </div>
                  </div>

                  <div className="settings-row px-3.5 py-2.5 flex items-center justify-between gap-3">
                    <span className="text-[13px] text-[var(--text-primary)]">
                      菜单栏显示实时余额
                    </span>
                    <ToggleSwitch
                      checked={settingsForm.showTrayBalance}
                      ariaLabel="菜单栏显示实时余额"
                      onChange={(next) =>
                        setSettingsForm((s) => ({ ...s, showTrayBalance: next }))
                      }
                    />
                  </div>

                  <div className="settings-row px-3.5 py-2.5 flex items-center justify-between gap-3">
                    <div className="min-w-0">
                      <div className="text-[13px] text-[var(--text-primary)]">
                        客户端优先简体中文
                      </div>
                      <div className="text-[11px] text-[var(--text-muted)]">
                        Codex / Claude 界面与中文回复 · 反重力系统菜单与中文交互
                      </div>
                    </div>
                    <ToggleSwitch
                      checked={settingsForm.preferChineseUi}
                      ariaLabel="客户端优先简体中文"
                      onChange={(next) =>
                        setSettingsForm((s) => ({ ...s, preferChineseUi: next }))
                      }
                    />
                  </div>

                  <div className="settings-row px-3.5 py-2.5 flex items-center justify-between gap-3">
                    <div className="min-w-0">
                      <div className="text-[13px] text-[var(--text-primary)]">
                        直连 Tako 网关（绕过系统代理）
                      </div>
                      <div className="text-[11px] text-[var(--text-muted)]">
                        默认不走系统代理直连网关，避免全局代理拖慢大请求上传与流式响应
                      </div>
                    </div>
                    <ToggleSwitch
                      checked={settingsForm.bypassSystemProxy}
                      ariaLabel="直连 Tako 网关（绕过系统代理）"
                      testId="settings-bypass-system-proxy-toggle"
                      onChange={(next) =>
                        setSettingsForm((s) => ({ ...s, bypassSystemProxy: next }))
                      }
                    />
                  </div>

                  <div className="settings-row px-3.5 py-2.5 flex items-center justify-between gap-3">
                    <span className="text-[13px] text-[var(--text-primary)]">
                      启动时自动检查更新
                    </span>
                    <ToggleSwitch
                      checked={settingsForm.autoCheckUpdates}
                      ariaLabel="启动时自动检查更新"
                      onChange={(next) =>
                        setSettingsForm((s) => ({ ...s, autoCheckUpdates: next }))
                      }
                    />
                  </div>

                  <div className="settings-row px-3.5 py-2.5 flex items-center justify-between gap-3">
                    <span className="text-[13px] text-[var(--text-primary)] shrink-0">
                      网关显示名称
                    </span>
                    <input
                      type="text"
                      maxLength={60}
                      placeholder="Tako"
                      value={settingsForm.gatewayDisplayName}
                      onChange={(e) =>
                        setSettingsForm((s) => ({
                          ...s,
                          gatewayDisplayName: e.target.value,
                        }))
                      }
                      className="w-[160px] h-7 px-2.5 rounded-[7px] border border-[var(--border)] bg-[var(--bg-card)] text-[12px] text-[var(--text-primary)] text-right focus:outline-none focus:border-[var(--accent)]"
                    />
                  </div>
                </div>
              </div>

              {/* Section 2: Unified Updates Card */}
              <div className="flex flex-col gap-1.5">
                <div className="px-1 flex items-center justify-between">
                  <span className="text-[11px] font-semibold uppercase tracking-wider text-[var(--text-muted)]">
                    软件更新
                  </span>
                  {lastCheckedAt && (
                    <span className="text-[11px] text-[var(--text-muted)] tabular-nums">
                      上次检查 {lastCheckedAt}
                    </span>
                  )}
                </div>

                <div className="settings-group">
                  {/* Self Update Row */}
                  <div className="settings-row px-3.5 py-2.5 flex flex-col gap-2">
                    <div className="flex items-center justify-between gap-2">
                      <div className="min-w-0">
                        <div className="flex items-center gap-2 flex-wrap">
                          <span className="text-[13px] font-medium text-[var(--text-primary)]">
                            Tako Switch
                          </span>
                          <span className="tabular-nums text-[11px] px-1.5 py-0.5 rounded bg-[var(--bg-subtle)] text-[var(--text-secondary)]">
                            v{dashboard?.appVersion ?? "2.0.3"}
                          </span>
                          <button
                            type="button"
                            onClick={() => setShowChangelogModal(true)}
                            className="inline-flex items-center gap-1 text-[11px] text-[var(--accent)] hover:underline cursor-pointer"
                          >
                            <FileText className="h-3 w-3" />
                            <span>更新日志</span>
                          </button>
                          {selfUpdateStatus === "latest" && (
                            <span className="inline-flex items-center gap-1 text-[11px] text-[var(--success)] font-medium">
                              <CheckCircle2 className="h-3 w-3" />
                              已是最新
                            </span>
                          )}
                          {selfUpdateStatus === "available" && selfUpdate && (
                            <span className="inline-flex items-center gap-1.5 text-[11px] text-[var(--accent)] font-medium">
                              <span>新版 v{selfUpdate.version}</span>
                              <span className="px-1.5 py-0.2 rounded bg-[var(--accent-soft)] border border-[var(--accent-border)] text-[10px]">
                                {selfUpdate.source === "github" ? "GitHub 源" : "国内加速源"}
                              </span>
                            </span>
                          )}
                        </div>
                        {selfUpdateError && (
                          <div className="text-[11px] text-[var(--danger)] truncate mt-0.5">
                            {selfUpdateError}
                          </div>
                        )}
                      </div>

                      {selfUpdateStatus === "downloading" || selfUpdateStatus === "installing" ? (
                        <span className="tabular-nums text-[12px] font-medium text-[var(--accent)] shrink-0">
                          {selfUpdateStatus === "installing"
                            ? "安装重启中..."
                            : `${selfUpdateProgress?.percent ?? 0}%`}
                        </span>
                      ) : selfUpdate ? (
                        <div className="flex items-center gap-1.5 shrink-0">
                          {selfUpdateStatus === "error" && (
                            <button
                              type="button"
                              onClick={() => void api.openUrl(TAKO_RELEASES_URL)}
                              className="h-7 px-2 rounded-[7px] border border-[var(--border)] bg-[var(--bg-card)] text-[12px] text-[var(--text-secondary)] hover:text-[var(--text-primary)] cursor-pointer"
                            >
                              下载页
                            </button>
                          )}
                          <button
                            type="button"
                            onClick={handleInstallSelfUpdate}
                            className="h-7 px-2.5 rounded-[7px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium cursor-pointer"
                          >
                            {selfUpdateStatus === "error" ? "重试更新" : "立即更新"}
                          </button>
                        </div>
                      ) : (
                        <button
                          type="button"
                          disabled={checkingUpdates}
                          onClick={() => void runUpdateChecks(false)}
                          className="h-7 px-2.5 rounded-[7px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-primary)] cursor-pointer disabled:opacity-60 shrink-0"
                        >
                          {checkingUpdates ? "检查中..." : "检查更新"}
                        </button>
                      )}
                    </div>

                    {selfUpdate &&
                      selfUpdateStatus !== "downloading" &&
                      selfUpdateStatus !== "installing" &&
                      selfUpdateNoteLines.length > 0 && (
                        <ul className="rounded-[8px] bg-[var(--bg-subtle)]/75 border border-[var(--border)] px-2.5 py-2 text-[11px] text-[var(--text-secondary)] space-y-1 leading-relaxed">
                          {selfUpdateNoteLines.map((line, idx) => (
                            <li key={idx} className="flex items-start gap-1.5">
                              <span className="text-[var(--accent)] select-none shrink-0">•</span>
                              <span>{line}</span>
                            </li>
                          ))}
                        </ul>
                      )}

                    {(selfUpdateStatus === "downloading" || selfUpdateStatus === "installing") &&
                      selfUpdateProgress && (
                        <div className="h-1 w-full rounded-full bg-[var(--bg-subtle)] overflow-hidden">
                          <div
                            className="h-full rounded-full bg-[var(--accent)] transition-all duration-150"
                            style={{ width: `${Math.max(4, selfUpdateProgress.percent)}%` }}
                          />
                        </div>
                      )}
                  </div>

                  {/* Desktop Apps Row */}
                  <div className="settings-row px-3.5 py-2.5 flex items-center justify-between gap-2">
                    <div className="min-w-0">
                      <div className="text-[13px] text-[var(--text-primary)]">
                        桌面客户端版本
                      </div>
                      <div className="text-[11px] text-[var(--text-muted)]">
                        {updatableApps.length > 0
                          ? `${updatableApps.map((a) => APP_DISPLAY_NAMES[a.id]).join(" · ")} 可更新`
                          : Object.keys(updatesMap).length > 0
                            ? "所有已安装客户端均为最新版"
                            : "同步官方清单，支持一键覆盖升级"}
                      </div>
                    </div>

                    {updatableApps.length > 0 ? (
                      <button
                        type="button"
                        disabled={batchUpdatingApps || busyApp !== null}
                        onClick={() => {
                          setShowSettingsModal(false);
                          void handleBatchUpgradeApps();
                        }}
                        className="h-7 px-2.5 rounded-[7px] bg-[var(--accent-soft)] hover:bg-[var(--accent-border)] text-[var(--accent)] border border-[var(--accent-border)] text-[12px] font-medium cursor-pointer disabled:opacity-60 shrink-0"
                      >
                        全部更新 ({updatableApps.length})
                      </button>
                    ) : (
                      <button
                        type="button"
                        disabled={checkingUpdates}
                        onClick={() => void runUpdateChecks(false)}
                        className="h-7 px-2.5 rounded-[7px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-primary)] cursor-pointer disabled:opacity-60 shrink-0"
                      >
                        {checkingUpdates ? "检查中..." : "检查客户端"}
                      </button>
                    )}
                  </div>
                </div>
              </div>

              {/* Section 3: Collapsible Advanced Settings */}
              <div className="flex flex-col gap-1.5">
                <button
                  type="button"
                  onClick={() => setShowAdvancedSettings((v) => !v)}
                  className="px-1 py-0.5 flex items-center justify-between text-[12px] font-medium text-[var(--text-secondary)] hover:text-[var(--text-primary)] cursor-pointer"
                >
                  <span>高级设置（网关地址 · 专用 Key · Manifest 源）</span>
                  <ChevronDown
                    className={`h-3.5 w-3.5 transition-transform duration-150 ${
                      showAdvancedSettings ? "rotate-180" : ""
                    }`}
                  />
                </button>

                {showAdvancedSettings && (
                  <div className="settings-group p-3 flex flex-col gap-2.5">
                    <label className="flex flex-col gap-1">
                      <span className="text-[var(--text-secondary)]">Tako 服务端地址</span>
                      <input
                        type="url"
                        value={settingsForm.baseUrl}
                        onChange={(e) =>
                          setSettingsForm((s) => ({ ...s, baseUrl: e.target.value }))
                        }
                        className="h-8 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] text-[12px] text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
                      />
                    </label>

                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
                      <label className="flex flex-col gap-1">
                        <span className="text-[var(--text-secondary)]">Claude 网关副标题</span>
                        <input
                          type="text"
                          maxLength={60}
                          placeholder="Tako AI Gateway"
                          value={settingsForm.gatewayDisplaySubtitle}
                          onChange={(e) =>
                            setSettingsForm((s) => ({
                              ...s,
                              gatewayDisplaySubtitle: e.target.value,
                            }))
                          }
                          className="h-8 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] text-[12px] text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
                        />
                      </label>

                      <label className="flex flex-col gap-1">
                        <span className="text-[var(--text-secondary)]">
                          Antigravity 专用 Key
                        </span>
                        <input
                          type="password"
                          placeholder="留空沿用主 Key"
                          value={settingsForm.agyApiKey}
                          onChange={(e) =>
                            setSettingsForm((s) => ({ ...s, agyApiKey: e.target.value }))
                          }
                          className="h-8 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] text-[12px] text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
                        />
                      </label>
                    </div>

                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
                      <label className="flex flex-col gap-1">
                        <span className="text-[var(--text-secondary)]">媒体输出目录</span>
                        <input
                          type="text"
                          value={settingsForm.outputDir}
                          onChange={(e) =>
                            setSettingsForm((s) => ({ ...s, outputDir: e.target.value }))
                          }
                          className="h-8 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] text-[12px] text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
                        />
                      </label>

                      <label className="flex flex-col gap-1">
                        <span className="text-[var(--text-secondary)]">外脑顾问模型</span>
                        <input
                          type="text"
                          value={settingsForm.consultModel}
                          onChange={(e) =>
                            setSettingsForm((s) => ({ ...s, consultModel: e.target.value }))
                          }
                          className="h-8 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] text-[12px] text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
                        />
                      </label>
                    </div>

                    <div className="flex items-center gap-1.5 flex-wrap">
                      <span className="text-[11px] text-[var(--text-muted)] shrink-0">
                        外脑预设:
                      </span>
                      {CONSULT_MODEL_PRESETS.map((preset) => {
                        const active = settingsForm.consultModel.trim() === preset;
                        return (
                          <button
                            key={preset}
                            type="button"
                            onClick={() =>
                              setSettingsForm((s) => ({ ...s, consultModel: preset }))
                            }
                            className={`px-2 py-0.5 rounded-md text-[11px] font-mono border transition cursor-pointer ${
                              active
                                ? "bg-[var(--accent-soft)] border-[var(--accent-border)] text-[var(--accent)] font-medium"
                                : "bg-[var(--bg-card)] border-[var(--border)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)]"
                            }`}
                          >
                            {preset}
                          </button>
                        );
                      })}
                    </div>

                    <div className="flex items-center justify-between gap-3 pt-1">
                      <span className="text-[var(--text-secondary)] leading-snug">
                        Codex 切 Tako 时保留官方 ChatGPT 登录（可保留远程操控/官方插件）
                      </span>
                      <ToggleSwitch
                        checked={settingsForm.codexPreserveOfficialAuth}
                        ariaLabel="Codex 保留官方 ChatGPT 登录"
                        onChange={(next) =>
                          setSettingsForm((s) => ({
                            ...s,
                            codexPreserveOfficialAuth: next,
                          }))
                        }
                      />
                    </div>

                    <div className="pt-1 border-t border-[var(--border)] flex flex-col gap-2">
                      <div className="flex items-center justify-between">
                        <span className="text-[var(--text-secondary)]">客户端 Manifest 更新源</span>
                        <button
                          type="button"
                          onClick={() =>
                            setSettingsForm((s) => ({
                              ...s,
                              manifestUrl: DEFAULT_MANIFEST_URL,
                              fallbackManifestUrl: DEFAULT_FALLBACK_MANIFEST_URL,
                            }))
                          }
                          className="text-[11px] text-[var(--accent)] hover:underline cursor-pointer"
                        >
                          恢复默认源
                        </button>
                      </div>
                      <input
                        type="url"
                        value={settingsForm.manifestUrl}
                        placeholder="主更新源 URL"
                        onChange={(e) =>
                          setSettingsForm((s) => ({ ...s, manifestUrl: e.target.value }))
                        }
                        className="h-8 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] text-[11px] text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
                      />
                      <input
                        type="url"
                        value={settingsForm.fallbackManifestUrl}
                        placeholder="备用更新源 URL"
                        onChange={(e) =>
                          setSettingsForm((s) => ({
                            ...s,
                            fallbackManifestUrl: e.target.value,
                          }))
                        }
                        className="h-8 px-2.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] text-[11px] text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
                      />
                    </div>
                  </div>
                )}
              </div>
            </div>

            {/* Modal Footer */}
            <div className="px-5 py-3 border-t border-[var(--border)] flex items-center justify-between gap-2 shrink-0 bg-[var(--bg-card)]">
              <button
                type="button"
                onClick={() => void api.openUrl(TAKO_FEEDBACK_URL)}
                className="inline-flex items-center gap-1.5 text-[12px] text-[var(--text-muted)] hover:text-[var(--text-primary)] cursor-pointer"
              >
                <MessageSquare className="h-3.5 w-3.5" />
                <span>问题反馈</span>
              </button>

              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => setShowSettingsModal(false)}
                  className="h-8 px-3 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-secondary)] cursor-pointer"
                >
                  取消
                </button>
                <button
                  type="submit"
                  disabled={savingSettings}
                  className="h-8 px-3.5 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-medium cursor-pointer disabled:opacity-60"
                >
                  {savingSettings ? "保存中..." : "保存设置"}
                </button>
              </div>
            </div>
          </form>
        </div>
      )}

      {showChangelogModal && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-[2px] p-4"
          onClick={() => setShowChangelogModal(false)}
        >
          <div
            className="w-full max-w-[520px] max-h-[82vh] flex flex-col rounded-[16px] border border-[var(--border)] bg-[var(--bg-card)] shadow-2xl overflow-hidden"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="px-5 py-3.5 border-b border-[var(--border)] flex items-center justify-between gap-2 shrink-0">
              <div className="flex items-center gap-2">
                <FileText className="h-4 w-4 text-[var(--accent)]" />
                <h2 className="text-[14px] font-semibold text-[var(--text-primary)]">
                  更新日志
                </h2>
                {dashboard?.appVersion && (
                  <span className="tabular-nums text-[11px] px-1.5 py-0.5 rounded bg-[var(--bg-subtle)] text-[var(--text-secondary)]">
                    当前 v{dashboard.appVersion}
                  </span>
                )}
              </div>
              <button
                type="button"
                aria-label="关闭更新日志"
                onClick={() => setShowChangelogModal(false)}
                className="p-1 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)] cursor-pointer"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="flex-1 overflow-y-auto px-5 py-4 flex flex-col gap-3.5">
              {selfUpdate &&
                !CHANGELOG_RELEASES.some((rel) => rel.version === selfUpdate.version) && (
                  <div className="rounded-[12px] border border-[var(--accent-border)] bg-[var(--accent-soft)]/70 p-3.5 flex flex-col gap-2">
                    <div className="flex items-center justify-between gap-2">
                      <div className="flex items-center gap-2">
                        <span className="text-[13px] font-semibold text-[var(--accent)] tabular-nums">
                          v{selfUpdate.version}
                        </span>
                        <span className="text-[10px] px-1.5 py-0.5 rounded bg-[var(--accent)] text-white font-medium">
                          新版本可更新
                        </span>
                      </div>
                      {selfUpdateStatus !== "downloading" &&
                        selfUpdateStatus !== "installing" && (
                          <button
                            type="button"
                            onClick={() => {
                              setShowChangelogModal(false);
                              void handleInstallSelfUpdate();
                            }}
                            className="h-6 px-2.5 rounded-md bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[11px] font-medium cursor-pointer"
                          >
                            立即更新
                          </button>
                        )}
                    </div>
                    {selfUpdateNoteLines.length > 0 ? (
                      <ul className="space-y-1.5 text-[12px] text-[var(--text-primary)] leading-relaxed">
                        {selfUpdateNoteLines.map((line, idx) => (
                          <li key={idx} className="flex items-start gap-2">
                            <span className="text-[var(--accent)] select-none shrink-0">•</span>
                            <span>{line}</span>
                          </li>
                        ))}
                      </ul>
                    ) : (
                      <div className="text-[12px] text-[var(--text-secondary)]">
                        包含稳定性与体验优化，点击「立即更新」即可升级。
                      </div>
                    )}
                  </div>
                )}

              {CHANGELOG_RELEASES.map((release) => {
                const isCurrent = dashboard?.appVersion === release.version;
                return (
                  <div
                    key={release.version}
                    className="rounded-[12px] border border-[var(--border)] bg-[var(--bg-subtle)]/45 p-3.5 flex flex-col gap-2"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <div className="flex items-center gap-2">
                        <span className="text-[13px] font-semibold text-[var(--text-primary)] tabular-nums">
                          v{release.version}
                        </span>
                        {isCurrent && (
                          <span className="text-[10px] px-1.5 py-0.5 rounded bg-[var(--accent-soft)] border border-[var(--accent-border)] text-[var(--accent)] font-medium">
                            当前版本
                          </span>
                        )}
                      </div>
                      {release.date && (
                        <span className="text-[11px] text-[var(--text-muted)] tabular-nums">
                          {release.date}
                        </span>
                      )}
                    </div>
                    <ul className="space-y-1.5 text-[12px] text-[var(--text-secondary)] leading-relaxed">
                      {release.items.map((item, idx) => (
                        <li key={idx} className="flex items-start gap-2">
                          <span className="text-[var(--accent)] select-none shrink-0">•</span>
                          <span>{item}</span>
                        </li>
                      ))}
                    </ul>
                  </div>
                );
              })}
            </div>

            <div className="px-5 py-3 border-t border-[var(--border)] flex items-center justify-between gap-2 shrink-0 bg-[var(--bg-card)]">
              <button
                type="button"
                onClick={() => void api.openUrl(TAKO_RELEASES_URL)}
                className="inline-flex items-center gap-1.5 text-[12px] text-[var(--text-muted)] hover:text-[var(--text-primary)] cursor-pointer"
              >
                <ExternalLink className="h-3.5 w-3.5" />
                <span>GitHub Releases</span>
              </button>
              <button
                type="button"
                onClick={() => setShowChangelogModal(false)}
                className="h-8 px-3.5 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-primary)] cursor-pointer"
              >
                关闭
              </button>
            </div>
          </div>
        </div>
      )}

      <UiPatchPermissionModal
        prompts={permissionPrompts}
        retrying={retryingPermission || togglingChineseUi || busyApp !== null}
        onOpenMacosSettings={() => void handleOpenMacosAppManagementSettings()}
        onRetry={() => void handleRetryPermissionAction()}
        onDismiss={() => setPermissionPrompts([])}
      />

      <ModelPickerModal
        appId={modelModalApp}
        appName={modelModalApp ? APP_DISPLAY_NAMES[modelModalApp] : ""}
        appRunning={
          modelModalApp
            ? Boolean(dashboard?.apps.find((a) => a.id === modelModalApp)?.running)
            : false
        }
        loggedIn={Boolean(dashboard?.account.loggedIn)}
        apps={dashboard?.apps}
        onClose={() => setModelModalApp(null)}
        onSaved={(savedAppId, nextState, restarted) => {
          const wasRunning = Boolean(
            dashboard?.apps.find((a) => a.id === savedAppId)?.running,
          );
          setDashboard((prev) =>
            prev
              ? {
                  ...prev,
                  state: nextState,
                  apps: restarted
                    ? prev.apps.map((a) =>
                        a.id === savedAppId ? { ...a, running: true } : a,
                      )
                    : prev.apps,
                }
              : prev,
          );
          setModelModalApp(null);
          if (restarted) {
            setNeedsRestart((prev) => ({ ...prev, [savedAppId]: false }));
            void loadDashboard(false);
            showTransientNote(`${APP_DISPLAY_NAMES[savedAppId]} 模型列表已保存并重启生效`);
          } else if (wasRunning) {
            setNeedsRestart((prev) => ({ ...prev, [savedAppId]: true }));
            showTransientNote(`${APP_DISPLAY_NAMES[savedAppId]} 模型设置已保存，重启应用后生效`);
          } else {
            showTransientNote(`${APP_DISPLAY_NAMES[savedAppId]} 模型设置已保存`);
          }
        }}
      />
    </div>
  );
}
