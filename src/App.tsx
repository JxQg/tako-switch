import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  Eye,
  EyeOff,
  FileText,
  Github,
  Home,
  KeyRound,
  Loader2,
  LogIn,
  LogOut,
  Maximize2,
  RefreshCw,
  RotateCcw,
  Save,
  Settings2,
  ShieldCheck,
  Terminal,
  Wand2,
  X
} from "lucide-react";
import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import {
  TakoApi,
  AccountProviderConfig,
  TakoAccount,
  ProviderCatalog,
  TakoProviderConfigService,
  TakoSessionStore,
  startTakoLogin,
  type TakoModel,
  type TakoUsage
} from "./integrations/tako";
import {
  APP_DISPLAY_VERSION,
  PROJECT_URL,
  AppUpdateStatus,
  checkForAppUpdate,
  getUpdateOpenUrl
} from "./appUpdates";

type ActiveTab = "home" | "import" | "current";

type CodexSandboxMode = "read-only" | "workspace-write" | "danger-full-access";
type CodexApprovalPolicy = "untrusted" | "on-request" | "never";
type CodexWindowsSandbox = "elevated" | "unelevated";
type ClaudePermissionMode = "default" | "acceptEdits" | "plan" | "auto" | "dontAsk" | "bypassPermissions";

type CodexFeatureOptions = {
  jsRepl: boolean | null;
  unifiedExec: boolean | null;
  shellSnapshot: boolean | null;
  memories: boolean | null;
};

type PlatformOptionsInput = {
  sandboxMode?: CodexSandboxMode | null;
  approvalPolicy?: CodexApprovalPolicy | null;
  windowsSandbox?: CodexWindowsSandbox | null;
  features?: CodexFeatureOptions;
  permissionsDefaultMode?: ClaudePermissionMode | null;
  skipDangerousModePermissionPrompt?: boolean | null;
};

type PlatformFormInput = {
  enabled: boolean;
  baseUrl: string;
  model?: string;
  options: PlatformOptionsInput;
};

type ConfigInput = {
  providerId: string;
  apiKey: string;
  platforms: {
    codex: PlatformFormInput;
    claude: PlatformFormInput;
  };
};

type ToolStatus = {
  name: string;
  installed: boolean;
  version?: string;
  error?: string;
};

type ExistingConfig = {
  target: string;
  path: string;
  exists: boolean;
  content: string;
};

type LoadedConfigs = {
  codex: ExistingConfig;
  claude: ExistingConfig;
};

type FilePreview = {
  target: string;
  path: string;
  exists: boolean;
  backupPath: string;
  before: string;
  after: string;
};

type EnvPreview = {
  name: string;
  maskedValue: string;
  note: string;
};

type PreviewResult = {
  files: FilePreview[];
  envUpdates: EnvPreview[];
  warnings: string[];
};

type DiffLineKind = "context" | "added" | "removed" | "modified";

type DiffLine = {
  kind: DiffLineKind;
  marker: " " | "+" | "-" | "~";
  text: string;
  oldLine?: number;
  newLine?: number;
};

type AppliedFile = {
  target: string;
  path: string;
  backupPath: string;
  created: boolean;
};

type ApplyResult = {
  files: AppliedFile[];
  envUpdates: string[];
  tools: ToolStatus[];
  warnings: string[];
};

type RestoreResult = {
  target: string;
  path: string;
  restoredFrom: string;
  deletedTarget: boolean;
};

type BusyState = "loading" | "login" | "logout" | "tako" | "preview" | "apply" | "restore" | null;
type PreviewModalContext = "import" | "home" | null;

const emptyPreview: PreviewResult = {
  files: [],
  envUpdates: [],
  warnings: []
};

const emptyConfigInput: ConfigInput = {
  providerId: "",
  apiKey: "",
  platforms: {
    codex: {
      enabled: true,
      baseUrl: "",
      model: "",
      options: {
        sandboxMode: null,
        approvalPolicy: null,
        windowsSandbox: null,
        features: {
          jsRepl: null,
          unifiedExec: null,
          shellSnapshot: null,
          memories: null
        }
      }
    },
    claude: {
      enabled: true,
      baseUrl: "",
      model: "",
      options: {
        permissionsDefaultMode: null,
        skipDangerousModePermissionPrompt: null
      }
    }
  }
};

