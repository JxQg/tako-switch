import { useCallback, useEffect, useState } from "react";
import {
  AlertCircle,
  Check,
  ChevronDown,
  FolderGit2,
  Layers,
  Plus,
  RefreshCw,
  Server,
  Sparkles,
  Trash2,
} from "lucide-react";
import {
  api,
  type MultiPlatformSyncSnapshot,
  type PlatformMcpItem,
  type PlatformSkillItem,
  type SyncPlatform,
} from "../lib/api";
import { StatusPill } from "./UiPrimitives";

const PLATFORMS: { id: SyncPlatform; label: string }[] = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "antigravity", label: "Antigravity" },
  { id: "workbuddy", label: "WorkBuddy" },
];

interface MultiPlatformSyncPanelProps {
  mode: "mcp" | "skill";
  onNotify?: (msg: string) => void;
}

export function MultiPlatformSyncPanel({
  mode,
  onNotify,
}: MultiPlatformSyncPanelProps) {
  const [snapshot, setSnapshot] = useState<MultiPlatformSyncSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [batchSyncing, setBatchSyncing] = useState(false);
  const [pathsOpen, setPathsOpen] = useState(false);

  const loadSnapshot = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await api.getMultiPlatformSyncSnapshot();
      setSnapshot(data);
    } catch (err) {
      setError(`扫描多平台配置失败：${String(err)}`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadSnapshot();
  }, [loadSnapshot]);

  const pickBestMcpSource = (item: PlatformMcpItem): SyncPlatform | null =>
    item.platforms[0] ?? null;

  const pickBestSkillSource = (item: PlatformSkillItem): SyncPlatform | null => {
    if (item.platforms.length === 0) return null;
    let best = item.platforms[0];
    let bestMs = item.modifiedMs[best] ?? 0;
    for (const p of item.platforms) {
      const ms = item.modifiedMs[p] ?? 0;
      if (ms > bestMs) {
        best = p;
        bestMs = ms;
      }
    }
    return best;
  };

  const handleToggleMcpPlatform = async (
    item: PlatformMcpItem,
    target: SyncPlatform,
  ) => {
    if (item.managedByTako || busyKey !== null || batchSyncing) return;
    const hasIt = item.platforms.includes(target);
    const key = `mcp:${item.name}:${target}`;
    setBusyKey(key);
    setError(null);
    try {
      if (hasIt) {
        if (item.platforms.length <= 1) {
          setError(`「${item.name}」仅存在于当前平台，为防止误删请至少保留一份配置`);
          return;
        }
        const next = await api.removeSyncedItem("mcp", item.name, target);
        setSnapshot(next);
        onNotify?.(`已从 ${platformLabel(target)} 移除 MCP「${item.name}」`);
      } else {
        const source = pickBestMcpSource(item);
        if (!source) return;
        const next = await api.syncMcpToPlatforms(item.name, source, [target]);
        setSnapshot(next);
        onNotify?.(`已将 MCP「${item.name}」同步至 ${platformLabel(target)}`);
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const handleAlignOneMcp = async (item: PlatformMcpItem) => {
    if (item.managedByTako || busyKey !== null || batchSyncing) return;
    const source = pickBestMcpSource(item);
    if (!source) return;
    const targets = PLATFORMS.map((p) => p.id).filter((p) => p !== source);
    setBusyKey(`mcp-all:${item.name}`);
    setError(null);
    try {
      const next = await api.syncMcpToPlatforms(item.name, source, targets);
      setSnapshot(next);
      onNotify?.(`已将 MCP「${item.name}」对齐同步至三端`);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const handleToggleSkillPlatform = async (
    item: PlatformSkillItem,
    target: SyncPlatform,
  ) => {
    if (busyKey !== null || batchSyncing) return;
    const hasIt = item.platforms.includes(target);
    const key = `skill:${item.name}:${target}`;
    setBusyKey(key);
    setError(null);
    try {
      if (hasIt) {
        if (item.platforms.length <= 1) {
          setError(`Skill「${item.name}」仅存在于当前平台，为防止误删请至少保留一份`);
          return;
        }
        const next = await api.removeSyncedItem("skill", item.name, target);
        setSnapshot(next);
        onNotify?.(`已从 ${platformLabel(target)} 移除 Skill「${item.name}」`);
      } else {
        const source = pickBestSkillSource(item);
        if (!source) return;
        const next = await api.syncSkillToPlatforms(item.name, source, [target]);
        setSnapshot(next);
        onNotify?.(`已将 Skill「${item.name}」同步至 ${platformLabel(target)}`);
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const handleAlignOneSkill = async (item: PlatformSkillItem) => {
    if (busyKey !== null || batchSyncing) return;
    const source = pickBestSkillSource(item);
    if (!source) return;
    const targets = PLATFORMS.map((p) => p.id).filter((p) => p !== source);
    setBusyKey(`skill-all:${item.name}`);
    setError(null);
    try {
      const next = await api.syncSkillToPlatforms(item.name, source, targets);
      setSnapshot(next);
      onNotify?.(`已将 Skill「${item.name}」以最新版本对齐至三端`);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusyKey(null);
    }
  };

  const handleSyncAll = async () => {
    if (batchSyncing || busyKey !== null) return;
    setBatchSyncing(true);
    setError(null);
    try {
      const allTargets: SyncPlatform[] = ["claude", "codex", "antigravity", "workbuddy"];
      if (mode === "mcp") {
        const res = await api.syncAllMcps(allTargets, null);
        setSnapshot(res.snapshot);
        onNotify?.(
          res.syncedWrites > 0
            ? `已同步 ${res.syncedItems} 个 MCP（共写入 ${res.syncedWrites} 处配置）`
            : "各端自定义 MCP 已全部对齐，无需变更",
        );
      } else {
        const res = await api.syncAllSkills(allTargets, null);
        setSnapshot(res.snapshot);
        onNotify?.(
          res.syncedWrites > 0
            ? `已同步 ${res.syncedItems} 个 Skill（共写入 ${res.syncedWrites} 处目录）`
            : "各端 Skills 已全部对齐，无需变更",
        );
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setBatchSyncing(false);
    }
  };

  const mcpItems = snapshot?.mcpItems ?? [];
  const customMcpCount = mcpItems.filter((m) => !m.managedByTako).length;
  const skillItems = snapshot?.skillItems ?? [];

  return (
    <div className="flex flex-col tako-list-stack">
      {/* Summary & Batch Action Bar */}
      <div className="tako-card-surface border border-[var(--border)] bg-[var(--bg-card)] tako-panel-head flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            {mode === "mcp" ? (
              <Server className="h-4 w-4 text-[var(--accent)] shrink-0" />
            ) : (
              <FolderGit2 className="h-4 w-4 text-[var(--accent)] shrink-0" />
            )}
            <h3 className="text-[13.5px] font-semibold text-[var(--text-primary)]">
              {mode === "mcp" ? "多平台 MCP 配置同步" : "多平台 Skills 目录同步"}
            </h3>
          </div>
          <p className="text-[11.5px] text-[var(--text-muted)] mt-1 leading-snug">
            {mode === "mcp"
              ? `自动在 Claude JSON、Codex TOML 与 Antigravity JSON 间无损转换，共检测到 ${customMcpCount} 个自定义 MCP。`
              : `自动识别各平台 SKILL.md 与附属脚本目录，共检测到 ${skillItems.length} 个 Skills。点击平台徽章即可单点同步。`}
          </p>
        </div>

        <div className="flex items-center gap-2 shrink-0">
          <button
            type="button"
            disabled={loading || batchSyncing}
            onClick={() => void loadSnapshot()}
            className="h-8 px-2.5 rounded-[9px] border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-subtle)] text-[12px] font-medium text-[var(--text-secondary)] inline-flex items-center gap-1.5 transition cursor-pointer disabled:opacity-50"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${loading ? "animate-spin" : ""}`} />
            <span>重新扫描</span>
          </button>
          <button
            type="button"
            disabled={
              loading ||
              batchSyncing ||
              busyKey !== null ||
              (mode === "mcp" ? customMcpCount === 0 : skillItems.length === 0)
            }
            onClick={() => void handleSyncAll()}
            className="h-8 px-3 rounded-[9px] bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-white text-[12px] font-semibold inline-flex items-center gap-1.5 transition cursor-pointer disabled:opacity-45"
          >
            <Sparkles className="h-3.5 w-3.5" />
            <span>{batchSyncing ? "同步中..." : "一键三端对齐"}</span>
          </button>
        </div>
      </div>

      {error && (
        <div
          className="rounded-[10px] border px-3.5 py-2 text-[12px] flex items-center gap-2"
          style={{
            backgroundColor: "var(--danger-soft)",
            borderColor: "rgba(201, 76, 76, 0.25)",
            color: "var(--danger)",
          }}
        >
          <AlertCircle className="h-4 w-4 shrink-0" />
          <span className="break-words">{error}</span>
        </div>
      )}

      {loading ? (
        <div className="rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] p-8 text-center text-[12.5px] text-[var(--text-muted)]">
          正在扫描 Claude / Codex / Antigravity 本机配置...
        </div>
      ) : mode === "mcp" ? (
        mcpItems.length === 0 ? (
          <div className="rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] p-8 text-center">
            <Layers className="h-6 w-6 text-[var(--text-muted)] mx-auto mb-2" />
            <div className="text-[13px] font-medium text-[var(--text-primary)]">
              暂未发现自定义 MCP 配置
            </div>
            <p className="text-[11.5px] text-[var(--text-muted)] mt-1">
              在任一客户端中添加 MCP Server 后，即可在此一键同步到其余平台。
            </p>
          </div>
        ) : (
          <div className="flex flex-col tako-list-stack">
            {mcpItems.map((item) => {
              const firstSpec = item.platforms[0]
                ? item.variants[item.platforms[0]]
                : undefined;
              const allSynced = item.platforms.length === PLATFORMS.length && !item.hasDiff;
              return (
                <div
                  key={item.name}
                  className="tako-card-surface border border-[var(--border)] bg-[var(--bg-card)] tako-client-row flex flex-wrap items-center justify-between gap-3"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="text-[13px] font-semibold text-[var(--text-primary)] font-mono">
                        {item.name}
                      </span>
                      {firstSpec && (
                        <StatusPill tone="neutral">{firstSpec.transport}</StatusPill>
                      )}
                      {item.managedByTako && (
                        <StatusPill tone="ok">Tako 内置托管</StatusPill>
                      )}
                      {item.hasDiff && !item.managedByTako && (
                        <StatusPill tone="warn">配置有差异</StatusPill>
                      )}
                    </div>
                    {firstSpec?.summary && (
                      <div className="text-[11.5px] text-[var(--text-muted)] font-mono truncate mt-1">
                        {firstSpec.summary}
                      </div>
                    )}
                  </div>

                  <div className="flex items-center gap-1.5 flex-wrap shrink-0">
                    {PLATFORMS.map((p) => {
                      const active = item.platforms.includes(p.id);
                      const isItemBusy = busyKey === `mcp:${item.name}:${p.id}`;
                      return (
                        <button
                          key={p.id}
                          type="button"
                          disabled={item.managedByTako || busyKey !== null || batchSyncing}
                          onClick={() => void handleToggleMcpPlatform(item, p.id)}
                          title={
                            item.managedByTako
                              ? "Tako 内置 MCP 请在「Tako 内置工具」中统一开关"
                              : active
                                ? `已在 ${p.label} 中配置 · 点击可从此平台移除`
                                : `点击同步至 ${p.label}`
                          }
                          className={`h-7 px-2.5 rounded-[8px] border text-[11.5px] font-medium inline-flex items-center gap-1 transition cursor-pointer disabled:opacity-55 disabled:cursor-default ${
                            active
                              ? "bg-[var(--success-soft)] border-[var(--success-border)] text-[var(--success)] hover:opacity-85"
                              : "bg-[var(--bg-subtle)]/70 border-dashed border-[var(--border-strong)] text-[var(--text-secondary)] hover:border-[var(--accent)] hover:text-[var(--accent)]"
                          }`}
                        >
                          {active ? (
                            <Check className="h-3 w-3 shrink-0" />
                          ) : (
                            <Plus className="h-3 w-3 shrink-0" />
                          )}
                          <span>{isItemBusy ? "..." : p.label}</span>
                        </button>
                      );
                    })}

                    {!item.managedByTako && !allSynced && (
                      <button
                        type="button"
                        disabled={busyKey !== null || batchSyncing}
                        onClick={() => void handleAlignOneMcp(item)}
                        className="h-7 px-2.5 rounded-[8px] border border-[var(--accent-border)] bg-[var(--accent-soft)] hover:bg-[var(--accent-border)] text-[var(--accent)] text-[11px] font-semibold transition cursor-pointer disabled:opacity-50"
                      >
                        {busyKey === `mcp-all:${item.name}` ? "对齐中..." : "三端对齐"}
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )
      ) : skillItems.length === 0 ? (
        <div className="rounded-[13px] border border-[var(--border)] bg-[var(--bg-card)] p-8 text-center">
          <FolderGit2 className="h-6 w-6 text-[var(--text-muted)] mx-auto mb-2" />
          <div className="text-[13px] font-medium text-[var(--text-primary)]">
            暂未检测到本机 Skills
          </div>
          <p className="text-[11.5px] text-[var(--text-muted)] mt-1">
            在 ~/.claude/skills、~/.codex/skills 或 ~/.gemini/config/skills 下包含 SKILL.md 的目录会自动出现在此。
          </p>
        </div>
      ) : (
        <div className="flex flex-col tako-list-stack">
          {skillItems.map((item) => {
            const allSynced = item.platforms.length === PLATFORMS.length && !item.hasDiff;
            const maxFiles = Math.max(0, ...Object.values(item.fileCounts).map((n) => n ?? 0));
            return (
              <div
                key={item.name}
                className="tako-card-surface border border-[var(--border)] bg-[var(--bg-card)] tako-client-row flex flex-wrap items-center justify-between gap-3"
              >
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-[13px] font-semibold text-[var(--text-primary)]">
                      {item.title || item.name}
                    </span>
                    {item.title !== item.name && (
                      <span className="text-[11px] font-mono text-[var(--text-muted)]">
                        {item.name}
                      </span>
                    )}
                    <StatusPill tone="neutral">{maxFiles} 个文件</StatusPill>
                    {item.hasDiff && <StatusPill tone="warn">版本有差异</StatusPill>}
                  </div>
                  {item.description && (
                    <p className="text-[11.5px] text-[var(--text-muted)] truncate mt-1">
                      {item.description}
                    </p>
                  )}
                </div>

                <div className="flex items-center gap-1.5 flex-wrap shrink-0">
                  {PLATFORMS.map((p) => {
                    const active = item.platforms.includes(p.id);
                    const isItemBusy = busyKey === `skill:${item.name}:${p.id}`;
                    return (
                      <button
                        key={p.id}
                        type="button"
                        disabled={busyKey !== null || batchSyncing}
                        onClick={() => void handleToggleSkillPlatform(item, p.id)}
                        title={
                          active
                            ? `已存在于 ${p.label} · 点击可从此平台移除副本`
                            : `点击将最新 Skill 同步至 ${p.label}`
                        }
                        className={`h-7 px-2.5 rounded-[8px] border text-[11.5px] font-medium inline-flex items-center gap-1 transition cursor-pointer disabled:opacity-55 ${
                          active
                            ? "bg-[var(--success-soft)] border-[var(--success-border)] text-[var(--success)] hover:opacity-85"
                            : "bg-[var(--bg-subtle)]/70 border-dashed border-[var(--border-strong)] text-[var(--text-secondary)] hover:border-[var(--accent)] hover:text-[var(--accent)]"
                        }`}
                      >
                        {active ? (
                          <Check className="h-3 w-3 shrink-0" />
                        ) : (
                          <Plus className="h-3 w-3 shrink-0" />
                        )}
                        <span>{isItemBusy ? "..." : p.label}</span>
                      </button>
                    );
                  })}

                  {!allSynced && (
                    <button
                      type="button"
                      disabled={busyKey !== null || batchSyncing}
                      onClick={() => void handleAlignOneSkill(item)}
                      className="h-7 px-2.5 rounded-[8px] border border-[var(--accent-border)] bg-[var(--accent-soft)] hover:bg-[var(--accent-border)] text-[var(--accent)] text-[11px] font-semibold transition cursor-pointer disabled:opacity-50"
                    >
                      {busyKey === `skill-all:${item.name}` ? "对齐中..." : "三端对齐"}
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* Collapsible Platform Paths Info */}
      {snapshot && (
        <div className="rounded-[11px] border border-[var(--border)] bg-[var(--bg-card)]/70 px-3.5 py-2.5">
          <button
            type="button"
            onClick={() => setPathsOpen((v) => !v)}
            className="w-full flex items-center justify-between text-[11.5px] font-medium text-[var(--text-secondary)] cursor-pointer"
          >
            <span className="inline-flex items-center gap-1.5">
              <Trash2 className="h-3 w-3 opacity-0 -ml-4" />
              <span>查看三端本机配置与 Skills 目录路径</span>
            </span>
            <ChevronDown
              className={`h-3.5 w-3.5 transition-transform ${pathsOpen ? "rotate-180" : ""}`}
            />
          </button>
          {pathsOpen && (
            <div className="mt-2.5 pt-2 border-t border-[var(--border)] flex flex-col gap-1.5 text-[11px] font-mono text-[var(--text-muted)]">
              {PLATFORMS.map((p) => {
                const info = snapshot.platformPaths[p.id];
                if (!info) return null;
                return (
                  <div key={p.id} className="flex flex-col gap-0.5">
                    <span className="font-sans font-semibold text-[var(--text-secondary)]">
                      {p.label}
                    </span>
                    <span className="break-all">
                      {mode === "mcp" ? info.mcpPath : info.skillsDir}
                    </span>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function platformLabel(p: SyncPlatform): string {
  if (p === "claude") return "Claude";
  if (p === "codex") return "Codex";
  if (p === "workbuddy") return "WorkBuddy";
  return "Antigravity";
}
