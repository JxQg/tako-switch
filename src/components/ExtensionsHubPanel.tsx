import { useCallback, useState } from "react";
import { FileText, Plug, Puzzle, Zap, type LucideIcon } from "lucide-react";
import type { ExtensionsSummary } from "../lib/api";
import type { ExtensionTab } from "./UiPrimitives";
import {
  EXT_PRIMARY_BUTTON,
  ExtMoreDropdown,
  type ExtensionActions,
  type ExtensionNavigationGuard,
} from "./extensions/ExtensionPrimitives";
import { McpHubView } from "./extensions/McpHubView";
import { PromptsHubView } from "./extensions/PromptsHubView";

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
  onCountChange: (tab: ExtensionTab, count: number) => void;
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
  const countPrompts = useCallback(
    (count: number) => onCountChange("prompts", count),
    [onCountChange],
  );
  const PrimaryIcon = actions.primary?.icon;
  const placeholder = activeTab === "skills" || activeTab === "plugins";
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
        {!placeholder && (
          <div className="flex shrink-0 items-center gap-1.5">
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
                {actions.primary.label}
              </button>
            )}
          </div>
        )}
      </div>
      {activeTab === "mcp" && (
        <McpHubView
          onActionsChange={setActions}
          onCountChange={countMcp}
          onGoSuperMcp={onGoSuperMcp}
          onNotify={onNotify}
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
      {placeholder && (
        <div
          role="tabpanel"
          id={`ext-panel-${activeTab}`}
          aria-labelledby={`ext-tab-${activeTab}`}
          className="flex min-h-[240px] min-w-0 flex-col items-center justify-center gap-3 border-y border-[var(--border)] text-[var(--text-muted)]"
        >
          {activeTab === "skills" ? (
            <Zap className="h-7 w-7" strokeWidth={1.4} />
          ) : (
            <Puzzle className="h-7 w-7" strokeWidth={1.4} />
          )}
          <h2 className="text-[14px] font-semibold text-[var(--text-primary)]">
            {activeTab === "skills" ? "Skills 技能" : "插件管理"}
          </h2>
          <span className="text-[12px]">暂未开放</span>
        </div>
      )}
    </section>
  );
}
