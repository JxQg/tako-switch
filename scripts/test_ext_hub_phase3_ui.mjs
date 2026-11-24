import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const output = path.join(root, "src-tauri", "target", "ext-hub-phase3-ui");
const baseUrl = process.env.EXT_HUB_PREVIEW_URL ?? "http://127.0.0.1:1433/";
await mkdir(output, { recursive: true });

function mockDesktop({ theme, density }) {
  localStorage.setItem("tako-switch-theme", theme);
  localStorage.setItem("tako-switch-density", density);
  const state = {
    baseUrl: "https://example.invalid",
    autoCheckUpdates: false,
    preferChineseUi: true,
    uiDensity: density,
    themePref: theme,
    mcp: {
      image: true,
      video: true,
      audio: true,
      search: true,
      consult: true,
      webPage: false,
      outputDir: "sandbox",
      consultModel: "example-model",
    },
  };
  const account = {
    loggedIn: true,
    userName: "Sandbox",
    planName: null,
    maskedKey: null,
    walletRemainUsd: 9,
    walletUsedUsd: 1,
    quota: null,
  };
  const apps = ["claude", "codex", "antigravity", "dsh-tako"].map((id) => ({
    id,
    name: id,
    mode: "official",
    mcpEnabled: false,
    hasBackup: false,
    running: false,
    install: {
      installed: true,
      version: "1.0.0",
      installPath: "sandbox",
      installKind: "test",
    },
    diagnostic: { ok: true, issues: [], notes: [] },
    chineseConfigured: true,
    uiPatch: {
      status: "patched",
      hasValidBackup: true,
      updateAvailable: false,
    },
  }));
  const plugin = (name, extra = {}) => ({
    id: `${name}@official`,
    name,
    displayName: name,
    version: "1.0.0",
    description: "A sandbox plugin with a focused description.",
    category: "Development",
    marketplace: "official",
    installPath: null,
    dirName: null,
    scope: "user",
    enabled: true,
    manageable: true,
    removable: true,
    needsConfiguration: false,
    ...extra,
  });
  const installed = {
    claude: [
      plugin("alpha", { displayName: "Alpha Desktop" }),
      plugin("very-long-plugin-identifier-for-layout-comparison", {
        displayName:
          "LongUnbrokenPluginDisplayNameThatMustNeverResizeTheToolbarOrCard",
        version: "2026.11.24-preview.100000000000",
        description:
          "LongUnbrokenDescriptionThatMustWrapInsideItsCardWithoutHorizontalOverflow".repeat(
            5,
          ),
        category: "LongUnbrokenCategoryNameForResponsiveLayoutVerification",
      }),
      plugin("project-only", {
        scope: "project",
        manageable: false,
        removable: false,
      }),
    ],
    codex: [plugin("codex-plugin")],
    antigravity: [
      plugin("agy-plugin", {
        id: "agy-plugin@antigravity-plugins-official",
        displayName: "Antigravity Plugin",
        dirName: "-12345",
        marketplace: "antigravity-plugins-official",
        enabled: false,
        needsConfiguration: true,
      }),
    ],
  };
  const available = {
    claude: [
      plugin("discovered", {
        displayName: "Discovered Plugin",
        category: "Discovery Category",
      }),
    ],
    codex: [plugin("codex-discovered")],
    antigravity: [
      plugin("agy-discovered", {
        id: "agy-discovered@antigravity-plugins-official",
        marketplace: "antigravity-plugins-official",
      }),
    ],
  };
  const markets = [
    {
      name: "official",
      source: "https://example.invalid/marketplace.json",
      installLocation: null,
      manageable: true,
    },
  ];
  window.__pluginCalls = [];
  window.__noCli = [];
  window.__pluginFailure = null;
  window.__pluginDelay = {};
  const callbacks = new Map();
  let callbackId = 0;
  window.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { label: "main", windowLabel: "main" },
    },
    transformCallback(callback) {
      callbacks.set(++callbackId, callback);
      return callbackId;
    },
    unregisterCallback(id) {
      callbacks.delete(id);
    },
    async invoke(command, args = {}) {
      if (command === "get_dashboard")
        return {
          account,
          state,
          apps,
          cachedManifest: null,
          localGatewayHealthy: false,
          localGatewayUrl: "http://127.0.0.1:18419",
          appVersion: "2.2.7",
          os: "windows",
          uiPatchPrompts: [],
        };
      if (command === "get_running_states")
        return Object.fromEntries(apps.map((app) => [app.id, false]));
      if (command === "save_settings") return Object.assign(state, args.input);
      if (command === "plugin:event|listen") return ++callbackId;
      if (command.includes("is_maximized")) return false;
      if (command.includes("is_focused")) return true;
      if (command === "get_plugins_hub_snapshot") {
        window.__pluginCalls.push({ command, ...args });
        const delay = window.__pluginDelay[args.platform] ?? 0;
        if (delay) await new Promise((resolve) => setTimeout(resolve, delay));
        const cliAvailable = !window.__noCli.includes(args.platform);
        return {
          platform: args.platform,
          cliAvailable,
          cliPath: cliAvailable ? "sandbox/desktop/resources/cli" : null,
          cliSource: cliAvailable
            ? args.platform === "codex"
              ? "path"
              : "builtin"
            : null,
          marketplacesSupported: args.platform !== "antigravity",
          installed: cliAvailable
            ? structuredClone(installed[args.platform])
            : [],
          available:
            cliAvailable && args.includeAvailable
              ? structuredClone(
                  available[args.platform].filter(
                    (entry) =>
                      !installed[args.platform].some(
                        (item) => item.id === entry.id,
                      ),
                  ),
                )
              : [],
          marketplaces:
            cliAvailable &&
            args.includeAvailable &&
            args.platform !== "antigravity"
              ? structuredClone(markets)
              : [],
          catalogFromCache: false,
          warnings: [],
        };
      }
      if (
        [
          "toggle_plugin_enabled",
          "install_plugin_entry",
          "update_plugin_entry",
          "uninstall_plugin_entry",
          "add_plugin_marketplace",
          "remove_plugin_marketplace",
        ].includes(command)
      ) {
        window.__pluginCalls.push({ command, ...args });
        if (window.__pluginFailure) {
          const failure = window.__pluginFailure;
          window.__pluginFailure = null;
          throw failure;
        }
        if (command === "toggle_plugin_enabled") {
          const entry = installed[args.platform].find(
            (entry) => entry.id === args.id || entry.dirName === args.id,
          );
          entry.enabled = args.enabled;
          entry.needsConfiguration =
            args.platform === "antigravity" && !args.enabled;
        }
        if (command === "install_plugin_entry") {
          const entry = available[args.platform].find(
            (entry) => entry.id === args.id,
          );
          if (entry)
            installed[args.platform].push({
              ...entry,
              enabled: args.platform !== "antigravity",
              needsConfiguration: args.platform === "antigravity",
              dirName: args.platform === "antigravity" ? "-98765" : null,
            });
        }
        if (command === "uninstall_plugin_entry")
          installed[args.platform] = installed[args.platform].filter(
            (entry) => entry.id !== args.id && entry.dirName !== args.id,
          );
        if (command === "add_plugin_marketplace")
          markets.push({
            name: "sandbox-added",
            source: args.source,
            manageable: true,
            installLocation: null,
          });
        if (command === "remove_plugin_marketplace")
          markets.splice(
            markets.findIndex((entry) => entry.name === args.name),
            1,
          );
        return {
          platform: args.platform,
          pluginId: args.id ?? args.name ?? args.source,
          needsConfiguration:
            args.platform === "antigravity" &&
            command === "install_plugin_entry",
          message: "插件操作已完成",
        };
      }
      return null;
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
}

