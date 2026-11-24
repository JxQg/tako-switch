import type {
  AppId,
  AppSnapshot,
  ClientModelFilter,
  ThemePref,
  UiDensity,
} from "../lib/api";

export type NavRoute = "overview" | "general" | "mcp" | "extensions" | AppId;
export type ExtensionTab = "mcp" | "skills" | "plugins" | "prompts";
export type { ThemePref, UiDensity };

export const CLIENT_ORDER: AppId[] = ["claude", "codex", "antigravity", "dsh-tako"];

export const APP_DISPLAY_NAMES: Record<AppId, string> = {
  claude: "Claude Desktop",
  codex: "Codex",
  antigravity: "Antigravity",
  "dsh-tako": "DSH Tako",
};

export const APP_FULL_NAMES: Record<AppId, string> = {
  claude: "Claude Desktop",
  codex: "Codex Desktop",
  antigravity: "Google Antigravity",
  "dsh-tako": "DSH Tako",
};

export const APP_SUBTITLES: Record<AppId, string> = {
  claude: "模型、运行模式与界面简体中文，仅影响 Claude Desktop。",
  codex: "模型目录、官方登录保留与运行模式，仅影响 Codex。",
  antigravity: "32 个自定义模型槽位、专用 Key 与界面简体中文，仅影响 Antigravity。",
  "dsh-tako": "密钥同步与独立登录，仅影响 DSH Tako 原生客户端。",
};

export function formatVersion(version?: string | null): string {
  if (!version) return "";
  return `v${version.replace(/^v/i, "")}`;
}

export function formatBytes(bytes?: number | null): string {
  if (!bytes || bytes <= 0) return "";
  if (bytes >= 1_073_741_824) return `${(bytes / 1_073_741_824).toFixed(2)} GB`;
  return `${(bytes / 1_048_576).toFixed(0)} MB`;
}

export function formatTimeHHMM(date = new Date()): string {
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

export function calcPercent(used?: number | null, limit?: number | null): number {
  if (!limit || limit <= 0 || !used || used <= 0) return 0;
  return Math.min(100, Math.round((used / limit) * 100));
}

export function installStageLabel(stage: string): string {
  switch (stage) {
    case "downloading":
      return "下载中";
    case "verifying":
      return "校验中";
    case "closing":
      return "退出旧版";
    case "installing":
      return "安装中";
    case "configuring":
      return "恢复配置";
    case "done":
      return "已完成";
    case "cancelled":
      return "已取消";
    case "error":
      return "失败";
    default:
      return "处理中";
  }
}

export function formatModelFilterLabel(
  appId: AppId,
  filter?: ClientModelFilter,
): string {
  if (!filter) return "推荐模型";
  const count = filter.enabledIds.length;
  if (appId === "antigravity") {
    if (filter.mode === "recommended") {
      return count > 0 ? `推荐 · ${count} / 32 槽位` : "推荐精简";
    }
    if (filter.mode === "custom") {
      return `自选 · ${count} / 32 槽位`;
    }
    return "全部模型（最多 32 槽位）";
  }
  if (filter.mode === "recommended") {
    return count > 0 ? `推荐 · ${count} 个模型` : "推荐精简";
  }
  if (filter.mode === "custom") {
    return count > 0 ? `自选 · ${count} 个模型` : "自定义列表";
  }
  return "全部模型";
}

export function sidebarShortStatus(app?: AppSnapshot): {
  text: string;
  tone: "ok" | "accent" | "neutral" | "muted";
} {
  if (!app || !app.install.installed) {
    return { text: "未装", tone: "muted" };
  }
  if (app.id === "dsh-tako") {
    return app.mode === "tako"
      ? { text: "同步", tone: "ok" }
      : { text: "独立", tone: "neutral" };
  }
  return app.mode === "tako"
    ? { text: "Tako", tone: "accent" }
    : { text: "官方", tone: "neutral" };
}

export function clientModeLabel(app: AppSnapshot): string {
  if (app.id === "dsh-tako") {
    return app.mode === "tako" ? "密钥已同步" : "独立登录";
  }
  return app.mode === "tako" ? "Tako 已接管" : "官方原版";
}

export function ToggleSwitch({
  checked,
  onChange,
  disabled = false,
  ariaLabel,
  testId,
  "data-testid": dataTestId,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  ariaLabel?: string;
  testId?: string;
  "data-testid"?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      data-testid={dataTestId ?? testId}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative inline-flex h-[21px] w-[36px] shrink-0 items-center rounded-full transition-colors duration-150 cursor-pointer disabled:opacity-50 ${
        checked ? "bg-[var(--accent)]" : "bg-[var(--border-strong)]"
      }`}
    >
      <span
        className={`inline-block h-[17px] w-[17px] rounded-full bg-white shadow-sm transition-transform duration-150 ${
          checked ? "translate-x-[17px]" : "translate-x-[2px]"
        }`}
      />
    </button>
  );
}

export function StatusPill({
  tone = "neutral",
  children,
}: {
  tone?: "ok" | "warn" | "accent" | "neutral";
  children: React.ReactNode;
}) {
  const toneClass =
    tone === "ok"
      ? "bg-[var(--success-soft)] text-[var(--success)]"
      : tone === "warn"
        ? "bg-[var(--warning-soft)] text-[var(--warning)]"
        : tone === "accent"
          ? "bg-[var(--accent-soft)] text-[var(--accent)]"
          : "bg-[var(--bg-subtle)] text-[var(--text-secondary)]";

  return (
    <span
      className={`inline-flex items-center gap-1.5 h-[var(--density-pill-h)] px-2.5 rounded-full text-[11px] font-semibold shrink-0 whitespace-nowrap ${toneClass}`}
    >
      {children}
    </span>
  );
}

export function SegmentedControl<T extends string>({
  options,
  value,
  onChange,
  disabled = false,
  testId,
}: {
  options: { id: T; label: string }[];
  value: T;
  onChange: (next: T) => void;
  disabled?: boolean;
  testId?: string;
}) {
  return (
    <div
      data-testid={testId}
      className="inline-flex p-[3px] rounded-[9px] bg-[var(--bg-pill)] border border-[var(--border)]/70 shrink-0"
    >
      {options.map((opt) => {
        const active = value === opt.id;
        return (
          <button
            key={opt.id}
            type="button"
            data-testid={testId ? `${testId}-${opt.id}` : undefined}
            disabled={disabled}
            onClick={() => onChange(opt.id)}
            className={`h-[var(--density-ctrl-sm-h)] min-w-[64px] px-2.5 rounded-[7px] text-[12px] transition cursor-pointer disabled:opacity-50 whitespace-nowrap ${
              active
                ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-semibold shadow-[var(--shadow-segment)]"
                : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
            }`}
          >
            {opt.label}
          </button>
        );
      })}
    </div>
  );
}