function App() {
  const [activeTab, setActiveTab] = useState<ActiveTab>("home");
  const apiKeyInputRef = useRef<HTMLInputElement | null>(null);
  const [providerCatalog, setProviderCatalog] = useState<ProviderCatalog | null>(null);
  const provider = useMemo(
    () => (providerCatalog ? TakoProviderConfigService.getDefaultProvider(providerCatalog) : null),
    [providerCatalog]
  );
  const [form, setForm] = useState<ConfigInput>(emptyConfigInput);
  const [tools, setTools] = useState<ToolStatus[]>([]);
  const [configs, setConfigs] = useState<LoadedConfigs | null>(null);
  const [preview, setPreview] = useState<PreviewResult>(emptyPreview);
  const [previewModalContext, setPreviewModalContext] = useState<PreviewModalContext>(null);
  const [result, setResult] = useState<ApplyResult | null>(null);
  const [restoreResult, setRestoreResult] = useState<RestoreResult | null>(null);
  const [homeImportOpen, setHomeImportOpen] = useState(false);
  const [homeImportForm, setHomeImportForm] = useState<ConfigInput>(emptyConfigInput);
  const [homePreview, setHomePreview] = useState<PreviewResult>(emptyPreview);
  const [homeResult, setHomeResult] = useState<ApplyResult | null>(null);
  const [homeRestoreResult, setHomeRestoreResult] = useState<RestoreResult | null>(null);
  const [showImportApiKey, setShowImportApiKey] = useState(false);
  const [showHomeApiKey, setShowHomeApiKey] = useState(false);
  const [takoAccount, setTakoAccount] = useState<TakoAccount>({
    loggedIn: false,
    name: null,
    plan: null,
    offline: false
  });
  const [takoUsage, setTakoUsage] = useState<TakoUsage | null>(null);
  const [takoModels, setTakoModels] = useState<TakoModel[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [infoMessage, setInfoMessage] = useState<string | null>(null);
  const [updateStatus, setUpdateStatus] = useState<AppUpdateStatus | null>(null);
  const [updateDialogOpen, setUpdateDialogOpen] = useState(false);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [busy, setBusy] = useState<BusyState>(null);

  const validation = useMemo(() => validateLocal(form, provider), [form, provider]);
  const homeImportValidation = useMemo(() => validateLocal(homeImportForm, provider), [homeImportForm, provider]);
  const canSubmit = validation.length === 0 && busy === null;
  const canHomeImportSubmit = homeImportValidation.length === 0 && busy === null;
  const loading = busy !== null;
  const modalOpen = homeImportOpen || previewModalContext !== null || updateDialogOpen;
  useBodyScrollLock(modalOpen);

  useEffect(() => {
    void refreshState();
    void refreshUpdateStatus({ silent: true });
  }, []);

  useEffect(() => {
    if (provider && !form.providerId) {
      setForm(createProviderConfigInput(provider, form.apiKey, takoModels));
      setHomeImportForm(createProviderConfigInput(provider, homeImportForm.apiKey, takoModels));
    }
  }, [provider, form.providerId, form.apiKey, homeImportForm.apiKey, takoModels]);

  useEffect(() => {
    if (activeTab === "import" && provider && !form.apiKey.trim()) {
      window.setTimeout(() => apiKeyInputRef.current?.focus(), 50);
    }
  }, [activeTab, form.apiKey, provider]);

  async function refreshState() {
    setBusy("loading");
    setError(null);
    try {
      const [storedSession, loadedProviderCatalog] = await Promise.all([
        TakoSessionStore.load(),
        TakoProviderConfigService.loadCatalog()
      ]);
      const migrationResult = await invoke<ApplyResult | null>("migrate_legacy_codex_config", {
        apiKey: storedSession?.apiKey
      });
      const [toolStatuses, loadedConfigs, storedApplyResult] = await Promise.all([
        invoke<ToolStatus[]>("detect_tools"),
        invoke<LoadedConfigs>("load_current_configs"),
        invoke<ApplyResult | null>("load_latest_apply_result")
      ]);
      const loadedProvider = TakoProviderConfigService.getDefaultProvider(loadedProviderCatalog);
      setProviderCatalog(loadedProviderCatalog);
      setForm((current) =>
        current.providerId ? current : createProviderConfigInput(loadedProvider, current.apiKey, takoModels)
      );
      setHomeImportForm((current) =>
        current.providerId ? current : createProviderConfigInput(loadedProvider, current.apiKey, takoModels)
      );
      setTools(toolStatuses);
      setConfigs(loadedConfigs);
      if (loadedProviderCatalog.warning) {
        setError(loadedProviderCatalog.warning);
      }
      const latestResult = migrationResult ?? storedApplyResult;
      if (latestResult) {
        setResult(latestResult);
        setHomeResult(latestResult);
      }
      if (storedSession) {
        await restoreTakoSession(storedSession.apiKey);
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  async function createPreview(event?: FormEvent) {
    event?.preventDefault();
    if (!canSubmit) return;
    setBusy("preview");
    setError(null);
    setResult(null);
    setRestoreResult(null);
    try {
      const nextPreview = await invoke<PreviewResult>("preview_changes", { input: form });
      setPreview(nextPreview);
      setPreviewModalContext("import");
      setActiveTab("import");
    } catch (err) {
      setError(String(err));
      setPreview(emptyPreview);
      setPreviewModalContext(null);
    } finally {
      setBusy(null);
    }
  }

  async function applyConfigs() {
    if (!canSubmit) return;
    setBusy("apply");
    setError(null);
    setRestoreResult(null);
    try {
      const nextPreview = await invoke<PreviewResult>("preview_changes", { input: form });
      setPreview(nextPreview);
      setPreviewModalContext(null);
      const applyResult = await invoke<ApplyResult>("apply_configs", { input: form });
      setResult(applyResult);
      setHomeResult(applyResult);
      setTools(applyResult.tools);
      await refreshConfigsOnly();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  async function refreshConfigsOnly() {
    try {
      setConfigs(await invoke<LoadedConfigs>("load_current_configs"));
    } catch (err) {
      setError(String(err));
    }
  }

  async function restore(file: AppliedFile) {
    setBusy("restore");
    setError(null);
    setRestoreResult(null);
    try {
      const restored = await invoke<RestoreResult>("restore_backup", {
        target: file.target,
        backupPath: file.backupPath
      });
      setRestoreResult(restored);
      setHomeRestoreResult(restored);
      setResult(null);
      setHomeResult(null);
      await refreshConfigsOnly();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  async function handleHomeImport() {
    if (!form.apiKey.trim()) {
      await handleTakoLogin({ openImportModal: true });
      return;
    }
    openHomeImportModal(form.apiKey);
  }

  async function handleTakoLogin(options: { openImportModal?: boolean } = {}) {
    setBusy("login");
    setError(null);
    setRestoreResult(null);
    try {
      const loginResult = await startTakoLogin();
      if (!loginResult.ok || !loginResult.apiKey) {
        setError(loginResult.error || "Tako 登录失败，请重试或手动粘贴 ApiKey。");
        setActiveTab("import");
        window.setTimeout(() => apiKeyInputRef.current?.focus(), 50);
        return false;
      }

      await TakoSessionStore.save(loginResult.apiKey);
      setTakoAccount({
        loggedIn: true,
        name: loginResult.name,
        plan: loginResult.plan,
        offline: false
      });
      setForm((current) => ({
        ...(provider ? withProviderDefaults(current, provider, takoModels) : current),
        apiKey: loginResult.apiKey || current.apiKey
      }));
      setPreview(emptyPreview);
      setPreviewModalContext(null);
      const details = await loadTakoDetails(loginResult.apiKey);
      applyTakoDetails(details);
      if (options.openImportModal) {
        openHomeImportModal(loginResult.apiKey, details.models);
      }
      return true;
    } catch (err) {
      setError(String(err));
      return false;
    } finally {
      setBusy(null);
    }
  }

  async function restoreTakoSession(apiKey: string) {
    setForm((current) => ({
      ...(provider ? withProviderDefaults(current, provider, takoModels) : current),
      apiKey
    }));

    const identity = await TakoApi.currentIdentity(apiKey);
    if (!identity.loggedIn) {
      await TakoSessionStore.clear();
      setTakoAccount({ loggedIn: false, name: null, plan: null, offline: false });
      setTakoUsage(null);
      setTakoModels([]);
      setForm((current) => ({ ...current, apiKey: "" }));
      setError("已保存的 Tako ApiKey 已失效，请重新登录。");
      return;
    }

    if (identity.offline) {
      setTakoAccount(identity);
      setTakoUsage(null);
      setTakoModels([]);
      return;
    }

    const details = await loadTakoDetails(apiKey, identity);
    applyTakoDetails(details);
  }

  async function refreshTakoDetails(apiKey = form.apiKey) {
    if (!apiKey.trim()) {
      setError("请先登录 Tako 或手动粘贴 ApiKey。");
      return;
    }

    setBusy("tako");
    setError(null);
    try {
      const details = await loadTakoDetails(apiKey);
      applyTakoDetails(details);
      if (!details.identity.loggedIn) {
        await TakoSessionStore.clear();
        setForm((current) => ({ ...current, apiKey: "" }));
        setError("Tako ApiKey 已失效，请重新登录。");
      } else if (details.usage && !details.usage.ok && details.usage.error) {
        setError(details.usage.error);
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  async function loadTakoDetails(apiKey: string, knownIdentity?: TakoAccount) {
    const identity = knownIdentity ?? (await TakoApi.currentIdentity(apiKey));
    if (!identity.loggedIn || identity.offline) {
      return { identity, usage: null, models: [] };
    }

    const [usageResult, modelsResult] = await Promise.allSettled([TakoApi.usage(apiKey), TakoApi.listModels(apiKey)]);
    return {
      identity,
      usage: usageResult.status === "fulfilled" ? usageResult.value : null,
      models: modelsResult.status === "fulfilled" ? modelsResult.value : []
    };
  }

  function applyTakoDetails({
    identity,
    usage,
    models
  }: {
    identity: TakoAccount;
    usage: TakoUsage | null;
    models: TakoModel[];
  }) {
    setTakoAccount({
      loggedIn: identity.loggedIn,
      name: identity.name,
      plan: identity.plan,
      offline: identity.offline
    });
    setTakoUsage(usage);
    setTakoModels(models);
    const defaultCodexModel = selectDefaultCodexModel(models);
    if (defaultCodexModel) {
      setForm((current) => ({
        ...current,
        platforms: {
          ...current.platforms,
          codex: {
            ...current.platforms.codex,
            model: defaultCodexModel
          }
        }
      }));
    }
  }

  function openHomeImportModal(apiKey: string, models = takoModels) {
    if (!provider) return;
    const draft = createProviderConfigInput(provider, apiKey, models);
    setHomeImportForm(draft);
    setForm(draft);
    setHomePreview(emptyPreview);
    setPreview(emptyPreview);
    setPreviewModalContext(null);
    setHomeResult(null);
    setHomeRestoreResult(null);
    setRestoreResult(null);
    setShowHomeApiKey(false);
    setHomeImportOpen(true);
  }

  async function createHomePreview(event?: FormEvent) {
    event?.preventDefault();
    if (!canHomeImportSubmit) return;
    setBusy("preview");
    setError(null);
    setHomeResult(null);
    setResult(null);
    setHomeRestoreResult(null);
    setRestoreResult(null);
    try {
      const nextPreview = await invoke<PreviewResult>("preview_changes", { input: homeImportForm });
      setForm(homeImportForm);
      setHomePreview(nextPreview);
      setPreview(nextPreview);
      setPreviewModalContext("home");
    } catch (err) {
      setError(String(err));
      setHomePreview(emptyPreview);
      setPreview(emptyPreview);
      setPreviewModalContext(null);
    } finally {
      setBusy(null);
    }
  }

  async function applyHomeConfigs() {
    if (!canHomeImportSubmit) return;
    setBusy("apply");
    setError(null);
    setHomeRestoreResult(null);
    try {
      const nextPreview = await invoke<PreviewResult>("preview_changes", { input: homeImportForm });
      setForm(homeImportForm);
      setHomePreview(nextPreview);
      setPreview(nextPreview);
      setPreviewModalContext(null);
      const applyResult = await invoke<ApplyResult>("apply_configs", { input: homeImportForm });
      setHomeResult(applyResult);
      setResult(applyResult);
      setTools(applyResult.tools);
      await refreshConfigsOnly();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  async function restoreHome(file: AppliedFile) {
    setBusy("restore");
    setError(null);
    setHomeRestoreResult(null);
    try {
      const restored = await invoke<RestoreResult>("restore_backup", {
        target: file.target,
        backupPath: file.backupPath
      });
      setHomeRestoreResult(restored);
      setRestoreResult(restored);
      setHomeResult(null);
      setResult(null);
      await refreshConfigsOnly();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  async function handleTakoLogout() {
    setBusy("logout");
    setError(null);
    try {
      await TakoApi.logout();
      await TakoSessionStore.clear();
      setTakoAccount({ loggedIn: false, name: null, plan: null, offline: false });
      setTakoUsage(null);
      setTakoModels([]);
      setForm((current) => ({ ...current, apiKey: "" }));
      setPreview(emptyPreview);
      setHomePreview(emptyPreview);
      setPreviewModalContext(null);
      setHomeResult(null);
      setHomeRestoreResult(null);
      setHomeImportOpen(false);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  async function refreshUpdateStatus({ silent = false }: { silent?: boolean } = {}) {
    if (checkingUpdate) return;
    setCheckingUpdate(true);
    if (!silent) {
      setError(null);
      setInfoMessage(null);
    }

    try {
      const nextStatus = await checkForAppUpdate();
      setUpdateStatus(nextStatus);
      if (nextStatus.available) {
        if (!silent) {
          setUpdateDialogOpen(true);
        }
      } else if (!silent) {
        setInfoMessage(`已是最新版本：${APP_DISPLAY_VERSION}`);
      }
    } catch (err) {
      if (!silent) {
        setError(`检查更新失败：${String(err)}`);
      }
    } finally {
      setCheckingUpdate(false);
    }
  }

  async function handleUpdateButtonClick() {
    if (updateStatus?.available) {
      setUpdateDialogOpen(true);
      return;
    }

    await refreshUpdateStatus();
  }

  async function openProjectHome() {
    setError(null);
    setInfoMessage(null);
    try {
      await TakoApi.openExternal(PROJECT_URL);
    } catch (err) {
      setError(String(err));
    }
  }

  async function openUpdateDownload() {
    if (!updateStatus) return;
    setError(null);
    setInfoMessage(null);
    try {
      await TakoApi.openExternal(getUpdateOpenUrl(updateStatus));
      setUpdateDialogOpen(false);
    } catch (err) {
      setError(String(err));
    }
  }

  return (
    <main className="app-shell">
      <header className="topbar">
        <div>
          <p className="eyebrow app-brand">
            <span>Tako Switch</span>
            <span className="version-badge">{APP_DISPLAY_VERSION}</span>
          </p>
          <h1>Codex / Claude Code 一键配置</h1>
        </div>
        <div className="topbar-actions">
          <button className="update-button" type="button" onClick={handleUpdateButtonClick} disabled={checkingUpdate}>
            {checkingUpdate ? <Loader2 className="spin" /> : <RefreshCw />}
            <span>更新</span>
            {updateStatus?.available && <span className="new-badge">NEW</span>}
          </button>
          <button className="icon-button" type="button" onClick={openProjectHome} title="打开 GitHub 项目主页">
            <Github />
          </button>
          <button className="icon-button" type="button" onClick={refreshState} disabled={loading} title="重新检测">
            {busy === "loading" ? <Loader2 className="spin" /> : <RefreshCw />}
          </button>
        </div>
      </header>

      <nav className="tabs" aria-label="主要页面">
        <TabButton active={activeTab === "home"} onClick={() => setActiveTab("home")} icon={<Home />}>
          首页
        </TabButton>
        <TabButton
          active={activeTab === "import"}
          onClick={() => setActiveTab("import")}
          icon={<Wand2 />}
        >
          导入配置
        </TabButton>
        <TabButton
          active={activeTab === "current"}
          onClick={() => setActiveTab("current")}
          icon={<FileText />}
        >
          当前配置
        </TabButton>
      </nav>

      {error && (
        <div className="notice error" role="alert">
          <AlertTriangle />
          <span>{error}</span>
        </div>
      )}

      {infoMessage && (
        <div className="notice success" role="status">
          <CheckCircle2 />
          <span>{infoMessage}</span>
        </div>
      )}

      {!provider && <EmptyState text="正在读取服务商配置。" />}

      {activeTab === "home" && provider && (
        <HomeTab
          busy={busy}
          tools={tools}
          onRefresh={refreshState}
          onImport={handleHomeImport}
          onLogin={handleTakoLogin}
          onLogout={handleTakoLogout}
          onRefreshTako={() => refreshTakoDetails()}
          provider={provider}
          takoAccount={takoAccount}
          takoModels={takoModels}
          takoUsage={takoUsage}
        />
      )}

      {activeTab === "import" && provider && (
        <ImportTab
          apiKeyInputRef={apiKeyInputRef}
          busy={busy}
          canSubmit={canSubmit}
          form={form}
          models={takoModels}
          result={result}
          restoreResult={restoreResult}
          showApiKey={showImportApiKey}
          validation={validation}
          onApply={applyConfigs}
          onCreatePreview={createPreview}
          provider={provider}
          onRestore={restore}
          onToggleApiKey={() => setShowImportApiKey((current) => !current)}
          setForm={setForm}
        />
      )}

      {activeTab === "current" && <CurrentTab configs={configs} />}

      {homeImportOpen && provider && (
        <HomeImportModal
          busy={busy}
          canSubmit={canHomeImportSubmit}
          form={homeImportForm}
          models={takoModels}
          provider={provider}
          result={homeResult}
          restoreResult={homeRestoreResult}
          showApiKey={showHomeApiKey}
          validation={homeImportValidation}
          onApply={applyHomeConfigs}
          onClose={() => setHomeImportOpen(false)}
          onConfirm={() => setHomeImportOpen(false)}
          onCreatePreview={createHomePreview}
          onRestore={restoreHome}
          onToggleApiKey={() => setShowHomeApiKey((current) => !current)}
          setForm={setHomeImportForm}
        />
      )}

      {previewModalContext && hasPreviewContent(previewModalContext === "home" ? homePreview : preview) && (
        <PreviewModal
          busy={busy}
          preview={previewModalContext === "home" ? homePreview : preview}
          primaryLabel={previewModalContext === "home" ? "导入配置" : "应用配置"}
          onApply={previewModalContext === "home" ? applyHomeConfigs : applyConfigs}
          onClose={() => setPreviewModalContext(null)}
        />
      )}

      {updateDialogOpen && updateStatus && (
        <UpdateModal
          update={updateStatus}
          onClose={() => setUpdateDialogOpen(false)}
          onOpenDownload={openUpdateDownload}
        />
      )}
    </main>
  );
}

function HomeTab({
  busy,
  takoAccount,
  takoModels,
  takoUsage,
  tools,
  onImport,
  onLogin,
  onLogout,
  onRefresh,
  onRefreshTako,
  provider
}: {
  busy: BusyState;
  takoAccount: TakoAccount;
  takoModels: TakoModel[];
  takoUsage: TakoUsage | null;
  tools: ToolStatus[];
  onImport: () => void;
  onLogin: () => void;
  onLogout: () => void;
  onRefresh: () => void;
  onRefreshTako: () => void;
  provider: AccountProviderConfig;
}) {
  const loading = busy !== null;

  return (
    <section className="home-layout">
      <div className="panel home-main">
        <div className="panel-heading">
          <ShieldCheck />
          <div>
            <h2>本机客户端状态</h2>
            <p>确认工具是否已安装，然后进入安全预览导入流程。</p>
          </div>
        </div>

        <section className="status-strip" aria-label="工具检测状态">
          {tools.length === 0 ? (
            <StatusItem label="检测中" detail="正在读取本机状态" installed={false} />
          ) : (
            tools.map((tool) => (
              <StatusItem
                key={tool.name}
                label={tool.name}
                detail={tool.version || tool.error || "未检测到命令"}
                installed={tool.installed}
              />
            ))
          )}
        </section>

        <div className="button-row">
          <button className="secondary" type="button" disabled={loading} onClick={onRefresh}>
            {busy === "loading" ? <Loader2 className="spin" /> : <RefreshCw />}
            <span>重新检测</span>
          </button>
        </div>
      </div>

      <section className="panel home-main" aria-label={provider.account.label}>
        <div className="panel-heading">
          <KeyRound />
          <div>
            <h2>{provider.account.label}</h2>
            <p>通过浏览器授权获取 ApiKey，确认预览后再写入本机配置。</p>
          </div>
        </div>

        <div className={takoAccount.loggedIn ? "status-item tako-account-card installed" : "status-item tako-account-card"}>
          <KeyRound />
          <div>
            <strong>{takoAccount.loggedIn ? takoAccount.name || "Tako 已登录" : provider.account.loginStatusLabel}</strong>
            <span>
              {takoAccount.loggedIn
                ? takoAccount.offline
                  ? "离线模式：已保留当前 ApiKey。"
                  : takoAccount.plan || "ApiKey 已自动填入导入配置。"
                : provider.account.loginDescription}
            </span>
          </div>
        </div>

        {(takoUsage || takoModels.length > 0) && (
          <div className="tako-detail-grid">
            {takoUsage && (
              <>
                <UsageTile label="5 小时" window={takoUsage.window} />
                <UsageTile label="今日" window={takoUsage.daily} />
                <UsageTile label="本周" window={takoUsage.weekly} />
              </>
            )}
            {takoModels.length > 0 && (
              <div className="tako-model-summary">
                <strong>可用模型</strong>
                <span>{takoModels.length} 个模型</span>
                <small>{summarizeModelClients(takoModels)}</small>
              </div>
            )}
          </div>
        )}

        <div className="button-row compact-row">
          <button className="secondary" type="button" disabled={loading} onClick={onLogin}>
            {busy === "login" ? <Loader2 className="spin" /> : <LogIn />}
            <span>{takoAccount.loggedIn ? "重新登录" : `登录 ${provider.name}`}</span>
          </button>
          <button className="secondary" type="button" disabled={loading || !takoAccount.loggedIn} onClick={onRefreshTako}>
            {busy === "tako" ? <Loader2 className="spin" /> : <RefreshCw />}
            <span>刷新账户</span>
          </button>
          <button className="secondary" type="button" disabled={loading || !takoAccount.loggedIn} onClick={onLogout}>
            {busy === "logout" ? <Loader2 className="spin" /> : <LogOut />}
            <span>登出</span>
          </button>
          <button className="primary import-action wide" type="button" disabled={loading} onClick={onImport}>
            {busy === "preview" ? <Loader2 className="spin" /> : <Wand2 />}
            <span>一键导入 {provider.name} 配置</span>
          </button>
        </div>
      </section>
    </section>
  );
}

function HomeImportModal({
  busy,
  canSubmit,
  form,
  models,
  provider,
  result,
  restoreResult,
  showApiKey,
  validation,
  onApply,
  onClose,
  onConfirm,
  onCreatePreview,
  onRestore,
  onToggleApiKey,
  setForm
}: {
  busy: BusyState;
  canSubmit: boolean;
  form: ConfigInput;
  models: TakoModel[];
  provider: AccountProviderConfig;
  result: ApplyResult | null;
  restoreResult: RestoreResult | null;
  showApiKey: boolean;
  validation: string[];
  onApply: () => void;
  onClose: () => void;
  onConfirm: () => void;
  onCreatePreview: (event?: FormEvent) => void;
  onRestore: (file: AppliedFile) => void;
  onToggleApiKey: () => void;
  setForm: React.Dispatch<React.SetStateAction<ConfigInput>>;
}) {
  const modalPanelRef = useRef<HTMLElement | null>(null);
  useScrollBoundaryGuard(modalPanelRef);

  return (
    <div className="modal-backdrop" role="presentation">
      <section
        ref={modalPanelRef}
        className="modal-panel"
        role="dialog"
        aria-modal="true"
        aria-label={`一键导入 ${provider.name} 配置`}
      >
        <div className="modal-header">
          <div>
            <p className="eyebrow">{provider.name}</p>
            <h2>一键导入配置</h2>
          </div>
          <button className="icon-button" type="button" onClick={onClose} title="关闭">
            <X />
          </button>
        </div>

        {(result || restoreResult) && (
          <ResultsPanel
            busy={busy}
            className="modal-results modal-results-top"
            confirmLabel="确认"
            result={result}
            restoreResult={restoreResult}
            successMessage="配置已保存成功。确认后将关闭窗口。"
            onConfirm={result ? onConfirm : undefined}
            onRestore={onRestore}
          />
        )}

        <div className="modal-grid modal-flow-grid preview-hidden">
          <ProviderConfigForm
            apiKeyReadOnly
            baseUrlsReadOnly
            busy={busy}
            canSubmit={canSubmit}
            className="modal-config"
            description="确认写入目标、网关地址和模型；点击应用配置会自动生成预览并保存。"
            form={form}
            icon={<Settings2 />}
            models={models}
            provider={provider}
            showApiKey={showApiKey}
            title="一键导入配置"
            validation={validation}
            onApply={onApply}
            onCreatePreview={onCreatePreview}
            onToggleApiKey={onToggleApiKey}
            setForm={setForm}
          />
        </div>
      </section>
    </div>
  );
}

function ImportTab({
  apiKeyInputRef,
  busy,
  canSubmit,
  form,
  models,
  result,
  restoreResult,
  showApiKey,
  validation,
  onApply,
  onCreatePreview,
  provider,
  onRestore,
  onToggleApiKey,
  setForm
}: {
  apiKeyInputRef: React.MutableRefObject<HTMLInputElement | null>;
  busy: BusyState;
  canSubmit: boolean;
  form: ConfigInput;
  models: TakoModel[];
  result: ApplyResult | null;
  restoreResult: RestoreResult | null;
  showApiKey: boolean;
  validation: string[];
  onApply: () => void;
  onCreatePreview: (event?: FormEvent) => void;
  provider: AccountProviderConfig;
  onRestore: (file: AppliedFile) => void;
  onToggleApiKey: () => void;
  setForm: React.Dispatch<React.SetStateAction<ConfigInput>>;
}) {
  return (
    <>
      <div className="workspace import-workspace">
        <ProviderConfigForm
          apiKeyInputRef={apiKeyInputRef}
          busy={busy}
          canSubmit={canSubmit}
          className="panel form-panel"
          description="填写网关、密钥和需要写入的客户端；也可以直接应用，系统会先生成预览再保存。"
          form={form}
          icon={<Settings2 />}
          models={models}
          provider={provider}
          showApiKey={showApiKey}
          title="导入配置"
          validation={validation}
          onApply={onApply}
          onCreatePreview={onCreatePreview}
          onToggleApiKey={onToggleApiKey}
          setForm={setForm}
        />
      </div>

      <ResultsPanel
        busy={busy}
        className="panel results-panel"
        result={result}
        restoreResult={restoreResult}
        successMessage="配置已保存成功。下方可以查看写入路径和备份位置。"
        onRestore={onRestore}
      />
    </>
  );
}

function ProviderConfigForm({
  apiKeyInputRef,
  apiKeyReadOnly = false,
  baseUrlsReadOnly = false,
  busy,
  canSubmit,
  className,
  description,
  form,
  icon,
  models,
  provider,
  showApiKey,
  title,
  validation,
  onApply,
  onCreatePreview,
  onToggleApiKey,
  setForm
}: {
  apiKeyInputRef?: React.MutableRefObject<HTMLInputElement | null>;
  apiKeyReadOnly?: boolean;
  baseUrlsReadOnly?: boolean;
  busy: BusyState;
  canSubmit: boolean;
  className: string;
  description: string;
  form: ConfigInput;
  icon: React.ReactNode;
  models: TakoModel[];
  provider: AccountProviderConfig;
  showApiKey: boolean;
  title: string;
  validation: string[];
  onApply: () => void;
  onCreatePreview: (event?: FormEvent) => void;
  onToggleApiKey: () => void;
  setForm: React.Dispatch<React.SetStateAction<ConfigInput>>;
}) {
  const codexModels = sortCodexModels(models.filter(isCodexModel));
  const claudeModels = models.filter(isClaudeModel);
  const codexModel = form.platforms.codex.model || "";
  const claudeModel = form.platforms.claude.model || "";
  const codexModelNotInList = codexModel && codexModels.every((model) => model.id !== codexModel);
  const claudeModelNotInList = claudeModel && claudeModels.every((model) => model.id !== claudeModel);
  const [codexModelMenuOpen, setCodexModelMenuOpen] = useState(false);
  const [claudeModelMenuOpen, setClaudeModelMenuOpen] = useState(false);

  useEffect(() => {
    if (!codexModelMenuOpen && !claudeModelMenuOpen) return;

    function closeMenus() {
      setCodexModelMenuOpen(false);
      setClaudeModelMenuOpen(false);
    }

    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        closeMenus();
      }
    }

    window.addEventListener("click", closeMenus);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("click", closeMenus);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [codexModelMenuOpen, claudeModelMenuOpen]);

  function toggleCodex(enabled: boolean) {
    setForm((current) =>
      updatePlatform("codex", {
        enabled,
        model:
          enabled && !current.platforms.codex.model
            ? selectDefaultCodexModel(models) || provider.platforms.codex?.defaults.model || ""
            : current.platforms.codex.model
      })(current)
    );
  }

  return (
    <form className={className} onSubmit={onCreatePreview}>
      <div className="panel-heading">
        {icon}
        <div>
          <h2>{title}</h2>
          <p>{description}</p>
        </div>
      </div>

      <fieldset className="target-grid">
        <label className={form.platforms.codex.enabled ? "target active" : "target"}>
          <input type="checkbox" checked={form.platforms.codex.enabled} onChange={(event) => toggleCodex(event.target.checked)} />
          <span className="switch-track" aria-hidden="true" />
          <Terminal />
          <span>Codex</span>
        </label>
        <label className={form.platforms.claude.enabled ? "target active" : "target"}>
          <input
            type="checkbox"
            checked={form.platforms.claude.enabled}
            onChange={(event) => setForm(updatePlatform("claude", { enabled: event.target.checked }))}
          />
          <span className="switch-track" aria-hidden="true" />
          <Terminal />
          <span>Claude Code</span>
        </label>
      </fieldset>

      <div className="field-grid">
        <label className="field">
          <span>Codex OpenAI 网关地址</span>
          <input
            readOnly={baseUrlsReadOnly}
            value={form.platforms.codex.baseUrl}
            disabled={!form.platforms.codex.enabled}
            placeholder={provider.platforms.codex?.defaults.baseUrl || ""}
            onChange={(event) => setForm(updatePlatform("codex", { baseUrl: event.target.value }))}
          />
        </label>
        <label className="field">
          <span>Claude Code 网关地址</span>
          <input
            readOnly={baseUrlsReadOnly}
            value={form.platforms.claude.baseUrl}
            disabled={!form.platforms.claude.enabled}
            placeholder={provider.platforms.claude?.defaults.baseUrl || ""}
            onChange={(event) => setForm(updatePlatform("claude", { baseUrl: event.target.value }))}
          />
        </label>
      </div>

      <label className="field">
        <span>API Key / Token</span>
        <div className={apiKeyReadOnly ? "secret-input readonly-secret" : "secret-input"}>
          <KeyRound />
          <input
            ref={apiKeyInputRef}
            readOnly={apiKeyReadOnly}
            type={showApiKey ? "text" : "password"}
            value={form.apiKey}
            placeholder={`粘贴 ${provider.name} ApiKey`}
            onChange={(event) =>
              setForm((current) => ({
                ...current,
                apiKey: event.target.value
              }))
            }
          />
          <button
            className="icon-button inline-icon"
            type="button"
            onClick={onToggleApiKey}
            title={showApiKey ? "隐藏 ApiKey" : "查看 ApiKey"}
          >
            {showApiKey ? <EyeOff /> : <Eye />}
          </button>
        </div>
      </label>

      <div className="field-grid">
        <label className="field">
          <span>Codex 模型</span>
          {codexModels.length > 0 ? (
            <ModelSelect
              open={codexModelMenuOpen}
              selectedModelId={codexModel}
              models={codexModels}
              disabled={!form.platforms.codex.enabled}
              customModelId={codexModelNotInList ? codexModel : ""}
              placeholder="选择 Codex 模型"
              onOpenChange={setCodexModelMenuOpen}
              onSelect={(modelId) => setForm(updatePlatform("codex", { model: modelId }))}
            />
          ) : (
            <input
              value={codexModel}
              disabled={!form.platforms.codex.enabled}
              placeholder={provider.platforms.codex?.defaults.model || "gpt-5.4"}
              onChange={(event) => setForm(updatePlatform("codex", { model: event.target.value }))}
            />
          )}
        </label>
        <label className="field">
          <span>Claude 模型</span>
          {claudeModels.length > 0 ? (
            <ModelSelect
              open={claudeModelMenuOpen}
              selectedModelId={claudeModel}
              models={claudeModels}
              disabled={!form.platforms.claude.enabled}
              customModelId={claudeModelNotInList ? claudeModel : ""}
              placeholder="留空则使用 Claude Code 默认模型"
              clearLabel="使用 Claude Code 默认模型"
              onOpenChange={setClaudeModelMenuOpen}
              onSelect={(modelId) => setForm(updatePlatform("claude", { model: modelId }))}
            />
          ) : (
            <input
              value={claudeModel}
              disabled={!form.platforms.claude.enabled}
              placeholder="留空则使用 Claude Code 默认模型"
              onChange={(event) => setForm(updatePlatform("claude", { model: event.target.value }))}
            />
          )}
        </label>
      </div>

      <AdvancedConfigSection form={form} setForm={setForm} />

      {validation.length > 0 && (
        <div className="notice soft">
          <AlertTriangle />
          <span>{validation[0]}</span>
        </div>
      )}

      <div className="button-row">
        <button className="secondary" type="submit" disabled={!canSubmit}>
          {busy === "preview" ? <Loader2 className="spin" /> : <Eye />}
          <span>生成预览</span>
        </button>
        <button className="primary" type="button" disabled={!canSubmit} onClick={onApply}>
          {busy === "apply" ? <Loader2 className="spin" /> : <Save />}
          <span>应用配置</span>
        </button>
      </div>
    </form>
  );
}

const CODEX_SANDBOX_OPTIONS: Array<{ label: string; value: CodexSandboxMode }> = [
  { label: "只读", value: "read-only" },
  { label: "工作区写入", value: "workspace-write" },
  { label: "完全访问", value: "danger-full-access" }
];

const CODEX_APPROVAL_OPTIONS: Array<{ label: string; value: CodexApprovalPolicy }> = [
  { label: "不受信任时询问", value: "untrusted" },
  { label: "需要时询问", value: "on-request" },
  { label: "从不询问", value: "never" }
];

const CODEX_WINDOWS_SANDBOX_OPTIONS: Array<{ label: string; value: CodexWindowsSandbox }> = [
  { label: "提升权限模式", value: "elevated" },
  { label: "非提升权限模式", value: "unelevated" }
];

const CLAUDE_PERMISSION_OPTIONS: Array<{ label: string; value: ClaudePermissionMode }> = [
  { label: "默认询问", value: "default" },
  { label: "自动接受编辑", value: "acceptEdits" },
  { label: "计划模式", value: "plan" },
  { label: "自动模式", value: "auto" },
  { label: "减少询问", value: "dontAsk" },
  { label: "绕过权限检查", value: "bypassPermissions" }
];

function AdvancedConfigSection({
  form,
  setForm
}: {
  form: ConfigInput;
  setForm: React.Dispatch<React.SetStateAction<ConfigInput>>;
}) {
  const codexOptions = normalizeCodexOptions(form.platforms.codex.options);
  const claudeOptions = normalizeClaudeOptions(form.platforms.claude.options);
  const codexFullAccess =
    codexOptions.sandboxMode === "danger-full-access" && codexOptions.approvalPolicy === "never";
  const claudeBypassPermissions = claudeOptions.permissionsDefaultMode === "bypassPermissions";

  function updateCodexOptions(options: PlatformOptionsInput) {
    setForm(updatePlatform("codex", { options }));
  }

  function updateClaudeOptions(options: PlatformOptionsInput) {
    setForm(updatePlatform("claude", { options }));
  }

  function setCodexFeature(feature: keyof CodexFeatureOptions, value: boolean | null) {
    updateCodexOptions({
      features: {
        ...codexOptions.features,
        [feature]: value
      }
    });
  }

  function toggleCodexFullAccess(enabled: boolean) {
    updateCodexOptions({
      sandboxMode: enabled ? "danger-full-access" : null,
      approvalPolicy: enabled ? "never" : null
    });
  }

  function setClaudePermissionMode(value: ClaudePermissionMode | null) {
    updateClaudeOptions({
      permissionsDefaultMode: value,
      skipDangerousModePermissionPrompt:
        value === "bypassPermissions" ? claudeOptions.skipDangerousModePermissionPrompt : null
    });
  }

  return (
    <section className="advanced-config" aria-label="高级配置">
      <div className="advanced-config-heading">
        <div>
          <h3>高级配置</h3>
          <p>未选择的项目不会修改现有配置；生成预览后可以查看实际写入内容。</p>
        </div>
      </div>

      <div className="advanced-config-grid">
        <div className={!form.platforms.codex.enabled ? "advanced-group disabled" : "advanced-group"}>
          <div className="advanced-group-title">
            <strong>Codex</strong>
            <code>~/.codex/config.toml</code>
          </div>

          <label className="danger-check">
            <input
              type="checkbox"
              checked={codexFullAccess}
              disabled={!form.platforms.codex.enabled}
              onChange={(event) => toggleCodexFullAccess(event.target.checked)}
            />
            <span className="switch-track" aria-hidden="true" />
            <span>开启完全访问权限</span>
          </label>
          {codexFullAccess && (
            <div className="notice danger compact-notice">
              <AlertTriangle />
              <span>会写入完全访问和从不询问，Codex 可跨目录和网络执行操作。</span>
            </div>
          )}

          <div className="advanced-select-grid">
            <TypedSelect<CodexSandboxMode>
              code="sandbox_mode"
              disabled={!form.platforms.codex.enabled}
              label="Codex 沙箱权限"
              options={CODEX_SANDBOX_OPTIONS}
              value={codexOptions.sandboxMode}
              onChange={(value) => updateCodexOptions({ sandboxMode: value })}
            />
            <TypedSelect<CodexApprovalPolicy>
              code="approval_policy"
              disabled={!form.platforms.codex.enabled}
              label="审批策略"
              options={CODEX_APPROVAL_OPTIONS}
              value={codexOptions.approvalPolicy}
              onChange={(value) => updateCodexOptions({ approvalPolicy: value })}
            />
            <TypedSelect<CodexWindowsSandbox>
              code="[windows].sandbox"
              disabled={!form.platforms.codex.enabled}
              label="Windows 沙箱模式"
              options={CODEX_WINDOWS_SANDBOX_OPTIONS}
              value={codexOptions.windowsSandbox}
              onChange={(value) => updateCodexOptions({ windowsSandbox: value })}
            />
          </div>

          <p className="advanced-help">Windows 沙箱模式不是完全访问权限，只影响 Windows 原生沙箱运行方式。</p>

          <div className="feature-list">
            <FeatureToggle
              code="[features].js_repl"
              defaultValue={false}
              disabled={!form.platforms.codex.enabled}
              label="JavaScript REPL"
              value={codexOptions.features.jsRepl}
              onChange={(value) => setCodexFeature("jsRepl", value)}
            />
            <FeatureToggle
              code="[features].unified_exec"
              defaultValue={false}
              disabled={!form.platforms.codex.enabled}
              label="统一执行器"
              value={codexOptions.features.unifiedExec}
              onChange={(value) => setCodexFeature("unifiedExec", value)}
            />
            <FeatureToggle
              code="[features].shell_snapshot"
              defaultValue={true}
              disabled={!form.platforms.codex.enabled}
              label="Shell 快照"
              value={codexOptions.features.shellSnapshot}
              onChange={(value) => setCodexFeature("shellSnapshot", value)}
            />
            <FeatureToggle
              code="[features].memories"
              defaultValue={true}
              disabled={!form.platforms.codex.enabled}
              label="记忆功能"
              value={codexOptions.features.memories}
              onChange={(value) => setCodexFeature("memories", value)}
            />
          </div>
        </div>

        <div className={!form.platforms.claude.enabled ? "advanced-group disabled" : "advanced-group"}>
          <div className="advanced-group-title">
            <strong>Claude Code</strong>
            <code>~/.claude/settings.json</code>
          </div>

          <TypedSelect<ClaudePermissionMode>
            code="permissions.defaultMode"
            disabled={!form.platforms.claude.enabled}
            label="Claude Code 权限模式"
            options={CLAUDE_PERMISSION_OPTIONS}
            value={claudeOptions.permissionsDefaultMode}
            onChange={setClaudePermissionMode}
          />

          <label className="danger-check">
            <input
              type="checkbox"
              checked={claudeOptions.skipDangerousModePermissionPrompt === true}
              disabled={!form.platforms.claude.enabled || !claudeBypassPermissions}
              onChange={(event) =>
                updateClaudeOptions({
                  skipDangerousModePermissionPrompt: event.target.checked
                })
              }
            />
            <span className="switch-track" aria-hidden="true" />
            <span>跳过危险模式确认提示</span>
          </label>
          <p className="advanced-help">
            仅选择“绕过权限检查”时可用；未选择时不会修改现有
            <code>skipDangerousModePermissionPrompt</code>。
          </p>
          {claudeBypassPermissions && (
            <div className="notice danger compact-notice">
              <AlertTriangle />
              <span>绕过权限检查会减少安全确认，只建议在可信目录中使用。</span>
            </div>
          )}
        </div>
      </div>
    </section>
  );
}

function TypedSelect<T extends string>({
  code,
  disabled,
  label,
  options,
  value,
  onChange
}: {
  code: string;
  disabled?: boolean;
  label: string;
  options: Array<{ label: string; value: T }>;
  value: T | null | undefined;
  onChange: (value: T | null) => void;
}) {
  return (
    <label className="advanced-select">
      <span>{label}</span>
      <code>{code}</code>
      <div className="select-shell">
        <select
          disabled={disabled}
          value={value || ""}
          onChange={(event) => onChange((event.target.value || null) as T | null)}
        >
          <option value="">不修改</option>
          {options.map((option) => (
            <option value={option.value} key={option.value}>
              {option.label}
            </option>
          ))}
        </select>
        <ChevronDown />
      </div>
    </label>
  );
}

function FeatureToggle({
  code,
  defaultValue,
  disabled,
  label,
  value,
  onChange
}: {
  code: string;
  defaultValue: boolean;
  disabled?: boolean;
  label: string;
  value: boolean | null | undefined;
  onChange: (value: boolean | null) => void;
}) {
  const selected = value !== null && value !== undefined;

  return (
    <div className={selected ? "feature-toggle selected" : "feature-toggle"}>
      <label className="feature-toggle-check">
        <input
          type="checkbox"
          checked={selected}
          disabled={disabled}
          onChange={(event) => onChange(event.target.checked ? defaultValue : null)}
        />
        <span className="switch-track" aria-hidden="true" />
        <span>{label}</span>
      </label>
      <code>{code}</code>
      <div className="segmented-toggle" aria-label={`${label} 写入值`}>
        <button
          type="button"
          disabled={disabled || !selected}
          className={selected && value === true ? "active" : ""}
          onClick={() => onChange(true)}
        >
          开
        </button>
        <button
          type="button"
          disabled={disabled || !selected}
          className={selected && value === false ? "active" : ""}
          onClick={() => onChange(false)}
        >
          关
        </button>
      </div>
    </div>
  );
}

function normalizeCodexOptions(options: PlatformOptionsInput): Required<Pick<PlatformOptionsInput, "features">> &
  Pick<PlatformOptionsInput, "sandboxMode" | "approvalPolicy" | "windowsSandbox"> {
  return {
    sandboxMode: options.sandboxMode ?? null,
    approvalPolicy: options.approvalPolicy ?? null,
    windowsSandbox: options.windowsSandbox ?? null,
    features: {
      jsRepl: options.features?.jsRepl ?? null,
      unifiedExec: options.features?.unifiedExec ?? null,
      shellSnapshot: options.features?.shellSnapshot ?? null,
      memories: options.features?.memories ?? null
    }
  };
}

function normalizeClaudeOptions(options: PlatformOptionsInput): Pick<
  PlatformOptionsInput,
  "permissionsDefaultMode" | "skipDangerousModePermissionPrompt"
> {
  return {
    permissionsDefaultMode: options.permissionsDefaultMode ?? null,
    skipDangerousModePermissionPrompt: options.skipDangerousModePermissionPrompt ?? null
  };
}

function PreviewPanel({
  className,
  compact = false,
  preview
}: {
  className: string;
  compact?: boolean;
  preview: PreviewResult;
}) {
  if (!hasPreviewContent(preview)) return null;

  return (
    <section className={className} aria-label="写入预览">
      <div className="panel-heading compact-heading">
        <ShieldCheck />
        <div>
          <h2>写入预览</h2>
          <p>密钥会被遮罩，应用前会自动生成备份。</p>
        </div>
      </div>

      {preview.envUpdates.length > 0 && (
        <div className="env-list">
          {preview.envUpdates.map((item) => (
            <div className="env-row" key={item.name}>
              <span>{item.name}</span>
              <code>{item.maskedValue}</code>
              <small>{item.note}</small>
            </div>
          ))}
        </div>
      )}

      {preview.warnings.map((warning) => (
        <div className="notice soft" key={warning}>
          <AlertTriangle />
          <span>{warning}</span>
        </div>
      ))}

      {preview.files.length > 0 && (
        <div className={compact ? "preview-stack compact-preview" : "preview-stack"}>
          {preview.files.map((file) => (
            <PreviewBlock key={file.target} file={file} />
          ))}
        </div>
      )}
    </section>
  );
}

function PreviewModal({
  busy,
  onApply,
  onClose,
  preview,
  primaryLabel
}: {
  busy: BusyState;
  onApply: () => void;
  onClose: () => void;
  preview: PreviewResult;
  primaryLabel: string;
}) {
  const modalPanelRef = useRef<HTMLElement | null>(null);
  const applying = busy === "apply";
  const loading = busy !== null;
  useScrollBoundaryGuard(modalPanelRef);

  function handleApply() {
    onClose();
    onApply();
  }

  return (
    <div className="modal-backdrop" role="presentation">
      <section
        ref={modalPanelRef}
        className="modal-panel preview-modal"
        role="dialog"
        aria-modal="true"
        aria-label="写入预览"
      >
        <div className="modal-header">
          <div>
            <p className="eyebrow">写入确认</p>
            <h2>写入预览</h2>
          </div>
          <button className="icon-button" type="button" onClick={onClose} title="关闭预览" disabled={loading}>
            <X />
          </button>
        </div>

        <PreviewPanel compact className="modal-preview preview-modal-body" preview={preview} />

        <div className="button-row result-actions">
          <button className="primary" type="button" disabled={loading} onClick={handleApply}>
            {applying ? <Loader2 className="spin" /> : <Save />}
            <span>{primaryLabel}</span>
          </button>
          <button className="secondary" type="button" disabled={loading} onClick={onClose}>
            <X />
            <span>关闭</span>
          </button>
        </div>
      </section>
    </div>
  );
}

function ResultsPanel({
  busy,
  className,
  confirmLabel = "确认",
  result,
  restoreResult,
  successMessage,
  onConfirm,
  onRestore
}: {
  busy: BusyState;
  className: string;
  confirmLabel?: string;
  result: ApplyResult | null;
  restoreResult: RestoreResult | null;
  successMessage: string;
  onConfirm?: () => void;
  onRestore: (file: AppliedFile) => void;
}) {
  const loading = busy !== null;

  return (
    <section className={className} aria-label="结果与恢复">
      <div className="panel-heading compact-heading">
        <CheckCircle2 />
        <div>
          <h2>结果与恢复</h2>
          <p>查看写入路径和最近一次备份。</p>
        </div>
      </div>

      {!result && !restoreResult ? (
        <EmptyState text="还没有写入结果。" />
      ) : (
        <div className="result-grid">
          {result && (
            <div className="notice success">
              <CheckCircle2 />
              <span>{successMessage}</span>
            </div>
          )}

          {result?.files.map((file) => (
            <div className="result-row" key={`${file.target}-${file.path}`}>
              <div>
                <strong>{file.target === "codex" ? "Codex" : "Claude Code"}</strong>
                <span>{file.created ? "已创建配置" : "已更新配置"}</span>
                <code>{file.path}</code>
                <small>备份：{file.backupPath}</small>
              </div>
              <button className="secondary compact" type="button" onClick={() => onRestore(file)} disabled={loading}>
                {busy === "restore" ? <Loader2 className="spin" /> : <RotateCcw />}
                <span>恢复</span>
              </button>
            </div>
          ))}

          {restoreResult && (
            <div className="notice success">
              <CheckCircle2 />
              <span>
                已恢复 {restoreResult.target}：
                {restoreResult.deletedTarget ? "目标文件已删除" : restoreResult.path}
              </span>
            </div>
          )}

          {onConfirm && result && (
            <div className="button-row result-actions">
              <button className="primary" type="button" onClick={onConfirm}>
                <CheckCircle2 />
                <span>{confirmLabel}</span>
              </button>
            </div>
          )}
        </div>
      )}
    </section>
  );
}

function CurrentTab({ configs }: { configs: LoadedConfigs | null }) {
  return (
    <section className="panel current-panel standalone-panel">
      <div className="panel-heading">
        <Terminal />
        <div>
          <h2>当前配置</h2>
          <p>只读视图，用于确认现有文件位置。</p>
        </div>
      </div>
      {configs ? (
        <div className="current-grid">
          <CurrentConfigBlock config={configs.codex} />
          <CurrentConfigBlock config={configs.claude} />
        </div>
      ) : (
        <EmptyState text="正在读取当前配置。" />
      )}
    </section>
  );
}

function TabButton({
  active,
  children,
  icon,
  onClick
}: {
  active: boolean;
  children: string;
  icon: React.ReactNode;
  onClick: () => void;
}) {
  return (
    <button className={active ? "tab active" : "tab"} type="button" onClick={onClick}>
      {icon}
      <span>{children}</span>
    </button>
  );
}

function StatusItem({
  label,
  detail,
  installed
}: {
  label: string;
  detail: string;
  installed: boolean;
}) {
  return (
    <div className={installed ? "status-item installed" : "status-item"}>
      {installed ? <CheckCircle2 /> : <AlertTriangle />}
      <div>
        <strong>{label}</strong>
        <span>{detail}</span>
      </div>
    </div>
  );
}

function ModelSelect({
  clearLabel,
  customModelId,
  disabled,
  models,
  onOpenChange,
  onSelect,
  open,
  placeholder,
  selectedModelId
}: {
  clearLabel?: string;
  customModelId: string;
  disabled: boolean;
  models: TakoModel[];
  onOpenChange: (open: boolean) => void;
  onSelect: (modelId: string) => void;
  open: boolean;
  placeholder: string;
  selectedModelId: string;
}) {
  const selectRef = useRef<HTMLDivElement | null>(null);
  const [dropUp, setDropUp] = useState(false);
  const selectedModel = models.find((model) => model.id === selectedModelId);
  const selectedLabel = selectedModel?.name || selectedModel?.id || selectedModelId || placeholder;
  const modelOptions = customModelId
    ? [{ id: customModelId, name: customModelId, provider: "", clients: [] }, ...models]
    : models;

  useEffect(() => {
    if (!open) return;

    function updateMenuDirection() {
      const bounds = selectRef.current?.getBoundingClientRect();
      if (!bounds) return;

      const spaceBelow = window.innerHeight - bounds.bottom;
      const spaceAbove = bounds.top;
      setDropUp(spaceBelow < 300 && spaceAbove > spaceBelow);
    }

    updateMenuDirection();
    window.addEventListener("resize", updateMenuDirection);
    window.addEventListener("scroll", updateMenuDirection, true);
    return () => {
      window.removeEventListener("resize", updateMenuDirection);
      window.removeEventListener("scroll", updateMenuDirection, true);
    };
  }, [open]);

  function handleSelect(modelId: string) {
    onSelect(modelId);
    onOpenChange(false);
  }

  return (
    <div
      ref={selectRef}
      className={["model-select", open ? "open" : "", dropUp ? "drop-up" : ""].filter(Boolean).join(" ")}
      onClick={(event) => event.stopPropagation()}
    >
      <button
        className="model-select-trigger"
        type="button"
        disabled={disabled}
        aria-expanded={open}
        onClick={() => onOpenChange(!open)}
      >
        <span>{selectedLabel}</span>
      </button>

      {open && (
        <div className="model-select-menu" role="listbox">
          {clearLabel && (
            <button
              className={!selectedModelId ? "model-option selected" : "model-option"}
              type="button"
              role="option"
              aria-selected={!selectedModelId}
              onClick={() => handleSelect("")}
            >
              <span className="model-option-name muted">{clearLabel}</span>
            </button>
          )}
          {modelOptions.map((model) => {
            const label = model.name || model.id;
            const selected = model.id === selectedModelId;

            return (
              <button
                className={selected ? "model-option selected" : "model-option"}
                type="button"
                role="option"
                aria-selected={selected}
                key={`${model.id}-${model.provider}`}
                onClick={() => handleSelect(model.id)}
              >
                <span className="model-option-name">{label}</span>
                {model.provider && <span className="provider-tag">{model.provider}</span>}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

function UsageTile({ label, window }: { label: string; window: { used: number; limit: number } }) {
  const percent = window.limit > 0 ? Math.min(100, Math.max(0, (window.used / window.limit) * 100)) : 0;
  return (
    <div className="usage-tile">
      <strong>{label}</strong>
      <span>
        {formatUsage(window.used)} / {formatUsage(window.limit)}
      </span>
      <div className="usage-bar" aria-hidden="true">
        <div style={{ width: `${percent}%` }} />
      </div>
    </div>
  );
}

function summarizeModelClients(models: TakoModel[]) {
  const clients = new Set(models.flatMap((model) => model.clients));
  if (clients.size === 0) return "暂未识别客户端分类";
  return Array.from(clients)
    .map((client) => (client === "claude" ? "Claude" : client === "codex" ? "Codex" : client))
    .join(" / ");
}

function createProviderConfigInput(provider: AccountProviderConfig, apiKey: string, models: TakoModel[]): ConfigInput {
  const codex = TakoProviderConfigService.getPlatform(provider, "codex");
  const claude = TakoProviderConfigService.getPlatform(provider, "claude");
  return {
    providerId: provider.id,
    apiKey,
    platforms: {
      codex: {
        enabled: codex.enabled,
        baseUrl: codex.defaults.baseUrl,
        model: selectDefaultCodexModel(models) || codex.defaults.model || "",
        options: {
          sandboxMode: null,
          approvalPolicy: null,
          windowsSandbox: null,
          features: {
            jsRepl: null,
            unifiedExec: null,
            shellSnapshot: null,
            memories: null
          }
        }
      },
      claude: {
        enabled: claude.enabled,
        baseUrl: claude.defaults.baseUrl,
        model: claude.defaults.model || "",
        options: {
          permissionsDefaultMode: null,
          skipDangerousModePermissionPrompt: null
        }
      }
    }
  };
}

function withProviderDefaults(form: ConfigInput, provider: AccountProviderConfig, models: TakoModel[]): ConfigInput {
  const defaults = createProviderConfigInput(provider, form.apiKey, models);
  return {
    ...defaults,
    ...form,
    providerId: form.providerId || defaults.providerId,
    platforms: {
      codex: {
        ...defaults.platforms.codex,
        ...form.platforms.codex,
        options: mergePlatformOptions(defaults.platforms.codex.options, form.platforms.codex.options)
      },
      claude: {
        ...defaults.platforms.claude,
        ...form.platforms.claude,
        options: mergePlatformOptions(defaults.platforms.claude.options, form.platforms.claude.options)
      }
    }
  };
}

function mergePlatformOptions(defaults: PlatformOptionsInput, current?: PlatformOptionsInput): PlatformOptionsInput {
  return {
    ...defaults,
    ...current,
    features: {
      ...(defaults.features || {
        jsRepl: null,
        unifiedExec: null,
        shellSnapshot: null,
        memories: null
      }),
      ...(current?.features || {})
    }
  };
}

function updatePlatform(
  platformId: "codex" | "claude",
  patch: Partial<PlatformFormInput>
): (current: ConfigInput) => ConfigInput {
  return (current) => ({
    ...current,
    platforms: {
      ...current.platforms,
      [platformId]: {
        ...current.platforms[platformId],
        ...patch,
        options:
          patch.options === undefined
            ? current.platforms[platformId].options
            : mergePlatformOptions(current.platforms[platformId].options, patch.options)
      }
    }
  });
}

function selectDefaultCodexModel(models: TakoModel[]) {
  const sortedModels = sortCodexModels(models.filter(isCodexModel));
  return sortedModels.find((model) => model.provider.toLowerCase().includes("openai"))?.id || sortedModels[0]?.id || "";
}

function isCodexModel(model: TakoModel) {
  return model.clients.includes("codex") || model.provider.toLowerCase().includes("openai");
}

function sortCodexModels(models: TakoModel[]) {
  return [...models].sort(compareCodexModels);
}

function compareCodexModels(left: TakoModel, right: TakoModel) {
  const leftKey = parseCodexModelSortKey(left.id || left.name);
  const rightKey = parseCodexModelSortKey(right.id || right.name);

  if (leftKey.familyRank !== rightKey.familyRank) {
    return leftKey.familyRank - rightKey.familyRank;
  }

  if (leftKey.hasVersion !== rightKey.hasVersion) {
    return leftKey.hasVersion ? -1 : 1;
  }

  const maxLength = Math.max(leftKey.numbers.length, rightKey.numbers.length);
  for (let index = 0; index < maxLength; index += 1) {
    const leftNumber = leftKey.numbers[index] ?? -1;
    const rightNumber = rightKey.numbers[index] ?? -1;
    if (leftNumber !== rightNumber) {
      return rightNumber - leftNumber;
    }
  }

  if (leftKey.hasLetters !== rightKey.hasLetters) {
    return leftKey.hasLetters ? 1 : -1;
  }

  const letterCompare = leftKey.letters.localeCompare(rightKey.letters, undefined, {
    numeric: true,
    sensitivity: "base"
  });
  if (letterCompare !== 0) return letterCompare;

  return leftKey.normalized.localeCompare(rightKey.normalized, undefined, {
    numeric: true,
    sensitivity: "base"
  });
}

function parseCodexModelSortKey(value: string) {
  const normalized = value.trim().toLowerCase();
  const afterGpt = normalized.replace(/^gpt[-_]?/, "");
  const numbers = Array.from(afterGpt.matchAll(/\d+/g), (match) => Number(match[0]));
  const letters = afterGpt.replace(/\d+/g, "").replace(/[._-]+/g, "");
  const familyRank = normalized.startsWith("gpt-image") ? 1 : 0;

  return {
    familyRank,
    hasVersion: numbers.length > 0,
    hasLetters: letters.length > 0,
    letters,
    normalized,
    numbers
  };
}

function isClaudeModel(model: TakoModel) {
  const providerName = model.provider.toLowerCase();
  return model.clients.includes("claude") || providerName.includes("anthropic") || providerName.includes("claude");
}

function hasPreviewContent(preview: PreviewResult) {
  return preview.files.length > 0 || preview.envUpdates.length > 0 || preview.warnings.length > 0;
}

function buildDiffLines(before: string, after: string): DiffLine[] {
  const beforeLines = splitLines(before);
  const afterLines = splitLines(after);

  if (beforeLines.length === 0 && afterLines.length === 0) return [];

  const lcs = buildLcsMatrix(beforeLines, afterLines);
  const lines: DiffLine[] = [];
  let oldIndex = 0;
  let newIndex = 0;

  while (oldIndex < beforeLines.length || newIndex < afterLines.length) {
    if (oldIndex < beforeLines.length && newIndex < afterLines.length && beforeLines[oldIndex] === afterLines[newIndex]) {
      lines.push({
        kind: "context",
        marker: " ",
        text: beforeLines[oldIndex],
        oldLine: oldIndex + 1,
        newLine: newIndex + 1
      });
      oldIndex += 1;
      newIndex += 1;
      continue;
    }

    const removed: DiffLine[] = [];
    const added: DiffLine[] = [];

    while (
      oldIndex < beforeLines.length &&
      (newIndex >= afterLines.length || lcs[oldIndex + 1][newIndex] >= lcs[oldIndex][newIndex + 1])
    ) {
      removed.push({
        kind: "removed",
        marker: "-",
        text: beforeLines[oldIndex],
        oldLine: oldIndex + 1
      });
      oldIndex += 1;

      if (oldIndex < beforeLines.length && newIndex < afterLines.length && beforeLines[oldIndex] === afterLines[newIndex]) {
        break;
      }
    }

    while (
      newIndex < afterLines.length &&
      (oldIndex >= beforeLines.length || lcs[oldIndex][newIndex + 1] > lcs[oldIndex + 1][newIndex])
    ) {
      added.push({
        kind: "added",
        marker: "+",
        text: afterLines[newIndex],
        newLine: newIndex + 1
      });
      newIndex += 1;

      if (oldIndex < beforeLines.length && newIndex < afterLines.length && beforeLines[oldIndex] === afterLines[newIndex]) {
        break;
      }
    }

    const pairedCount = Math.min(removed.length, added.length);
    for (let index = 0; index < pairedCount; index += 1) {
      lines.push({
        kind: "modified",
        marker: "~",
        text: `${removed[index].text} -> ${added[index].text}`,
        oldLine: removed[index].oldLine,
        newLine: added[index].newLine
      });
    }
    lines.push(...removed.slice(pairedCount), ...added.slice(pairedCount));
  }

  return lines;
}

function buildLcsMatrix(beforeLines: string[], afterLines: string[]) {
  const rows = beforeLines.length + 1;
  const columns = afterLines.length + 1;
  const matrix = Array.from({ length: rows }, () => Array<number>(columns).fill(0));

  for (let row = beforeLines.length - 1; row >= 0; row -= 1) {
    for (let column = afterLines.length - 1; column >= 0; column -= 1) {
      matrix[row][column] =
        beforeLines[row] === afterLines[column]
          ? matrix[row + 1][column + 1] + 1
          : Math.max(matrix[row + 1][column], matrix[row][column + 1]);
    }
  }

  return matrix;
}

function splitLines(value: string) {
  if (!value) return [];
  return value.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n");
}

function formatUsage(value: number) {
  return value.toFixed(value >= 10 ? 1 : 2);
}

function formatDate(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleDateString();
}

function PreviewBlock({ file }: { file: FilePreview }) {
  const [expanded, setExpanded] = useState(false);
  const title = file.target === "codex" ? "Codex config.toml" : "Claude settings.json";
  const diffLines = useMemo(() => buildDiffLines(file.before, file.after), [file.after, file.before]);

  return (
    <article className="preview-block">
      <div className="preview-block-header">
        <div className="file-meta">
          <strong>{title}</strong>
          <span>{file.exists ? "更新已有文件" : "创建新文件"}</span>
          <code>{file.path}</code>
          <small>备份将写入：{file.backupPath}</small>
        </div>
        <button className="secondary compact" type="button" onClick={() => setExpanded(true)} title="全窗口查看 diff">
          <Maximize2 />
          <span>展开</span>
        </button>
      </div>
      <DiffSummary compact lines={diffLines} />
      {expanded && <DiffFullscreenModal diffLines={diffLines} file={file} title={title} onClose={() => setExpanded(false)} />}
    </article>
  );
}

function DiffSummary({ compact = false, lines }: { compact?: boolean; lines: DiffLine[] }) {
  const changedLines = lines.filter((line) => line.kind !== "context");
  const displayLines = compact ? (changedLines.length > 0 ? changedLines : lines).slice(0, 14) : lines;
  const hiddenCount = compact ? Math.max(0, (changedLines.length > 0 ? changedLines.length : lines.length) - displayLines.length) : 0;

  if (displayLines.length === 0) {
    return <EmptyState text="没有检测到内容差异。" />;
  }

  return (
    <div className={compact ? "diff-summary compact-diff-summary" : "diff-summary"} role="list">
      {displayLines.map((line, index) => (
        <div className={`diff-line ${line.kind}`} role="listitem" key={`${line.kind}-${line.oldLine || 0}-${line.newLine || 0}-${index}`}>
          <span className="diff-marker" aria-hidden="true">
            {line.marker}
          </span>
          <span className="diff-line-number">{formatDiffLineNumber(line)}</span>
          <code>{line.text || " "}</code>
        </div>
      ))}
      {hiddenCount > 0 && <div className="diff-more">还有 {hiddenCount} 行变化，展开查看完整 diff。</div>}
    </div>
  );
}

function DiffFullscreenModal({
  diffLines,
  file,
  onClose,
  title
}: {
  diffLines: DiffLine[];
  file: FilePreview;
  onClose: () => void;
  title: string;
}) {
  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  return (
    <div className="diff-fullscreen-backdrop" role="presentation">
      <section className="diff-fullscreen-panel" role="dialog" aria-modal="true" aria-label={`${title} 全窗口 diff`}>
        <div className="modal-header">
          <div>
            <p className="eyebrow">{file.exists ? "更新已有文件" : "创建新文件"}</p>
            <h2>{title}</h2>
            <code>{file.path}</code>
            <small>备份将写入：{file.backupPath}</small>
          </div>
          <button className="icon-button" type="button" onClick={onClose} title="关闭 diff">
            <X />
          </button>
        </div>

        <div className="diff-fullscreen-body">
          <DiffSummary lines={diffLines} />
        </div>
      </section>
    </div>
  );
}

function formatDiffLineNumber(line: DiffLine) {
  if (line.oldLine && line.newLine) return `${line.oldLine} -> ${line.newLine}`;
  if (line.oldLine) return String(line.oldLine);
  if (line.newLine) return String(line.newLine);
  return "";
}

function CurrentConfigBlock({ config }: { config: ExistingConfig }) {
  const [expanded, setExpanded] = useState(false);
  const title = config.target === "codex" ? "Codex" : "Claude Code";
  const status = config.exists ? "已存在" : "未创建";
  const content = config.content || "(文件不存在或为空)";

  return (
    <>
      <article className="current-block">
        <div className="current-block-header">
          <div>
            <strong>{title}</strong>
            <span>{status}</span>
          </div>
          <button className="icon-button" type="button" onClick={() => setExpanded(true)} title={`展开查看 ${title} 配置`}>
            <Maximize2 />
          </button>
        </div>
        <code>{config.path}</code>
        <ReadOnlyConfigViewer content={content} />
      </article>

      {expanded && (
        <CurrentConfigFullscreenModal
          config={config}
          content={content}
          status={status}
          title={title}
          onClose={() => setExpanded(false)}
        />
      )}
    </>
  );
}

function EmptyState({ text }: { text: string }) {
  return <div className="empty-state">{text}</div>;
}

function ReadOnlyConfigViewer({ content, expanded = false }: { content: string; expanded?: boolean }) {
  return (
    <pre className={expanded ? "current-config-view expanded" : "current-config-view"} tabIndex={0}>
      <code>{content}</code>
    </pre>
  );
}

function CurrentConfigFullscreenModal({
  config,
  content,
  onClose,
  status,
  title
}: {
  config: ExistingConfig;
  content: string;
  onClose: () => void;
  status: string;
  title: string;
}) {
  const modalPanelRef = useRef<HTMLElement | null>(null);
  useBodyScrollLock(true);
  useScrollBoundaryGuard(modalPanelRef);

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  return (
    <div className="diff-fullscreen-backdrop" role="presentation">
      <section
        ref={modalPanelRef}
        className="diff-fullscreen-panel current-fullscreen-panel"
        role="dialog"
        aria-modal="true"
        aria-label={`${title} 当前配置`}
      >
        <div className="modal-header current-fullscreen-header">
          <div>
            <p className="eyebrow">只读配置</p>
            <h2>{title}</h2>
            <span>{status}</span>
            <code>{config.path}</code>
          </div>
          <button className="icon-button" type="button" onClick={onClose} title="关闭当前配置">
            <X />
          </button>
        </div>

        <div className="diff-fullscreen-body current-fullscreen-body">
          <ReadOnlyConfigViewer content={content} expanded />
        </div>
      </section>
    </div>
  );
}

function useBodyScrollLock(locked: boolean) {
  useEffect(() => {
    if (!locked) return;

    const { body, documentElement } = document;
    const previousBodyOverflow = body.style.overflow;
    const previousBodyPaddingRight = body.style.paddingRight;
    const previousHtmlOverscroll = documentElement.style.overscrollBehavior;
    const scrollbarWidth = window.innerWidth - documentElement.clientWidth;

    body.style.overflow = "hidden";
    if (scrollbarWidth > 0) {
      body.style.paddingRight = `${scrollbarWidth}px`;
    }
    documentElement.style.overscrollBehavior = "none";

    return () => {
      body.style.overflow = previousBodyOverflow;
      body.style.paddingRight = previousBodyPaddingRight;
      documentElement.style.overscrollBehavior = previousHtmlOverscroll;
    };
  }, [locked]);
}

function useScrollBoundaryGuard(ref: React.RefObject<HTMLElement>) {
  useEffect(() => {
    const panelElement = ref.current;
    if (!panelElement) return;
    const panel: HTMLElement = panelElement;

    function handleWheel(event: WheelEvent) {
      const { deltaY } = event;
      if (deltaY === 0) return;
      const scrollTarget = findScrollableAncestor(event.target, panel) ?? panel;
      const canScroll = scrollTarget.scrollHeight > scrollTarget.clientHeight + 1;
      if (!canScroll) {
        event.preventDefault();
        event.stopPropagation();
        return;
      }

      const scrollingDown = deltaY > 0;
      const atTop = scrollTarget.scrollTop <= 0;
      const atBottom = scrollTarget.scrollTop + scrollTarget.clientHeight >= scrollTarget.scrollHeight - 1;

      if ((scrollingDown && atBottom) || (!scrollingDown && atTop)) {
        event.preventDefault();
      }
      event.stopPropagation();
    }

    panel.addEventListener("wheel", handleWheel, { passive: false });
    return () => panel.removeEventListener("wheel", handleWheel);
  }, [ref]);
}

function findScrollableAncestor(target: EventTarget | null, boundary: HTMLElement) {
  let element = target instanceof HTMLElement ? target : null;
  while (element && boundary.contains(element)) {
    const style = window.getComputedStyle(element);
    const canScrollY =
      /(auto|scroll|overlay)/.test(style.overflowY) && element.scrollHeight > element.clientHeight + 1;
    if (canScrollY) return element;
    if (element === boundary) break;
    element = element.parentElement;
  }
  return null;
}

function UpdateModal({
  onClose,
  onOpenDownload,
  update
}: {
  onClose: () => void;
  onOpenDownload: () => void;
  update: AppUpdateStatus;
}) {
  const modalPanelRef = useRef<HTMLElement | null>(null);
  useScrollBoundaryGuard(modalPanelRef);

  return (
    <div className="modal-backdrop" role="presentation">
      <section
        ref={modalPanelRef}
        className="modal-panel update-modal"
        role="dialog"
        aria-modal="true"
        aria-label="应用更新"
      >
        <div className="modal-header">
          <div>
            <p className="eyebrow">Tako Switch 更新</p>
            <h2>发现新版本 v{update.latestVersion}</h2>
            <p className="update-summary">
              当前版本 {APP_DISPLAY_VERSION}
              {update.publishedAt ? ` · 发布于 ${formatDate(update.publishedAt)}` : ""}
            </p>
          </div>
          <button className="icon-button" type="button" onClick={onClose} title="关闭更新">
            <X />
          </button>
        </div>

        <div className="update-details">
          <div className="update-version-grid">
            <div>
              <span>当前版本</span>
              <strong>{APP_DISPLAY_VERSION}</strong>
            </div>
            <div>
              <span>最新版本</span>
              <strong>v{update.latestVersion}</strong>
            </div>
          </div>

          <div className="notice soft">
            <AlertTriangle />
            <span>
              当前阶段会打开 GitHub Release 安装包，由系统或浏览器完成下载与安装。后续可切换到 Tauri 官方自动更新。
            </span>
          </div>

          <div className="update-asset">
            <strong>{update.asset ? "将打开当前平台安装包" : "未找到当前平台安装包"}</strong>
            <span>{update.asset?.name || "将打开 GitHub Release 页面手动选择下载。"}</span>
          </div>

          {update.releaseNotes && (
            <label className="update-notes">
              <span>Release Notes</span>
              <textarea readOnly value={update.releaseNotes} />
            </label>
          )}
        </div>

        <div className="button-row result-actions">
          <button className="secondary" type="button" onClick={onClose}>
            <X />
            <span>稍后</span>
          </button>
          <button className="primary" type="button" onClick={onOpenDownload}>
            <RefreshCw />
            <span>{update.asset ? "打开安装包" : "打开 Release"}</span>
          </button>
        </div>
      </section>
    </div>
  );
}

function validateLocal(form: ConfigInput, provider: AccountProviderConfig | null) {
  const errors: string[] = [];
  if (!provider) {
    errors.push("正在读取服务商配置。");
    return errors;
  }
  if (!form.platforms.codex.enabled && !form.platforms.claude.enabled) {
    errors.push("至少选择 Codex 或 Claude Code。");
  }
  if (!form.apiKey.trim()) {
    errors.push("API Key / Token 不能为空。");
  }
  if (form.platforms.codex.enabled && !/^https?:\/\/.+/i.test(form.platforms.codex.baseUrl.trim())) {
    errors.push("Codex 地址必须以 http:// 或 https:// 开头。");
  }
  if (form.platforms.claude.enabled && !/^https?:\/\/.+/i.test(form.platforms.claude.baseUrl.trim())) {
    errors.push("Claude Code 地址必须以 http:// 或 https:// 开头。");
  }
  if (form.platforms.codex.enabled && provider.platforms.codex?.rules.model?.required && !form.platforms.codex.model?.trim()) {
    errors.push("选择 Codex 时必须填写 Codex 模型。");
  }
  if (form.platforms.codex.enabled) {
    const codexOptions = normalizeCodexOptions(form.platforms.codex.options);
    if (codexOptions.sandboxMode && !CODEX_SANDBOX_OPTIONS.some((option) => option.value === codexOptions.sandboxMode)) {
      errors.push("Codex 沙箱权限不是有效选项。");
    }
    if (codexOptions.approvalPolicy && !CODEX_APPROVAL_OPTIONS.some((option) => option.value === codexOptions.approvalPolicy)) {
      errors.push("Codex 审批策略不是有效选项。");
    }
    if (codexOptions.windowsSandbox && !CODEX_WINDOWS_SANDBOX_OPTIONS.some((option) => option.value === codexOptions.windowsSandbox)) {
      errors.push("Windows 沙箱模式不是有效选项。");
    }
  }
  if (form.platforms.claude.enabled) {
    const claudeOptions = normalizeClaudeOptions(form.platforms.claude.options);
    if (
      claudeOptions.permissionsDefaultMode &&
      !CLAUDE_PERMISSION_OPTIONS.some((option) => option.value === claudeOptions.permissionsDefaultMode)
    ) {
      errors.push("Claude Code 权限模式不是有效选项。");
    }
  }
  const forbiddenClaudeSuffixes = provider.platforms.claude?.rules.baseUrl?.forbidPathSuffixes || [];
  const claudePath = getUrlPath(form.platforms.claude.baseUrl);
  if (
    form.platforms.claude.enabled &&
    forbiddenClaudeSuffixes.some((suffix) => claudePath.endsWith(suffix.replace(/\/+$/, "")))
  ) {
    errors.push("Claude Code 网关地址不要包含 /v1，请填写域名根地址。");
  }
  return errors;
}

function getUrlPath(value: string) {
  try {
    return new URL(value.trim()).pathname.replace(/\/+$/, "");
  } catch {
    return "";
  }
}

export default App;
