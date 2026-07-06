import { listen } from "@tauri-apps/api/event";
import { TakoApi, type TakoLoginSession } from "./api";

const AUTHORIZE_URL = "https://tako.shiroha.tech/app/authorize";

type TakoAuthEvent = {
  key: string;
  state: string | null;
};

function genState() {
  const buffer = new Uint8Array(16);
  crypto.getRandomValues(buffer);
  return Array.from(buffer, (value) => value.toString(16).padStart(2, "0")).join("");
}

export async function startTakoLogin(timeoutMs = 5 * 60 * 1000): Promise<TakoLoginSession> {
  const state = genState();

  return new Promise<TakoLoginSession>((resolve) => {
    let settled = false;
    let unlisten: (() => void) | null = null;
    let timer: ReturnType<typeof setTimeout> | null = null;

    function cleanup() {
      if (timer) clearTimeout(timer);
      if (unlisten) unlisten();
    }

    function finish(result: TakoLoginSession) {
      if (settled) return;
      settled = true;
      cleanup();
      resolve(result);
    }

    listen<TakoAuthEvent>("tako-auth", async (event) => {
      const payload = event.payload;
      if (payload.state !== state) return;

      try {
        const result = await TakoApi.applyKey(payload.key);
        finish({ ...result, apiKey: result.ok ? payload.key : undefined });
      } catch (err) {
        finish({
          ok: false,
          name: null,
          plan: null,
          error: String(err)
        });
      }
    })
      .then((nextUnlisten) => {
        unlisten = nextUnlisten;
        const url = `${AUTHORIZE_URL}?state=${encodeURIComponent(state)}&redirect=takoswitch`;
        TakoApi.openExternal(url).catch((err) => {
          finish({
            ok: false,
            name: null,
            plan: null,
            error: `无法打开浏览器：${String(err)}`
          });
        });
        timer = setTimeout(() => {
          finish({
            ok: false,
            name: null,
            plan: null,
            error: "授权超时，请重试或手动粘贴 ApiKey。"
          });
        }, timeoutMs);
      })
      .catch((err) => {
        finish({
          ok: false,
          name: null,
          plan: null,
          error: `监听授权回调失败：${String(err)}`
        });
      });
  });
}
