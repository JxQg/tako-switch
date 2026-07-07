import { invoke } from "@tauri-apps/api/core";

export type WriterBinding = {
  storage: string;
  name: string;
};

export type PlatformWriter = {
  kind: string;
  bindings: Record<string, WriterBinding>;
  constants: Record<string, string>;
};

export type PlatformDefaults = {
  baseUrl: string;
  model?: string | null;
};

export type PlatformRules = {
  baseUrl?: {
    forbidPathSuffixes?: string[];
  } | null;
  model?: {
    required?: boolean;
  } | null;
};

export type PlatformDefinition = {
  enabled: boolean;
  defaults: PlatformDefaults;
  rules: PlatformRules;
  writer: PlatformWriter;
};

export type AccountProviderConfig = {
  id: string;
  name: string;
  account: {
    label: string;
    loginStatusLabel: string;
    loginDescription: string;
    authServiceUrl: string;
    keysUrl: string;
  };
  platforms: Record<string, PlatformDefinition>;
};

export type ProviderCatalog = {
  defaultProviderId: string;
  providers: AccountProviderConfig[];
  source: string;
  warning: string | null;
};

export class TakoProviderConfigService {
  static loadCatalog() {
    return invoke<ProviderCatalog>("load_provider_catalog");
  }

  static getDefaultProvider(catalog: ProviderCatalog) {
    const provider = catalog.providers.find((item) => item.id === catalog.defaultProviderId);
    if (!provider) {
      throw new Error(`Default account provider not found: ${catalog.defaultProviderId}`);
    }
    return provider;
  }

  static getPlatform(provider: AccountProviderConfig, platformId: "codex" | "claude") {
    const platform = provider.platforms[platformId];
    if (!platform) {
      throw new Error(`Provider ${provider.id} does not support ${platformId}`);
    }
    return platform;
  }
}
