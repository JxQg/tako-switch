import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { AlertCircle, ExternalLink, FileText, Globe, MessageSquare, X } from "lucide-react";
import {
  api,
  parseUiPatchBlockError,
  TAKO_FEEDBACK_URL,
  TAKO_RELEASES_URL,
  type AccountStatus,
  type AppId,
  type AppMode,
  type AppSnapshot,
  type AppUpdateStatus,
  type DashboardPayload,
  type ExtensionsSummary,
  type InstallProgressEvent,
  type McpToolsConfig,
  type SelfUpdateCheckResult,
  type SelfUpdateProgressEvent,
  type SettingsPatch,
  type UiPatchPermissionPrompt,
  type UiPatchProgressEvent,
} from "./lib/api";
import { CHANGELOG_RELEASES, parseReleaseNotesLines } from "./lib/changelog";
import { ClientSettingsPanel } from "./components/ClientSettingsPanel";
import { GeneralSettingsPanel } from "./components/GeneralSettingsPanel";
import { ExtensionsHubPanel } from "./components/ExtensionsHubPanel";
import type { ExtensionNavigationGuard } from "./components/extensions/ExtensionPrimitives";
import { McpCenterPanel } from "./components/McpCenterPanel";
import type { McpToolKey } from "./components/McpInlineTheater";
import { ModelPickerModal } from "./components/ModelPickerModal";
import { OverviewPanel } from "./components/OverviewPanel";
import { TopHeaderBar } from "./components/TopHeaderBar";
import { UiPatchPermissionModal } from "./components/UiPatchPermissionModal";
import {
  APP_DISPLAY_NAMES,
  formatTimeHHMM,
  type ExtensionTab,
  type NavRoute,
  type ThemePref,
  type UiDensity,
} from "./components/UiPrimitives";
import {
  useNativeWindowState,
  WindowResizeHandles,
} from "./components/WindowChrome";

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

function loadInitialTheme(): ThemePref {
  try {
    const saved = localStorage.getItem("tako-switch-theme");
    if (saved === "light" || saved === "dark" || saved === "system") return saved;
  } catch {
    // ignore storage errors
  }
  return "system";
}

function loadInitialDensity(): UiDensity {
  try {
    const saved = localStorage.getItem("tako-switch-density");
    if (saved === "compact" || saved === "comfortable") return saved;
  } catch {
    // ignore storage errors
  }
  return "compact";
}

