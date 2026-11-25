import { invoke } from "@tauri-apps/api/core";

export type AppId = "dsh-tako" | "codex" | "claude" | "antigravity";
export type AppMode = "official" | "tako";
export type UiDensity = "compact" | "comfortable";
export type ThemePref = "system" | "light" | "dark";

export interface QuotaSummary {
  windowCostLimit: number;
  windowUsed: number;
  windowMinutes: number;
  dailyCostLimit: number;
  dailyUsed: number;
  weeklyCostLimit: number;
  weeklyUsed: number;
}

export interface AccountStatus {
  loggedIn: boolean;
  baseUrl: string;
  maskedKey: string | null;
  userName: string | null;
  planName: string | null;
  walletRemain: number | null;
  walletUsed: number | null;
  walletRemainUsd: number | null;
  walletUsedUsd: number | null;
  quota: QuotaSummary | null;
  error: string | null;
}

export interface McpToolsConfig {
  image: boolean;
  video: boolean;
  audio: boolean;
  search: boolean;
  consult: boolean;
  webPage?: boolean;
  edgeoneApiToken?: string | null;
  edgeoneProjectName?: string | null;
  outputDir: string;
  consultModel: string;
}

export interface NodeRuntimeInfo {
  available: boolean;
  version: string | null;
  major: number | null;
  meetsMinVersion: boolean;
  nodePath: string | null;
  npxPath: string | null;
}

export type ModelFilterMode = "recommended" | "custom" | "all";

export interface ClientModelFilter {
  mode: ModelFilterMode;
  enabledIds: string[];
}

export interface ModelFiltersConfig {
  dshTako?: ClientModelFilter;
  claude: ClientModelFilter;
  codex: ClientModelFilter;
  antigravity: ClientModelFilter;
}

export interface ModelChannelPrice {
  name: string;
  ratio: number;
  standardLoadUsdPerM: number | null;
}

export interface ClientModelOption {
  id: string;
  displayName: string;
  family: string;
  description: string;
  contextWindow: number | null;
  recommended: boolean;
  visible: boolean;
  effortLevels: string[];
  isFree: boolean;
  isDynamicPricing: boolean;
  standardLoadUsdPerM: number | null;
  standardLoadMaxUsdPerM: number | null;
  realizedUsdPerM: number | null;
  cacheHitRate: number | null;
  inputUsdPerM: number | null;
  cacheUsdPerM: number | null;
  outputUsdPerM: number | null;
  requestUsd: number | null;
  quotaType: number;
  modelRatio: number | null;
  officialMultiplier: number | null;
  promoEndsOn: string | null;
  capabilities: string[];
  tags: string[];
  vendorName: string | null;
  channels: ModelChannelPrice[];
  avgTps: number | null;
  avgLatencyMs: number | null;
  successRate: number | null;
}

export interface ClientModelsResponse {
  appId: AppId;
  filter: ClientModelFilter;
  recommendedIds: string[];
  visibleCount: number;
  totalCount: number;
  models: ClientModelOption[];
}

export interface SwitchState {
  baseUrl: string;
  agyApiKey?: string | null;
  manifestUrl: string;
  fallbackManifestUrl: string;
  autoCheckUpdates: boolean;
  codexPreserveOfficialAuth?: boolean;
  gatewayDisplayName?: string;
  gatewayDisplaySubtitle?: string;
  preferChineseUi?: boolean;
  showTrayBalance?: boolean;
  bypassSystemProxy?: boolean;
  telemetryEnabled?: boolean;
  uiDensity?: UiDensity;
  themePref?: ThemePref;
  mcp: McpToolsConfig;
  modelFilters?: ModelFiltersConfig;
  explicitOfficialApps?: AppId[];
}

export interface InstallInfo {
  installed: boolean;
  version: string | null;
  installPath: string | null;
  installKind: string | null;
}

export interface DiagnosticResult {
  ok: boolean;
  issues: string[];
  notes: string[];
}

export type UiPatchStatus =
  | "not_applicable"
  | "not_installed"
  | "unpatched"
  | "patched"
  | "outdated_after_upgrade"
  | "unsupported";

export type UiPatchBlockKind =
  | "macos_app_management"
  | "windows_uac"
  | "windows_file_lock"
  | "windows_msix_read_only";

export interface UiPatchPermissionPrompt {
  appId: AppId;
  appName: string;
  action: "patch" | "restore" | string;
  kind: UiPatchBlockKind;
  targetPath: string | null;
  message: string;
}

export const UI_PATCH_BLOCK_PREFIX = "[TAKO_UI_PATCH_BLOCK]";

