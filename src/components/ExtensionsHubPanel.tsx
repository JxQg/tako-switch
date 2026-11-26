import { useCallback, useState } from "react";
import { FileText, Plug, Puzzle, Zap, type LucideIcon } from "lucide-react";
import type { ExtensionsSummary } from "../lib/api";
import type { ExtensionTab } from "./UiPrimitives";
import {
  EXT_BUTTON,
  EXT_PRIMARY_BUTTON,
  ExtMoreDropdown,
  type ExtensionActions,
  type ExtensionNavigationGuard,
} from "./extensions/ExtensionPrimitives";
import { McpHubView } from "./extensions/McpHubView";
import { PluginsHubView } from "./extensions/PluginsHubView";
import { PromptsHubView } from "./extensions/PromptsHubView";
import { SkillsHubView } from "./extensions/SkillsHubView";

export const EXTENSION_TABS: {
  id: ExtensionTab;
  label: string;
  icon: LucideIcon;
  count: keyof ExtensionsSummary;
}[] = [
  { id: "mcp", label: "通用 MCP", icon: Plug, count: "mcpCount" },
  { id: "skills", label: "Skills 技能", icon: Zap, count: "skillsCount" },
  { id: "plugins", label: "插件管理", icon: Puzzle, count: "pluginsCount" },
  { id: "prompts", label: "提示词预设", icon: FileText, count: "promptsCount" },
];

interface ExtensionsHubPanelProps {
  activeTab: ExtensionTab;
  onTabChange: (tab: ExtensionTab) => void;
  summary: ExtensionsSummary | null;
  onCountChange: (
    tab: ExtensionTab,
    count: number,
    updatableCount?: number,
  ) => void;
  onGoSuperMcp: () => void;
  onNotify?: (message: string) => void;
  onNavigationGuardChange: (guard: ExtensionNavigationGuard | null) => void;
}

export function ExtensionsHubPanel({
  activeTab,
  onTabChange,
  summary,
  onCountChange,
  onGoSuperMcp,
  onNotify,
  onNavigationGuardChange,
}: ExtensionsHubPanelProps) {
  const [actions, setActions] = useState<ExtensionActions>({});
  const countMcp = useCallback(
    (count: number) => onCountChange("mcp", count),
    [onCountChange],
  );
  const countSkills = useCallback(
    (count: number, updatableCount?: number) =>
      onCountChange("skills", count, updatableCount),
    [onCountChange],
  );
  const countPlugins = useCallback(
    (count: number) => onCountChange("plugins", count),
    [onCountChange],
  );
  const countPrompts = useCallback(
    (count: number) => onCountChange("prompts", count),
    [onCountChange],
  );

  const SecondaryIcon = actions.secondary?.icon;
  const PrimaryIcon = actions.primary?.icon;

  return (
    <section
      className="tako-page-sub flex min-w-0 flex-col"
      data-testid="extensions-hub-panel"
    >
      <div className="ext-hub-toolbar flex min-w-0 items-center justify-between gap-2 border-b border-[var(--border)] pb-3">
        <div
          role="tablist"
          aria-label="通用扩展"
          className="ext-hub-tabs inline-flex min-w-0 items-center rounded-lg border border-[var(--border)] bg-[var(--bg-pill)] p-0.5"
        >
          {EXTENSION_TABS.map(({ id, label, icon: Icon, count }) => (
            <button
              key={id}
              type="button"
              role="tab"
              id={`ext-tab-${id}`}
              aria-controls={`ext-panel-${id}`}
              aria-selected={activeTab === id}
              data-testid={`ext-hub-tab-${id}`}
              onClick={() => onTabChange(id)}
              className={`ext-hub-tab flex min-w-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-md text-[11.5px] cursor-pointer ${activeTab === id ? "bg-[var(--bg-card)] font-semibold text-[var(--text-primary)] shadow-[var(--shadow-segment)]" : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"}`}
            >
              <Icon className="h-3.5 w-3.5 shrink-0" strokeWidth={1.8} />
              <span className="truncate">{label}</span>
              <span className="min-w-[16px] shrink-0 rounded bg-[var(--bg-subtle)] px-1 text-center text-[10px] text-[var(--text-muted)] tabular-nums">
                {summary?.[count] ?? 0}
              </span>
            </button>
          ))}
        </div>
        <div
          data-testid="ext-hub-actions"
          className="flex shrink-0 items-center gap-1.5"
        >
          {actions.secondary && SecondaryIcon && (
            <button
              type="button"
              className={EXT_BUTTON}
              data-testid={actions.secondary.testId}
              disabled={actions.secondary.disabled}
              onClick={actions.secondary.onClick}
            >
              <SecondaryIcon
                className={`h-3.5 w-3.5 ${actions.secondary.spinning ? "animate-spin" : ""}`}
              />
              <span>{actions.secondary.label}</span>
            </button>
          )}
          {actions.more && (
            <ExtMoreDropdown
              items={actions.more}
              disabled={actions.disabled}
            />
          )}
          {actions.primary && PrimaryIcon && (
            <button
              type="button"
              className={EXT_PRIMARY_BUTTON}
              data-testid={actions.primary.testId}
              disabled={actions.primary.disabled}
              onClick={actions.primary.onClick}
            >
              <PrimaryIcon className="h-3.5 w-3.5" />
              <span>{actions.primary.label}</span>
            </button>
          )}
        </div>
      </div>
      {activeTab === "mcp" && (
        <McpHubView
          onActionsChange={setActions}
          onCountChange={countMcp}
          onGoSuperMcp={onGoSuperMcp}
          onNotify={onNotify}
        />
      )}
      {activeTab === "skills" && (
        <SkillsHubView
          onActionsChange={setActions}
          onCountChange={countSkills}
          onNotify={onNotify}
        />
      )}
      {activeTab === "plugins" && (
        <PluginsHubView
          onActionsChange={setActions}
          onCountChange={countPlugins}
        />
      )}
      {activeTab === "prompts" && (
        <PromptsHubView
          onActionsChange={setActions}
          onCountChange={countPrompts}
          onNotify={onNotify}
          onNavigationGuardChange={onNavigationGuardChange}
        />
      )}
    </section>
  );
}
