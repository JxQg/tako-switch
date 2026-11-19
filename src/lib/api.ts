import { invoke } from "@tauri-apps/api/core";

export type AppId = "codex" | "claude" | "antigravity";
export type AppMode = "official" | "tako";

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
  outputDir: string;
  consultModel: string;
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
  mcp: McpToolsConfig;
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

export interface AppSnapshot {
  id: AppId;
  name: string;
  mode: AppMode;
  mcpEnabled: boolean;
  hasBackup: boolean;
  running: boolean;
  install: InstallInfo;
  diagnostic: DiagnosticResult;
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
}

export const DEFAULT_MANIFEST_URL =
  "https://dsh-tako-dl.oss-cn-shanghai.aliyuncs.com/tako-switch/manifest.json";
export const DEFAULT_FALLBACK_MANIFEST_URL =
  "https://raw.githubusercontent.com/Barrierml/tako-switch/main/manifest/manifest.json";
export const TAKO_FEEDBACK_URL = "https://github.com/Barrierml/tako-switch/issues";
export const TAKO_RELEASES_URL = "https://github.com/Barrierml/tako-switch/releases/latest";

export interface InstallProgressEvent {
  appId: AppId;
  stage: string;
  percent: number;
  message: string;
}

export interface InstallOutcome {
  appId: AppId;
  modeReapplied: AppMode;
  snapshot: AppSnapshot;
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
  mcp?: McpToolsConfig;
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
  setAppMode: (appId: AppId, mode: AppMode, autoRestart: boolean) =>
    invoke<AppSnapshot>("set_app_mode", { appId, mode, autoRestart }),
  restartApp: (appId: AppId) => invoke<AppSnapshot>("restart_app", { appId }),
  launchApp: (appId: AppId) => invoke<AppSnapshot>("launch_app", { appId }),
  diagnoseApp: (appId: AppId) => invoke<DiagnosticResult>("diagnose_app", { appId }),
  checkAppUpdates: () => invoke<ManifestCheckResult>("check_app_updates"),
  installOrUpgradeApp: (appId: AppId) =>
    invoke<InstallOutcome>("install_or_upgrade_app", { appId }),
  cancelAppInstall: (appId: AppId) =>
    invoke<void>("cancel_app_install", { appId }),
  ensureLocalGateway: () => invoke<boolean>("ensure_local_gateway"),
  checkSelfUpdateSmart: () => invoke<SelfUpdateCheckResult>("check_self_update_smart"),
  installSelfUpdate: () => invoke<void>("install_self_update"),
};
