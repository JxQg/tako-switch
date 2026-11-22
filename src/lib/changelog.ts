import changelogRaw from "../../CHANGELOG.md?raw";

export interface ChangelogRelease {
  version: string;
  date: string | null;
  items: string[];
}

const HEADING_RE =
  /^##\s+\[?v?([0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?)\]?(?:\s*(?:\(([^)]+)\)|-\s*(\S+)))?/;

const GENERIC_FALLBACK_RE = /^Tako Switch v?[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$/i;

function normalizeLine(raw: string): string {
  return raw
    .trim()
    .replace(/^(?:[-*•]\s+|\d+[.)]\s+)/, "")
    .trim();
}

export function parseReleaseNotesLines(notes: string | null | undefined): string[] {
  if (!notes) return [];
  const trimmed = notes.trim();
  if (!trimmed || GENERIC_FALLBACK_RE.test(trimmed)) return [];

  const lines: string[] = [];
  for (const rawLine of trimmed.split(/\r?\n/)) {
    const cleaned = normalizeLine(rawLine);
    if (!cleaned || cleaned.startsWith("#")) continue;
    lines.push(cleaned);
  }
  return lines;
}

export function parseChangelog(markdown: string): ChangelogRelease[] {
  const releases: ChangelogRelease[] = [];
  let current: ChangelogRelease | null = null;

  for (const rawLine of markdown.split(/\r?\n/)) {
    const trimmed = rawLine.trim();
    const headingMatch = HEADING_RE.exec(trimmed);
    if (headingMatch) {
      if (current && current.items.length > 0) {
        releases.push(current);
      }
      current = {
        version: headingMatch[1],
        date: (headingMatch[2] || headingMatch[3] || "").trim() || null,
        items: [],
      };
      continue;
    }

    if (trimmed.startsWith("## ")) {
      if (current && current.items.length > 0) {
        releases.push(current);
      }
      current = null;
      continue;
    }

    if (!current) continue;
    const cleaned = normalizeLine(rawLine);
    if (!cleaned || cleaned.startsWith("#")) continue;
    current.items.push(cleaned);
  }

  if (current && current.items.length > 0) {
    releases.push(current);
  }

  return releases;
}

export const CHANGELOG_RELEASES: ChangelogRelease[] = parseChangelog(changelogRaw);
