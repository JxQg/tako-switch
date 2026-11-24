import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";

const root = fileURLToPath(new URL("../", import.meta.url));
const output = path.join(root, "src-tauri", "target", "phase2-ui");
await mkdir(output, { recursive: true });
const preview = process.argv.includes("--preview");
const server = await createServer({
  root,
  plugins: preview
    ? [
        {
          name: "skills-hub-isolated-preview",
          transformIndexHtml: {
            order: "pre",
            handler: () => [
              {
                tag: "script",
                injectTo: "head-prepend",
                children: `(${installMock.toString()})({theme: "light", density: "compact"});`,
              },
            ],
          },
        },
      ]
    : [],
  server: { host: "127.0.0.1", port: preview ? 18042 : 0, strictPort: false },
});
let browser;

function installMock({ theme, density, native = false }) {
  localStorage.setItem("tako-switch-theme", theme);
  localStorage.setItem("tako-switch-density", density);
  const platforms = ["claude", "codex", "antigravity"];
  const callbacks = new Map();
  const listeners = new Map();
  let counter = 0;
  window.isTauri = native;
  window.__emitNativeDrop = (paths) => {
    for (const id of listeners.get("tauri://drag-drop") ?? []) {
      callbacks.get(id)?.({
        event: "tauri://drag-drop",
        id,
        payload: { paths, position: { x: 400, y: 200 } },
      });
    }
  };
  const calls = [];
  window.__skillsCalls = calls;
  const record = (id, name, source, updateAvailable = false) => ({
    skill: {
      id,
      name,
      description:
        "Preserve configuration and review changes across all supported desktop clients. ".repeat(
          3,
        ),
      source,
      repoOwner: source === "github" ? "example" : null,
      repoName: source === "github" ? "skills" : null,
      repoBranch: source === "github" ? "main" : null,
      repoSubdir: `skills/${id}`,
      installedHash: "installed",
      remoteHash: updateAvailable ? "remote" : "installed",
      updateAvailable,
      enabledPlatforms: platforms.slice(),
    },
    masterPath: `sandbox/.tako/switch/skills/${id}`,
    platforms: Object.fromEntries(
      platforms.map((platform, index) => [
        platform,
        {
          enabled: true,
          actualMethod: index === 1 ? "copy" : "symlink",
          hasDiff: false,
          targets: [
            {
              path: `sandbox/${platform}/skills/${id}`,
              actualMethod: index === 1 ? "copy" : "symlink",
              hasDiff: false,
            },
          ],
        },
      ]),
    ),
    locallyModified: false,
  });
  const hub = {
    syncMethod: "auto",
    defaultTargets: platforms.slice(),
    masterDir: "sandbox/.tako/switch/skills",
    skills: [
      record(
        "code-review",
        "Code review and compatibility validation with an intentionally long skill title",
        "github",
        true,
      ),
      record("build-plan", "构建计划与跨平台依赖检查", "local"),
      record("release-notes", "Release notes", "zip"),
    ],
    repos: [
      {
        owner: "example-with-a-long-owner",
        name: "a-long-repository-name-for-truncation-checks",
        branch: "main",
        enabled: true,
        archiveUrlOverride: null,
      },
    ],
  };
  const state = {
    uiDensity: density,
    themePref: theme,
    autoCheckUpdates: false,
    preferChineseUi: true,
    explicitOfficialApps: [],
    mcp: {
      image: true,
      video: false,
      audio: false,
      search: true,
      consult: false,
      webPage: false,
      outputDir: "sandbox/media",
      consultModel: "example-model",
    },
  };
  const account = {
    loggedIn: true,
    baseUrl: "https://example.invalid",
    maskedKey: "masked",
    userName: "Workspace User With Long Name",
    planName: "Professional Workspace Plan",
    walletRemainUsd: 12.34,
    walletUsedUsd: 1.23,
    quota: null,
    error: null,
  };
  const dashboard = {
    account,
    state,
    apps: [],
    cachedManifest: null,
    localGatewayHealthy: false,
    localGatewayUrl: "http://127.0.0.1:18419",
    appVersion: "2.2.7",
    os: "windows",
    uiPatchPrompts: [],
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: (_event, id) => callbacks.delete(id),
  };
  window.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { label: "main", windowLabel: "main" },
    },
    transformCallback: (callback) => {
      const id = ++counter;
      callbacks.set(id, callback);
      return id;
    },
    unregisterCallback: (id) => callbacks.delete(id),
    invoke: async (command, payload = {}) => {
      calls.push({ command, payload: structuredClone(payload) });
      if (command === "plugin:event|listen") {
        const entries = listeners.get(payload.event) ?? [];
        entries.push(payload.handler);
        listeners.set(payload.event, entries);
        return payload.handler;
      }
      if (command === "plugin:event|unlisten") return;
      if (command === "get_dashboard") return structuredClone(dashboard);
      if (command === "get_running_states") return {};
      if (command === "save_settings") {
        Object.assign(state, payload.input);
        return structuredClone(state);
      }
      if (
        ["get_skills_hub_snapshot", "scan_adopt_local_skills"].includes(command)
      )
        return structuredClone(hub);
      if (command === "toggle_skill_platform") {
        const skill = hub.skills.find((item) => item.skill.id === payload.id);
        skill.platforms[payload.platform].enabled = payload.enabled;
        skill.platforms[payload.platform].actualMethod = payload.enabled
          ? "copy"
          : null;
        return structuredClone(hub);
      }
      if (command === "align_skill_platforms") {
        const skill = hub.skills.find((item) => item.skill.id === payload.id);
        for (const platform of platforms)
          skill.platforms[platform].enabled =
            payload.targets.includes(platform);
        return structuredClone(hub);
      }
      if (command === "set_skill_sync_method") {
        hub.syncMethod = payload.method;
        return structuredClone(hub);
      }
      if (command === "set_skill_default_targets") {
        hub.defaultTargets = payload.targets;
        return structuredClone(hub);
      }
      if (command === "uninstall_skill") {
        hub.skills = hub.skills.filter((item) => item.skill.id !== payload.id);
        return structuredClone(hub);
      }
      if (command === "update_installed_skill") {
        hub.skills.find(
          (item) => item.skill.id === payload.id,
        ).skill.updateAvailable = false;
        return structuredClone(hub);
      }
      if (command === "import_local_skill") {
        hub.skills.push(record("imported", "Imported skill", "zip"));
        return structuredClone(hub);
      }
      if (command === "upsert_skill_repo") {
        hub.repos = hub.repos.filter(
          (repo) =>
            repo.owner !== payload.repo.owner ||
            repo.name !== payload.repo.name,
        );
        hub.repos.push(payload.repo);
        return structuredClone(hub);
      }
      if (command === "remove_skill_repo") {
        hub.repos = hub.repos.filter(
          (repo) => repo.owner !== payload.owner || repo.name !== payload.name,
        );
        return structuredClone(hub);
      }
      if (command === "discover_repo_skills")
        return ["archive-skill", "script-skill"].map((id) => ({
          id,
          name:
            id === "archive-skill"
              ? "Repository skill with a very long display name that must not resize columns"
              : "Script skill",
          description: "Archive content with scripts and reference assets.",
          repoOwner: payload.owner,
          repoName: payload.name,
          repoBranch: "main",
          repoSubdir: `skills/${id}`,
          remoteHash: id,
          installedId: null,
          updateAvailable: false,
        }));
      if (command === "install_discovered_skill") {
        const id = payload.subdir.split("/").at(-1);
        const skill = record(id, id, "github");
        skill.skill.repoOwner = payload.owner;
        skill.skill.repoName = payload.name;
        hub.skills.push(skill);
        return structuredClone(hub);
      }
      return null;
    },
  };
}