export function parseUiPatchBlockError(err: unknown): UiPatchPermissionPrompt | null {
  const raw = typeof err === "string" ? err : err instanceof Error ? err.message : String(err ?? "");
  const idx = raw.indexOf(UI_PATCH_BLOCK_PREFIX);
  if (idx < 0) return null;
  const rest = raw.slice(idx + UI_PATCH_BLOCK_PREFIX.length).trimStart();
  if (!rest.startsWith("{")) return null;

  let depth = 0;
  let inString = false;
  let escaped = false;
  let endIdx = -1;
  for (let i = 0; i < rest.length; i += 1) {
    const ch = rest[i];
    if (inString) {
      if (escaped) {
        escaped = false;
        continue;
      }
      if (ch === "\\") {
        escaped = true;
        continue;
      }
      if (ch === '"') {
        inString = false;
      }
      continue;
    }
    if (ch === '"') {
      inString = true;
    } else if (ch === "{") {
      depth += 1;
    } else if (ch === "}") {
      depth -= 1;
      if (depth === 0) {
        endIdx = i + 1;
        break;
      }
    }
  }
  if (endIdx < 0) return null;
  try {
    const parsed = JSON.parse(rest.slice(0, endIdx)) as Partial<UiPatchPermissionPrompt>;
    if (!parsed || !parsed.appId || !parsed.kind || !parsed.message) {
      return null;
    }
    return {
      appId: parsed.appId,
      appName: parsed.appName ?? parsed.appId,
      action: parsed.action ?? "patch",
      kind: parsed.kind,
      targetPath: parsed.targetPath ?? null,
      message: parsed.message,
    };
  } catch {
    return null;
  }
}

export interface UiPatchInfo {
  status: UiPatchStatus;
  patchVersion: string | null;
  targetPatchVersion?: string | null;
  updateAvailable?: boolean;
  hasValidBackup: boolean;
  targetAsarPath: string | null;
  note: string | null;
}

export interface UiPatchAutoSyncEvent {
  updatedApps: AppId[];
  deferredRunningApps: AppId[];
}

export interface AppSnapshot {
  id: AppId;
  name: string;
  mode: AppMode;
  mcpEnabled: boolean;
  hasBackup: boolean;
  running: boolean;
  install: InstallInfo;
  diagnostic: DiagnosticResult;
  chineseConfigured: boolean;
  uiPatch: UiPatchInfo;
}

export interface PlatformArtifact {
  url: string;
  fallbackUrl?: string;
  version?: string;
  sha256?: string;
  sha512?: string;
  size?: number;
  installType: string;
  appBundleName?: string;
}

export interface Manifest {
  schemaVersion: number;
  generatedAt: string;
  apps: Record<
    string,
    {
      displayName: string;
      version: string;
      platforms: Record<string, PlatformArtifact>;
    }
  >;
}

export interface AppUpdateStatus {
  appId: AppId;
  installed: boolean;
  installedVersion: string | null;
  remoteVersion: string | null;
  hasUpdate: boolean;
  platformSupported: boolean;
  artifact: PlatformArtifact | null;
}

export interface ManifestCheckResult {
  sourceUrl: string;
  manifest: Manifest;
  updates: AppUpdateStatus[];
}

export interface DashboardPayload {
  account: AccountStatus;
  state: SwitchState;
  apps: AppSnapshot[];
  cachedManifest: Manifest | null;
  localGatewayHealthy: boolean;
  localGatewayUrl: string;
  appVersion: string;
  os: string;
  uiPatchPrompts?: UiPatchPermissionPrompt[];
}

export const DEFAULT_MANIFEST_URL =
  "https://dsh-tako-dl.oss-cn-shanghai.aliyuncs.com/tako-switch/manifest.json";
export const DEFAULT_FALLBACK_MANIFEST_URL =
  "https://raw.githubusercontent.com/tako-dev/tako-switch/main/manifest/manifest.json";
export const TAKO_FEEDBACK_URL = "https://github.com/tako-dev/tako-switch/issues";
export const TAKO_RELEASES_URL = "https://github.com/tako-dev/tako-switch/releases/latest";

export interface InstallProgressEvent {
  appId: AppId;
  stage: string;
  percent: number;
  message: string;
}

export interface UiPatchProgressEvent {
  scope: "all" | AppId;
  action: "patch" | "restore";
  stage: "preparing" | "config" | "patching" | "restoring" | "finalizing" | "done" | "error";
  percent: number;
  message: string;
  currentApp?: AppId | null;
}

export interface InstallOutcome {
  appId: AppId;
  modeReapplied: AppMode;
  snapshot: AppSnapshot;
  downloadRoute?: string | null;
  downloadBytes?: number | null;
  downloadDurationMs?: number | null;
  downloadSpeedKbps?: number | null;
}

