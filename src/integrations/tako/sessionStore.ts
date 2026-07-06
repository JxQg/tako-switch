import { Store } from "@tauri-apps/plugin-store";

const SESSION_STORE_PATH = "tako-session.json";
const SESSION_KEY = "session";

export type TakoStoredSession = {
  providerId: "tako";
  apiKey: string;
  savedAt: string;
};

async function loadSessionStore() {
  return Store.load(SESSION_STORE_PATH);
}

function isStoredSession(value: unknown): value is TakoStoredSession {
  if (!value || typeof value !== "object") return false;
  const session = value as Partial<TakoStoredSession>;
  return session.providerId === "tako" && typeof session.apiKey === "string" && session.apiKey.trim().length > 0;
}

export class TakoSessionStore {
  static async load() {
    const store = await loadSessionStore();
    const session = await store.get<unknown>(SESSION_KEY);
    return isStoredSession(session) ? session : null;
  }

  static async save(apiKey: string) {
    const store = await loadSessionStore();
    const session: TakoStoredSession = {
      providerId: "tako",
      apiKey,
      savedAt: new Date().toISOString()
    };
    await store.set(SESSION_KEY, session);
    await store.save();
    return session;
  }

  static async clear() {
    const store = await loadSessionStore();
    await store.delete(SESSION_KEY);
    await store.save();
  }
}
