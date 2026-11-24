import {
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useState,
} from "react";
import {
  ArrowRight,
  ArrowRightLeft,
  Check,
  Download,
  Pencil,
  Plug,
  Plus,
  RefreshCw,
  Save,
  Trash2,
  Upload,
} from "lucide-react";
import {
  api,
  type ExtPlatform,
  type McpEntry,
  type McpHubSnapshot,
  type McpSpec,
} from "../../lib/api";
import { SegmentedControl, ToggleSwitch } from "../UiPrimitives";
import {
  EXT_BUTTON,
  EXT_ICON_BUTTON,
  EXT_INPUT,
  EXT_PLATFORMS,
  EXT_PRIMARY_BUTTON,
  ExtError,
  ExtHelpPopover,
  ExtMatrixHeader,
  ExtSearchInput,
  ExtSlideDrawer,
  useExtensionBackup,
  type ExtensionActions,
} from "./ExtensionPrimitives";

interface McpHubViewProps {
  onActionsChange: (actions: ExtensionActions) => void;
  onCountChange: (count: number) => void;
  onGoSuperMcp: () => void;
  onNotify?: (message: string) => void;
}

const EMPTY_SPEC: McpSpec = {
  transport: "stdio",
  command: "",
  args: [],
  env: {},
  url: null,
  headers: {},
  extra: {},
};
const TEMPLATES = [
  {
    id: "context7",
    name: "Context7",
    command: "npx",
    args: ["-y", "@upstash/context7-mcp"],
  },
  {
    id: "playwright",
    name: "Playwright",
    command: "npx",
    args: ["-y", "@playwright/mcp@latest"],
  },
  {
    id: "fetch",
    name: "Fetch (uvx)",
    command: "uvx",
    args: ["mcp-server-fetch"],
  },
  {
    id: "sequential-thinking",
    name: "Sequential Thinking",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-sequential-thinking"],
  },
];

function newEntry(): McpEntry {
  return {
    id: "",
    name: "",
    spec: { ...EMPTY_SPEC },
    platforms: [],
    variants: {},
    hasDiff: false,
  };
}

function parseStringMap(raw: string, label: string): Record<string, string> {
  const value: unknown = JSON.parse(raw || "{}");
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    Object.values(value).some((item) => typeof item !== "string")
  ) {
    throw new Error(`${label}必须是字符串键值对象`);
  }
  return value as Record<string, string>;
}

function summary(spec: McpSpec): string {
  return spec.transport === "http"
    ? (spec.url ?? "")
    : [spec.command, ...spec.args].filter(Boolean).join(" ");
}

