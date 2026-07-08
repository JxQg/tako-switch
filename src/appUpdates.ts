import appConfig from "../src-tauri/tauri.conf.json";

export const PROJECT_URL = "https://github.com/JxQg/tako-switch";
const LATEST_RELEASE_API_URL = "https://api.github.com/repos/JxQg/tako-switch/releases/latest";

export const APP_VERSION = appConfig.version;
export const APP_DISPLAY_VERSION = `v${normalizeVersion(APP_VERSION)}`;

export type UpdatePlatform = "windows" | "macos" | "unsupported";

export type UpdateAsset = {
  name: string;
  downloadUrl: string;
};

export type AppUpdateStatus = {
  available: boolean;
  currentVersion: string;
  latestVersion: string;
  platform: UpdatePlatform;
  asset: UpdateAsset | null;
  releaseUrl: string;
  releaseNotes: string;
  publishedAt: string | null;
  checkedAt: string;
};

type GitHubReleaseAsset = {
  name: string;
  browser_download_url: string;
};

type GitHubRelease = {
  tag_name: string;
  html_url: string;
  body?: string | null;
  published_at?: string | null;
  assets?: GitHubReleaseAsset[];
};

type ParsedVersion = {
  core: [number, number, number];
  pre: string[];
};

export async function checkForAppUpdate(): Promise<AppUpdateStatus> {
  const response = await fetch(LATEST_RELEASE_API_URL, {
    headers: {
      Accept: "application/vnd.github+json"
    }
  });

  if (!response.ok) {
    throw new Error(`GitHub Releases returned ${response.status}`);
  }

  const release = (await response.json()) as GitHubRelease;
  const latestVersion = normalizeVersion(release.tag_name);
  const platform = detectCurrentPlatform();
  const asset = selectReleaseAsset(release.assets || [], platform);

  return {
    available: compareVersions(latestVersion, APP_VERSION) > 0,
    currentVersion: APP_VERSION,
    latestVersion,
    platform,
    asset,
    releaseUrl: release.html_url,
    releaseNotes: release.body?.trim() || "",
    publishedAt: release.published_at || null,
    checkedAt: new Date().toISOString()
  };
}

export function getUpdateOpenUrl(update: AppUpdateStatus) {
  return update.asset?.downloadUrl || update.releaseUrl;
}

export function normalizeVersion(version: string) {
  return version.trim().replace(/^v/i, "");
}

export function compareVersions(a: string, b: string): number {
  const parsedA = parseVersion(a);
  const parsedB = parseVersion(b);
  if (!parsedA || !parsedB) return 0;

  for (let index = 0; index < 3; index += 1) {
    const difference = parsedA.core[index] - parsedB.core[index];
    if (difference !== 0) return difference < 0 ? -1 : 1;
  }

  return comparePreRelease(parsedA.pre, parsedB.pre);
}

export function detectCurrentPlatform(): UpdatePlatform {
  if (typeof navigator === "undefined") return "unsupported";

  const platformText = `${navigator.platform} ${navigator.userAgent}`.toLowerCase();
  if (platformText.includes("win")) return "windows";
  if (platformText.includes("mac") || platformText.includes("darwin")) return "macos";
  return "unsupported";
}

export function selectReleaseAsset(assets: GitHubReleaseAsset[], platform: UpdatePlatform): UpdateAsset | null {
  const matches = (predicate: (assetName: string) => boolean) => {
    const asset = assets.find((candidate) => predicate(candidate.name.toLowerCase()));
    return asset
      ? {
          name: asset.name,
          downloadUrl: asset.browser_download_url
        }
      : null;
  };

  if (platform === "windows") {
    return (
      matches((name) => name.includes("windows") && name.includes("x64") && name.endsWith("-setup.exe")) ||
      matches((name) => name.includes("windows") && name.endsWith("-setup.exe")) ||
      matches((name) => name.includes("windows") && name.includes("x64") && name.endsWith(".msi")) ||
      matches((name) => name.includes("windows") && name.endsWith(".msi"))
    );
  }

  if (platform === "macos") {
    return (
      matches((name) => name.includes("darwin") && name.includes("universal") && name.endsWith(".dmg")) ||
      matches((name) => name.includes("darwin") && name.endsWith(".dmg")) ||
      matches((name) => name.endsWith(".dmg"))
    );
  }

  return null;
}

function parseVersion(version: string): ParsedVersion | null {
  const match = normalizeVersion(version).match(/^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?/);
  if (!match) return null;

  return {
    core: [Number(match[1]), Number(match[2]), Number(match[3])],
    pre: match[4] ? match[4].split(".") : []
  };
}

function comparePreRelease(a: string[], b: string[]): number {
  if (a.length === 0 && b.length === 0) return 0;
  if (a.length === 0) return 1;
  if (b.length === 0) return -1;

  const length = Math.min(a.length, b.length);
  for (let index = 0; index < length; index += 1) {
    const currentA = a[index];
    const currentB = b[index];
    const aIsNumber = /^\d+$/.test(currentA);
    const bIsNumber = /^\d+$/.test(currentB);

    if (aIsNumber && bIsNumber) {
      const difference = Number(currentA) - Number(currentB);
      if (difference !== 0) return difference < 0 ? -1 : 1;
    } else if (aIsNumber) {
      return -1;
    } else if (bIsNumber) {
      return 1;
    } else if (currentA !== currentB) {
      return currentA < currentB ? -1 : 1;
    }
  }

  if (a.length === b.length) return 0;
  return a.length < b.length ? -1 : 1;
}