async function assertLayout(page, label) {
  const layout = await page.evaluate(() => {
    const targets = [
      "ext-hub-actions",
      "ext-plugins-cli-status",
      "ext-plugins-search",
      "ext-plugins-platform-antigravity",
      "ext-plugins-subtab-marketplaces",
    ];
    const outside = targets.flatMap((id) => {
      const element = document.querySelector(`[data-testid="${id}"]`);
      if (!element) return [];
      const box = element.getBoundingClientRect();
      return box.left < -1 || box.right > innerWidth + 1
        ? [{ id, left: box.left, right: box.right }]
        : [];
    });
    const clippedButtons = Array.from(
      document.querySelectorAll('[data-testid="extensions-hub-panel"] button'),
    )
      .filter(
        (button) =>
          button.offsetParent && button.scrollWidth > button.clientWidth + 2,
      )
      .map((button) => button.textContent);
    const top = document
      .querySelector('[data-testid="ext-hub-actions"]')
      .getBoundingClientRect();
    const nav = document
      .querySelector('nav[aria-label="扩展类型"]')
      .getBoundingClientRect();
    const overlap =
      top.left < nav.right - 1 &&
      top.right > nav.left + 1 &&
      top.top < nav.bottom - 1 &&
      top.bottom > nav.top + 1;
    return {
      width: innerWidth,
      scrollWidth: document.documentElement.scrollWidth,
      mainScrollWidth: document.querySelector("main").scrollWidth,
      outside,
      clippedButtons,
      overlap,
    };
  });
  assert.ok(
    layout.scrollWidth <= layout.width,
    `${label}: page overflow ${JSON.stringify(layout)}`,
  );
  assert.ok(
    layout.mainScrollWidth <= layout.width,
    `${label}: main overflow ${JSON.stringify(layout)}`,
  );
  assert.deepEqual(layout.outside, [], `${label}: controls outside viewport`);
  assert.deepEqual(
    layout.clippedButtons,
    [],
    `${label}: clipped button labels`,
  );
  assert.equal(layout.overlap, false, `${label}: actions overlap tabs`);
}