export function McpHubView({
  onActionsChange,
  onCountChange,
  onGoSuperMcp,
  onNotify,
}: McpHubViewProps) {
  const [snapshot, setSnapshot] = useState<McpHubSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<"all" | "diff">("all");
  const deferredQuery = useDeferredValue(query.trim().toLowerCase());
  const [drawer, setDrawer] = useState<McpEntry | null>(null);
  const [deleteEntry, setDeleteEntry] = useState<McpEntry | null>(null);

  const receive = useCallback(
    (next: McpHubSnapshot) => {
      setSnapshot(next);
      onCountChange(next.entries.length);
    },
    [onCountChange],
  );
  const run = useCallback(
    async (operation: () => Promise<McpHubSnapshot>, note?: string) => {
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
  const reload = useCallback(() => run(api.getMcpHubSnapshot), [run]);
  useEffect(() => {
    void reload();
  }, [reload]);

  const importBackup = useCallback(
    async (backup: string) => {
      receive(await api.importMcpBackup(backup));
      onNotify?.("MCP 备份已恢复");
    },
    [onNotify, receive],
  );
  const backup = useExtensionBackup(api.exportMcpBackup, importBackup);
  useEffect(() => {
    onActionsChange({
      disabled: busy,
      primary: {
        label: "添加 MCP",
        icon: Plus,
        testId: "ext-mcp-add-btn",
        onClick: () => setDrawer(newEntry()),
        disabled: busy,
      },
      more: [
        {
          label: "重新扫描本机配置",
          icon: RefreshCw,
          onClick: () => {
            void reload();
          },
        },
        {
          label: "对齐全部差异",
          icon: ArrowRightLeft,
          disabled: !snapshot?.entries.some((entry) => entry.hasDiff),
          onClick: () => {
            void run(api.alignAllMcpEntries, "全部差异已对齐");
          },
        },
        {
          label: "导出 JSON 备份",
          icon: Download,
          onClick: () => {
            void backup.begin("export");
          },
        },
        {
          label: "导入 JSON 备份",
          icon: Upload,
          onClick: () => {
            void backup.begin("import");
          },
        },
      ],
    });
  }, [onActionsChange, busy, reload, run, snapshot, backup.begin]);

  const entries = useMemo(
    () =>
      (snapshot?.entries ?? []).filter(
        (entry) =>
          (filter !== "diff" || entry.hasDiff) &&
          (!deferredQuery ||
            `${entry.name} ${entry.id} ${summary(entry.spec)}`
              .toLowerCase()
              .includes(deferredQuery)),
      ),
    [snapshot, deferredQuery, filter],
  );
  const closeEditor = useCallback(() => setDrawer(null), []);
  const closeDelete = useCallback(() => setDeleteEntry(null), []);

  return (
    <div
      className="flex min-w-0 flex-col gap-3"
      role="tabpanel"
      id="ext-panel-mcp"
      aria-labelledby="ext-tab-mcp"
    >
      <div
        data-testid="ext-mcp-tako-notice"
        className="flex min-w-0 items-center justify-between gap-2 border-b border-[var(--border)] pb-3 text-[11px] text-[var(--text-muted)]"
      >
        <span
          className="min-w-0 truncate"
          title="Tako 超级 MCP（图像 / 视频 / 语音 / 搜索 / 会诊 / 网站上线）已独立托管，不在此列表显示"
        >
          Tako 超级 MCP（图像 / 视频 / 语音 / 搜索 / 会诊 /
          网站上线）已独立托管，不在此列表显示
        </span>
        <button
          type="button"
          data-testid="ext-mcp-goto-super"
          onClick={onGoSuperMcp}
          className="inline-flex shrink-0 items-center gap-0.5 whitespace-nowrap text-[var(--accent)] hover:underline cursor-pointer"
        >
          前往 Tako 超级 MCP <ArrowRight className="h-3 w-3" />
        </button>
      </div>
      <ExtError message={error} />
      <div className="flex min-w-0 items-center gap-2">
        <ExtSearchInput
          value={query}
          onChange={setQuery}
          placeholder="搜索 MCP 名称、命令或地址..."
          testId="ext-mcp-search"
        />
        <SegmentedControl
          value={filter}
          onChange={setFilter}
          options={[
            { id: "all", label: "全部" },
            { id: "diff", label: "有差异" },
          ]}
        />
        <span className="min-w-0 flex-1 truncate text-right text-[11px] text-[var(--text-muted)] tabular-nums">
          {entries.length} 项
        </span>
        <ExtHelpPopover>
          <div className="flex flex-col gap-2">
            {EXT_PLATFORMS.map((platform) => (
              <div key={platform.id}>
                <strong>{platform.label}</strong>
                {snapshot?.platformPaths[platform.id]?.map((path) => (
                  <div key={path} className="break-all font-mono text-[10.5px]">
                    {path}
                  </div>
                ))}
              </div>
            ))}
            <p>
              对齐以中心配置或选定平台为准，仅同步已开启的平台。全部关闭后仍保留中心记录。
            </p>
          </div>
        </ExtHelpPopover>
      </div>
      {Boolean(snapshot?.warnings.length) && (
        <div className="flex min-w-0 items-center justify-between gap-2 text-[11px] text-[var(--warning)]">
          <span className="truncate">
            {snapshot?.warnings.length} 项配置未能收编，原文件保持不变
          </span>
          <ExtHelpPopover label="查看扫描问题">
            {snapshot?.warnings.map((warning) => (
              <p className="mb-2 break-all" key={warning}>
                {warning}
              </p>
            ))}
          </ExtHelpPopover>
        </div>
      )}
      <div className="min-w-0 overflow-hidden rounded-lg border border-[var(--border)] bg-[var(--bg-card)]">
        <ExtMatrixHeader />
        {entries.map((entry) => (
          <div
            key={entry.id}
            data-testid={`ext-mcp-row-${entry.id}`}
            className="grid min-w-0 items-center gap-1 border-b border-[var(--border)] px-3 py-3 last:border-b-0"
            style={{
              gridTemplateColumns: "minmax(0, 1fr) 70px 64px 76px 86px",
            }}
          >
            <div className="min-w-0 pr-2">
              <div className="flex min-w-0 items-center gap-1.5">
                <span
                  className="min-w-0 truncate text-[12px] font-semibold text-[var(--text-primary)]"
                  title={entry.name}
                >
                  {entry.name}
                </span>
                {entry.hasDiff && (
                  <span className="shrink-0 whitespace-nowrap rounded bg-[var(--warning-soft)] px-1 text-[10px] text-[var(--warning)]">
                    有差异
                  </span>
                )}
              </div>
              <div
                className="mt-1 truncate font-mono text-[10.5px] text-[var(--text-muted)]"
                title={summary(entry.spec)}
              >
                {summary(entry.spec)}
              </div>
            </div>
            {EXT_PLATFORMS.map((platform) => (
              <div key={platform.id} className="flex justify-center">
                <ToggleSwitch
                  checked={entry.platforms.includes(platform.id)}
                  disabled={busy}
                  ariaLabel={`${entry.name} ${platform.label}`}
                  data-testid={`ext-mcp-toggle-${entry.id}-${platform.id}`}
                  onChange={(enabled) => {
                    void run(() =>
                      api.toggleMcpPlatform(entry.id, platform.id, enabled),
                    );
                  }}
                />
              </div>
            ))}
            <div className="flex shrink-0 items-center justify-end whitespace-nowrap">
              <button
                type="button"
                className={EXT_ICON_BUTTON}
                disabled={busy || !entry.hasDiff}
                onClick={() => {
                  void run(
                    () => api.alignMcpEntry(entry.id),
                    "已按中心配置对齐",
                  );
                }}
                title="按中心配置对齐已开启平台"
                aria-label={`对齐 ${entry.name}`}
              >
                <ArrowRightLeft className="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                className={EXT_ICON_BUTTON}
                disabled={busy}
                onClick={() => setDrawer(entry)}
                title="编辑 MCP"
                aria-label={`编辑 ${entry.name}`}
              >
                <Pencil className="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                className={EXT_ICON_BUTTON}
                disabled={busy}
                onClick={() => setDeleteEntry(entry)}
                title="删除 MCP"
                aria-label={`删除 ${entry.name}`}
              >
                <Trash2 className="h-3.5 w-3.5" />
              </button>
            </div>
          </div>
        ))}
        {entries.length === 0 && (
          <div className="flex min-h-[180px] flex-col items-center justify-center gap-2 px-4 text-[12px] text-[var(--text-muted)]">
            <Plug className="h-6 w-6" strokeWidth={1.5} />
            <span>
              {busy && !snapshot
                ? "正在扫描本机配置..."
                : query || filter === "diff"
                  ? "没有符合条件的 MCP"
                  : "暂无通用 MCP"}
            </span>
          </div>
        )}
      </div>
      {drawer && (
        <McpEditor
          key={drawer.id || "new"}
          entry={drawer}
          existing={Boolean(drawer.id)}
          onClose={closeEditor}
          onSaved={(next) => {
            receive(next);
            closeEditor();
            onNotify?.("MCP 配置已保存");
          }}
        />
      )}
      {deleteEntry && (
        <ExtSlideDrawer
          title={`删除 ${deleteEntry.name}`}
          onClose={closeDelete}
          footer={
            <>
              <button
                type="button"
                className={EXT_BUTTON}
                onClick={closeDelete}
                disabled={busy}
              >
                取消
              </button>
              <button
                type="button"
                className={EXT_PRIMARY_BUTTON}
                disabled={busy}
                onClick={() => {
                  void run(
                    () => api.deleteMcpEntry(deleteEntry.id),
                    "MCP 已删除",
                  ).then((ok) => {
                    if (ok) closeDelete();
                  });
                }}
              >
                <Trash2 className="h-3.5 w-3.5" />
                {busy ? "删除中..." : "确认删除"}
              </button>
            </>
          }
        >
          <p className="text-[12px] leading-relaxed text-[var(--text-secondary)]">
            删除后会从三端配置和中心库中移除此服务。仅需停用时，可关闭列表中的平台开关。
          </p>
          <ExtError message={error} />
        </ExtSlideDrawer>
      )}
      {backup.drawer}
    </div>
  );
}

function McpEditor({
  entry,
  existing,
  onClose,
  onSaved,
}: {
  entry: McpEntry;
  existing: boolean;
  onClose: () => void;
  onSaved: (snapshot: McpHubSnapshot) => void;
}) {
  const [draft, setDraft] = useState(entry);
  const [mode, setMode] = useState<"form" | "paste">("form");
  const [args, setArgs] = useState(JSON.stringify(entry.spec.args));
  const [env, setEnv] = useState(JSON.stringify(entry.spec.env, null, 2));
  const [headers, setHeaders] = useState(
    JSON.stringify(entry.spec.headers, null, 2),
  );
  const [snippet, setSnippet] = useState("");
  const [parsed, setParsed] = useState<McpEntry[]>([]);
  const [parseError, setParseError] = useState<string | null>(null);
  const [parsing, setParsing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [targets, setTargets] = useState<ExtPlatform[]>(entry.platforms);
  const [template, setTemplate] = useState("");

  useEffect(() => {
    if (mode !== "paste" || !snippet.trim()) {
      setParsed([]);
      setParseError(null);
      setParsing(false);
      return;
    }
    let cancelled = false;
    setParsing(true);
    setParseError(null);
    const timer = window.setTimeout(() => {
      void api
        .parseMcpSnippet(snippet)
        .then((items) => {
          if (cancelled) return;
          if (existing && items.length !== 1) {
            setParseError("编辑时请粘贴单个 MCP 配置");
            setParsed([]);
          } else setParsed(items);
        })
        .catch((err: unknown) => {
          if (!cancelled) {
            setParseError(String(err));
            setParsed([]);
          }
        })
        .finally(() => {
          if (!cancelled) setParsing(false);
        });
    }, 350);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [mode, snippet, existing]);

  const fill = (spec: McpSpec) => {
    setDraft((previous) => ({ ...previous, spec }));
    setArgs(JSON.stringify(spec.args));
    setEnv(JSON.stringify(spec.env, null, 2));
    setHeaders(JSON.stringify(spec.headers, null, 2));
  };
  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      let items: McpEntry[];
      if (mode === "paste") {
        if (!parsed.length || parsing || parseError)
          throw new Error("请先粘贴有效的 MCP 配置");
        items = parsed.map((item) => ({
          ...item,
          id: existing ? entry.id : item.id,
          name: existing ? draft.name : item.name,
          platforms: targets,
        }));
      } else {
        const parsedArgs: unknown = JSON.parse(args || "[]");
        if (
          !Array.isArray(parsedArgs) ||
          parsedArgs.some((arg) => typeof arg !== "string")
        )
          throw new Error("命令参数必须是 JSON 字符串数组");
        const spec: McpSpec = {
          ...draft.spec,
          args: parsedArgs,
          env: parseStringMap(env, "环境变量"),
          headers: parseStringMap(headers, "请求头"),
        };
        if (spec.transport === "http") {
          spec.command = null;
          spec.args = [];
          spec.env = {};
        } else {
          spec.url = null;
          spec.headers = {};
        }
        items = [{ ...draft, spec, platforms: targets }];
      }
      const result =
        items.length === 1
          ? await api.upsertMcpEntry(items[0])
          : await api.importMcpBackup(JSON.stringify({ entries: items }));
      onSaved(result);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <ExtSlideDrawer
      title={existing ? "编辑 MCP" : "添加 MCP"}
      onClose={onClose}
      footer={
        <>
          <button
            type="button"
            className={EXT_BUTTON}
            disabled={busy}
            onClick={onClose}
          >
            取消
          </button>
          <button
            type="button"
            className={EXT_PRIMARY_BUTTON}
            disabled={busy || (mode === "paste" && (parsing || !parsed.length))}
            onClick={() => void save()}
          >
            <Save className="h-3.5 w-3.5" />
            {busy ? "保存中..." : "保存并同步"}
          </button>
        </>
      }
    >
      <div className="flex min-w-0 flex-col gap-4">
        <SegmentedControl
          value={mode}
          onChange={setMode}
          options={[
            { id: "form", label: "表单配置 / 模板" },
            { id: "paste", label: "智能粘贴 JSON / TOML" },
          ]}
        />
        <ExtError message={error} />
        {mode === "form" ? (
          <>
            <label className="ext-field">
              常用模板
              <select
                className={EXT_INPUT}
                value={template}
                onChange={(event) => {
                  setTemplate(event.target.value);
                  const selected = TEMPLATES.find(
                    (item) => item.id === event.target.value,
                  );
                  if (!selected) return;
                  setDraft((previous) => ({
                    ...previous,
                    id: existing ? previous.id : selected.id,
                    name: selected.name,
                  }));
                  fill({
                    ...EMPTY_SPEC,
                    command: selected.command,
                    args: selected.args,
                  });
                }}
              >
                <option value="">选择模板</option>
                {TEMPLATES.map((item) => (
                  <option key={item.id} value={item.id}>
                    {item.name}
                  </option>
                ))}
              </select>
            </label>
            <div className="grid min-w-0 grid-cols-2 gap-3">
              <label className="ext-field">
                服务标识
                <input
                  className={EXT_INPUT}
                  aria-label="MCP 服务标识"
                  value={draft.id}
                  disabled={existing}
                  onChange={(event) =>
                    setDraft({ ...draft, id: event.target.value })
                  }
                  placeholder="my-mcp"
                />
              </label>
              <label className="ext-field">
                显示名称
                <input
                  className={EXT_INPUT}
                  aria-label="MCP 显示名称"
                  value={draft.name}
                  onChange={(event) =>
                    setDraft({ ...draft, name: event.target.value })
                  }
                  placeholder="My MCP"
                />
              </label>
            </div>
            <SegmentedControl
              value={draft.spec.transport}
              onChange={(transport) =>
                setDraft({ ...draft, spec: { ...draft.spec, transport } })
              }
              options={[
                { id: "stdio", label: "stdio" },
                { id: "http", label: "HTTP" },
              ]}
            />
            {draft.spec.transport === "stdio" ? (
              <>
                <label className="ext-field">
                  命令
                  <input
                    className={`${EXT_INPUT} font-mono`}
                    aria-label="MCP 命令"
                    value={draft.spec.command ?? ""}
                    onChange={(event) =>
                      setDraft({
                        ...draft,
                        spec: { ...draft.spec, command: event.target.value },
                      })
                    }
                    placeholder="npx / uvx / 可执行文件路径"
                  />
                </label>
                <label className="ext-field">
                  命令参数 (JSON)
                  <textarea
                    className={`${EXT_INPUT} min-h-[64px] font-mono`}
                    value={args}
                    onChange={(event) => setArgs(event.target.value)}
                    placeholder={'["-y", "example-mcp"]'}
                  />
                </label>
                <label className="ext-field">
                  环境变量 (JSON)
                  <textarea
                    className={`${EXT_INPUT} min-h-[72px] font-mono`}
                    value={env}
                    onChange={(event) => setEnv(event.target.value)}
                  />
                </label>
              </>
            ) : (
              <>
                <label className="ext-field">
                  服务地址
                  <input
                    className={`${EXT_INPUT} font-mono`}
                    aria-label="MCP 服务地址"
                    value={draft.spec.url ?? ""}
                    onChange={(event) =>
                      setDraft({
                        ...draft,
                        spec: { ...draft.spec, url: event.target.value },
                      })
                    }
                    placeholder="https://example.com/mcp"
                  />
                </label>
                <label className="ext-field">
                  请求头 (JSON)
                  <textarea
                    className={`${EXT_INPUT} min-h-[96px] font-mono`}
                    value={headers}
                    onChange={(event) => setHeaders(event.target.value)}
                  />
                </label>
              </>
            )}
            {Object.keys(entry.variants).length > 0 && (
              <label className="ext-field">
                采用平台配置
                <select
                  className={EXT_INPUT}
                  defaultValue=""
                  onChange={(event) => {
                    const spec = event.target.value
                      ? entry.variants[event.target.value as ExtPlatform]
                      : entry.spec;
                    if (spec) fill(spec);
                  }}
                >
                  <option value="">中心配置</option>
                  {EXT_PLATFORMS.filter(
                    (platform) => entry.variants[platform.id],
                  ).map((platform) => (
                    <option key={platform.id} value={platform.id}>
                      {platform.label}
                    </option>
                  ))}
                </select>
              </label>
            )}
          </>
        ) : (
          <>
            <textarea
              aria-label="MCP JSON 或 TOML 片段"
              className={`${EXT_INPUT} min-h-[180px] resize-y font-mono`}
              value={snippet}
              onChange={(event) => setSnippet(event.target.value)}
              placeholder={
                '{"mcpServers":{"my-mcp":{"command":"npx","args":[]}}}'
              }
            />
            <ExtError message={parseError} />
            {parsing && (
              <p className="text-[12px] text-[var(--text-muted)]">
                正在解析...
              </p>
            )}
            {parsed.map((item, index) => (
              <div
                key={index}
                className="flex min-w-0 items-center gap-2 text-[12px] text-[var(--success)]"
              >
                <Check className="h-4 w-4 shrink-0" />
                <input
                  aria-label="解析后的服务标识"
                  disabled={existing}
                  className={`${EXT_INPUT} font-mono`}
                  value={existing ? entry.id : item.id}
                  onChange={(event) =>
                    setParsed((items) =>
                      items.map((value, position) =>
                        position === index
                          ? {
                              ...value,
                              id: event.target.value,
                              name: event.target.value,
                            }
                          : value,
                      ),
                    )
                  }
                />
                <span className="shrink-0 uppercase">
                  {item.spec.transport}
                </span>
              </div>
            ))}
          </>
        )}
        <fieldset className="flex flex-col gap-2.5 border-t border-[var(--border)] pt-3">
          <legend className="text-[12px] font-medium text-[var(--text-secondary)]">
            立即同步到
          </legend>
          <div className="flex flex-wrap gap-4">
            {EXT_PLATFORMS.map((platform) => (
              <label
                key={platform.id}
                className="flex items-center gap-1.5 text-[12px] text-[var(--text-primary)]"
              >
                <input
                  type="checkbox"
                  checked={targets.includes(platform.id)}
                  onChange={(event) =>
                    setTargets((previous) =>
                      event.target.checked
                        ? [...previous, platform.id]
                        : previous.filter((target) => target !== platform.id),
                    )
                  }
                  className="accent-[var(--accent)]"
                />
                {platform.label}
              </label>
            ))}
          </div>
        </fieldset>
      </div>
    </ExtSlideDrawer>
  );
}
