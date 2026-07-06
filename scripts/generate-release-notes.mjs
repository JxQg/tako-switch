#!/usr/bin/env bun

import { execFileSync } from "node:child_process";

const [, , rawTag] = process.argv;
const tag = rawTag || process.env.GITHUB_REF_NAME || "";
const semverPattern =
  /^v(?<version>(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?)$/;

if (!semverPattern.test(tag)) {
  console.error(
    `Expected a release tag like v0.1.0, received "${tag || "<empty>"}".`,
  );
  process.exit(1);
}

function git(args, fallback = "") {
  try {
    return execFileSync("git", args, {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
  } catch {
    return fallback;
  }
}

function resolveRevision(ref) {
  return git(["rev-parse", "--verify", "--quiet", ref]) ? ref : "HEAD";
}

const revision = resolveRevision(tag);
const previousTag = git(["describe", "--tags", "--abbrev=0", `${revision}^`]);
const range = previousTag ? `${previousTag}..${revision}` : revision;
const logFormat = "%H%x1f%s%x1f%b%x1e";
const rawLog = git(["log", "--reverse", `--format=${logFormat}`, range]);
const commitPattern =
  /^(?<type>[a-z]+)(?:\((?<scope>[^)]+)\))?(?<breaking>!)?:\s*(?<summary>.+)$/i;
const categoryOrder = [
  "Breaking Changes",
  "Features",
  "Fixes",
  "Build",
  "Performance",
  "Documentation",
  "Tests",
  "Maintenance",
  "Reverts",
  "Other Changes",
];
const typeCategory = new Map([
  ["feat", "Features"],
  ["fix", "Fixes"],
  ["build", "Build"],
  ["ci", "Build"],
  ["perf", "Performance"],
  ["docs", "Documentation"],
  ["test", "Tests"],
  ["tests", "Tests"],
  ["refactor", "Maintenance"],
  ["style", "Maintenance"],
  ["chore", "Maintenance"],
  ["revert", "Reverts"],
]);
const groups = new Map(categoryOrder.map((title) => [title, []]));
const repo = process.env.GITHUB_REPOSITORY;
const serverUrl = process.env.GITHUB_SERVER_URL || "https://github.com";

function commitReference(sha) {
  const shortSha = sha.slice(0, 7);

  if (!repo) {
    return shortSha;
  }

  return `[${shortSha}](${serverUrl}/${repo}/commit/${sha})`;
}

function addCommit(record) {
  const [sha, subject, body = ""] = record.split("\x1f");
  const match = commitPattern.exec(subject);
  const hasBreakingFooter = /^BREAKING[ -]CHANGE:/m.test(body);
  const type = match?.groups?.type?.toLowerCase();
  const scope = match?.groups?.scope;
  const summary = match?.groups?.summary || subject;
  const title =
    match?.groups?.breaking || hasBreakingFooter
      ? "Breaking Changes"
      : typeCategory.get(type || "") || "Other Changes";
  const scopeText = scope ? ` (${scope})` : "";

  groups.get(title).push(`- ${summary}${scopeText} (${commitReference(sha)})`);
}

for (const record of rawLog.split("\x1e")) {
  if (record.trim()) {
    addCommit(record.trim());
  }
}

const lines = [
  "Installers are attached below.",
  "",
  "- Windows: NSIS `.exe` and MSI `.msi`",
  "- macOS: universal Apple Silicon + Intel `.dmg`",
  "",
];

if (previousTag) {
  lines.push(`Changes since \`${previousTag}\`:`, "");
} else {
  lines.push("Changes included in this release:", "");
}

let hasChanges = false;

for (const title of categoryOrder) {
  const entries = groups.get(title);

  if (entries.length === 0) {
    continue;
  }

  hasChanges = true;
  lines.push(`## ${title}`, "", ...entries, "");
}

if (!hasChanges) {
  lines.push("No commits found for this release.", "");
}

console.log(lines.join("\n").trimEnd());
