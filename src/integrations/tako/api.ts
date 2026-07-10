import { invoke } from "@tauri-apps/api/core";

export type TakoUsageWindow = {
  used: number;
  limit: number;
};

export type TakoUsage = {
  ok: boolean;
  window: TakoUsageWindow;
  daily: TakoUsageWindow;
  weekly: TakoUsageWindow;
  planName: string | null;
  error: string | null;
};

export type TakoLoginResult = {
  ok: boolean;
  name: string | null;
  plan: string | null;
  error: string | null;
};

export type TakoLoginSession = TakoLoginResult & {
  apiKey?: string;
};

export type TakoModel = {
  id: string;
  name: string;
  provider: string;
  clients: string[];
};

export type TakoIdentity = {
  loggedIn: boolean;
  name: string | null;
  plan: string | null;
  offline: boolean;
};

export class TakoApi {
  static openExternal(url: string) {
    return invoke<void>("open_external", { url });
  }

  static openToolApp(tool: string) {
    return invoke<void>("open_tool_app", { tool });
  }

  static login(apiKey: string) {
    return invoke<TakoLoginResult>("tako_login", { apiKey });
  }

  static applyKey(apiKey: string) {
    return invoke<TakoLoginResult>("tako_apply_key", { apiKey });
  }

  static currentIdentity(apiKey?: string) {
    return invoke<TakoIdentity>("tako_current_identity", { apiKey });
  }

  static logout() {
    return invoke<boolean>("tako_logout");
  }

  static usage(apiKey: string) {
    return invoke<TakoUsage>("tako_usage", { apiKey });
  }

  static listModels(apiKey: string) {
    return invoke<TakoModel[]>("tako_list_models", { apiKey });
  }
}