async function verifyBounds(page, label) {
  const result = await page.evaluate(() => {
    const outside = Array.from(
      document.querySelectorAll(
        "header, header button, [data-testid=ext-skills-view] button, [data-testid=ext-skills-view] input, [role=dialog], [role=menu], [role=tooltip]",
      ),
    )
      .map((element) => ({
        text:
          element.getAttribute("aria-label") ||
          element.textContent.slice(0, 60),
        rect: element.getBoundingClientRect(),
      }))
      .filter(
        ({ rect }) =>
          rect.width > 0 &&
          (rect.left < -1 || rect.right > window.innerWidth + 1),
      )
      .map(({ text, rect }) => ({ text, left: rect.left, right: rect.right }));
    const headers = Array.from(
      document.querySelector("header")?.children ?? [],
    ).map((element) => element.getBoundingClientRect());
    return {
      scrollWidth: document.documentElement.scrollWidth,
      width: window.innerWidth,
      outside,
      headerOverlap:
        headers.length === 3 && headers[0].right > headers[2].left + 1,
    };
  });
  assert.ok(
    result.scrollWidth <= result.width,
    `${label}: horizontal overflow ${JSON.stringify(result)}`,
  );
  assert.deepEqual(result.outside, [], `${label}: out-of-bounds controls`);
  assert.equal(result.headerOverlap, false, `${label}: overlapping titlebar`);
}

