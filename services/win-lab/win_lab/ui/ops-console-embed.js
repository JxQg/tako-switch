(function (global) {
  "use strict";

  const params = new URLSearchParams(global.location.search);
  const proxyMatch = /^(\/api\/plugins\/[^/]+)/.exec(global.location.pathname);
  const PROXY_PREFIX = proxyMatch ? proxyMatch[1] : "";
  const EMBED = params.get("embed") === "1";
  const HOST_NAV = params.get("host_nav") === "1";

  function postToHost(payload) {
    if (!EMBED || !global.parent || global.parent === global) return;
    try {
      global.parent.postMessage(payload, global.location.origin);
    } catch {
      // ignore cross-origin parent
    }
  }

  function applyTheme(theme) {
    const next = theme === "dark" ? "dark" : "light";
    if (global.document && global.document.body) {
      global.document.body.dataset.theme = next;
    }
    if (global.document && global.document.documentElement) {
      global.document.documentElement.classList.toggle("dark", next === "dark");
    }
    return next;
  }

  function init(options) {
    const pluginId = (options && options.pluginId) || "win-lab";
    const onTheme = options && options.onTheme;
    const onNavigate = options && options.onNavigate;

    const initialTheme = applyTheme(params.get("theme"));

    global.addEventListener("message", function (ev) {
      if (ev.origin !== global.location.origin) return;
      const msg = ev.data;
      if (!msg || typeof msg !== "object") return;
      if (msg.type === "ops-console:theme" || msg.type === "ops:theme") {
        const next = applyTheme(msg.theme);
        if (typeof onTheme === "function") onTheme(next);
      } else if (
        (msg.type === "ops-console:navigate" || msg.type === "ops:subroute") &&
        typeof msg.path === "string"
      ) {
        if (typeof onNavigate === "function") onNavigate(msg.path);
      }
    });

    if (EMBED && typeof global.ResizeObserver !== "undefined" && global.document && global.document.body) {
      let lastH = 0;
      const ro = new global.ResizeObserver(function () {
        const h = Math.max(520, Math.ceil(global.document.body.scrollHeight + 16));
        if (Math.abs(h - lastH) >= 4) {
          lastH = h;
          postToHost({ type: "ops-console:resize", pluginId, height: h });
        }
      });
      ro.observe(global.document.body);
    }

    return {
      isEmbed: EMBED,
      hostNav: HOST_NAV,
      theme: initialTheme,
      proxyPrefix: PROXY_PREFIX,
      setTheme: applyTheme,
      notifyRoute: function (path) {
        postToHost({ type: "ops-console:route", pluginId, path });
      },
      apiUrl: function (path) {
        return PROXY_PREFIX && path.startsWith("/") ? PROXY_PREFIX + path : path;
      },
      apiFetch: function (path, opts) {
        const url = PROXY_PREFIX && path.startsWith("/") ? PROXY_PREFIX + path : path;
        return global.fetch(url, opts);
      },
    };
  }

  global.OpsConsoleEmbed = { init };
})(window);
