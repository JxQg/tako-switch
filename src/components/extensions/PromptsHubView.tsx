import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import {
  Check,
  ChevronDown,
  Download,
  FileText,
  LockKeyhole,
  Plus,
  RefreshCw,
  Save,
  Send,
  Trash2,
  Upload,
} from "lucide-react";
import {
  api,
  type ExtPlatform,
  type PromptPreset,
  type PromptsHubSnapshot,
} from "../../lib/api";
import { ToggleSwitch } from "../UiPrimitives";
import {
  EXT_BUTTON,
  EXT_ICON_BUTTON,
  EXT_INPUT,
  EXT_PLATFORMS,
  EXT_PRIMARY_BUTTON,
  ExtError,
  ExtHelpPopover,
  ExtSlideDrawer,
  useExtensionBackup,
  type ExtensionActions,
  type ExtensionNavigationGuard,
} from "./ExtensionPrimitives";

interface PromptsHubViewProps {
  onActionsChange: (actions: ExtensionActions) => void;
  onCountChange: (count: number) => void;
  onNotify?: (message: string) => void;
  onNavigationGuardChange: (guard: ExtensionNavigationGuard | null) => void;
}

function formatSize(bytes: number): string {
  return bytes >= 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${bytes} B`;
}

export function PromptsHubView({
  onActionsChange,
  onCountChange,
  onNotify,
  onNavigationGuardChange,
}: PromptsHubViewProps) {
  const [snapshot, setSnapshot] = useState<PromptsHubSnapshot | null>(null);
  const [platform, setPlatform] = useState<ExtPlatform>("claude");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [draft, setDraft] = useState<PromptPreset | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [newOpen, setNewOpen] = useState(false);
  const [newName, setNewName] = useState("");
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [pendingNavigation, setPendingNavigation] = useState<
    (() => void) | null
  >(null);
  const [syncOpen, setSyncOpen] = useState(false);
  const [syncTargets, setSyncTargets] = useState<ExtPlatform[]>([]);
  const [syncActivate, setSyncActivate] = useState(false);
  const syncRef = useRef<HTMLDivElement>(null);

  const state = snapshot?.platforms[platform];
  const selected = state?.presets.find((preset) => preset.id === selectedId);
  const dirty = Boolean(
    draft &&
    selected &&
    (draft.name !== selected.name || draft.content !== selected.content),
  );

  const receive = useCallback(
    (next: PromptsHubSnapshot) => {
      setSnapshot(next);
      onCountChange(
        EXT_PLATFORMS.reduce(
          (count, item) => count + next.platforms[item.id].presets.length,
          0,
        ),
      );
    },
    [onCountChange],
  );
  const run = useCallback(
    async (operation: () => Promise<PromptsHubSnapshot>, note?: string) => {
      setBusy(true);
      setError(null);
      try {
        receive(await operation());
        if (note) onNotify?.(note);
        return true;
      } catch (err) {
        setError(String(err));
        return false;
      } finally {
        setBusy(false);
      }
    },
    [onNotify, receive],
  );

  const reload = useCallback(() => run(api.getPromptsHubSnapshot), [run]);
  useEffect(() => {
    void reload();
  }, [reload]);
  useLayoutEffect(() => {
    if (!state) return;
    if (selectedId && state.presets.some((preset) => preset.id === selectedId))
      return;
    setSelectedId(state.activeId ?? state.presets[0]?.id ?? null);
  }, [state, selectedId]);
  useLayoutEffect(() => {
    setDraft(selected ? { ...selected } : null);
  }, [selected]);

  useEffect(() => {
    if (!syncOpen) return;
    const dismiss = (event: PointerEvent) => {
      if (
        event.target instanceof Node &&
        !syncRef.current?.contains(event.target)
      )
        setSyncOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setSyncOpen(false);
      }
    };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [syncOpen]);

  const navigate = useCallback(
    (action: () => void) => {
      if (dirty) setPendingNavigation(() => action);
      else action();
    },
    [dirty],
  );
  useLayoutEffect(() => {
    onNavigationGuardChange(navigate);
    return () => onNavigationGuardChange(null);
  }, [navigate, onNavigationGuardChange]);
  const importBackup = useCallback(
    async (backup: string) => {
      receive(await api.importPromptsBackup(backup));
      onNotify?.("提示词备份已恢复");
    },
    [receive, onNotify],
  );
  const backup = useExtensionBackup(api.exportPromptsBackup, importBackup);
  useEffect(() => {
    onActionsChange({
      disabled: busy,
      primary: {
        label: "新建预设",
        icon: Plus,
        testId: "ext-prompts-new-btn",
        disabled: busy,
        onClick: () =>
          navigate(() => {
            setNewName("");
            setNewOpen(true);
          }),
      },
      more: [
        {
          label: "重新读取磁盘",
          icon: RefreshCw,
          onClick: () =>
            navigate(() => {
              void reload();
            }),
        },
        {
          label: "导出预设 JSON",
          icon: Download,
          onClick: () => {
            void backup.begin("export");
          },
        },
        {
          label: "导入预设 JSON",
          icon: Upload,
          onClick: () =>
            navigate(() => {
              void backup.begin("import");
            }),
        },
      ],
    });
  }, [onActionsChange, busy, navigate, reload, backup.begin]);

  const save = async () => {
    if (!draft) return;
    await run(
      () => api.upsertPromptPreset(platform, draft),
      "提示词修改已保存",
    );
  };
  const closeNew = useCallback(() => setNewOpen(false), []);
  const closeDelete = useCallback(() => setDeleteOpen(false), []);
  const closeNavigation = useCallback(() => setPendingNavigation(null), []);
  const create = async () => {
    const preset = {
      id: `prompt-${crypto.randomUUID()}`,
      name: newName.trim(),
      content: "",
    };
    if (
      await run(() => api.upsertPromptPreset(platform, preset), "预设已创建")
    ) {
      setSelectedId(preset.id);
      closeNew();
    }
  };
  const sync = async () => {
    if (!draft) return;
    if (dirty && !(await run(() => api.upsertPromptPreset(platform, draft))))
      return;
    if (
      await run(
        () =>
          api.syncPromptToPlatforms(
            platform,
            draft.id,
            syncTargets,
            syncActivate,
          ),
        "预设已同步到所选平台",
      )
    )
      setSyncOpen(false);
  };

  return (
    <div
      role="tabpanel"
      id="ext-panel-prompts"
      aria-labelledby="ext-tab-prompts"
      className="flex min-w-0 flex-col gap-3"
    >
      <ExtError message={error} />
      <div className="flex min-w-0 flex-col gap-2 border-b border-[var(--border)] pb-3">
        <div className="flex min-w-0 items-center justify-between gap-3">
          <div
            className="inline-flex shrink-0 rounded-lg border border-[var(--border)] bg-[var(--bg-pill)] p-0.5"
            role="tablist"
            aria-label="提示词平台"
          >
            {EXT_PLATFORMS.map((item) => (
              <button
                type="button"
                role="tab"
                aria-selected={platform === item.id}
                key={item.id}
                data-testid={`ext-prompts-platform-${item.id}`}
                disabled={busy}
                onClick={() =>
                  navigate(() => {
                    setSelectedId(null);
                    setPlatform(item.id);
                    setSyncTargets([]);
                    setSyncOpen(false);
                  })
                }
                className={`ext-button whitespace-nowrap rounded-md px-2.5 text-[11.5px] cursor-pointer disabled:opacity-50 ${platform === item.id ? "bg-[var(--bg-card)] font-semibold text-[var(--text-primary)] shadow-[var(--shadow-segment)]" : "text-[var(--text-secondary)]"}`}
              >
                {item.label}
              </button>
            ))}
          </div>
          <div className="flex min-w-0 items-center justify-end gap-2 text-[10.5px] text-[var(--text-muted)]">
            <span className="min-w-0 truncate font-mono" title={state?.path}>
              {state?.path}
            </span>
            <span className="shrink-0 whitespace-nowrap tabular-nums">
              {formatSize(state?.liveBytes ?? 0)}
            </span>
            <span className="inline-flex shrink-0 items-center gap-1 whitespace-nowrap rounded bg-[var(--success-soft)] px-1.5 py-1 text-[var(--success)]">
              <LockKeyhole className="h-3 w-3" />
              汉化规则共存保护
            </span>
            <ExtHelpPopover label="提示词目标路径">
              <span className="break-all font-mono">{state?.path}</span>
            </ExtHelpPopover>
          </div>
        </div>
      </div>
      <div
        className="grid min-w-0 items-stretch gap-3"
        style={{
          gridTemplateColumns: "minmax(210px, 0.92fr) minmax(0, 1.35fr)",
        }}
      >
        <div className="flex min-w-0 flex-col gap-2">
          <div className="flex min-w-0 items-center justify-between gap-2 px-0.5 text-[11px] text-[var(--text-muted)]">
            <span>预设</span>
            <span className="tabular-nums">
              {state?.presets.length ?? 0} 项
            </span>
          </div>
          {state?.presets.map((preset) => (
            <div
              key={preset.id}
              className={`min-w-0 rounded-lg border p-3 ${selectedId === preset.id ? "border-[var(--accent-border)] bg-[var(--accent-soft)]" : "border-[var(--border)] bg-[var(--bg-card)]"}`}
            >
              <div className="flex min-w-0 items-center justify-between gap-2">
                <button
                  type="button"
                  className="min-w-0 flex-1 truncate text-left text-[12px] font-semibold text-[var(--text-primary)] cursor-pointer"
                  title={preset.name}
                  disabled={busy}
                  onClick={() => navigate(() => setSelectedId(preset.id))}
                >
                  {preset.name}
                </button>
                <ToggleSwitch
                  checked={state.activeId === preset.id}
                  disabled={busy}
                  ariaLabel={`${preset.name} 启用状态`}
                  data-testid={`ext-prompts-toggle-${preset.id}`}
                  onChange={(enabled) =>
                    navigate(() => {
                      void run(
                        () =>
                          api.activatePromptPreset(
                            platform,
                            enabled ? preset.id : null,
                          ),
                        enabled ? "预设已启用" : "预设已停用",
                      );
                    })
                  }
                />
              </div>
              <button
                type="button"
                disabled={busy}
                onClick={() => navigate(() => setSelectedId(preset.id))}
                className="mt-2 block w-full min-w-0 text-left cursor-pointer"
              >
                <div className="truncate text-[11px] text-[var(--text-muted)]">
                  {preset.content.split("\n").find((line) => line.trim()) ||
                    "空白预设"}
                </div>
                <div className="mt-2 flex items-center justify-between gap-2 text-[10px] text-[var(--text-muted)]">
                  <span>
                    {state.activeId === preset.id ? "当前启用" : "已保存"}
                  </span>
                  <span className="tabular-nums">
                    {formatSize(
                      new TextEncoder().encode(preset.content).length,
                    )}
                  </span>
                </div>
              </button>
            </div>
          ))}
          {!state?.presets.length && (
            <div className="flex min-h-[150px] flex-col items-center justify-center gap-2 border-y border-[var(--border)] text-[12px] text-[var(--text-muted)]">
              <FileText className="h-5 w-5" />
              <span>
                {busy && !snapshot ? "正在读取预设..." : "暂无提示词预设"}
              </span>
            </div>
          )}
        </div>
        <div className="flex min-w-0 flex-col">
          <div className="relative flex min-w-0 flex-wrap items-center justify-end gap-1.5 pb-2">
            <div className="relative min-w-0" ref={syncRef}>
              <button
                type="button"
                data-testid="ext-prompts-sync-other-btn"
                disabled={!draft || busy}
                onClick={() => setSyncOpen((open) => !open)}
                aria-expanded={syncOpen}
                className={`${EXT_BUTTON} !px-2 !text-[11px]`}
              >
                同步到其他端 <ChevronDown className="h-3 w-3" />
              </button>
              {syncOpen && (
                <div className="absolute left-0 top-full z-30 mt-1 w-[220px] rounded-lg border border-[var(--border)] bg-[var(--bg-card)] p-3 shadow-[var(--shadow-modal)]">
                  <div className="flex flex-col gap-2.5">
                    {EXT_PLATFORMS.filter((item) => item.id !== platform).map(
                      (item) => (
                        <label
                          key={item.id}
                          className="flex items-center gap-2 text-[12px] text-[var(--text-primary)]"
                        >
                          <input
                            type="checkbox"
                            checked={syncTargets.includes(item.id)}
                            onChange={(event) =>
                              setSyncTargets((targets) =>
                                event.target.checked
                                  ? [...targets, item.id]
                                  : targets.filter(
                                      (target) => target !== item.id,
                                    ),
                              )
                            }
                            className="accent-[var(--accent)]"
                          />
                          {item.label}
                        </label>
                      ),
                    )}
                  </div>
                  <label className="mt-3 flex items-center gap-2 border-t border-[var(--border)] pt-3 text-[11px] text-[var(--text-secondary)]">
                    <input
                      type="checkbox"
                      checked={syncActivate}
                      onChange={(event) =>
                        setSyncActivate(event.target.checked)
                      }
                      className="accent-[var(--accent)]"
                    />
                    同步后立即启用
                  </label>
                  <button
                    type="button"
                    className={`${EXT_PRIMARY_BUTTON} mt-3 w-full`}
                    disabled={!syncTargets.length || busy}
                    onClick={() => void sync()}
                  >
                    <Send className="h-3 w-3" />
                    同步预设
                  </button>
                </div>
              )}
            </div>
            <button
              type="button"
              title="删除预设"
              aria-label="删除预设"
              disabled={!draft || busy}
              onClick={() => setDeleteOpen(true)}
              className={EXT_ICON_BUTTON}
            >
              <Trash2 className="h-3.5 w-3.5" />
            </button>
            <button
              type="button"
              data-testid="ext-prompts-save-btn"
              className={`${EXT_PRIMARY_BUTTON} !px-2 !text-[11px]`}
              disabled={!draft || !dirty || busy}
              onClick={() => void save()}
            >
              <Save className="h-3.5 w-3.5" />
              {busy ? "保存中..." : "保存修改"}
            </button>
          </div>
          <div className="flex min-h-[290px] min-w-0 flex-1 flex-col overflow-hidden rounded-lg border border-[var(--border)] bg-[var(--bg-card)]">
            {draft ? (
              <>
                <div className="flex min-w-0 items-center gap-2 border-b border-[var(--border)] px-3 py-2">
                  <input
                    aria-label="预设名称"
                    value={draft.name}
                    onChange={(event) =>
                      setDraft({ ...draft, name: event.target.value })
                    }
                    className="min-w-0 flex-1 truncate bg-transparent text-[12px] font-semibold text-[var(--text-primary)] outline-none"
                  />
                  {dirty && (
                    <span className="shrink-0 whitespace-nowrap text-[10px] text-[var(--warning)]">
                      未保存
                    </span>
                  )}
                </div>
                <textarea
                  data-testid="ext-prompts-editor"
                  aria-label="提示词 Markdown 正文"
                  spellCheck={false}
                  value={draft.content}
                  onChange={(event) =>
                    setDraft({ ...draft, content: event.target.value })
                  }
                  className="min-h-[240px] w-full min-w-0 flex-1 resize-y bg-transparent p-3 font-mono text-[12px] leading-relaxed text-[var(--text-primary)] outline-none"
                />
              </>
            ) : (
              <div className="flex flex-1 items-center justify-center px-3 text-[12px] text-[var(--text-muted)]">
                未选择预设
              </div>
            )}
            <div className="flex min-w-0 items-center gap-1.5 border-t border-[var(--border)] bg-[var(--bg-subtle)] px-3 py-2 text-[10.5px] text-[var(--text-muted)]">
              <LockKeyhole className="h-3 w-3 shrink-0" />
              <span className="min-w-0 leading-relaxed">
                {state?.hasChineseBlock
                  ? "汉化托管块已锁定，保存预设不会修改汉化规则。"
                  : "用户正文与汉化规则独立保存。"}
              </span>
            </div>
          </div>
        </div>
      </div>
      {newOpen && (
        <ExtSlideDrawer
          title="新建提示词预设"
          onClose={closeNew}
          footer={
            <button
              type="button"
              className={EXT_PRIMARY_BUTTON}
              disabled={busy || !newName.trim()}
              onClick={() => void create()}
            >
              <Plus className="h-3.5 w-3.5" />
              {busy ? "创建中..." : "创建预设"}
            </button>
          }
        >
          <div className="flex flex-col gap-3">
            <ExtError message={error} />
            <label className="ext-field">
              预设名称
              <input
                autoFocus
                aria-label="新预设名称"
                className={EXT_INPUT}
                value={newName}
                onChange={(event) => setNewName(event.target.value)}
              />
            </label>
            <div className="text-[12px] text-[var(--text-muted)]">
              {EXT_PLATFORMS.find((item) => item.id === platform)?.label}
            </div>
          </div>
        </ExtSlideDrawer>
      )}
      {deleteOpen && (
        <ExtSlideDrawer
          title="删除提示词预设"
          onClose={closeDelete}
          footer={
            <>
              <button
                type="button"
                className={EXT_BUTTON}
                onClick={closeDelete}
              >
                取消
              </button>
              <button
                type="button"
                className={EXT_PRIMARY_BUTTON}
                disabled={busy || !draft}
                onClick={() => {
                  if (draft)
                    void run(
                      () => api.deletePromptPreset(platform, draft.id),
                      "预设已删除",
                    ).then((ok) => {
                      if (ok) closeDelete();
                    });
                }}
              >
                <Trash2 className="h-3.5 w-3.5" />
                确认删除
              </button>
            </>
          }
        >
          <p className="break-words text-[12px] leading-relaxed text-[var(--text-secondary)]">
            确定删除「{draft?.name}
            」？若该预设正在启用，将清空用户正文，保留汉化托管块。
          </p>
          <ExtError message={error} />
        </ExtSlideDrawer>
      )}
      {pendingNavigation && (
        <ExtSlideDrawer
          title="尚有未保存的修改"
          onClose={closeNavigation}
          footer={
            <>
              <button
                type="button"
                className={EXT_BUTTON}
                onClick={closeNavigation}
              >
                继续编辑
              </button>
              <button
                type="button"
                className={EXT_PRIMARY_BUTTON}
                onClick={() => {
                  const next = pendingNavigation;
                  setPendingNavigation(null);
                  setDraft(selected ? { ...selected } : null);
                  next();
                }}
              >
                <Check className="h-3.5 w-3.5" />
                放弃修改并继续
              </button>
            </>
          }
        >
          <p className="text-[12px] text-[var(--text-secondary)]">
            当前预设的修改尚未保存。
          </p>
        </ExtSlideDrawer>
      )}
      {backup.drawer}
    </div>
  );
}