export default function App() {
  const [activeRoute, setActiveRoute] = useState<NavRoute>("overview");
  const [dashboard, setDashboard] = useState<DashboardPayload | null>(null);
  const [updatesMap, setUpdatesMap] = useState<Partial<Record<AppId, AppUpdateStatus>>>({});
  const [installProgress, setInstallProgress] = useState<
    Partial<Record<AppId, InstallProgressEvent>>
  >({});
  const [needsRestart, setNeedsRestart] = useState<Partial<Record<AppId, boolean>>>({});
  const [modelModalApp, setModelModalApp] = useState<AppId | null>(null);

  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const [busyApp, setBusyApp] = useState<AppId | null>(null);
  const [batchUpdatingApps, setBatchUpdatingApps] = useState(false);
  const [cancellingApp, setCancellingApp] = useState<AppId | null>(null);
  const [fixingGateway, setFixingGateway] = useState(false);
  const [savingMcp, setSavingMcp] = useState(false);
  const [savingSettings, setSavingSettings] = useState(false);
  const [loggingIn, setLoggingIn] = useState(false);

  const [togglingChineseUi, setTogglingChineseUi] = useState(false);
  const [uiPatchProgress, setUiPatchProgress] = useState<UiPatchProgressEvent | null>(null);
  const [unpatchedPromptApps, setUnpatchedPromptApps] = useState<AppId[]>([]);
  const initialChinesePromptCheckedRef = useRef(false);
  const uiPatchClearTimerRef = useRef<number | null>(null);
  const uiPatchFloorRef = useRef<number>(0);
  const [permissionPrompts, setPermissionPrompts] = useState<UiPatchPermissionPrompt[]>([]);
  const [retryingPermission, setRetryingPermission] = useState(false);
  const permissionRetryRef = useRef<(() => Promise<void>) | null>(null);

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
  const [showChangelogModal, setShowChangelogModal] = useState(false);
  const [mcpInitialTab, setMcpInitialTab] = useState<McpToolKey | null>(null);
  const [extensionTab, setExtensionTab] = useState<ExtensionTab>("mcp");
  const [extensionsSummary, setExtensionsSummary] = useState<ExtensionsSummary | null>(null);
  const extensionNavigationGuardRef = useRef<ExtensionNavigationGuard | null>(null);
  const onExtensionNavigationGuardChange = useCallback((guard: ExtensionNavigationGuard | null) => {
    extensionNavigationGuardRef.current = guard;
  }, []);
  const navigate = useCallback((route: NavRoute) => {
    const action = () => setActiveRoute(route);
    if (extensionNavigationGuardRef.current) extensionNavigationGuardRef.current(action); else action();
  }, []);
  const changeExtensionTab = useCallback((tab: ExtensionTab) => {
    const action = () => setExtensionTab(tab);
    if (extensionNavigationGuardRef.current) extensionNavigationGuardRef.current(action); else action();
  }, []);

  const [themePref, setThemePref] = useState<ThemePref>(loadInitialTheme);
  const [uiDensity, setUiDensity] = useState<UiDensity>(loadInitialDensity);
  const [headerScrolled, setHeaderScrolled] = useState(false);
  const mainScrollRef = useRef<HTMLElement | null>(null);
  const appearanceHydratedRef = useRef(false);
  const [systemDark, setSystemDark] = useState<boolean>(() =>
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-color-scheme: dark)").matches,
  );
  const initialAutoCheckDone = useRef(false);
  const runningSyncInFlight = useRef(false);

  const isDark = themePref === "dark" || (themePref === "system" && systemDark);

  useEffect(() => {
    const mq = window.matchMedia?.("(prefers-color-scheme: dark)");
    if (!mq) return;
    const onChange = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  const handleChangeTheme = useCallback((next: ThemePref) => {
    setThemePref(next);
    void api
      .saveSettings({ themePref: next })
      .then((saved) => {
        setDashboard((prev) => (prev ? { ...prev, state: saved } : prev));
      })
      .catch(() => {});
  }, []);

  const handleChangeDensity = useCallback((next: UiDensity) => {
    setUiDensity(next);
    void api
      .saveSettings({ uiDensity: next })
      .then((saved) => {
        setDashboard((prev) => (prev ? { ...prev, state: saved } : prev));
      })
      .catch(() => {});
  }, []);

  const toggleThemeQuick = useCallback(() => {
    handleChangeTheme(isDark ? "light" : "dark");
  }, [isDark, handleChangeTheme]);

  useEffect(() => {
    if (activeRoute === "overview") return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      if (document.querySelector('[role="dialog"][aria-modal="true"]')) return;
      if (
        showKeyModal ||
        showChangelogModal ||
        modelModalApp !== null ||
        permissionPrompts.length > 0
      ) {
        return;
      }
      // Let inputs or subpage drawers consume Escape first when focused
      const activeTag = document.activeElement?.tagName?.toLowerCase();
      if (activeTag === "input" || activeTag === "textarea") return;
      if (activeRoute === "mcp") return;
      navigate("overview");
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [activeRoute, showKeyModal, showChangelogModal, modelModalApp, permissionPrompts.length, navigate]);

  const selfUpdateNoteLines = useMemo(
    () => parseReleaseNotesLines(selfUpdate?.notes),
    [selfUpdate?.notes],
  );

  useEffect(() => {
    const root = document.documentElement;
    root.setAttribute("data-theme", isDark ? "dark" : "light");
    try {
      localStorage.setItem("tako-switch-theme", themePref);
    } catch {
      // ignore storage errors
    }
    void api.syncWindowTheme(themePref, isDark).catch(() => {});
  }, [themePref, isDark]);

  useEffect(() => {
    const root = document.documentElement;
    root.setAttribute("data-density", uiDensity);
    try {
      localStorage.setItem("tako-switch-density", uiDensity);
    } catch {
      // ignore storage errors
    }
  }, [uiDensity]);

  useEffect(() => {
    if (appearanceHydratedRef.current || !dashboard) return;
    appearanceHydratedRef.current = true;
    let storedDensity: string | null = null;
    let storedTheme: string | null = null;
    try {
      storedDensity = localStorage.getItem("tako-switch-density");
      storedTheme = localStorage.getItem("tako-switch-theme");
    } catch {
      // ignore storage errors
    }
    if (
      dashboard.state.uiDensity &&
      (storedDensity === null || dashboard.state.uiDensity !== "compact")
    ) {
      setUiDensity(dashboard.state.uiDensity);
    }
    if (
      dashboard.state.themePref &&
      (storedTheme === null || dashboard.state.themePref !== "system")
    ) {
      setThemePref(dashboard.state.themePref);
    }
  }, [dashboard]);

  useEffect(() => {
    mainScrollRef.current?.scrollTo({ top: 0 });
    setHeaderScrolled(false);
  }, [activeRoute]);

  const showTransientNote = useCallback((msg: string) => {
    setStatusNote(msg);
    window.setTimeout(() => {
      setStatusNote((prev) => (prev === msg ? null : prev));
    }, 3000);
  }, []);

  useEffect(() => {
    let cancelled = false;
    void api.getExtensionsSummary().then((summary) => {
      if (!cancelled) setExtensionsSummary(summary);
    }).catch((err: unknown) => {
      if (!cancelled) setErrorMsg(`读取扩展中心失败：${String(err)}`);
    });
    return () => { cancelled = true; };
  }, [activeRoute]);

  const handleExtensionCountChange = useCallback(
    (tab: ExtensionTab, count: number, updatableCount?: number) => {
      const keys: Record<ExtensionTab, keyof ExtensionsSummary> = {
        mcp: "mcpCount",
        skills: "skillsCount",
        plugins: "pluginsCount",
        prompts: "promptsCount",
      };
      setExtensionsSummary((previous) => ({
        mcpCount: 0,
        skillsCount: 0,
        skillsUpdatableCount: 0,
        pluginsCount: 0,
        promptsCount: 0,
        ...previous,
        [keys[tab]]: count,
        ...(tab === "skills" && updatableCount !== undefined
          ? { skillsUpdatableCount: updatableCount }
          : {}),
      }));
    },
    [],
  );

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
    setSelfUpdateError(null);
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
      if (!silent) {
        setSelfUpdateStatus((prev) =>
          prev === "downloading" || prev === "installing" ? prev : "checking",
        );
        setSelfUpdateError(null);
      }

      try {
        const [manifestRes, selfRes] = await Promise.allSettled([
          api.checkAppUpdates(),
          api.checkSelfUpdateSmart(),
        ]);

        if (manifestRes.status === "fulfilled") {
          const map: Partial<Record<AppId, AppUpdateStatus>> = {};
          for (const item of manifestRes.value.updates) {
            map[item.appId] = item;
          }
          setUpdatesMap(map);
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

    void loadDashboard(true).then((data) => {
      if (disposed || !data) return;
      if (!initialChinesePromptCheckedRef.current) {
        initialChinesePromptCheckedRef.current = true;
        const unpatchedIds = data.apps
          .filter((a) => {
            if (!a.install.installed || a.id === "dsh-tako") return false;
            if (a.id === "claude" || a.id === "antigravity") {
              return a.uiPatch.status === "unpatched";
            }
            return a.id === "codex" && !a.chineseConfigured;
          })
          .map((a) => a.id);
        if (unpatchedIds.length > 0) {
          setUnpatchedPromptApps(unpatchedIds);
        }
      }
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
        window.setTimeout(() => {
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

    const unlistenUiPatchProgressPromise = listen<UiPatchProgressEvent>(
      "ui-patch-progress",
      (event) => {
        const payload = event.payload;
        if (uiPatchClearTimerRef.current !== null) {
          window.clearTimeout(uiPatchClearTimerRef.current);
          uiPatchClearTimerRef.current = null;
        }
        uiPatchFloorRef.current = payload.percent;
        setUiPatchProgress((prev) => {
          const sameRun =
            prev && prev.scope === payload.scope && prev.action === payload.action;
          const nextPercent =
            payload.stage === "done" || payload.stage === "error"
              ? 100
              : sameRun
                ? Math.max(prev.percent, payload.percent)
                : payload.percent;
          return { ...payload, percent: nextPercent };
        });
        if (payload.stage === "done" || payload.stage === "error") {
          uiPatchClearTimerRef.current = window.setTimeout(
            () => {
              setUiPatchProgress(null);
              uiPatchClearTimerRef.current = null;
            },
            payload.stage === "done" ? 1500 : 800,
          );
        }
      },
    );

    const unlistenUiPatchAutoSyncedPromise = listen<{
      updatedApps: AppId[];
      deferredRunningApps: AppId[];
    }>("ui-patch-auto-synced", (event) => {
      const updated = event.payload.updatedApps ?? [];
      const deferred = event.payload.deferredRunningApps ?? [];
      if (updated.length > 0) {
        const names = updated.map((id) => APP_DISPLAY_NAMES[id] ?? id).join("、");
        showTransientNote(`已自动更新至最新简体中文汉化（${names}）`);
        void loadDashboard(false);
      } else if (deferred.length > 0) {
        const names = deferred.map((id) => APP_DISPLAY_NAMES[id] ?? id).join("、");
        showTransientNote(`检测到 ${names} 有新版汉化，关闭应用后将自动更新，也可在首页立即更新`);
        void loadDashboard(false);
      }
    });

    return () => {
      disposed = true;
      if (uiPatchClearTimerRef.current !== null) {
        window.clearTimeout(uiPatchClearTimerRef.current);
        uiPatchClearTimerRef.current = null;
      }
      void unlistenProgressPromise.then((fn) => fn());
      void unlistenSelfProgressPromise.then((fn) => fn());
      void unlistenSelfStatusPromise.then((fn) => fn());
      void unlistenStatePromise.then((fn) => fn());
      void unlistenUpdatesPromise.then((fn) => fn());
      void unlistenAccountPromise.then((fn) => fn());
      void unlistenUiPatchProgressPromise.then((fn) => fn());
      void unlistenUiPatchAutoSyncedPromise.then((fn) => fn());
    };
  }, [applySelfUpdateCheckResult, loadDashboard, runUpdateChecks, showTransientNote]);

  // Smoothly advance visual progress between Rust milestones so ASAR repack & codesign never feel frozen
  useEffect(() => {
    if (
      !uiPatchProgress ||
      uiPatchProgress.stage === "done" ||
      uiPatchProgress.stage === "error"
    ) {
      return;
    }
    const timer = window.setInterval(() => {
      setUiPatchProgress((prev) => {
        if (!prev || prev.stage === "done" || prev.stage === "error") return prev;
        const ceiling = Math.min(94, uiPatchFloorRef.current + 22);
        if (prev.percent >= ceiling) return prev;
        const step = prev.percent < ceiling - 8 ? 2 : 1;
        return { ...prev, percent: Math.min(ceiling, prev.percent + step) };
      });
    }, 380);
    return () => window.clearInterval(timer);
  }, [uiPatchProgress?.scope, uiPatchProgress?.action, uiPatchProgress?.stage]);

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
        showTransientNote("本地统一网关已就绪");
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
    const pendingAutoTako =
      Boolean(dashboard?.account.loggedIn) &&
      app.mode === "official" &&
      !(dashboard?.state.explicitOfficialApps ?? []).includes(app.id);
    if ((app.mode === mode && !pendingAutoTako) || busyApp === app.id) return;
    setBusyApp(app.id);
    setErrorMsg(null);
    try {
      const updated = await api.setAppMode(app.id, mode, false);
      const gwOk =
        app.id !== "dsh-tako" && updated.mode === "tako"
          ? await api.ensureLocalGateway().catch(() => true)
          : undefined;
      setDashboard((prev) => {
        if (!prev) return prev;
        const prevExplicit = prev.state.explicitOfficialApps ?? [];
        const nextExplicit =
          mode === "official"
            ? Array.from(new Set([...prevExplicit, app.id]))
            : prevExplicit.filter((id) => id !== app.id);
        return {
          ...prev,
          localGatewayHealthy: gwOk ?? prev.localGatewayHealthy,
          state: {
            ...prev.state,
            explicitOfficialApps: nextExplicit,
          },
          apps: prev.apps.map((item) => (item.id === app.id ? updated : item)),
        };
      });
      if (mode === "official" && pendingAutoTako) {
        showTransientNote(`${APP_DISPLAY_NAMES[app.id]} 已固定为官方模式`);
      } else if (updated.running) {
        setNeedsRestart((prev) => ({ ...prev, [app.id]: true }));
        showTransientNote(`${APP_DISPLAY_NAMES[app.id]} 已切换模式，重启应用后生效`);
      } else {
        showTransientNote(`${APP_DISPLAY_NAMES[app.id]} 模式已切换`);
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
    const prevMode = dashboard?.apps.find((a) => a.id === appId)?.mode;
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
      if (prevMode === "official" && updated.mode === "tako") {
        showTransientNote(`已自动切至 Tako 模式并重启 ${APP_DISPLAY_NAMES[appId]}`);
      } else {
        showTransientNote(`${APP_DISPLAY_NAMES[appId]} 已重启`);
      }
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setBusyApp(null);
    }
  };

  const handleLaunchApp = async (appId: AppId) => {
    setBusyApp(appId);
    setErrorMsg(null);
    const prevMode = dashboard?.apps.find((a) => a.id === appId)?.mode;
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
      if (prevMode === "official" && updated.mode === "tako") {
        showTransientNote(`已自动切至 Tako 模式并启动 ${APP_DISPLAY_NAMES[appId]}`);
      }
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
      // ignore cancel race
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

  const runSetPreferChineseUi = async (nextEnable: boolean) => {
    setTogglingChineseUi(true);
    setErrorMsg(null);
    if (uiPatchClearTimerRef.current !== null) {
      window.clearTimeout(uiPatchClearTimerRef.current);
      uiPatchClearTimerRef.current = null;
    }
    uiPatchFloorRef.current = 8;
    setUiPatchProgress({
      scope: "all",
      action: nextEnable ? "patch" : "restore",
      stage: "preparing",
      percent: 8,
      message: nextEnable ? "正在准备三端简体中文汉化..." : "正在准备恢复三端原版界面...",
      currentApp: null,
    });
    try {
      const nextDashboard = await api.setPreferChineseUi(nextEnable, true, false);
      setDashboard(nextDashboard);
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
          a.uiPatch.status === "outdated_after_upgrade" ||
          Boolean(a.uiPatch.updateAvailable)),
    );
    const anyInstalledMissingZh = dashboard.apps.some(
      (a) => a.id !== "dsh-tako" && a.install.installed && !a.chineseConfigured,
    );
    const nextEnable =
      currentEnabled && (needsUiPatch || anyInstalledMissingZh) ? true : !currentEnabled;
    await runSetPreferChineseUi(nextEnable);
  };

  const handlePatchAppUi = async (app: AppSnapshot) => {
    if (busyApp !== null) return;
    const label = APP_DISPLAY_NAMES[app.id] ?? app.name;
    setBusyApp(app.id);
    setErrorMsg(null);
    if (uiPatchClearTimerRef.current !== null) {
      window.clearTimeout(uiPatchClearTimerRef.current);
      uiPatchClearTimerRef.current = null;
    }
    uiPatchFloorRef.current = 10;
    setUiPatchProgress({
      scope: app.id,
      action: "patch",
      stage: "preparing",
      percent: 10,
      message: `正在准备 ${label} 简体中文界面补丁...`,
      currentApp: app.id,
    });
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
        showTransientNote(`${label} 界面汉化已注入，重启应用后生效`);
      } else {
        showTransientNote(`${label} 完整简体中文已生效`);
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
    if (uiPatchClearTimerRef.current !== null) {
      window.clearTimeout(uiPatchClearTimerRef.current);
      uiPatchClearTimerRef.current = null;
    }
    uiPatchFloorRef.current = 10;
    setUiPatchProgress({
      scope: app.id,
      action: "restore",
      stage: "preparing",
      percent: 10,
      message: `正在准备还原 ${label} 原版界面...`,
      currentApp: app.id,
    });
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

  const handleSaveSettingsPatch = async (patch: SettingsPatch, note?: string) => {
    if (!dashboard || savingSettings) return;
    setSavingSettings(true);
    setErrorMsg(null);
    try {
      const saved = await api.saveSettings(patch);
      setDashboard((prev) => (prev ? { ...prev, state: saved } : prev));
      if (note) showTransientNote(note);
    } catch (err) {
      setErrorMsg(`保存设置失败：${String(err)}`);
    } finally {
      setSavingSettings(false);
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

  const handleToggleWebPage = async (next: boolean) => {
    if (!dashboard || savingMcp) return;
    const currentMcp = dashboard.state.mcp;
    const nextMcp: McpToolsConfig = { ...currentMcp, webPage: next };

    setSavingMcp(true);
    setDashboard((prev) =>
      prev ? { ...prev, state: { ...prev.state, mcp: nextMcp } } : prev,
    );

    try {
      const savedState = await api.saveSettings({ mcp: nextMcp });
      setDashboard((prev) => (prev ? { ...prev, state: savedState } : prev));
      showTransientNote(
        next ? "已开启网站上线（Tako Web Page）" : "已关闭网站上线（Tako Web Page）",
      );
    } catch (err) {
      setDashboard((prev) =>
        prev ? { ...prev, state: { ...prev.state, mcp: currentMcp } } : prev,
      );
      setErrorMsg(`更新网站上线开关失败：${String(err)}`);
    } finally {
      setSavingMcp(false);
    }
  };

  const handleSaveEdgeOneConfig = async (token: string, projectName: string) => {
    if (!dashboard || savingMcp) return;
    const currentMcp = dashboard.state.mcp;
    const nextMcp: McpToolsConfig = {
      ...currentMcp,
      edgeoneApiToken: token || null,
      edgeoneProjectName: projectName || null,
    };

    setSavingMcp(true);
    try {
      const savedState = await api.saveSettings({ mcp: nextMcp });
      setDashboard((prev) => (prev ? { ...prev, state: savedState } : prev));
      showTransientNote("EdgeOne 高级配置已保存");
    } catch (err) {
      setErrorMsg(`保存 EdgeOne 配置失败：${String(err)}`);
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

  const isWindows = useMemo(() => {
    if (dashboard?.os) return dashboard.os === "windows";
    return (
      typeof navigator !== "undefined" &&
      navigator.userAgent.toLowerCase().includes("win")
    );
  }, [dashboard?.os]);

  const nativeWindow = useNativeWindowState(isWindows);

  const mcpEnabledCount = useMemo(() => {
    if (!dashboard) return 0;
    const mcp = dashboard.state.mcp;
    return [
      mcp.image,
      mcp.video,
      mcp.audio,
      mcp.search,
      mcp.consult,
      Boolean(mcp.webPage),
    ].filter(Boolean).length;
  }, [dashboard]);

  const anyTakoMode = useMemo(
    () => Boolean(dashboard?.apps.some((a) => a.id !== "dsh-tako" && a.mode === "tako")),
    [dashboard?.apps],
  );

  const selectedClientApp = useMemo(() => {
    if (
      !dashboard ||
      activeRoute === "overview" ||
      activeRoute === "general" ||
      activeRoute === "mcp" ||
      activeRoute === "extensions"
    ) {
      return null;
    }
    return dashboard.apps.find((a) => a.id === activeRoute) ?? null;
  }, [dashboard, activeRoute]);

  const hasPendingUpdates =
    selfUpdateStatus === "available" || updatableApps.length > 0;

  return (
    <div className="h-screen w-full flex flex-col app-surface overflow-hidden">
      <WindowResizeHandles
        enabled={isWindows && !nativeWindow.isMaximized}
        onStartResize={nativeWindow.startResize}
      />

      {/* Seamless Integrated Header Bar (Windows undecorated + macOS overlay-safe) */}
      <TopHeaderBar
        isMac={isMac}
        isWindows={isWindows}
        scrolled={headerScrolled}
        windowFocused={nativeWindow.isFocused}
        isMaximized={nativeWindow.isMaximized}
        appVersion={dashboard?.appVersion ?? "2.2.7"}
        activeRoute={activeRoute}
        account={dashboard?.account}
        isDark={isDark}
        hasPendingUpdates={hasPendingUpdates}
        refreshing={refreshing}
        checkingUpdates={checkingUpdates}
        loggingIn={loggingIn}
        onGoHome={() => navigate("overview")}
        onToggleTheme={toggleThemeQuick}
        onRefresh={() => void handleRefresh()}
        onOAuthLogin={() => void handleOAuthLogin()}
        onOpenKeyModal={() => setShowKeyModal(true)}
        onLogout={() => void handleLogout()}
        onOpenGeneralSettings={() => navigate("general")}
        onMinimizeWindow={nativeWindow.minimize}
        onToggleMaximizeWindow={nativeWindow.toggleMaximize}
        onCloseWindow={nativeWindow.close}
        onTitleBarContextMenu={() => {
          void api.showWindowSystemMenu().catch(() => {});
        }}
      />

      {/* Single-Column Main Scroll Viewport (Home or Secondary Page) */}
      <main
        ref={mainScrollRef}
        onScroll={(e) => {
          const nextScrolled = e.currentTarget.scrollTop > 4;
          if (nextScrolled !== headerScrolled) {
            setHeaderScrolled(nextScrolled);
          }
        }}
        className="flex-1 min-h-0 overflow-y-auto subtle-scroll relative"
      >
        {/* Inline Error Strip (dismissible) */}
        {errorMsg && (
          <div className="w-full max-w-[var(--density-page-max-w)] mx-auto px-[var(--density-page-px)] pt-3">
            <div
              className="rounded-[10px] border px-3.5 py-2 text-[12px] flex items-center justify-between gap-2"
              style={{
                backgroundColor: "var(--danger-soft)",
                borderColor: "rgba(201, 76, 76, 0.25)",
                color: "var(--danger)",
              }}
            >
              <div className="flex items-center gap-2 min-w-0">
                <AlertCircle className="h-4 w-4 shrink-0" />
                <span className="break-words">{errorMsg}</span>
              </div>
              <button
                type="button"
                onClick={() => setErrorMsg(null)}
                className="p-0.5 rounded hover:bg-black/5 cursor-pointer shrink-0"
                aria-label="关闭错误提示"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          </div>
        )}

        {activeRoute === "overview" && (
          <OverviewPanel
            dashboard={dashboard}
            loading={loading}
            updatesMap={updatesMap}
            installProgress={installProgress}
            needsRestart={needsRestart}
            updatableApps={updatableApps}
            batchUpdatingApps={batchUpdatingApps}
            busyApp={busyApp}
            cancellingApp={cancellingApp}
            togglingChineseUi={togglingChineseUi}
            uiPatchProgress={uiPatchProgress}
            savingMcp={savingMcp}
            mcpEnabledCount={mcpEnabledCount}
            mcpTotalCount={6}
            selfUpdateSummary={{
              status: selfUpdateStatus,
              version: selfUpdate?.version,
              source: selfUpdate?.source,
              percent: selfUpdateProgress?.percent,
            }}
            onNavigate={navigate}
            onOpenMcpDetail={(tab) => {
              setMcpInitialTab(tab ?? null);
              setActiveRoute("mcp");
            }}
            extensionsSummary={extensionsSummary}
            onOpenExtensions={(tab) => {
              if (tab) setExtensionTab(tab);
              setActiveRoute("extensions");
            }}
            onSetMode={(app, mode) => void handleSetMode(app, mode)}
            onLaunchApp={(id) => void handleLaunchApp(id)}
            onRestartApp={(id) => void handleRestartApp(id)}
            onCloseApp={(id) => void handleCloseApp(id)}
            onInstallOrUpgrade={(id) => void handleInstallOrUpgrade(id)}
            onCancelInstall={(id) => void handleCancelAppInstall(id)}
            onToggleMainChineseUi={() => void handleToggleMainChineseUi()}
            onToggleCoreMcp={(key) => void handleToggleMcpTool(key)}
            onToggleWebPageMcp={(next) => void handleToggleWebPage(next)}
            onOpenModelSquare={() => setModelModalApp("dsh-tako")}
            onBatchUpgradeApps={() => void handleBatchUpgradeApps()}
            onInstallSelfUpdate={() => void handleInstallSelfUpdate()}
            onOpenChangelog={() => setShowChangelogModal(true)}
          />
        )}

        {activeRoute === "general" && (
          <GeneralSettingsPanel
            dashboard={dashboard}
            themePref={themePref}
            onChangeTheme={handleChangeTheme}
            uiDensity={uiDensity}
            onChangeDensity={handleChangeDensity}
            anyTakoMode={anyTakoMode}
            fixingGateway={fixingGateway}
            loggingIn={loggingIn}
            savingSettings={savingSettings}
            checkingUpdates={checkingUpdates}
            lastCheckedAt={lastCheckedAt}
            updatableApps={updatableApps}
            batchUpdatingApps={batchUpdatingApps}
            busyApp={busyApp}
            selfUpdateView={{
              status: selfUpdateStatus,
              version: selfUpdate?.version,
              source: selfUpdate?.source,
              percent: selfUpdateProgress?.percent,
              error: selfUpdateError,
            }}
            mcpEnabledCount={mcpEnabledCount}
            mcpTotalCount={6}
            onEnsureGateway={() => void handleEnsureGateway()}
            onOAuthLogin={() => void handleOAuthLogin()}
            onOpenKeyModal={() => setShowKeyModal(true)}
            onLogout={() => void handleLogout()}
            onRefreshAccount={() => void loadDashboard(true)}
            onSavePatch={handleSaveSettingsPatch}
            onCheckUpdates={() => void runUpdateChecks(false)}
            onInstallSelfUpdate={() => void handleInstallSelfUpdate()}
            onBatchUpgradeApps={() => void handleBatchUpgradeApps()}
            onOpenChangelog={() => setShowChangelogModal(true)}
            onNavigate={navigate}
          />
        )}

        {activeRoute === "mcp" && (
          <McpCenterPanel
            mcp={dashboard?.state.mcp ?? null}
            savingMcp={savingMcp}
            loggedIn={Boolean(dashboard?.account.loggedIn)}
            loggingIn={loggingIn}
            initialDrawerTab={mcpInitialTab}
            onConsumeInitialTab={() => setMcpInitialTab(null)}
            onGoHome={() => setActiveRoute("overview")}
            onToggleCoreTool={(key) => void handleToggleMcpTool(key)}
            onToggleWebPage={handleToggleWebPage}
            onSaveEdgeOneConfig={handleSaveEdgeOneConfig}
            onSelectConsultModel={(model) => void handleSelectConsultModel(model)}
            onRequestLogin={() => void handleOAuthLogin()}
          />
        )}

        {activeRoute === "extensions" && (
          <ExtensionsHubPanel
            activeTab={extensionTab}
            onTabChange={changeExtensionTab}
            summary={extensionsSummary}
            onCountChange={handleExtensionCountChange}
            onGoSuperMcp={() => setActiveRoute("mcp")}
            onNotify={showTransientNote}
            onNavigationGuardChange={onExtensionNavigationGuardChange}
          />
        )}

        {selectedClientApp && dashboard && (
          <ClientSettingsPanel
            app={selectedClientApp}
            dashboard={dashboard}
            update={updatesMap[selectedClientApp.id]}
            progress={installProgress[selectedClientApp.id]}
            uiPatchProgress={
              uiPatchProgress &&
              (uiPatchProgress.scope === selectedClientApp.id ||
                uiPatchProgress.currentApp === selectedClientApp.id)
                ? uiPatchProgress
                : null
            }
            isBusy={busyApp === selectedClientApp.id}
            cancelling={cancellingApp === selectedClientApp.id}
            needsRestart={Boolean(needsRestart[selectedClientApp.id])}
            savingInlineSetting={savingSettings}
            onSetMode={(app, mode) => void handleSetMode(app, mode)}
            onLaunchApp={(id) => void handleLaunchApp(id)}
            onRestartApp={(id) => void handleRestartApp(id)}
            onCloseApp={(id) => void handleCloseApp(id)}
            onInstallOrUpgrade={(id) => void handleInstallOrUpgrade(id)}
            onCancelInstall={(id) => void handleCancelAppInstall(id)}
            onRediagnose={(id) => void handleRediagnose(id)}
            onPatchAppUi={(app) => void handlePatchAppUi(app)}
            onRestoreAppUi={(app) => void handleRestoreAppUi(app)}
            onOpenModelPicker={(id) => setModelModalApp(id)}
            onSaveInlineSettings={handleSaveSettingsPatch}
          />
        )}

        {activeRoute !== "overview" &&
          activeRoute !== "general" &&
          activeRoute !== "mcp" &&
          activeRoute !== "extensions" &&
          !selectedClientApp && (
            <div className="w-full px-5 py-12 text-center text-[13px] text-[var(--text-muted)]">
              {loading ? "正在读取客户端状态..." : "未找到该客户端配置"}
            </div>
          )}
      </main>

      {/* Slim Bottom Status Bar */}
      <footer className="tako-statusbar shrink-0 border-t border-[var(--border)] bg-[var(--bg-card)]/80 flex items-center justify-between gap-3 text-[11.5px] text-[var(--text-secondary)] select-none">
        <div className="flex items-center gap-2 min-w-0">
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
              onClick={() => void handleEnsureGateway()}
              className="inline-flex items-center gap-1.5 text-[var(--warning)] hover:underline cursor-pointer disabled:opacity-60 min-w-0"
            >
              <span className="h-1.5 w-1.5 rounded-full bg-[var(--warning)] shrink-0" />
              <span className="truncate">
                {fixingGateway ? "正在修复网关..." : "本地网关未运行 · 点击修复"}
              </span>
            </button>
          )}
        </div>

        <div className="flex items-center gap-2.5 shrink-0">
          <button
            type="button"
            onClick={() => navigate("general")}
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
            onClick={() => setShowChangelogModal(true)}
            className="text-[var(--text-muted)] hover:text-[var(--text-primary)] transition cursor-pointer"
          >
            更新日志
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

      {/* Transient Floating Toast */}
      {statusNote && (
        <div
          role="status"
          className="toast-enter fixed bottom-5 left-1/2 -translate-x-1/2 z-40 px-3.5 py-2 rounded-full bg-[var(--btn-dark-bg)] text-[var(--btn-dark-fg)] text-[12px] font-medium shadow-lg pointer-events-none"
        >
          {statusNote}
        </div>
      )}

      {/* Manual API Key Login Modal */}
      {showKeyModal && (
        <div
          className="fixed inset-0 z-50 modal-backdrop flex items-center justify-center p-4"
          onClick={() => setShowKeyModal(false)}
        >
          <form
            onSubmit={handleKeyLogin}
            onClick={(e) => e.stopPropagation()}
            className="w-full max-w-[380px] rounded-[15px] border border-[var(--border)] bg-[var(--bg-card)] p-5 shadow-[var(--shadow-modal)] flex flex-col gap-3.5"
          >
            <div className="flex items-center justify-between">
              <h3 className="text-[14px] font-semibold text-[var(--text-primary)]">
                手动填入 Tako API Key
              </h3>
              <button
                type="button"
                aria-label="关闭弹窗"
                onClick={() => setShowKeyModal(false)}
                className="p-1 rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-subtle)] cursor-pointer"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <p className="text-[12px] text-[var(--text-secondary)]">
              粘贴以 <code className="font-mono">sk-</code> 开头的 Tako Key，验证后同步到本机配置。
            </p>
            <input
              type="password"
              autoFocus
              value={apiKeyInput}
              onChange={(e) => setApiKeyInput(e.target.value)}
              placeholder="sk-..."
              className="h-9 px-3 rounded-[9px] border border-[var(--border)] bg-[var(--bg-canvas)] text-[12.5px] font-mono text-[var(--text-primary)] focus:outline-none focus:border-[var(--accent)]"
            />
            <div className="flex items-center justify-end gap-2 pt-1">
              <button
                type="button"
                onClick={() => setShowKeyModal(false)}
                className="h-8 px-3 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-secondary)] cursor-pointer"
              >
                取消
              </button>
              <button
                type="submit"
                disabled={loggingIn || !apiKeyInput.trim()}
                className="h-8 px-3.5 rounded-[8px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-semibold cursor-pointer disabled:opacity-50"
              >
                {loggingIn ? "验证中..." : "验证并登录"}
              </button>
            </div>
          </form>
        </div>
      )}

      {/* Changelog Modal */}
      {showChangelogModal && (
        <div
          className="fixed inset-0 z-50 modal-backdrop flex items-center justify-center p-4"
          onClick={() => setShowChangelogModal(false)}
        >
          <div
            className="w-full max-w-[520px] max-h-[82vh] flex flex-col rounded-[16px] border border-[var(--border)] bg-[var(--bg-card)] shadow-[var(--shadow-modal)] overflow-hidden"
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

            <div className="flex-1 overflow-y-auto subtle-scroll px-5 py-4 flex flex-col gap-3.5">
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

      {/* Startup Unpatched Chinese Localization Prompt Modal */}
      {unpatchedPromptApps.length > 0 && !uiPatchProgress && (
        <div
          className="fixed inset-0 z-50 bg-black/40 backdrop-blur-[2px] flex items-center justify-center p-4"
          onClick={() => setUnpatchedPromptApps([])}
        >
          <div
            data-testid="startup-chinese-prompt-modal"
            className="w-full max-w-[420px] rounded-[16px] border border-[var(--border)] bg-[var(--bg-card)] shadow-[var(--shadow-md)] p-5 flex flex-col gap-3.5"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-start justify-between gap-3">
              <div className="flex items-center gap-2.5 min-w-0">
                <div className="w-9 h-9 rounded-[10px] bg-[var(--accent-soft)] border border-[var(--accent-border)] text-[var(--accent)] flex items-center justify-center shrink-0">
                  <Globe className="h-4.5 w-4.5" />
                </div>
                <div className="min-w-0">
                  <div className="text-[14.5px] font-semibold text-[var(--text-primary)]">
                    检测到应用尚未汉化
                  </div>
                  <div className="text-[11.5px] text-[var(--text-muted)]">
                    可一键开启简体中文，支持随时无损还原
                  </div>
                </div>
              </div>
              <button
                type="button"
                aria-label="关闭"
                onClick={() => setUnpatchedPromptApps([])}
                className="w-7 h-7 rounded-[8px] flex items-center justify-center text-[var(--text-muted)] hover:bg-[var(--bg-subtle)] cursor-pointer shrink-0"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="text-[12.5px] text-[var(--text-secondary)] leading-relaxed">
              检测到当前已安装的{" "}
              <span className="font-semibold text-[var(--text-primary)]">
                {unpatchedPromptApps.map((id) => APP_DISPLAY_NAMES[id] ?? id).join("、")}
              </span>{" "}
              还没有汉化，是否要直接一键汉化？后续汉化词典更新时，Tako Switch 也会自动帮你同步更新。
            </div>

            <div className="flex items-center justify-end gap-2 pt-1">
              <button
                type="button"
                onClick={() => setUnpatchedPromptApps([])}
                className="h-8.5 px-3.5 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12.5px] font-medium text-[var(--text-secondary)] cursor-pointer"
              >
                暂不汉化
              </button>
              <button
                type="button"
                data-testid="startup-chinese-prompt-confirm"
                disabled={togglingChineseUi || busyApp !== null}
                onClick={() => {
                  setUnpatchedPromptApps([]);
                  void runSetPreferChineseUi(true);
                }}
                className="h-8.5 px-4 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12.5px] font-medium cursor-pointer disabled:opacity-60"
              >
                立即汉化
              </button>
            </div>
          </div>
        </div>
      )}

      {/* UI Patch Permission Modal */}
      <UiPatchPermissionModal
        prompts={permissionPrompts}
        retrying={retryingPermission || togglingChineseUi || busyApp !== null}
        onOpenMacosSettings={() => void handleOpenMacosAppManagementSettings()}
        onRetry={() => void handleRetryPermissionAction()}
        onDismiss={() => setPermissionPrompts([])}
      />

      {/* Model Picker & Plaza Modal */}
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
