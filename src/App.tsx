import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle,
  CheckCircle2,
  Eye,
  EyeOff,
  FileText,
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

type ActiveTab = "home" | "import" | "current";

type PlatformFormInput = {
  enabled: boolean;
  baseUrl: string;
  model?: string;
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
      model: ""
    },
    claude: {
      enabled: true,
      baseUrl: "",
      model: ""
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
  const [busy, setBusy] = useState<BusyState>(null);

  const validation = useMemo(() => validateLocal(form, provider), [form, provider]);
  const homeImportValidation = useMemo(() => validateLocal(homeImportForm, provider), [homeImportForm, provider]);
  const canSubmit = validation.length === 0 && busy === null;
  const canHomeImportSubmit = homeImportValidation.length === 0 && busy === null;
  const loading = busy !== null;

  useEffect(() => {
    void refreshState();
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
      const [toolStatuses, loadedConfigs, storedSession, storedApplyResult, loadedProviderCatalog] = await Promise.all([
        invoke<ToolStatus[]>("detect_tools"),
        invoke<LoadedConfigs>("load_current_configs"),
        TakoSessionStore.load(),
        invoke<ApplyResult | null>("load_latest_apply_result"),
        TakoProviderConfigService.loadCatalog()
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
      if (storedApplyResult) {
        setResult(storedApplyResult);
        setHomeResult(storedApplyResult);
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
      setActiveTab("import");
    } catch (err) {
      setError(String(err));
      setPreview(emptyPreview);
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
    } catch (err) {
      setError(String(err));
      setHomePreview(emptyPreview);
      setPreview(emptyPreview);
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
      setHomeResult(null);
      setHomeRestoreResult(null);
      setHomeImportOpen(false);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  }

  return (
    <main className="app-shell">
      <header className="topbar">
        <div>
          <p className="eyebrow">Tako Switch</p>
          <h1>Codex / Claude Code 一键配置</h1>
        </div>
        <button className="icon-button" onClick={refreshState} disabled={loading} title="重新检测">
          {busy === "loading" ? <Loader2 className="spin" /> : <RefreshCw />}
        </button>
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
          preview={preview}
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
          preview={homePreview}
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
  preview,
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
  preview: PreviewResult;
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
  return (
    <div className="modal-backdrop" role="presentation">
      <section className="modal-panel" role="dialog" aria-modal="true" aria-label={`一键导入 ${provider.name} 配置`}>
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

        <div className="modal-grid modal-flow-grid">
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
          <PreviewPanel
            compact
            className="modal-preview"
            emptyText="点击“应用配置”会自动生成预览并保存；也可以先生成预览查看差异。"
            preview={preview}
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
  preview,
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
  preview: PreviewResult;
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
      <div className="workspace">
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

        <PreviewPanel
          className="panel"
          emptyText="点击“生成预览”查看将写入的配置，或直接点击“应用配置”。"
          preview={preview}
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
  const codexModels = models.filter(isCodexModel);
  const codexModel = form.platforms.codex.model || "";
  const codexModelNotInList = codexModel && codexModels.every((model) => model.id !== codexModel);

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
          <Terminal />
          <span>Codex</span>
        </label>
        <label className={form.platforms.claude.enabled ? "target active" : "target"}>
          <input
            type="checkbox"
            checked={form.platforms.claude.enabled}
            onChange={(event) => setForm(updatePlatform("claude", { enabled: event.target.checked }))}
          />
          <Terminal />
          <span>Claude Code</span>
        </label>
      </fieldset>

      <div className="field-grid">
        <label className="field">
          <span>Codex OpenAI 兼容地址</span>
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
            <select
              value={codexModel}
              disabled={!form.platforms.codex.enabled}
              onChange={(event) => setForm(updatePlatform("codex", { model: event.target.value }))}
            >
              {codexModelNotInList && <option value={codexModel}>{codexModel}</option>}
              {codexModels.map((model) => (
                <option key={model.id} value={model.id}>
                  {model.name || model.id}
                  {model.provider ? `  ${model.provider}` : ""}
                </option>
              ))}
            </select>
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
          <input
            value={form.platforms.claude.model || ""}
            disabled={!form.platforms.claude.enabled}
            placeholder="留空则使用 Claude Code 默认模型"
            onChange={(event) => setForm(updatePlatform("claude", { model: event.target.value }))}
          />
        </label>
      </div>

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

function PreviewPanel({
  className,
  compact = false,
  emptyText,
  preview
}: {
  className: string;
  compact?: boolean;
  emptyText: string;
  preview: PreviewResult;
}) {
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

      {preview.files.length === 0 ? (
        <EmptyState text={emptyText} />
      ) : (
        <div className={compact ? "preview-stack compact-preview" : "preview-stack"}>
          {preview.files.map((file) => (
            <PreviewBlock key={file.target} file={file} />
          ))}
        </div>
      )}
    </section>
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
          <p>查看写入路径、环境变量提示和最近一次备份。</p>
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

          {result?.envUpdates.map((item) => (
            <div className="result-row" key={item}>
              <div>
                <strong>环境变量</strong>
                <span>{item}</span>
              </div>
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
        model: selectDefaultCodexModel(models) || codex.defaults.model || ""
      },
      claude: {
        enabled: claude.enabled,
        baseUrl: claude.defaults.baseUrl,
        model: claude.defaults.model || ""
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
        ...form.platforms.codex
      },
      claude: {
        ...defaults.platforms.claude,
        ...form.platforms.claude
      }
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
        ...patch
      }
    }
  });
}

function selectDefaultCodexModel(models: TakoModel[]) {
  return (
    models.find((model) => model.provider.toLowerCase().includes("openai"))?.id ||
    models.find(isCodexModel)?.id ||
    ""
  );
}

function isCodexModel(model: TakoModel) {
  return model.clients.includes("codex") || model.provider.toLowerCase().includes("openai");
}

function formatUsage(value: number) {
  return value.toFixed(value >= 10 ? 1 : 2);
}

function PreviewBlock({ file }: { file: FilePreview }) {
  const [expanded, setExpanded] = useState(false);
  const title = file.target === "codex" ? "Codex config.toml" : "Claude settings.json";

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
      <div className="diff-grid">
        <label>
          <span>当前</span>
          <textarea readOnly value={file.before || "(文件不存在或为空)"} />
        </label>
        <label>
          <span>写入后</span>
          <textarea readOnly value={file.after} />
        </label>
      </div>
      {expanded && <DiffFullscreenModal file={file} title={title} onClose={() => setExpanded(false)} />}
    </article>
  );
}

function DiffFullscreenModal({
  file,
  onClose,
  title
}: {
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
          </div>
          <button className="icon-button" type="button" onClick={onClose} title="关闭 diff">
            <X />
          </button>
        </div>

        <div className="diff-fullscreen-grid">
          <label>
            <span>当前</span>
            <textarea readOnly value={file.before || "(文件不存在或为空)"} />
          </label>
          <label>
            <span>写入后</span>
            <textarea readOnly value={file.after} />
          </label>
        </div>
      </section>
    </div>
  );
}

function CurrentConfigBlock({ config }: { config: ExistingConfig }) {
  return (
    <article className="current-block">
      <strong>{config.target === "codex" ? "Codex" : "Claude Code"}</strong>
      <span>{config.exists ? "已存在" : "未创建"}</span>
      <code>{config.path}</code>
      <textarea readOnly value={config.content || "(文件不存在或为空)"} />
    </article>
  );
}

function EmptyState({ text }: { text: string }) {
  return <div className="empty-state">{text}</div>;
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