export interface SelfUpdateCheckResult {
  available: boolean;
  version: string | null;
  notes: string | null;
  source: "oss" | "github" | string;
}

export interface SelfUpdateProgressEvent {
  stage: string;
  downloaded: number;
  total: number | null;
}

export interface SettingsPatch {
  baseUrl?: string;
  agyApiKey?: string;
  manifestUrl?: string;
  fallbackManifestUrl?: string;
  autoCheckUpdates?: boolean;
  codexPreserveOfficialAuth?: boolean;
  gatewayDisplayName?: string;
  gatewayDisplaySubtitle?: string;
  preferChineseUi?: boolean;
  showTrayBalance?: boolean;
  bypassSystemProxy?: boolean;
  telemetryEnabled?: boolean;
  uiDensity?: UiDensity;
  themePref?: ThemePref;
  mcp?: McpToolsConfig;
}

export interface FrontendErrorInput {
  source: "frontend_error" | "unhandled_rejection" | "tauri_invoke";
  category?: string;
  errorType: string;
  message: string;
  location?: string;
  stack?: string;
  context?: Record<string, unknown>;
}

export type FeedbackCategory =
  | "bug"
  | "install"
  | "i18n"
  | "mcp"
  | "feature"
  | "other";

export interface FeedbackSubmission {
  category: FeedbackCategory;
  title?: string;
  description: string;
  contact?: string;
  includeDiagnostics: boolean;
}

export interface FeedbackReceipt {
  ok: boolean;
  id: number;
}

export interface LiveDemoRequest {
  tool: "image" | "video" | "audio" | "search" | "consult";
  prompt: string;
  model?: string;
  voice?: string;
  instructions?: string;
  aspectRatio?: string;
  provider?: string;
  resumeTaskId?: string;
}

export interface LiveDemoCitation {
  title: string;
  url: string;
  snippet: string;
  domain: string;
}

export interface LiveDemoResult {
  tool: "image" | "video" | "audio" | "search" | "consult";
  status: "completed" | "pending";
  model: string;
  elapsedMs: number;
  savedPath: string | null;
  previewDataUrl: string | null;
  textOutput: string | null;
  taskId: string | null;
  citations: LiveDemoCitation[];
}

export type SyncPlatform = "claude" | "codex" | "antigravity";

export interface SyncedMcpSpec {
  transport: string;
  command?: string | null;
  args: string[];
  env: Record<string, string>;
  url?: string | null;
  headers: Record<string, string>;
  summary: string;
}

export interface PlatformMcpItem {
  name: string;
  managedByTako: boolean;
  platforms: SyncPlatform[];
  variants: Partial<Record<SyncPlatform, SyncedMcpSpec>>;
  hasDiff: boolean;
}

export interface PlatformSkillItem {
  name: string;
  title: string;
  description: string;
  platforms: SyncPlatform[];
  fileCounts: Partial<Record<SyncPlatform, number>>;
  modifiedMs: Partial<Record<SyncPlatform, number>>;
  hasDiff: boolean;
}

export interface PlatformSyncPaths {
  mcpPath: string;
  skillsDir: string;
}

export interface MultiPlatformSyncSnapshot {
  mcpItems: PlatformMcpItem[];
  skillItems: PlatformSkillItem[];
  platformPaths: Partial<Record<SyncPlatform, PlatformSyncPaths>>;
}

export interface BatchSyncOutcome {
  syncedItems: number;
  syncedWrites: number;
  snapshot: MultiPlatformSyncSnapshot;
}

