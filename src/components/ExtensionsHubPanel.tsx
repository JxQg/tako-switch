import { Blocks, FileText, FolderGit2, Server } from "lucide-react";
import { SkillsHubView } from "./extensions/SkillsHubView";

export function ExtensionsHubPanel({
  onOpenMcp,
  onNotify,
}: {
  onOpenMcp: () => void;
  onNotify?: (message: string) => void;
}) {
  return (
    <div className="tako-page-sub flex min-w-0 flex-col">
      <SkillsHubView
        onNotify={onNotify}
        renderHeader={(actions, count) => (
          <div className="flex min-w-0 flex-wrap items-center justify-between gap-x-3 gap-y-2 border-b border-[var(--border)] pb-3">
            <nav
              aria-label="扩展分类"
              className="flex min-w-0 items-center gap-1"
            >
              {[
                { id: "mcp", label: "MCP", Icon: Server, onClick: onOpenMcp },
                {
                  id: "skills",
                  label: "Skills",
                  Icon: FolderGit2,
                  onClick: undefined,
                },
                {
                  id: "plugins",
                  label: "插件",
                  Icon: Blocks,
                  onClick: undefined,
                },
                {
                  id: "prompts",
                  label: "提示词",
                  Icon: FileText,
                  onClick: undefined,
                },
              ].map(({ id, label, Icon, onClick }) => (
                <button
                  key={id}
                  type="button"
                  aria-current={id === "skills" ? "page" : undefined}
                  disabled={id === "plugins" || id === "prompts"}
                  onClick={onClick}
                  title={
                    id === "plugins" || id === "prompts"
                      ? "由对应扩展模块提供"
                      : label
                  }
                  className={`inline-flex h-[var(--density-ctrl-h)] shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-[7px] px-2 text-[12px] disabled:cursor-default disabled:opacity-40 ${id === "skills" ? "bg-[var(--accent-soft)] font-semibold text-[var(--accent)]" : "text-[var(--text-secondary)] hover:bg-[var(--bg-hover)]"}`}
                >
                  <Icon className="h-3.5 w-3.5 shrink-0" />
                  <span>{label}</span>
                  {id === "skills" && (
                    <span className="tabular-nums text-[10px]">{count}</span>
                  )}
                </button>
              ))}
            </nav>
            <div className="ml-auto flex shrink-0 items-center gap-2">
              {actions}
            </div>
          </div>
        )}
      />
    </div>
  );
}
