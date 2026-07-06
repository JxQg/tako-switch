#!/usr/bin/env bun

import { readFileSync, writeFileSync } from "node:fs";

const [, , rawTag, ...flags] = process.argv;
const checkOnly = flags.includes("--check");
const validateOnly = flags.includes("--validate-only");
const tag = rawTag || process.env.GITHUB_REF_NAME || "";
const semverPattern =
  /^v(?<version>(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?)$/;
const match = semverPattern.exec(tag);

if (!match?.groups?.version) {
  console.error(
    `Expected a release tag like v0.1.0, received "${tag || "<empty>"}".`,
  );
  process.exit(1);
}

const version = match.groups.version;

if (validateOnly) {
  console.log(`Validated app version ${version} from tag ${tag}.`);
  process.exit(0);
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

function planJsonVersion(path) {
  const data = readJson(path);
  if (data.version === version) {
    return null;
  }

  data.version = version;
  return { path, write: () => writeJson(path, data) };
}

function planCargoVersion(path) {
  const source = readFileSync(path, "utf8");
  const newline = source.includes("\r\n") ? "\r\n" : "\n";
  const lines = source.split(/\r?\n/);
  let inPackageSection = false;
  let versionLineIndex = -1;
  let currentVersion = "";

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    const section = /^\s*\[([^\]]+)\]\s*$/.exec(line);

    if (section) {
      inPackageSection = section[1] === "package";
      continue;
    }

    if (!inPackageSection) {
      continue;
    }

    const versionLine = /^version\s*=\s*"([^"]+)"\s*$/.exec(line);

    if (versionLine) {
      versionLineIndex = index;
      currentVersion = versionLine[1];
      break;
    }
  }

  if (versionLineIndex === -1) {
    throw new Error(`Could not find package version in ${path}.`);
  }

  if (currentVersion === version) {
    return null;
  }

  lines[versionLineIndex] = `version = "${version}"`;

  return {
    path,
    write: () => writeFileSync(path, lines.join(newline)),
  };
}

const plannedChanges = [
  planJsonVersion("package.json"),
  planJsonVersion("src-tauri/tauri.conf.json"),
  planCargoVersion("src-tauri/Cargo.toml"),
].filter(Boolean);

if (checkOnly && plannedChanges.length > 0) {
  console.error(
    `Version ${version} is not synchronized in: ${plannedChanges
      .map((change) => change.path)
      .join(", ")}`,
  );
  process.exit(1);
}

if (!checkOnly) {
  for (const change of plannedChanges) {
    change.write();
  }
}

const action = checkOnly ? "Verified" : "Synchronized";
console.log(`${action} app version ${version} from tag ${tag}.`);
