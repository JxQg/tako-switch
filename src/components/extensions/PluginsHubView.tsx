import {
  useCallback,
  useDeferredValue,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  AlertCircle,
  Check,
  Download,
  Loader2,
  Plus,
  Puzzle,
  RefreshCw,
  Store,
  Trash2,
  X,
} from "lucide-react";
import {
  api,
  parsePluginError,
  type ExtPlatform,
  type PluginActionOutcome,
  type PluginEntry,
  type PluginError,
  type PluginsHubSnapshot,
} from "../../lib/api";
import { ToggleSwitch } from "../UiPrimitives";
import {
  EXT_BUTTON,
  EXT_INPUT,
  EXT_PLATFORMS,
  EXT_PRIMARY_BUTTON,
  ExtHelpPopover,
  ExtSearchInput,
  ExtSlideDrawer,
  type ExtensionActions,
} from "./ExtensionPrimitives";

type PluginTab = "installed" | "discover" | "marketplaces";
type PendingRemoval =
  | { kind: "plugin"; entry: PluginEntry }
  | { kind: "marketplace"; name: string };

export function PluginsHubView({
  onActionsChange,
  onCountChange,
}: {
  onActionsChange: (actions: ExtensionActions) => void;
  onCountChange: (count: number) => void;
}) {
  const [platform, setPlatform] = useState<ExtPlatform>("claude");
  const [tab, setTab] = useState<PluginTab>("installed");
  const [snapshot, setSnapshot] = useState<PluginsHubSnapshot | null>(null);
  const [catalogLoaded, setCatalogLoaded] = useState(false);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<PluginError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [marketOpen, setMarketOpen] = useState(false);
  const [source, setSource] = useState("");
  const [removal, setRemoval] = useState<PendingRemoval | null>(null);
  const deferredSearch = useDeferredValue(search);
  const requestId = useRef(0);
  const operationInFlight = useRef(false);
  const currentPlatform = useRef(platform);
  currentPlatform.current = platform;

  const load = useCallback(
    async (includeAvailable: boolean) => {
      const id = ++requestId.current;
      setLoading(true);
      setError(null);
      try {
        const next = await api.getPluginsHubSnapshot(
          platform,
          includeAvailable,
        );
        if (id !== requestId.current || platform !== currentPlatform.current)
          return;
        setSnapshot((previous) =>
          !includeAvailable && previous?.platform === platform
            ? {
                ...next,
                available: previous.available.filter(
                  (candidate) =>
                    !next.installed.some(
                      (installed) => installed.id === candidate.id,
                    ),
                ),
                marketplaces: previous.marketplaces,
                warnings: previous.warnings,
                catalogFromCache: previous.catalogFromCache,
              }
            : next,
        );
        if (includeAvailable) setCatalogLoaded(true);
        void api
          .getExtensionsSummary()
          .then((summary) => {
            onCountChange(
              summary && typeof summary.pluginsCount === "number"
                ? summary.pluginsCount
                : next.installed.length,
            );
          })
          .catch(() => onCountChange(next.installed.length));
      } catch (failure) {
        if (id === requestId.current && platform === currentPlatform.current)
          setError(parsePluginError(failure));
      } finally {
        if (id === requestId.current && platform === currentPlatform.current)
          setLoading(false);
      }
    },
    [platform, onCountChange],
  );

  useEffect(() => {
    setSnapshot(null);
    setCatalogLoaded(false);
    setTab("installed");
    setSearch("");
    setNotice(null);
    void load(false);
    return () => {
      requestId.current += 1;
    };
  }, [load]);

  useEffect(() => {
    if (
      tab !== "installed" &&
      snapshot?.cliAvailable &&
      !catalogLoaded &&
      !loading &&
      !error
    )
      void load(true);
  }, [tab, snapshot?.cliAvailable, catalogLoaded, loading, error, load]);

  const closeMarket = useCallback(() => setMarketOpen(false), []);
  const closeRemoval = useCallback(() => setRemoval(null), []);
  const canManageMarket =
    platform !== "antigravity" &&
    snapshot?.cliAvailable === true &&
    snapshot.marketplacesSupported;

  useLayoutEffect(() => {
    onActionsChange({
      secondary: {
        label: "刷新",
        icon: RefreshCw,
        testId: "ext-plugins-refresh-btn",
        disabled: loading || busy !== null,
        spinning: loading,
        onClick: () => void load(tab !== "installed"),
      },
      primary: {
        label: "添加市场",
        icon: Plus,
        testId: "ext-plugins-market-btn",
        disabled: !canManageMarket || loading || busy !== null,
        onClick: () => {
          setSource("");
          setMarketOpen(true);
        },
      },
    });
  }, [onActionsChange, canManageMarket, loading, busy, load, tab]);

  const runAction = async (
    key: string,
    command: () => Promise<PluginActionOutcome>,
    after?: () => void,
  ) => {
    if (operationInFlight.current) return;
    operationInFlight.current = true;
    setBusy(key);
    setError(null);
    setNotice(null);
    try {
      const outcome = await command();
      if (currentPlatform.current !== outcome.platform) return;
      setNotice(
        outcome.needsConfiguration
          ? "插件已安装，请在客户端内完成授权配置。"
          : outcome.message,
      );
      after?.();
      if (tab === "installed") setCatalogLoaded(false);
      await load(tab !== "installed" || key.startsWith("marketplace:"));
    } catch (failure) {
      setError(parsePluginError(failure));
    } finally {
      operationInFlight.current = false;
      setBusy(null);
    }
  };

  const entries = useMemo(() => {
    const query = deferredSearch.trim().toLocaleLowerCase();
    const items =
      tab === "discover"
        ? (snapshot?.available ?? [])
        : (snapshot?.installed ?? []);
    return items.filter((entry) =>
      [
        entry.displayName,
        entry.name,
        entry.id,
        entry.category ?? "",
        entry.description,
      ].some((text) => text.toLocaleLowerCase().includes(query)),
    );
  }, [snapshot, tab, deferredSearch]);
  const marketplaces = useMemo(() => {
    const query = deferredSearch.trim().toLocaleLowerCase();
    return (snapshot?.marketplaces ?? []).filter((marketplace) =>
      `${marketplace.name} ${marketplace.source}`
        .toLocaleLowerCase()
        .includes(query),
    );
  }, [snapshot?.marketplaces, deferredSearch]);
  const selectedCount =
    tab === "marketplaces" ? marketplaces.length : entries.length;
  const targetId = (entry: PluginEntry) =>
    platform === "antigravity" ? (entry.dirName ?? entry.name) : entry.id;
  const inUse = error?.errorCode === "PLUGIN_IN_USE";
  const needsTerminal = error?.errorCode === "PLUGIN_NEEDS_TERMINAL_CONFIRM";

  return (
    <section
      data-testid="ext-plugins-view"
      className="min-w-0 flex flex-col gap-[var(--density-list-gap)]"
      aria-busy={loading || busy !== null}
    >
      <div className="flex min-w-0 flex-wrap items-center justify-between gap-x-2 gap-y-2 border-b border-[var(--border)] pb-3">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <div
            role="tablist"
            aria-label="插件平台"
            className="inline-flex min-w-0 rounded-lg border border-[var(--border)] bg-[var(--bg-pill)] p-0.5"
          >
            {EXT_PLATFORMS.map((item) => (
              <button
                key={item.id}
                type="button"
                role="tab"
                aria-selected={platform === item.id}
                data-testid={`ext-plugins-platform-${item.id}`}
                disabled={busy !== null}
                onClick={() => setPlatform(item.id)}
                className={`h-[var(--density-ctrl-sm-h)] shrink-0 whitespace-nowrap rounded-md px-2.5 text-[12px] cursor-pointer disabled:opacity-45 ${platform === item.id ? "bg-[var(--bg-card)] font-semibold text-[var(--text-primary)] shadow-[var(--shadow-segment)]" : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"}`}
              >
                {item.label}
              </button>
            ))}
          </div>
          <span
            data-testid="ext-plugins-cli-status"
            title={snapshot?.cliPath ?? undefined}
            className={`inline-flex h-[var(--density-pill-h)] shrink-0 items-center gap-1 whitespace-nowrap rounded-full px-2 text-[10.5px] ${snapshot?.cliAvailable ? "bg-[var(--success-soft)] text-[var(--success)]" : "bg-[var(--bg-subtle)] text-[var(--text-muted)]"}`}
          >
            {snapshot?.cliAvailable && <Check className="h-3 w-3" />}
            {snapshot?.cliAvailable
              ? snapshot.cliSource === "builtin"
                ? "内置 CLI"
                : "PATH CLI"
              : loading
                ? "探测中..."
                : "未检出 CLI"}
          </span>
        </div>
        <div
          role="tablist"
          aria-label="插件列表"
          className="inline-flex shrink-0 gap-0.5 rounded-lg bg-[var(--bg-pill)] p-0.5"
        >
          {(
            [
              { id: "installed", label: "已安装" },
              { id: "discover", label: "发现" },
              { id: "marketplaces", label: "市场源" },
            ] as const
          ).map((item) => (
            <button
              key={item.id}
              type="button"
              role="tab"
              aria-selected={tab === item.id}
              data-testid={`ext-plugins-subtab-${item.id}`}
              disabled={
                busy !== null ||
                (item.id === "marketplaces" && platform === "antigravity")
              }
              onClick={() => setTab(item.id)}
              className={`h-[var(--density-ctrl-sm-h)] shrink-0 whitespace-nowrap rounded-md px-2.5 text-[12px] cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed ${tab === item.id ? "bg-[var(--bg-card)] text-[var(--text-primary)] font-medium shadow-[var(--shadow-segment)]" : "text-[var(--text-secondary)]"}`}
            >
              {item.label}
            </button>
          ))}
        </div>
      </div>

      <div className="flex min-w-0 items-center gap-2">
        <ExtSearchInput
          value={search}
          onChange={setSearch}
          placeholder="搜索插件名称或分类..."
          testId="ext-plugins-search"
        />
        <span className="ml-auto shrink-0 whitespace-nowrap text-[11px] tabular-nums text-[var(--text-muted)]">
          {selectedCount} {tab === "marketplaces" ? "个市场" : "个插件"}
        </span>
        <ExtHelpPopover label="CLI 探测规则">
          <p>
            依次检测桌面端内置 CLI、用户 CLI 安装目录和系统 PATH。未检出 CLI
            的客户端暂不支持插件管理。
          </p>
          {snapshot?.cliPath && (
            <p className="mt-2 break-all font-mono text-[11px]">
              {snapshot.cliPath}
            </p>
          )}
        </ExtHelpPopover>
      </div>

      {error && (
        <div
          role="alert"
          data-testid="ext-plugins-error"
          className="flex min-w-0 items-start gap-2 rounded-lg border border-[var(--warning-border)] bg-[var(--warning-soft)] p-3 text-[12px] text-[var(--warning)]"
        >
          <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
          <div className="min-w-0 flex-1 break-words">
            <p>
              {inUse
                ? `插件正在被占用，请先关闭运行中的 ${EXT_PLATFORMS.find((item) => item.id === platform)?.label} 客户端会话后重试。`
                : needsTerminal
                  ? "该市场需要交互式确认，请先在终端使用对应 CLI 确认并信任市场来源，然后重试。"
                  : error.message}
            </p>
            {error.detail && (
              <details className="mt-1">
                <summary className="cursor-pointer">错误详情</summary>
                <pre className="mt-1 max-h-28 overflow-auto whitespace-pre-wrap break-all text-[11px] selectable">
                  {error.detail}
                </pre>
              </details>
            )}
          </div>
          <button
            type="button"
            title="关闭错误提示"
            aria-label="关闭错误提示"
            onClick={() => setError(null)}
            className="shrink-0 cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      )}
      {notice && (
        <p
          role="status"
          data-testid="ext-plugins-notice"
          className="rounded-lg bg-[var(--success-soft)] px-3 py-2 text-[12px] text-[var(--success)] break-words"
        >
          {notice}
        </p>
      )}
      {(snapshot?.warnings.length ?? 0) > 0 && (
        <p
          role="status"
          className="rounded-lg bg-[var(--warning-soft)] px-3 py-2 text-[11.5px] text-[var(--warning)] break-words"
        >
          {snapshot?.warnings.join(" ")}
        </p>
      )}

      {loading && (
        <div
          role="status"
          className="flex min-h-24 items-center justify-center gap-2 text-[12px] text-[var(--text-muted)]"
        >
          <Loader2 className="h-4 w-4 animate-spin" />
          正在读取插件...
        </div>
      )}
      {!loading && snapshot?.cliAvailable === false && (
        <div
          data-testid="ext-plugins-empty-unsupported"
          className="flex min-h-[210px] flex-col items-center justify-center gap-3 rounded-lg border border-[var(--border)] bg-[var(--bg-card)] px-5 text-center"
        >
          <Puzzle
            className="h-7 w-7 text-[var(--text-muted)]"
            strokeWidth={1.5}
          />
          <p className="max-w-full text-[13px] text-[var(--text-secondary)] break-words">
            未检测到对应 CLI，当前客户端暂不支持该功能
          </p>
        </div>
      )}
      {!loading && snapshot?.cliAvailable && tab !== "marketplaces" && (
        <div className="grid min-w-0 grid-cols-1 min-[640px]:grid-cols-2 gap-[var(--density-list-gap)]">
          {entries.map((entry) => (
            <article
              key={`${entry.id}:${entry.scope ?? "user"}`}
              data-testid="ext-plugin-card"
              className="min-w-0 rounded-lg border border-[var(--border)] bg-[var(--bg-card)] px-[var(--density-card-px)] py-[var(--density-card-py)] flex flex-col gap-2"
            >
              <div className="flex min-w-0 items-start justify-between gap-2">
                <div className="min-w-0 flex-1">
                  <h3
                    className="truncate text-[13px] font-semibold"
                    title={entry.displayName}
                  >
                    {entry.displayName}
                  </h3>
                  <p
                    className="min-h-4 truncate text-[10.5px] tabular-nums text-[var(--text-muted)]"
                    title={entry.version ?? undefined}
                  >
                    {entry.version
                      ? `v${entry.version.replace(/^v/, "")}`
                      : "版本未提供"}
                  </p>
                </div>
                {tab === "installed" ? (
                  <ToggleSwitch
                    checked={entry.enabled}
                    disabled={!entry.manageable || busy !== null}
                    ariaLabel={`${entry.enabled ? "停用" : "启用"} ${entry.displayName}`}
                    onChange={(enabled) =>
                      void runAction(entry.id, () =>
                        api.togglePluginEnabled(
                          platform,
                          targetId(entry),
                          enabled,
                        ),
                      )
                    }
                  />
                ) : (
                  <button
                    type="button"
                    disabled={busy !== null}
                    onClick={() =>
                      void runAction(entry.id, () =>
                        api.installPluginEntry(platform, entry.id),
                      )
                    }
                    className={`${EXT_PRIMARY_BUTTON} h-7 px-2 text-[11px]`}
                  >
                    {busy === entry.id ? (
                      <Loader2 className="h-3 w-3 animate-spin" />
                    ) : (
                      <Download className="h-3 w-3" />
                    )}
                    安装
                  </button>
                )}
              </div>
              <div className="flex min-h-5 min-w-0 flex-wrap gap-1">
                {entry.category && (
                  <span
                    className="max-w-full truncate rounded-full bg-[var(--bg-subtle)] px-2 py-0.5 text-[10px] text-[var(--text-secondary)]"
                    title={entry.category}
                  >
                    {entry.category}
                  </span>
                )}
                {!entry.manageable && (
                  <span className="rounded-full bg-[var(--bg-subtle)] px-2 py-0.5 text-[10px] text-[var(--text-muted)]">
                    {entry.scope === "project" ? "项目管理" : "只读"}
                  </span>
                )}
                {entry.needsConfiguration && (
                  <span className="rounded-full bg-[var(--warning-soft)] px-2 py-0.5 text-[10px] text-[var(--warning)]">
                    需客户端授权
                  </span>
                )}
              </div>
              <p
                className="min-h-9 min-w-0 line-clamp-2 break-words text-[11.5px] leading-[18px] text-[var(--text-secondary)]"
                title={entry.description}
              >
                {entry.description || "暂无简介"}
              </p>
              <footer className="mt-auto min-w-0 flex items-center gap-1.5 border-t border-[var(--border)] pt-2">
                <code
                  className="min-w-0 flex-1 truncate font-mono text-[10px] text-[var(--text-muted)]"
                  title={entry.id}
                >
                  {entry.id}
                </code>
                {tab === "installed" && (
                  <div className="flex shrink-0 items-center gap-2 whitespace-nowrap text-[11px]">
                    <button
                      type="button"
                      title={`更新 ${entry.displayName}`}
                      disabled={
                        !entry.manageable ||
                        busy !== null ||
                        (platform === "antigravity" && !entry.marketplace)
                      }
                      onClick={() =>
                        void runAction(entry.id, () =>
                          api.updatePluginEntry(platform, entry.id),
                        )
                      }
                      className="inline-flex items-center gap-1 text-[var(--accent)] cursor-pointer disabled:opacity-40"
                    >
                      <RefreshCw
                        className={`h-3 w-3 ${busy === entry.id ? "animate-spin" : ""}`}
                      />
                      更新
                    </button>
                    <span className="text-[var(--text-muted)]">·</span>
                    <button
                      type="button"
                      title={`卸载 ${entry.displayName}`}
                      disabled={!entry.removable || busy !== null}
                      onClick={() => setRemoval({ kind: "plugin", entry })}
                      className="inline-flex items-center gap-1 text-[var(--danger)] cursor-pointer disabled:opacity-40"
                    >
                      <Trash2 className="h-3 w-3" />
                      卸载
                    </button>
                  </div>
                )}
              </footer>
            </article>
          ))}
        </div>
      )}
      {!loading && snapshot?.cliAvailable && tab === "marketplaces" && (
        <div className="min-w-0 divide-y divide-[var(--border)]">
          {marketplaces.map((marketplace) => (
            <article
              key={marketplace.name}
              className="flex min-w-0 items-center gap-3 py-3"
            >
              <Store className="h-4 w-4 shrink-0 text-[var(--text-muted)]" />
              <div className="min-w-0 flex-1">
                <h3
                  className="truncate text-[12px] font-semibold"
                  title={marketplace.name}
                >
                  {marketplace.name}
                </h3>
                <p
                  className="truncate font-mono text-[11px] text-[var(--text-muted)]"
                  title={marketplace.source}
                >
                  {marketplace.source || "来源未提供"}
                </p>
              </div>
              <button
                type="button"
                disabled={busy !== null || !marketplace.manageable}
                title={`移除市场 ${marketplace.name}`}
                aria-label={`移除市场 ${marketplace.name}`}
                onClick={() =>
                  setRemoval({ kind: "marketplace", name: marketplace.name })
                }
                className={`${EXT_BUTTON} w-8 px-0 text-[var(--danger)]`}
              >
                <Trash2 className="h-3.5 w-3.5" />
              </button>
            </article>
          ))}
        </div>
      )}
      {!loading && snapshot?.cliAvailable && selectedCount === 0 && (
        <p className="py-10 text-center text-[12px] text-[var(--text-muted)]">
          {search
            ? "没有匹配的结果"
            : tab === "installed"
              ? "暂无已安装插件"
              : tab === "discover"
                ? "暂无可发现插件"
                : "暂无市场源"}
        </p>
      )}

      {marketOpen && (
        <ExtSlideDrawer
          title="添加市场"
          onClose={closeMarket}
          busy={busy !== null}
        >
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void runAction(
                "marketplace:add",
                () => api.addPluginMarketplace(platform, source.trim()),
                closeMarket,
              );
            }}
            className="flex min-w-0 flex-col gap-4"
          >
            <label className="flex min-w-0 flex-col gap-2 text-[12px] text-[var(--text-secondary)]">
              市场来源
              <input
                required
                value={source}
                onChange={(event) => setSource(event.target.value)}
                disabled={busy !== null}
                placeholder="owner/repo 或 https://..."
                className={`${EXT_INPUT} font-mono`}
              />
            </label>
            {error && (
              <p
                role="alert"
                className="break-words text-[12px] text-[var(--danger)]"
              >
                {error.message}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <button
                type="button"
                disabled={busy !== null}
                onClick={closeMarket}
                className={EXT_BUTTON}
              >
                取消
              </button>
              <button
                type="submit"
                disabled={!source.trim() || busy !== null}
                className={EXT_PRIMARY_BUTTON}
              >
                {busy ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Plus className="h-3.5 w-3.5" />
                )}
                添加市场
              </button>
            </div>
          </form>
        </ExtSlideDrawer>
      )}
      {removal && (
        <ExtSlideDrawer
          title={removal.kind === "plugin" ? "卸载插件" : "移除市场"}
          onClose={closeRemoval}
          busy={busy !== null}
        >
          <p className="break-words text-[13px] text-[var(--text-secondary)]">
            确认{removal.kind === "plugin" ? "卸载" : "移除"}「
            {removal.kind === "plugin"
              ? removal.entry.displayName
              : removal.name}
            」？
          </p>
          {error && (
            <p
              role="alert"
              className="mt-3 break-words text-[12px] text-[var(--danger)]"
            >
              {error.message}
            </p>
          )}
          <div className="mt-5 flex justify-end gap-2">
            <button
              type="button"
              disabled={busy !== null}
              onClick={closeRemoval}
              className={EXT_BUTTON}
            >
              取消
            </button>
            <button
              type="button"
              disabled={busy !== null}
              onClick={() =>
                void runAction(
                  removal.kind === "plugin"
                    ? removal.entry.id
                    : "marketplace:remove",
                  () =>
                    removal.kind === "plugin"
                      ? api.uninstallPluginEntry(
                          platform,
                          targetId(removal.entry),
                        )
                      : api.removePluginMarketplace(platform, removal.name),
                  closeRemoval,
                )
              }
              className={`${EXT_BUTTON} text-[var(--danger)]`}
            >
              {busy ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Trash2 className="h-3.5 w-3.5" />
              )}
              确认{removal.kind === "plugin" ? "卸载" : "移除"}
            </button>
          </div>
        </ExtSlideDrawer>
      )}
    </section>
  );
}
