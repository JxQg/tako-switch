import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const modulePath = process.env.PLAYWRIGHT_MODULE;
if (!modulePath)
  throw new Error(
    "Set PLAYWRIGHT_MODULE to an installed playwright/index.mjs path.",
  );
const { chromium } = await import(pathToFileURL(path.resolve(modulePath)).href);
const baseUrl = process.env.EXT_HUB_PREVIEW_URL ?? "http://127.0.0.1:1420";
const output = path.resolve("src-tauri/target/ext-phase1-ui");
await mkdir(output, { recursive: true });

function installMock({ theme, density }) {
  localStorage.setItem("tako-switch-theme", theme);
  localStorage.setItem("tako-switch-density", density);
  const platforms = ["claude", "codex", "antigravity"];
  const longName =
    "Production documentation and browser automation service with a long name";
  const spec = {
    transport: "stdio",
    command: "npx",
    args: ["-y", "example-server"],
    env: {},
    url: null,
    headers: {},
    extra: {},
  };
  const mcp = {
    entries: [
      {
        id: "docs",
        name: longName,
        spec: structuredClone(spec),
        platforms: [...platforms],
        variants: Object.fromEntries(
          platforms.map((platform) => [platform, structuredClone(spec)]),
        ),
        hasDiff: true,
      },
      {
        id: "remote",
        name: "Remote Docs",
        spec: {
          ...spec,
          transport: "http",
          command: null,
          args: [],
          url: "https://example.com/mcp",
        },
        platforms: [],
        variants: {},
        hasDiff: false,
      },
    ],
    platformPaths: {
      claude: [
        "C:/sandbox/claude_1p/claude_desktop_config.json",
        "C:/sandbox/.claude.json",
      ],
      codex: ["C:/sandbox/.codex/config.toml"],
      antigravity: ["C:/sandbox/.gemini/config/mcp_config.json"],
    },
    warnings: [],
  };
  const prompts = {
    platforms: Object.fromEntries(
      platforms.map((platform) => [
        platform,
        {
          presets: [
            {
              id: "rules",
              name: "Engineering rules with a deliberately long preset name",
              content:
                "# Engineering rules\n\nPreserve existing behavior and cover changes with tests.\n",
            },
            {
              id: "review",
              name: "Code review",
              content: "# Review\nPrioritize actionable findings.",
            },
          ],
          activeId: "rules",
          path: `C:/sandbox/a-very-long-user-directory/clients/${platform}/${platform === "claude" ? "CLAUDE.md" : "AGENTS.md"}`,
          liveBytes: 1824,
          hasChineseBlock: true,
        },
      ]),
    ),
  };
  const state = {
    baseUrl: "https://example.com",
    manifestUrl: "https://example.com/manifest.json",
    fallbackManifestUrl: "https://example.com/manifest.json",
    autoCheckUpdates: false,
    preferChineseUi: false,
    showTrayBalance: false,
    bypassSystemProxy: true,
    themePref: theme,
    uiDensity: density,
    mcp: {
      image: true,
      video: true,
      audio: true,
      search: true,
      consult: true,
      webPage: false,
      outputDir: "C:/sandbox/media",
      consultModel: "example-model",
    },
  };
  const apps = ["claude", "codex", "antigravity", "dsh-tako"].map((id) => ({
    id,
    name: id,
    mode: "official",
    mcpEnabled: false,
    hasBackup: false,
    running: false,
    chineseConfigured: true,
    install: {
      installed: true,
      version: "1.2.3",
      installPath: null,
      installKind: null,
    },
    diagnostic: { ok: true, issues: [], notes: [] },
    uiPatch: {
      status: "patched",
      patchVersion: "test",
      updateAvailable: false,
      hasValidBackup: true,
      targetAsarPath: null,
      note: null,
    },
  }));
  const dashboard = {
    account: {
      loggedIn: false,
      baseUrl: "https://example.com",
      maskedKey: null,
      userName: null,
      planName: null,
      walletRemain: null,
      walletUsed: null,
      walletRemainUsd: null,
      walletUsedUsd: null,
      quota: null,
      error: null,
    },
    state,
    apps,
    cachedManifest: null,
    localGatewayHealthy: true,
    localGatewayUrl: "http://127.0.0.1:18419",
    appVersion: "2.2.7",
    os: "windows",
    uiPatchPrompts: [],
  };
  const clone = (value) => structuredClone(value);
  const calls = [];
  window.__EXT_UI_CALLS__ = calls;
  window.__EXT_UI_FIXTURE__ = { mcp, prompts };
  let callbackId = 0;
  window.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { label: "main", windowLabel: "main" },
    },
    transformCallback: () => ++callbackId,
    unregisterCallback: () => {},
    invoke: async (command, args = {}) => {
      calls.push({ command, args: clone(args) });
      if (command.startsWith("plugin:event|")) return ++callbackId;
      switch (command) {
        case "get_dashboard":
          return clone(dashboard);
        case "get_running_states":
          return Object.fromEntries(apps.map((app) => [app.id, false]));
        case "sync_window_theme":
          return null;
        case "save_settings":
          Object.assign(state, args.input);
          return clone(state);
        case "get_extensions_summary":
          return {
            mcpCount: mcp.entries.length,
            skillsCount: 4,
            pluginsCount: 0,
            promptsCount: platforms.reduce(
              (count, platform) =>
                count + prompts.platforms[platform].presets.length,
              0,
            ),
          };
        case "get_mcp_hub_snapshot":
          return clone(mcp);
        case "toggle_mcp_platform": {
          const item = mcp.entries.find((entry) => entry.id === args.id);
          item.platforms = args.enabled
            ? [...new Set([...item.platforms, args.platform])]
            : item.platforms.filter((platform) => platform !== args.platform);
          return clone(mcp);
        }
        case "upsert_mcp_entry":
          mcp.entries = [
            ...mcp.entries.filter((entry) => entry.id !== args.entry.id),
            clone(args.entry),
          ];
          return clone(mcp);
        case "delete_mcp_entry":
          mcp.entries = mcp.entries.filter((entry) => entry.id !== args.id);
          return clone(mcp);
        case "align_mcp_entry":
          mcp.entries.find((entry) => entry.id === args.id).hasDiff = false;
          return clone(mcp);
        case "align_all_mcp_entries":
          mcp.entries.forEach((entry) => {
            entry.hasDiff = false;
          });
          return clone(mcp);
        case "export_mcp_backup":
          return JSON.stringify({ entries: mcp.entries });
        case "import_mcp_backup": {
          const imported = JSON.parse(args.backup).entries;
          mcp.entries = [
            ...mcp.entries.filter(
              (entry) => !imported.some((item) => item.id === entry.id),
            ),
            ...imported,
          ];
          return clone(mcp);
        }
        case "parse_mcp_snippet": {
          const input = JSON.parse(args.snippet);
          return Object.entries(input.mcpServers).map(([id, value]) => ({
            id,
            name: id,
            spec: { ...spec, ...value },
            platforms: [],
            variants: {},
            hasDiff: false,
          }));
        }
        case "get_prompts_hub_snapshot":
          return clone(prompts);
        case "upsert_prompt_preset": {
          const target = prompts.platforms[args.platform];
          target.presets = [
            ...target.presets.filter((preset) => preset.id !== args.preset.id),
            clone(args.preset),
          ];
          return clone(prompts);
        }
        case "activate_prompt_preset":
          prompts.platforms[args.platform].activeId = args.id;
          return clone(prompts);
        case "delete_prompt_preset": {
          const target = prompts.platforms[args.platform];
          target.presets = target.presets.filter(
            (preset) => preset.id !== args.id,
          );
          if (target.activeId === args.id) target.activeId = null;
          return clone(prompts);
        }
        case "sync_prompt_to_platforms": {
          const source = prompts.platforms[args.source].presets.find(
            (preset) => preset.id === args.id,
          );
          for (const platform of args.targets) {
            const target = prompts.platforms[platform];
            target.presets = [
              ...target.presets.filter((preset) => preset.name !== source.name),
              clone(source),
            ];
            if (args.activate) target.activeId = source.id;
          }
          return clone(prompts);
        }
        case "export_prompts_backup":
          return JSON.stringify(prompts);
        case "import_prompts_backup":
          Object.assign(prompts, JSON.parse(args.backup));
          return clone(prompts);
        default:
          throw new Error(`Unexpected IPC during isolated UI test: ${command}`);
      }
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
}

