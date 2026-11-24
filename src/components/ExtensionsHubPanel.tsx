import { useState, type ReactNode } from "react";
import { FileText, Puzzle, Server, Sparkles } from "lucide-react";
import { PluginsHubView } from "./extensions/PluginsHubView";

export function ExtensionsHubPanel({
  onOpenSync,
}: {
  onOpenSync: (section: "mcp-sync" | "skill-sync") => void;
}) {
  const [actions, setActions] = useState<ReactNode>(null);
  const [pluginCount, setPluginCount] = useState(0);
  return (
    <div
      data-testid="extensions-hub-panel"
      className="tako-page-sub min-w-0 flex flex-col"
    >
      <header className="flex min-w-0 flex-wrap items-center justify-between gap-2">
        <nav
          aria-label="扩展类型"
          className="flex min-w-0 items-center gap-0.5 rounded-lg border border-[var(--border)] bg-[var(--bg-pill)] p-0.5"
        >
          <button
            type="button"
            title="通用 MCP"
            aria-label="通用 MCP"
            onClick={() => onOpenSync("mcp-sync")}
            className="h-[var(--density-ctrl-h)] shrink-0 whitespace-nowrap rounded-md px-2 text-[12px] text-[var(--text-secondary)] inline-flex items-center gap-1.5 hover:bg-[var(--bg-hover)] cursor-pointer"
          >
            <Server className="h-3.5 w-3.5" />
            <span>MCP</span>
          </button>
          <button
            type="button"
            title="Skills 技能"
            aria-label="Skills 技能"
            onClick={() => onOpenSync("skill-sync")}
            className="h-[var(--density-ctrl-h)] shrink-0 whitespace-nowrap rounded-md px-2 text-[12px] text-[var(--text-secondary)] inline-flex items-center gap-1.5 hover:bg-[var(--bg-hover)] cursor-pointer"
          >
            <Sparkles className="h-3.5 w-3.5" />
            <span>Skills</span>
          </button>
          <button
            type="button"
            aria-current="page"
            className="h-[var(--density-ctrl-h)] shrink-0 whitespace-nowrap rounded-md bg-[var(--bg-card)] px-2 text-[12px] font-semibold text-[var(--accent)] shadow-[var(--shadow-segment)] inline-flex items-center gap-1.5"
          >
            <Puzzle className="h-3.5 w-3.5" />
            <span>插件</span>
            <span className="tabular-nums text-[10px]">{pluginCount}</span>
          </button>
          <button
            type="button"
            title="当前版本暂不支持提示词预设"
            aria-label="提示词预设"
            disabled
            className="h-[var(--density-ctrl-h)] shrink-0 whitespace-nowrap rounded-md px-2 text-[12px] text-[var(--text-muted)] opacity-45 inline-flex items-center gap-1.5 disabled:cursor-not-allowed"
          >
            <FileText className="h-3.5 w-3.5" />
            <span>提示词</span>
          </button>
        </nav>
        <div
          data-testid="ext-hub-actions"
          className="ml-auto flex shrink-0 items-center gap-1.5"
        >
          {actions}
        </div>
      </header>
      <PluginsHubView
        onActionsChange={setActions}
        onCountChange={setPluginCount}
      />
    </div>
  );
}
