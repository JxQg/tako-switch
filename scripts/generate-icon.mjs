import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const iconsDir = join(root, "src-tauri", "icons");
const sourceIcon = join(iconsDir, "app-icon.svg");

mkdirSync(iconsDir, { recursive: true });
writeFileSync(
  sourceIcon,
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">
  <rect width="1024" height="1024" rx="192" fill="#173f4b"/>
  <path d="M228 270h568v130H577v354H447V400H228z" fill="#fff"/>
  <path d="M274 592c0-115 93-208 208-208h268v130H482c-43 0-78 35-78 78s35 78 78 78h268v130H482c-115 0-208-93-208-208z" fill="#33c7a1"/>
</svg>
`,
  "utf8"
);

const result = spawnSync(
  "bun",
  ["x", "tauri", "icon", sourceIcon, "--output", iconsDir],
  {
    cwd: root,
    stdio: "inherit",
    shell: false
  }
);

if (result.error) {
  console.error(result.error);
  process.exit(1);
}

if (result.status !== 0) {
  process.exit(result.status ?? 1);
}