export const api = {
  getDashboard: (refreshAccount: boolean) =>
    invoke<DashboardPayload>("get_dashboard", { refreshAccount }),
  getRunningStates: () =>
    invoke<Record<AppId, boolean>>("get_running_states"),
  loginOAuth: () => invoke<AccountStatus>("login_oauth"),
  loginWithKey: (key: string) => invoke<AccountStatus>("login_with_key", { key }),
  logoutAccount: () => invoke<AccountStatus>("logout_account"),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  saveSettings: (input: SettingsPatch) =>
    invoke<SwitchState>("save_settings", { input }),
  syncWindowTheme: (themePref: ThemePref, isDark: boolean) =>
    invoke<void>("sync_window_theme", { themePref, isDark }),
  showWindowSystemMenu: () => invoke<void>("show_window_system_menu"),
  setPreferChineseUi: (
    enable: boolean,
    patchAntigravity: boolean,
    autoRestart: boolean,
  ) =>
    invoke<DashboardPayload>("set_prefer_chinese_ui", {
      enable,
      patchAntigravity,
      autoRestart,
    }),
  patchAppUi: (appId: AppId, autoRestart: boolean) =>
    invoke<AppSnapshot>("patch_app_ui", { appId, autoRestart }),
  restoreAppUi: (appId: AppId, autoRestart: boolean) =>
    invoke<AppSnapshot>("restore_app_ui", { appId, autoRestart }),
  patchAntigravityUi: (autoRestart: boolean) =>
    invoke<AppSnapshot>("patch_antigravity_ui", { autoRestart }),
  restoreAntigravityUi: (autoRestart: boolean) =>
    invoke<AppSnapshot>("restore_antigravity_ui", { autoRestart }),
  openMacosAppManagementSettings: () =>
    invoke<void>("open_macos_app_management_settings"),
  runElevatedUiPatchWorker: (appId: AppId, action: "patch" | "restore") =>
    invoke<AppSnapshot>("run_elevated_ui_patch_worker", { appId, action }),
  setAppMode: (appId: AppId, mode: AppMode, autoRestart: boolean) =>
    invoke<AppSnapshot>("set_app_mode", { appId, mode, autoRestart }),
  restartApp: (appId: AppId) => invoke<AppSnapshot>("restart_app", { appId }),
  closeApp: (appId: AppId) => invoke<AppSnapshot>("close_app", { appId }),
  launchApp: (appId: AppId) => invoke<AppSnapshot>("launch_app", { appId }),
  diagnoseApp: (appId: AppId) => invoke<DiagnosticResult>("diagnose_app", { appId }),
  listClientModels: (appId: AppId, forceRefresh = false) =>
    invoke<ClientModelsResponse>("list_client_models", { appId, forceRefresh }),
  listPlazaModels: (forceRefresh = false) =>
    invoke<ClientModelsResponse>("list_plaza_models", { forceRefresh }),
  saveClientModelFilter: (appId: AppId, filter: ClientModelFilter) =>
    invoke<SwitchState>("save_client_model_filter", { appId, filter }),
  checkAppUpdates: () => invoke<ManifestCheckResult>("check_app_updates"),
  installOrUpgradeApp: (appId: AppId) =>
    invoke<InstallOutcome>("install_or_upgrade_app", { appId }),
  cancelAppInstall: (appId: AppId) =>
    invoke<void>("cancel_app_install", { appId }),
  ensureLocalGateway: () => invoke<boolean>("ensure_local_gateway"),
  checkSelfUpdateSmart: () => invoke<SelfUpdateCheckResult>("check_self_update_smart"),
  installSelfUpdate: () => invoke<void>("install_self_update"),
  runMcpLiveDemo: (input: LiveDemoRequest) =>
    invoke<LiveDemoResult>("run_mcp_live_demo", { input }),
  checkNodeRuntime: () => invoke<NodeRuntimeInfo>("check_node_runtime"),
  getMultiPlatformSyncSnapshot: () =>
    invoke<MultiPlatformSyncSnapshot>("get_multi_platform_sync_snapshot"),
  syncMcpToPlatforms: (
    name: string,
    source: SyncPlatform,
    targets: SyncPlatform[],
  ) =>
    invoke<MultiPlatformSyncSnapshot>("sync_mcp_to_platforms", {
      name,
      source,
      targets,
    }),
  syncAllMcps: (
    targets: SyncPlatform[],
    preferredSource?: SyncPlatform | null,
  ) =>
    invoke<BatchSyncOutcome>("sync_all_mcps", {
      targets,
      preferredSource: preferredSource ?? null,
    }),
  syncSkillToPlatforms: (
    name: string,
    source: SyncPlatform,
    targets: SyncPlatform[],
  ) =>
    invoke<MultiPlatformSyncSnapshot>("sync_skill_to_platforms", {
      name,
      source,
      targets,
    }),
  syncAllSkills: (
    targets: SyncPlatform[],
    preferredSource?: SyncPlatform | null,
  ) =>
    invoke<BatchSyncOutcome>("sync_all_skills", {
      targets,
      preferredSource: preferredSource ?? null,
    }),
  removeSyncedItem: (
    kind: "mcp" | "skill",
    name: string,
    platform: SyncPlatform,
  ) =>
    invoke<MultiPlatformSyncSnapshot>("remove_synced_item", {
      kind,
      name,
      platform,
    }),
  reportFrontendError: (input: FrontendErrorInput) =>
    invoke<void>("report_frontend_error", { input }).catch(() => {
      // Telemetry error capture must never disrupt the UI.
    }),
  submitUserFeedback: (input: FeedbackSubmission) =>
    invoke<FeedbackReceipt>("submit_user_feedback", { input }),
  getSanitizedDiagnosticsPreview: () =>
    invoke<Record<string, unknown>>("get_sanitized_diagnostics_preview"),
};