if (preview) {
  await server.listen();
  const address = server.httpServer.address();
  assert.ok(address && typeof address !== "string");
  console.log(`Isolated Skills Hub preview: http://127.0.0.1:${address.port}`);
  console.log(
    "Tauri IPC uses in-memory fixtures. No native client configuration is read or written.",
  );
  for (const signal of ["SIGINT", "SIGTERM"]) {
    process.once(signal, () => {
      void server.close().then(() => process.exit(0));
    });
  }
} else {
  try {
    await server.listen();
    const address = server.httpServer.address();
    assert.ok(address && typeof address !== "string");
    browser = await chromium.launch({ headless: true });
    let checks = 0;
    for (const [width, height] of [
      [720, 560],
      [820, 650],
    ]) {
      for (const theme of ["light", "dark"]) {
        for (const density of ["compact", "comfortable"]) {
          const context = await browser.newContext({
            viewport: { width, height },
            deviceScaleFactor: 1,
          });
          await context.addInitScript(installMock, { theme, density });
          const page = await context.newPage();
          const errors = [];
          page.on("pageerror", (error) => errors.push(error.message));
          await page.goto(`http://127.0.0.1:${address.port}`);
          await page.getByTestId("open-ext-hub-skills").click();
          await page.getByTestId("ext-skill-row-code-review").waitFor();
          const label = `${width}x${height}-${theme}-${density}`;
          await verifyBounds(page, `${label}-installed`);
          await page.screenshot({
            path: path.join(output, `${label}-installed.png`),
          });
          const help = page.getByRole("button", {
            name: "主副本与同步策略",
            exact: true,
          });
          await help.click();
          await page.getByRole("tooltip").waitFor();
          const helpBounds = await help.boundingBox();
          assert.equal(helpBounds.width, 26);
          assert.equal(helpBounds.height, 26);
          await verifyBounds(page, `${label}-help`);
          await page.keyboard.press("Escape");
          await page.getByTestId("ext-skills-search").fill("构建");
          await page.waitForFunction(
            () =>
              document.querySelectorAll('[data-testid^="ext-skill-row-"]')
                .length === 1,
          );
          assert.equal(
            await page.locator('[data-testid^="ext-skill-row-"]').count(),
            1,
          );
          await page.getByTestId("ext-skills-search").fill("");
          await page.waitForFunction(
            () =>
              document.querySelectorAll('[data-testid^="ext-skill-row-"]')
                .length === 3,
          );
          await page.getByRole("button", { name: "更多", exact: true }).click();
          await verifyBounds(page, `${label}-menu`);
          await page.getByRole("menuitemradio", { name: "强制复制" }).click();
          await page.getByTestId("ext-skills-import-btn").click();
          await page.getByRole("dialog").waitFor();
          await verifyBounds(page, `${label}-drawer`);
          await page.screenshot({
            path: path.join(output, `${label}-import.png`),
          });
          await page
            .getByRole("tab", { name: "本地目录", exact: true })
            .click();
          await page
            .getByPlaceholder("包含 SKILL.md 的目录")
            .fill("sandbox/imports/local-skill");
          await page.keyboard.press("Escape");
          assert.equal(await page.getByRole("dialog").count(), 0);
          assert.equal(await page.getByTestId("ext-skills-view").count(), 1);
          await page.getByTestId("ext-skills-subtab-discover").click();
          await page
            .getByRole("button", { name: "拉取技能", exact: true })
            .click();
          await page
            .getByRole("heading", { name: "Script skill", exact: true })
            .waitFor();
          await verifyBounds(page, `${label}-discover`);
          await page.screenshot({
            path: path.join(output, `${label}-discover.png`),
          });
          if (width === 720 && theme === "dark" && density === "comfortable") {
            await page
              .getByRole("button", { name: "安装", exact: true })
              .first()
              .click();
            await page.waitForFunction(() =>
              window.__skillsCalls.some(
                (call) => call.command === "install_discovered_skill",
              ),
            );
            await page
              .getByRole("button", { name: "添加仓库", exact: true })
              .click();
            await page
              .getByPlaceholder("owner/repo")
              .fill("another/repository");
            await page
              .getByRole("dialog")
              .getByRole("button", { name: "添加仓库", exact: true })
              .click();
            await page
              .getByRole("dialog")
              .getByText("another/repository", { exact: true })
              .waitFor();
            await page.keyboard.press("Escape");
            await page.getByTestId("ext-skills-import-btn").click();
            await page
              .getByRole("tab", { name: "ZIP 文件", exact: true })
              .click();
            await page.getByLabel("选择 ZIP 文件").setInputFiles({
              name: "skills.zip",
              mimeType: "application/zip",
              buffer: Buffer.from("zip-fixture"),
            });
            await page.getByTestId("ext-skills-import-confirm").click();
            await page.getByTestId("ext-skill-row-imported").waitFor();
            const payload = await page.evaluate(
              () =>
                window.__skillsCalls.findLast(
                  (call) => call.command === "import_local_skill",
                ).payload,
            );
            assert.equal(
              payload.input.zipBase64,
              Buffer.from("zip-fixture").toString("base64"),
            );
            assert.deepEqual(payload.input.targets, [
              "claude",
              "codex",
              "antigravity",
            ]);
            await page.getByTestId("ext-skill-imported-codex").click();
            assert.equal(
              await page
                .getByTestId("ext-skill-imported-codex")
                .getAttribute("aria-checked"),
              "false",
            );
            await page
              .getByRole("button", {
                name: "Imported skill 的操作",
                exact: true,
              })
              .click();
            await page
              .getByRole("menuitem", { name: "彻底卸载", exact: true })
              .click();
            await page
              .getByRole("dialog")
              .getByRole("button", { name: "彻底卸载", exact: true })
              .click();
            await page
              .getByTestId("ext-skill-row-imported")
              .waitFor({ state: "detached" });
          }
          assert.deepEqual(errors, [], `${label}: uncaught browser errors`);
          console.log(
            `PASS ${label}: installed, discovery, drawer, menu, search, bounds`,
          );
          checks += 1;
          await context.close();
        }
      }
    }
    const nativeContext = await browser.newContext({
      viewport: { width: 720, height: 560 },
    });
    await nativeContext.addInitScript(installMock, {
      theme: "dark",
      density: "compact",
      native: true,
    });
    const nativePage = await nativeContext.newPage();
    await nativePage.goto(`http://127.0.0.1:${address.port}`);
    await nativePage.getByTestId("open-ext-hub-skills").click();
    await nativePage.getByTestId("ext-skills-import-btn").click();
    await nativePage.evaluate(() =>
      window.__emitNativeDrop(["sandbox/imports/native-skills.zip"]),
    );
    await nativePage.getByText("native-skills.zip", { exact: true }).waitFor();
    await nativePage.getByTestId("ext-skills-import-confirm").click();
    await nativePage.getByTestId("ext-skill-row-imported").waitFor();
    const nativePayload = await nativePage.evaluate(
      () =>
        window.__skillsCalls.findLast(
          (call) => call.command === "import_local_skill",
        ).payload,
    );
    assert.equal(nativePayload.input.path, "sandbox/imports/native-skills.zip");
    await nativeContext.close();
    console.log("PASS native Tauri drag/drop path submission");
    console.log(
      `Skills Hub UI: ${checks}/8 viewport/theme/density combinations passed.`,
    );
    console.log(`Screenshots: ${output}`);
  } finally {
    await browser?.close();
    await server.close();
  }
}