const browser = await chromium.launch({
  channel: process.env.PLAYWRIGHT_CHANNEL ?? "msedge",
  headless: true,
});
const results = [];
try {
  for (const viewport of [
    { width: 820, height: 650 },
    { width: 720, height: 560 },
  ]) {
    for (const theme of ["light", "dark"]) {
      for (const density of ["compact", "comfortable"]) {
        const context = await browser.newContext({
          viewport,
          reducedMotion: "reduce",
        });
        await context.addInitScript(installMock, { theme, density });
        const page = await context.newPage();
        const errors = [];
        page.on("pageerror", (error) => errors.push(error.message));
        const prefix = `${viewport.width}x${viewport.height}-${theme}-${density}`;
        const checkLayout = async (view) => {
          const layout = await page.evaluate(() => {
            const visible = (element) => element.getClientRects().length > 0;
            const elements = Array.from(
              document.querySelectorAll(
                "button, input, textarea, [role='tabpanel'], [role='tablist']",
              ),
            ).filter(visible);
            const overflow = elements.flatMap((element) => {
              const rect = element.getBoundingClientRect();
              return rect.left < -1 || rect.right > innerWidth + 1
                ? [
                    `${element.getAttribute("data-testid") || element.getAttribute("aria-label") || element.tagName}: ${rect.left}, ${rect.right}`,
                  ]
                : [];
            });
            return {
              width: innerWidth,
              scrollWidth: document.documentElement.scrollWidth,
              mainWidth: document.querySelector("main").clientWidth,
              mainScrollWidth: document.querySelector("main").scrollWidth,
              overflow,
            };
          });
          assert.ok(
            layout.scrollWidth <= layout.width,
            `${prefix} ${view}: root overflow ${JSON.stringify(layout)}`,
          );
          assert.ok(
            layout.mainScrollWidth <= layout.mainWidth,
            `${prefix} ${view}: main overflow ${JSON.stringify(layout)}`,
          );
          assert.deepEqual(
            layout.overflow,
            [],
            `${prefix} ${view}: controls out of bounds`,
          );
          await page.screenshot({
            path: path.join(output, `${prefix}-${view}.png`),
          });
          results.push({ prefix, view, ...layout });
        };
        await page.goto(baseUrl);
        await page.getByTestId("open-ext-hub-mcp").waitFor();
        assert.equal(await page.getByTestId("open-multi-sync").count(), 0);
        const positions = await page
          .locator('[data-testid^="open-ext-hub-"]')
          .evaluateAll((elements) =>
            elements.map((element) => element.getBoundingClientRect().top),
          );
        assert.equal(new Set(positions).size, viewport.width >= 760 ? 1 : 2);
        await checkLayout("overview");
        await page.getByTestId("open-ext-hub-mcp").click();
        await page.getByTestId("ext-mcp-row-docs").waitFor();
        assert.equal(await page.getByRole("tab").count(), 4);
        await checkLayout("mcp");
        await page.getByTestId("ext-mcp-search").fill("no-such-server");
        await page.waitForFunction(
          () =>
            document.querySelectorAll('[data-testid^="ext-mcp-row-"]')
              .length === 0,
        );
        await page.getByTestId("ext-mcp-search").fill("");
        await page.getByTestId("ext-mcp-add-btn").click();
        await page.getByRole("dialog").waitFor();
        await checkLayout("mcp-drawer");
        await page.getByLabel("常用模板").selectOption("context7");
        await page
          .getByRole("button", { name: "保存并同步", exact: true })
          .click();
        await page.getByTestId("ext-mcp-row-context7").waitFor();
        await page.getByTestId("ext-mcp-toggle-context7-codex").click();
        await page.waitForFunction(() =>
          window.__EXT_UI_CALLS__.some(
            (call) =>
              call.command === "toggle_mcp_platform" &&
              call.args.id === "context7" &&
              call.args.platform === "codex",
          ),
        );
        await page.getByTestId("ext-mcp-add-btn").click();
        await page
          .getByRole("button", { name: "智能粘贴 JSON / TOML", exact: true })
          .click();
        await page
          .getByLabel("MCP JSON 或 TOML 片段")
          .fill(
            '{"mcpServers":{"pasted":{"command":"uvx","args":["example-server"]}}}',
          );
        await page.getByLabel("解析后的服务标识").waitFor();
        await page
          .getByRole("button", { name: "保存并同步", exact: true })
          .click();
        await page.getByTestId("ext-mcp-row-pasted").waitFor();
        await page.getByTestId("ext-hub-tab-prompts").click();
        await page.getByTestId("ext-prompts-editor").waitFor();
        await checkLayout("prompts");
        await page
          .getByTestId("ext-prompts-editor")
          .fill("# Updated rules\nKeep these changes.");
        await page.getByTestId("ext-prompts-save-btn").click();
        await page.waitForFunction(() =>
          window.__EXT_UI_CALLS__.some(
            (call) =>
              call.command === "upsert_prompt_preset" &&
              call.args.preset.content.includes("Updated rules"),
          ),
        );
        await page.getByTestId("ext-prompts-sync-other-btn").click();
        await page
          .getByRole("checkbox", { name: "Codex", exact: true })
          .check();
        await page
          .getByRole("button", { name: "同步预设", exact: true })
          .click();
        await page.waitForFunction(() =>
          window.__EXT_UI_CALLS__.some(
            (call) =>
              call.command === "sync_prompt_to_platforms" &&
              call.args.targets.join() === "codex",
          ),
        );
        await page.getByTestId("ext-prompts-platform-codex").click();
        await page.getByTestId("ext-prompts-editor").fill("# Unsaved");
        await page.waitForFunction(
          () =>
            !document.querySelector('[data-testid="ext-prompts-save-btn"]')
              .disabled,
        );
        await page.getByTestId("ext-hub-tab-skills").click();
        await page.getByRole("dialog", { name: "尚有未保存的修改" }).waitFor();
        await page
          .getByRole("button", { name: "继续编辑", exact: true })
          .click();
        await page.getByTestId("topbar-back-home").click();
        await page.getByRole("dialog", { name: "尚有未保存的修改" }).waitFor();
        await page
          .getByRole("button", { name: "继续编辑", exact: true })
          .click();
        await page.getByTestId("ext-prompts-platform-antigravity").click();
        await page.getByRole("dialog", { name: "尚有未保存的修改" }).waitFor();
        await page
          .getByRole("button", { name: "继续编辑", exact: true })
          .click();
        assert.equal(
          await page.getByTestId("ext-prompts-editor").inputValue(),
          "# Unsaved",
        );
        await page.getByTestId("ext-prompts-save-btn").click();
        await page.getByTestId("ext-prompts-platform-antigravity").click();
        await checkLayout("prompts-antigravity");
        await page.getByTestId("ext-prompts-new-btn").click();
        await page.getByLabel("新预设名称").fill("New preset");
        await page
          .getByRole("button", { name: "创建预设", exact: true })
          .click();
        await page
          .getByRole("button", { name: "New preset", exact: true })
          .waitFor();
        await page.getByTestId("ext-hub-tab-skills").click();
        await checkLayout("skills");
        await page.getByTestId("ext-hub-tab-plugins").click();
        await checkLayout("plugins");
        await page.getByTestId("ext-hub-tab-mcp").click();
        await page.getByTestId("ext-mcp-goto-super").click();
        await page.getByTestId("open-mcp-demo").waitFor();
        assert.equal(
          await page
            .getByRole("button", { name: "多平台 MCP 同步", exact: true })
            .count(),
          0,
        );
        await page.getByTestId("open-mcp-demo").click();
        await page.getByTestId("mcp-inline-theater").waitFor();
        await page
          .getByRole("button", { name: "关闭能力体验抽屉", exact: true })
          .click();
        assert.deepEqual(errors, [], `${prefix}: browser errors`);
        await context.close();
        console.log(
          `PASS ${prefix}: home, 4 tabs, MCP form/paste/toggle, prompts save/sync/dirty guard, super MCP`,
        );
      }
    }
  }
  await writeFile(
    path.join(output, "layout-results.json"),
    JSON.stringify(results, null, 2),
  );
  await writeFile(
    path.join(output, "preview.html"),
    `<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Tako Switch Extensions Preview</title></head><body><div id="root"></div><script type="module">${installMock.toString()}\ninstallMock({theme:"light",density:"compact"});\nawait import("/src/main.tsx");</script></body></html>`,
  );
  console.log(`Passed ${results.length} layout checks. Artifacts: ${output}`);
} finally {
  await browser.close();
}