const executablePath = process.env.EXT_HUB_CHROME;
const browser = await chromium.launch({
  headless: true,
  ...(executablePath
    ? { executablePath }
    : process.platform === "win32"
      ? { channel: "chrome" }
      : {}),
});
let scenarios = 0;
try {
  for (const [width, height] of [
    [720, 560],
    [820, 650],
  ]) {
    for (const theme of ["light", "dark"]) {
      for (const density of ["compact", "comfortable"]) {
        const label = `${width}x${height}-${theme}-${density}`;
        const context = await browser.newContext({
          viewport: { width, height },
          reducedMotion: "reduce",
        });
        const page = await context.newPage();
        const errors = [];
        page.on("pageerror", (error) => errors.push(String(error)));
        await page.addInitScript(mockDesktop, { theme, density });
        await page.goto(baseUrl);
        await page.getByTestId("open-ext-hub-plugins").click();
        await page
          .getByTestId("ext-plugins-cli-status")
          .filter({ hasText: "内置 CLI" })
          .waitFor();
        await page.getByText("Alpha Desktop", { exact: true }).waitFor();
        await assertLayout(page, label);
        assert.equal(
          await page.getByTestId("ext-hub-actions").getByRole("button").count(),
          2,
        );
        assert.equal(
          await page
            .getByRole("switch", { name: "停用 project-only" })
            .isDisabled(),
          true,
        );
        await page.screenshot({ path: path.join(output, `${label}.png`) });

        await page.getByTestId("ext-plugins-subtab-discover").click();
        await page.getByText("Discovered Plugin", { exact: true }).waitFor();
        await page.getByTestId("ext-plugins-search").fill("Discovery Category");
        await page.getByText("Discovered Plugin", { exact: true }).waitFor();
        assert.equal(await page.getByTestId("ext-plugin-card").count(), 1);
        await page.getByTestId("ext-plugins-search").fill("");
        await page.getByRole("button", { name: "安装", exact: true }).click();
        await page.getByTestId("ext-plugins-notice").waitFor();
        await page.getByTestId("ext-plugins-subtab-installed").click();
        await page.getByText("Discovered Plugin", { exact: true }).waitFor();

        await page.getByTestId("ext-plugins-subtab-marketplaces").click();
        await page
          .getByRole("heading", { name: "official", exact: true })
          .waitFor();
        await page.getByTestId("ext-plugins-market-btn").click();
        await page
          .getByRole("textbox", { name: "市场来源" })
          .fill("example/sandbox-market");
        await page
          .getByRole("dialog")
          .getByRole("button", { name: "添加市场", exact: true })
          .click();
        await page
          .getByRole("heading", { name: "sandbox-added", exact: true })
          .waitFor();
        await assertLayout(page, `${label}-marketplaces`);
        await page
          .getByRole("button", { name: "移除市场 sandbox-added", exact: true })
          .click();
        await page
          .getByRole("dialog")
          .getByRole("button", { name: "确认移除", exact: true })
          .click();
        await page
          .getByRole("heading", { name: "sandbox-added", exact: true })
          .waitFor({ state: "detached" });

        await page.getByTestId("ext-plugins-platform-antigravity").click();
        await page.getByText("Antigravity Plugin", { exact: true }).waitFor();
        assert.equal(
          await page.getByTestId("ext-plugins-market-btn").isDisabled(),
          true,
        );
        assert.equal(
          await page
            .getByTestId("ext-plugins-subtab-marketplaces")
            .isDisabled(),
          true,
        );
        await page
          .getByRole("switch", { name: "启用 Antigravity Plugin" })
          .click();
        await page
          .getByRole("switch", { name: "停用 Antigravity Plugin" })
          .waitFor();
        const toggle = await page.evaluate(() =>
          window.__pluginCalls.findLast(
            (call) => call.command === "toggle_plugin_enabled",
          ),
        );
        assert.equal(toggle.id, "-12345");
        await page.getByTestId("ext-plugins-subtab-discover").click();
        await page.getByRole("button", { name: "安装", exact: true }).click();
        await page
          .getByTestId("ext-plugins-notice")
          .filter({ hasText: "完成授权配置" })
          .waitFor();
        await page.getByTestId("ext-plugins-subtab-installed").click();
        await page.getByText("需客户端授权", { exact: true }).waitFor();
        await assertLayout(page, `${label}-antigravity`);

        await page.evaluate(() => window.__noCli.push("codex"));
        await page.getByTestId("ext-plugins-platform-codex").click();
        await page.getByTestId("ext-plugins-empty-unsupported").waitFor();
        assert.equal(
          await page.getByTestId("ext-plugins-empty-unsupported").innerText(),
          "未检测到对应 CLI，当前客户端暂不支持该功能",
        );
        assert.equal(
          await page.getByTestId("ext-plugins-market-btn").isDisabled(),
          true,
        );
        await assertLayout(page, `${label}-unsupported`);

        await page.getByTestId("ext-plugins-platform-claude").click();
        await page.getByText("Alpha Desktop", { exact: true }).waitFor();
        for (const errorCode of [
          "PLUGIN_IN_USE",
          "PLUGIN_NEEDS_TERMINAL_CONFIRM",
        ]) {
          await page.evaluate((errorCode) => {
            window.__pluginFailure = {
              errorCode,
              message: "操作需要处理",
              detail: "Sandbox CLI diagnostic",
            };
          }, errorCode);
          await page
            .getByRole("switch", { name: "停用 Alpha Desktop" })
            .click();
          await page.getByTestId("ext-plugins-error").waitFor();
          assert.match(
            await page.getByTestId("ext-plugins-error").innerText(),
            errorCode === "PLUGIN_IN_USE"
              ? /关闭运行中的 Claude/
              : /终端.*确认/,
          );
          await page
            .getByRole("button", { name: "关闭错误提示", exact: true })
            .click();
        }
        await page
          .getByRole("button", { name: "CLI 探测规则", exact: true })
          .click();
        await page.getByRole("tooltip").waitFor();
        await page.keyboard.press("Escape");
        assert.equal(await page.getByTestId("ext-plugins-view").count(), 1);
        await page.getByTestId("ext-plugins-market-btn").click();
        await page.getByRole("dialog").waitFor();
        await page.keyboard.press("Escape");
        await page.getByRole("dialog").waitFor({ state: "detached" });
        assert.equal(await page.getByTestId("ext-plugins-view").count(), 1);
        const alpha = page
          .getByTestId("ext-plugin-card")
          .filter({
            has: page.getByRole("heading", {
              name: "Alpha Desktop",
              exact: true,
            }),
          });
        await alpha
          .getByRole("button", { name: "更新", exact: true })
          .click();
        await page.getByTestId("ext-plugins-notice").waitFor();
        await alpha
          .getByRole("button", { name: "卸载", exact: true })
          .click();
        await page
          .getByRole("dialog")
          .getByRole("button", { name: "确认卸载", exact: true })
          .click();
        await page
          .getByText("Alpha Desktop", { exact: true })
          .waitFor({ state: "detached" });
        await page.getByTestId("ext-plugins-platform-codex").click();
        await page.getByTestId("ext-plugins-empty-unsupported").waitFor();
        await page.evaluate(() => {
          window.__noCli = [];
        });
        await page.getByTestId("ext-plugins-refresh-btn").click();
        await page
          .getByTestId("ext-plugins-cli-status")
          .filter({ hasText: "PATH CLI" })
          .waitFor();
        await page.getByText("codex-plugin", { exact: true }).waitFor();
        await page.evaluate(() => {
          window.__pluginDelay = { claude: 200 };
        });
        await page.getByTestId("ext-plugins-platform-claude").click();
        await page.getByTestId("ext-plugins-platform-antigravity").click();
        await page.getByText("Antigravity Plugin", { exact: true }).waitFor();
        await page.waitForTimeout(250);
        assert.equal(
          await page.getByText("codex-plugin", { exact: true }).count(),
          0,
        );
        assert.equal(
          await page
            .getByTestId("ext-plugins-platform-antigravity")
            .getAttribute("aria-selected"),
          "true",
        );
        await page.getByTestId("topbar-back-home").click();
        await page.getByTestId("open-ext-hub-plugins").waitFor();
        assert.deepEqual(errors, [], `${label}: uncaught browser errors`);
        await context.close();
        scenarios += 1;
        console.log(
          `PASS ${label}: layout, search, install, marketplace, directory toggle, unsupported CLI, errors, Escape`,
        );
      }
    }
  }
  console.log(
    `Passed ${scenarios} viewport/theme/density scenarios. Screenshots: ${output}`,
  );
} finally {
  await browser.close();
}
