import accountProvidersConfig from "./config/account-providers.json";

export type AccountProviderConfig = {
  id: string;
  name: string;
  accountLabel: string;
  loginStatusLabel: string;
  loginDescription: string;
  gatewayBaseUrl: string;
  authServiceUrl: string;
  keysApiUrl: string;
};

type AccountProvidersFile = {
  defaultProviderId: string;
  providers: AccountProviderConfig[];
};

const config = accountProvidersConfig as AccountProvidersFile;

export class TakoProviderConfigService {
  static getDefaultProvider() {
    const provider = config.providers.find((item) => item.id === config.defaultProviderId);
    if (!provider) {
      throw new Error(`Default account provider not found: ${config.defaultProviderId}`);
    }
    validateProvider(provider);
    return provider;
  }

  static getGatewayBaseUrl(provider: AccountProviderConfig) {
    return provider.gatewayBaseUrl;
  }
}

function validateProvider(provider: AccountProviderConfig) {
  const requiredFields: Array<keyof AccountProviderConfig> = [
    "id",
    "name",
    "accountLabel",
    "loginStatusLabel",
    "loginDescription",
    "gatewayBaseUrl",
    "authServiceUrl"
  ];

  for (const field of requiredFields) {
    if (!provider[field]?.trim()) {
      throw new Error(`Account provider ${provider.id || "(unknown)"} is missing ${field}`);
    }
  }
}
