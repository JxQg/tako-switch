import {
  useCallback,
  useDeferredValue,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  ArrowUp,
  ArrowUpCircle,
  Check,
  Download,
  FolderInput,
  FolderSearch,
  GitBranch,
  Link2,
  Loader2,
  Plus,
  RefreshCw,
  ScanSearch,
  Settings2,
  ShieldCheck,
  Trash2,
  Upload,
} from "lucide-react";
import {
  api,
  type DiscoverableSkill,
  type ExtPlatform,
  type ManagedSkill,
  type SkillRepo,
  type SkillsHubSnapshot,
  type SkillSource,
  type SkillSyncMethod,
} from "../../lib/api";
import { ToggleSwitch } from "../UiPrimitives";
import {
  EXT_BUTTON,
  EXT_INPUT,
  EXT_PLATFORMS,
  EXT_PRIMARY_BUTTON,
  ExtError,
  ExtHelpPopover,
  ExtMatrixHeader,
  ExtMoreDropdown,
  ExtPlatformChoices,
  ExtSearchInput,
  ExtSlideDrawer,
  type ExtensionActions,
} from "./ExtensionPrimitives";

const MATRIX_COLUMNS = "minmax(0, 1fr) 70px 64px 76px 92px";
const SOURCE_LABELS: Record<SkillSource, string> = {
  local: "本地",
  adopted: "已收编",
  zip: "ZIP",
  github: "GitHub",
};
const SYNC_METHODS: { id: SkillSyncMethod; label: string }[] = [
  { id: "auto", label: "软链优先，自动回退复制" },
  { id: "symlink", label: "仅使用软链" },
  { id: "copy", label: "强制复制" },
];
const MAX_ARCHIVE_BYTES = 200 * 1024 * 1024;
const INLINE_ZIP_THRESHOLD_BYTES = 16 * 1024 * 1024;
const ZIP_CHUNK_BYTES = 4 * 1024 * 1024;

function blobToBase64(blob: Blob): Promise<string> {
  return new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result !== "string") {
        reject(new Error("读取 ZIP 文件失败"));
        return;
      }
      resolve(reader.result.slice(reader.result.indexOf(",") + 1));
    };
    reader.onerror = () =>
      reject(reader.error ?? new Error("读取 ZIP 文件失败"));
    reader.readAsDataURL(blob);
  });
}

function sourceLabel(item: ManagedSkill): string {
  return item.skill.repoOwner && item.skill.repoName
    ? `${item.skill.repoOwner}/${item.skill.repoName}`
    : SOURCE_LABELS[item.skill.source];
}

export function SkillsHubView({
  onActionsChange,
  onCountChange,
  onNotify,
}: {
  onActionsChange: (actions: ExtensionActions) => void;
  onCountChange: (count: number, updatableCount?: number) => void;
  onNotify?: (message: string) => void;
}) {
  const [snapshot, setSnapshot] = useState<SkillsHubSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const mounted = useRef(true);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<"installed" | "discover">("installed");
  const [search, setSearch] = useState("");
  const query = useDeferredValue(search.trim().toLocaleLowerCase());
  const [discovered, setDiscovered] = useState<DiscoverableSkill[]>([]);
  const [scannedRepos, setScannedRepos] = useState<string[]>([]);
  const [drawer, setDrawer] = useState<"import" | "repos" | null>(null);
  const [uninstalling, setUninstalling] = useState<ManagedSkill | null>(null);
  const [importMode, setImportMode] = useState<"zip" | "path">("zip");
  const [importPath, setImportPath] = useState("");
  const [zipFile, setZipFile] = useState<File | null>(null);
  const [nativeZipPath, setNativeZipPath] = useState<string | null>(null);
  const [importTargets, setImportTargets] = useState<ExtPlatform[]>([]);
  const [dragging, setDragging] = useState(false);
  const [drawerError, setDrawerError] = useState<string | null>(null);
  const [repoInput, setRepoInput] = useState("");
  const [branchInput, setBranchInput] = useState("main");
  const [notice, setNotice] = useState<string | null>(null);
  const [dismissedWarningsSig, setDismissedWarningsSig] = useState<
    string | null
  >(null);

  useEffect(() => {
    mounted.current = true;
    void api
      .getSkillsHubSnapshot()
      .then((data) => {
        if (mounted.current) setSnapshot(data);
      })
      .catch((err: unknown) => {
        if (mounted.current) setError(String(err));
      })
      .finally(() => {
        if (mounted.current) setLoading(false);
      });
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (snapshot) {
      const updatable = snapshot.skills.filter(
        (item) => item.skill.updateAvailable,
      ).length;
      onCountChange(snapshot.skills.length, updatable);
    }
  }, [snapshot, onCountChange]);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), 4500);
    return () => window.clearTimeout(timer);
  }, [notice]);

  const chooseZip = useCallback((file: File) => {
    if (!file.name.toLowerCase().endsWith(".zip")) {
      setDrawerError("请选择 .zip 文件");
      return;
    }
    if (file.size > MAX_ARCHIVE_BYTES) {
      setDrawerError("ZIP 文件不能超过 200 MiB");
      return;
    }
    setZipFile(file);
    setNativeZipPath(null);
    setDrawerError(null);
  }, []);

  useEffect(() => {
    if (drawer !== "import" || !isTauri()) return;
    let disposed = false;
    const subscription = getCurrentWebview().onDragDropEvent((event) => {
      if (disposed || busyRef.current) return;
      if (event.payload.type === "over" || event.payload.type === "enter")
        setDragging(true);
      else setDragging(false);
      if (event.payload.type === "drop") {
        const path = event.payload.paths[0];
        if (
          !path ||
          !path.toLowerCase().endsWith(".zip") ||
          event.payload.paths.length !== 1
        ) {
          setDrawerError("请一次拖入一个 .zip 文件");
        } else {
          setImportMode("zip");
          setNativeZipPath(path);
          setZipFile(null);
          setDrawerError(null);
        }
      }
    });
    void subscription.catch((err: unknown) => {
      if (!disposed) setDrawerError(`无法接收原生拖放：${String(err)}`);
    });
    return () => {
      disposed = true;
      void subscription.then(
        (unlisten) => unlisten(),
        () => {},
      );
    };
  }, [drawer]);

  const run = async (operation: () => Promise<void>, inDrawer = false) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    (inDrawer ? setDrawerError : setError)(null);
    try {
      await operation();
    } catch (err) {
      if (mounted.current) (inDrawer ? setDrawerError : setError)(String(err));
    } finally {
      busyRef.current = false;
      if (mounted.current) setBusy(false);
    }
  };

  const notify = (message: string) => {
    setNotice(message);
    onNotify?.(message);
  };

  const applySnapshot = async (
    promise: Promise<SkillsHubSnapshot>,
    message?: string,
  ) => {
    const next = await promise;
    if (!mounted.current) return;
    setSnapshot(next);
    if (message) notify(message);
  };

  const refreshRepo = async (repo: SkillRepo) => {
    const list = await api.discoverRepoSkills(repo.owner, repo.name);
    const key = `${repo.owner}/${repo.name}`;
    if (!mounted.current) return;
    setScannedRepos((previous) => [...new Set([...previous, key])]);
    setDiscovered((previous) => [
      ...previous.filter(
        (skill) => `${skill.repoOwner}/${skill.repoName}` !== key,
      ),
      ...list,
    ]);
    setSnapshot(await api.getSkillsHubSnapshot());
  };

  const checkUpdates = () =>
    run(async () => {
      const failures: string[] = [];
      for (const repo of snapshot?.repos.filter((repo) => repo.enabled) ?? []) {
        try {
          await refreshRepo(repo);
        } catch (err) {
          failures.push(`${repo.owner}/${repo.name}: ${String(err)}`);
        }
      }
      if (failures.length > 0) throw new Error(failures.join("\n"));
      notify("已完成仓库更新检查");
    });

  const openImport = () => {
    setImportTargets(
      snapshot?.defaultTargets ?? EXT_PLATFORMS.map((platform) => platform.id),
    );
    setDrawerError(null);
    setDrawer("import");
  };
  const openRepos = () => {
    setDrawerError(null);
    setDrawer("repos");
  };

  const actionHandlersRef = useRef({
    scan: () => {},
    openRepos: () => {},
    checkUpdates: () => {},
    setSyncMethod: (_method: SkillSyncMethod) => {},
    openImport: () => {},
  });
  actionHandlersRef.current = {
    scan: () =>
      void run(() =>
        applySnapshot(api.scanAdoptLocalSkills(), "本机技能已收编"),
      ),
    openRepos,
    checkUpdates: () => void checkUpdates(),
    setSyncMethod: (method: SkillSyncMethod) =>
      void run(() =>
        applySnapshot(api.setSkillSyncMethod(method), "同步策略已更新"),
      ),
    openImport,
  };

  useLayoutEffect(() => {
    onActionsChange({
      disabled: busy || loading,
      more: [
        {
          label: "扫描收编本机技能",
          icon: ScanSearch,
          onClick: () => actionHandlersRef.current.scan(),
        },
        {
          label: "管理仓库源",
          icon: Settings2,
          onClick: () => actionHandlersRef.current.openRepos(),
        },
        {
          label: "检查更新",
          icon: RefreshCw,
          onClick: () => actionHandlersRef.current.checkUpdates(),
        },
        ...SYNC_METHODS.map((method) => ({
          label: method.label,
          icon: method.id === "copy" ? FolderInput : Link2,
          checked: snapshot?.syncMethod === method.id,
          onClick: () => actionHandlersRef.current.setSyncMethod(method.id),
        })),
      ],
      primary: {
        label: "导入技能",
        icon: Plus,
        testId: "ext-skills-import-btn",
        disabled: busy || loading,
        onClick: () => actionHandlersRef.current.openImport(),
      },
    });
  }, [onActionsChange, busy, loading, snapshot?.syncMethod]);

  const installed = (snapshot?.skills ?? []).filter((item) =>
    `${item.skill.name} ${item.skill.id} ${item.skill.description} ${sourceLabel(item)}`
      .toLocaleLowerCase()
      .includes(query),
  );
  const available = discovered.filter((item) =>
    `${item.name} ${item.description} ${item.repoOwner}/${item.repoName}`
      .toLocaleLowerCase()
      .includes(query),
  );
  const updates =
    snapshot?.skills.filter((item) => item.skill.updateAvailable).length ?? 0;

  const warnings = snapshot?.warnings ?? [];
  const warningsSig = warnings.join("\n");

  const handleImport = async () => {
    await run(async () => {
      let zipBase64: string | undefined;
      let uploadId: string | undefined;
      let path: string | undefined;
      if (importMode === "path") path = importPath.trim();
      else if (nativeZipPath) path = nativeZipPath;
      else if (zipFile) {
        if (zipFile.size <= INLINE_ZIP_THRESHOLD_BYTES) {
          zipBase64 = await blobToBase64(zipFile);
        } else {
          let stagedId: string | null = null;
          try {
            for (
              let offset = 0;
              offset < zipFile.size;
              offset += ZIP_CHUNK_BYTES
            ) {
              const slice = zipFile.slice(offset, offset + ZIP_CHUNK_BYTES);
              const chunkBase64 = await blobToBase64(slice);
              stagedId = await api.stageSkillZipChunk(stagedId, chunkBase64);
            }
            uploadId = stagedId ?? undefined;
          } catch (err) {
            if (stagedId) {
              void api.discardSkillZipUpload(stagedId).catch(() => {});
            }
            throw err;
          }
        }
      }
      if (!path && !zipBase64 && !uploadId)
        throw new Error("请选择 ZIP 文件或填写本地目录路径");
      const next = await api.importLocalSkill({
        path,
        zipBase64,
        ...(uploadId ? { uploadId } : {}),
        targets: importTargets,
      });
      setSnapshot(next);
      setTab("installed");
      setDrawer(null);
      setZipFile(null);
      setNativeZipPath(null);
      setImportPath("");
      notify("技能已导入");
    }, true);
  };

  const addRepo = () =>
    run(async () => {
      const parts = repoInput.trim().split("/");
      if (parts.length !== 2 || parts.some((part) => !part.trim()))
        throw new Error("请填写 owner/repo 格式的仓库源");
      await applySnapshot(
        api.upsertSkillRepo({
          owner: parts[0],
          name: parts[1],
          branch: branchInput.trim() || "main",
          enabled: true,
          archiveUrlOverride: null,
        }),
        "仓库源已添加",
      );
      setRepoInput("");
      setBranchInput("main");
    }, true);

  const repoRows = (
    <div className="min-w-0 divide-y divide-[var(--border)]">
      {(snapshot?.repos ?? []).map((repo) => (
        <div
          key={`${repo.owner}/${repo.name}`}
          className="flex min-w-0 items-center gap-2 py-2.5"
        >
          <GitBranch className="h-4 w-4 shrink-0 text-[var(--text-muted)]" />
          <div className="min-w-0 flex-1">
            <div
              className="truncate text-[12px] font-medium"
              title={`${repo.owner}/${repo.name}`}
            >
              {repo.owner}/{repo.name}
            </div>
            <div className="truncate text-[11px] text-[var(--text-muted)]">
              {repo.branch}
            </div>
          </div>
          <ToggleSwitch
            checked={repo.enabled}
            disabled={busy}
            ariaLabel={`启用仓库 ${repo.owner}/${repo.name}`}
            onChange={(enabled) =>
              void run(
                () => applySnapshot(api.upsertSkillRepo({ ...repo, enabled })),
                drawer === "repos",
              )
            }
          />
          <button
            type="button"
            title="刷新仓库"
            aria-label={`刷新 ${repo.owner}/${repo.name}`}
            disabled={busy || !repo.enabled}
            onClick={() =>
              void run(() => refreshRepo(repo), drawer === "repos")
            }
            className={`${EXT_BUTTON} !h-7 !w-7 !px-0`}
          >
            <RefreshCw className="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            title="移除仓库源"
            aria-label={`移除 ${repo.owner}/${repo.name}`}
            disabled={busy}
            onClick={() =>
              void run(async () => {
                await applySnapshot(api.removeSkillRepo(repo.owner, repo.name));
                setDiscovered((previous) =>
                  previous.filter(
                    (skill) =>
                      !(
                        skill.repoOwner === repo.owner &&
                        skill.repoName === repo.name
                      ),
                  ),
                );
              }, drawer === "repos")
            }
            className={`${EXT_BUTTON} !h-7 !w-7 !px-0 !text-[var(--danger)]`}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
    </div>
  );

  return (
    <>
      <section
        data-testid="ext-skills-view"
        aria-label="Skills 技能"
        aria-busy={busy || loading}
        className="flex min-w-0 flex-col gap-3"
      >
        <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
          <div
            role="tablist"
            aria-label="技能视图"
            className="inline-flex h-[var(--density-ctrl-h)] shrink-0 items-center gap-0.5 rounded-[7px] bg-[var(--bg-pill)] p-0.5"
          >
            {(["installed", "discover"] as const).map((value) => (
              <button
                key={value}
                type="button"
                role="tab"
                aria-selected={tab === value}
                data-testid={`ext-skills-subtab-${value}`}
                onClick={() => setTab(value)}
                className={`h-full cursor-pointer whitespace-nowrap rounded-[5px] px-2.5 text-[12px] ${tab === value ? "bg-[var(--bg-card)] font-semibold text-[var(--text-primary)] shadow-[var(--shadow-segment)]" : "text-[var(--text-secondary)]"}`}
              >
                {value === "installed"
                  ? `已安装 (${snapshot?.skills.length ?? 0})`
                  : "仓库发现"}
              </button>
            ))}
          </div>
          {updates > 0 && (
            <span className="inline-flex shrink-0 items-center gap-1 whitespace-nowrap text-[11px] font-semibold text-[var(--success)]">
              <ArrowUp className="h-3 w-3" />
              <span className="tabular-nums">{updates}</span> 项可更新
            </span>
          )}
          <div className="ml-auto flex min-w-[184px] max-w-[314px] flex-1 items-center justify-end gap-2">
            <ExtSearchInput
              value={search}
              onChange={setSearch}
              placeholder="搜索技能名称或来源..."
              testId="ext-skills-search"
            />
            <ExtHelpPopover label="主副本与同步策略">
              <p>
                主副本保存在 {snapshot?.masterDir || "~/.tako/switch/skills"}
                。关闭全部平台仍会保留主副本；只有彻底卸载会删除它。
              </p>
              <p className="mt-2">
                自动模式优先使用目录软链，无权限时回退为复制。平台开关下方显示实际方式；有差异时可从主副本重新对齐。
              </p>
            </ExtHelpPopover>
          </div>
        </div>
        <ExtError message={error} onDismiss={() => setError(null)} />
        {warnings.length > 0 && dismissedWarningsSig !== warningsSig && (
          <div
            data-testid="ext-skills-warnings"
            role="status"
            className="flex min-w-0 items-center justify-between gap-2 rounded-md border border-[var(--border)] bg-[var(--warning-soft)] px-2.5 py-1.5 text-[11px] text-[var(--warning)]"
          >
            <span className="min-w-0 truncate" title={warnings[0]}>
              已隔离跳过 {warnings.length} 个异常技能目录，其余技能不受影响
            </span>
            <div className="flex shrink-0 items-center gap-1.5">
              <ExtHelpPopover label="查看跳过原因">
                {warnings.map((warning) => (
                  <p className="mb-1.5 break-all last:mb-0" key={warning}>
                    {warning}
                  </p>
                ))}
              </ExtHelpPopover>
              <button
                type="button"
                onClick={() => setDismissedWarningsSig(warningsSig)}
                className="shrink-0 cursor-pointer whitespace-nowrap text-[11px] underline opacity-80 hover:opacity-100"
              >
                忽略
              </button>
            </div>
          </div>
        )}
        {notice && !onNotify && (
          <div role="status" className="text-[12px] text-[var(--success)]">
            {notice}
          </div>
        )}
        {busy && (
          <div
            role="status"
            className="flex items-center gap-1.5 text-[11px] text-[var(--text-muted)]"
          >
            <Loader2 className="h-3 w-3 animate-spin" />
            正在处理...
          </div>
        )}
        {loading ? (
          <div
            role="status"
            className="flex items-center justify-center gap-2 py-14 text-[12px] text-[var(--text-muted)]"
          >
            <Loader2 className="h-4 w-4 animate-spin" />
            正在读取本机技能...
          </div>
        ) : tab === "installed" ? (
          <>
            <div className="min-w-0 overflow-hidden rounded-lg border border-[var(--border)] bg-[var(--bg-card)]">
              <ExtMatrixHeader
                columns={MATRIX_COLUMNS}
                leadingLabel="技能名称 / 来源"
                trailingLabel="状态 / 操作"
              />
              {installed.map((item) => (
                <div
                  key={item.skill.id}
                  data-testid={`ext-skill-row-${item.skill.id}`}
                  className="grid min-w-0 items-center border-b border-[var(--border)] px-2 py-[var(--density-row-py)]"
                  style={{ gridTemplateColumns: MATRIX_COLUMNS }}
                >
                  <div className="min-w-0 pr-2">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <span
                        className="min-w-0 truncate text-[12.5px] font-semibold"
                        title={item.skill.name}
                      >
                        {item.skill.name}
                      </span>
                      <span
                        className="max-w-[90px] shrink-0 truncate rounded-[4px] bg-[var(--bg-subtle)] px-1.5 py-0.5 text-[10px] text-[var(--text-secondary)]"
                        title={sourceLabel(item)}
                      >
                        {SOURCE_LABELS[item.skill.source]}
                      </span>
                    </div>
                    <div
                      className="mt-0.5 truncate text-[11px] text-[var(--text-muted)]"
                      title={item.skill.description || sourceLabel(item)}
                    >
                      {item.skill.description || sourceLabel(item)}
                    </div>
                    {item.locallyModified && (
                      <div className="mt-0.5 truncate text-[10px] text-[var(--warning)]">
                        主副本有本地修改
                      </div>
                    )}
                  </div>
                  {EXT_PLATFORMS.map((platform) => {
                    const status = item.platforms[platform.id];
                    return (
                      <div
                        key={platform.id}
                        className="flex shrink-0 flex-col items-center gap-1"
                        title={status.targets
                          .map((target) => target.path)
                          .join("\n")}
                      >
                        <ToggleSwitch
                          checked={status.enabled}
                          disabled={busy}
                          testId={`ext-skill-${item.skill.id}-${platform.id}`}
                          ariaLabel={`${item.skill.name} 同步到 ${platform.label}`}
                          onChange={(enabled) =>
                            void run(() =>
                              applySnapshot(
                                api.toggleSkillPlatform(
                                  item.skill.id,
                                  platform.id,
                                  enabled,
                                ),
                              ),
                            )
                          }
                        />
                        <span
                          className={`h-3 whitespace-nowrap text-[9px] leading-3 ${status.hasDiff ? "text-[var(--warning)]" : "text-[var(--text-muted)]"}`}
                        >
                          {status.hasDiff
                            ? "有差异"
                            : status.actualMethod === "symlink"
                              ? "软链"
                              : status.actualMethod === "copy"
                                ? "复制"
                                : "未启用"}
                        </span>
                      </div>
                    );
                  })}
                  <div className="flex shrink-0 items-center justify-end gap-0.5 whitespace-nowrap">
                    <button
                      type="button"
                      disabled={
                        busy ||
                        !item.skill.updateAvailable ||
                        item.locallyModified
                      }
                      onClick={() =>
                        void run(() =>
                          applySnapshot(
                            api.updateInstalledSkill(item.skill.id),
                            "技能已更新",
                          ),
                        )
                      }
                      className={`inline-flex h-7 w-[54px] shrink-0 cursor-pointer items-center justify-center gap-0.5 rounded-[5px] text-[11px] disabled:cursor-default ${item.skill.updateAvailable ? "bg-[var(--success-soft)] font-medium text-[var(--success)] disabled:opacity-50" : "text-[var(--text-muted)]"}`}
                      title={
                        item.skill.source === "github"
                          ? "仓库版本状态"
                          : "本地技能"
                      }
                    >
                      {item.skill.updateAvailable ? (
                        <ArrowUpCircle className="h-3 w-3" />
                      ) : (
                        <Check className="h-3 w-3" />
                      )}
                      {item.skill.updateAvailable ? "更新" : "最新"}
                    </button>
                    <ExtMoreDropdown
                      iconOnly
                      label={`${item.skill.name} 的操作`}
                      disabled={busy}
                      items={[
                        {
                          label: "三端对齐",
                          icon: Link2,
                          onClick: () =>
                            void run(() =>
                              applySnapshot(
                                api.alignSkillPlatforms(
                                  item.skill.id,
                                  EXT_PLATFORMS.map((platform) => platform.id),
                                ),
                                "三端已按主副本对齐",
                              ),
                            ),
                        },
                        {
                          label: "彻底卸载",
                          icon: Trash2,
                          danger: true,
                          onClick: () => {
                            setDrawerError(null);
                            setUninstalling(item);
                          },
                        },
                      ]}
                    />
                  </div>
                </div>
              ))}
            </div>
            {installed.length === 0 && (
              <div className="flex flex-col items-center gap-2 py-12 text-center">
                <FolderInput className="h-7 w-7 text-[var(--text-muted)]" />
                <h3 className="text-[13px] font-medium">
                  {query ? "没有匹配的技能" : "暂无已安装技能"}
                </h3>
                {!query && (
                  <button
                    type="button"
                    onClick={openImport}
                    className="cursor-pointer text-[12px] text-[var(--accent)]"
                  >
                    导入技能
                  </button>
                )}
              </div>
            )}
          </>
        ) : (
          <div className="flex min-w-0 flex-col gap-4">
            <section className="min-w-0 border-y border-[var(--border)] py-3">
              <div className="flex min-w-0 items-center justify-between gap-2">
                <h3 className="text-[12px] font-semibold">GitHub 仓库源</h3>
                <button
                  type="button"
                  disabled={busy}
                  onClick={openRepos}
                  className={`${EXT_BUTTON} !h-7`}
                >
                  <Plus className="h-3 w-3" />
                  添加仓库
                </button>
              </div>
              {repoRows}
              {(snapshot?.repos.length ?? 0) === 0 && (
                <div className="py-3 text-[12px] text-[var(--text-muted)]">
                  暂无订阅仓库
                </div>
              )}
            </section>
            <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
              <span className="shrink-0 text-[11px] font-medium text-[var(--text-secondary)]">
                默认同步端
              </span>
              <ExtPlatformChoices
                value={snapshot?.defaultTargets ?? []}
                disabled={busy}
                onChange={(targets) =>
                  void run(() =>
                    applySnapshot(api.setSkillDefaultTargets(targets)),
                  )
                }
              />
            </div>
            <div className="grid min-w-0 grid-cols-2 gap-3">
              {available.map((skill) => {
                const installedItem = snapshot?.skills.find(
                  (item) =>
                    item.skill.repoOwner === skill.repoOwner &&
                    item.skill.repoName === skill.repoName &&
                    item.skill.repoSubdir === skill.repoSubdir &&
                    item.skill.repoBranch === skill.repoBranch,
                );
                return (
                  <article
                    key={`${skill.repoOwner}/${skill.repoName}/${skill.repoSubdir}`}
                    className="flex min-w-0 flex-col gap-2 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] p-3"
                  >
                    <div className="flex min-w-0 items-center gap-2">
                      <FolderSearch className="h-4 w-4 shrink-0 text-[var(--success)]" />
                      <h3
                        className="min-w-0 truncate text-[12.5px] font-semibold"
                        title={skill.name}
                      >
                        {skill.name}
                      </h3>
                    </div>
                    <p
                      className="min-w-0 truncate text-[11.5px] text-[var(--text-secondary)]"
                      title={skill.description}
                    >
                      {skill.description || skill.id}
                    </p>
                    <div className="mt-auto flex min-w-0 items-center gap-2">
                      <span
                        className="min-w-0 flex-1 truncate text-[10.5px] text-[var(--text-muted)]"
                        title={`${skill.repoOwner}/${skill.repoName}/${skill.repoSubdir}`}
                      >
                        {skill.repoOwner}/{skill.repoName}
                      </span>
                      <button
                        type="button"
                        disabled={
                          busy ||
                          Boolean(
                            installedItem &&
                            !installedItem.skill.updateAvailable,
                          )
                        }
                        onClick={() =>
                          void run(async () => {
                            await applySnapshot(
                              installedItem
                                ? api.updateInstalledSkill(
                                    installedItem.skill.id,
                                  )
                                : api.installDiscoveredSkill(
                                    skill.repoOwner,
                                    skill.repoName,
                                    skill.repoSubdir,
                                    snapshot?.defaultTargets ?? [],
                                  ),
                              installedItem ? "技能已更新" : "技能已安装",
                            );
                          })
                        }
                        className={`${EXT_BUTTON} !h-7 !px-2 !text-[11px]`}
                      >
                        {installedItem ? (
                          <Check className="h-3 w-3" />
                        ) : (
                          <Download className="h-3 w-3" />
                        )}
                        {installedItem
                          ? installedItem.skill.updateAvailable
                            ? "更新"
                            : "已安装"
                          : "安装"}
                      </button>
                    </div>
                  </article>
                );
              })}
            </div>
            {available.length === 0 && (
              <div className="flex flex-col items-center gap-2 py-8 text-center">
                <FolderSearch className="h-7 w-7 text-[var(--text-muted)]" />
                <span className="text-[12px] text-[var(--text-secondary)]">
                  {query
                    ? "没有匹配的技能"
                    : scannedRepos.length > 0
                      ? "仓库中暂无可发现技能"
                      : "尚未拉取仓库技能"}
                </span>
                {!query && (
                  <button
                    type="button"
                    disabled={
                      busy || !snapshot?.repos.some((repo) => repo.enabled)
                    }
                    onClick={() => void checkUpdates()}
                    className={EXT_BUTTON}
                  >
                    <RefreshCw className="h-3.5 w-3.5" />
                    拉取技能
                  </button>
                )}
              </div>
            )}
          </div>
        )}
      </section>
      {drawer === "import" && (
        <ExtSlideDrawer
          title="导入技能"
          busy={busy}
          onClose={() => setDrawer(null)}
          footer={
            <>
              <button
                type="button"
                disabled={busy}
                onClick={() => setDrawer(null)}
                className={EXT_BUTTON}
              >
                取消
              </button>
              <button
                type="button"
                data-testid="ext-skills-import-confirm"
                disabled={
                  busy ||
                  (importMode === "path"
                    ? !importPath.trim()
                    : !zipFile && !nativeZipPath)
                }
                onClick={() => void handleImport()}
                className={EXT_PRIMARY_BUTTON}
              >
                {busy ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Upload className="h-3.5 w-3.5" />
                )}
                导入
              </button>
            </>
          }
        >
          <div
            role="tablist"
            aria-label="导入来源"
            className="grid grid-cols-2 gap-1 rounded-[7px] bg-[var(--bg-pill)] p-1"
          >
            {(["zip", "path"] as const).map((mode) => (
              <button
                key={mode}
                type="button"
                role="tab"
                aria-selected={importMode === mode}
                disabled={busy}
                onClick={() => setImportMode(mode)}
                className={`h-8 cursor-pointer rounded-[5px] text-[12px] ${importMode === mode ? "bg-[var(--bg-card)] font-medium" : "text-[var(--text-secondary)]"}`}
              >
                {mode === "zip" ? "ZIP 文件" : "本地目录"}
              </button>
            ))}
          </div>
          <ExtError
            message={drawerError}
            onDismiss={() => setDrawerError(null)}
          />
          {importMode === "zip" ? (
            <label
              onDragOver={(event) => {
                event.preventDefault();
                if (!busy) setDragging(true);
              }}
              onDragLeave={() => setDragging(false)}
              onDrop={(event) => {
                event.preventDefault();
                setDragging(false);
                if (!busy && event.dataTransfer.files.length === 1)
                  chooseZip(event.dataTransfer.files[0]);
                else if (!busy) setDrawerError("请一次拖入一个 .zip 文件");
              }}
              className={`flex min-h-[168px] min-w-0 cursor-pointer flex-col items-center justify-center gap-3 rounded-[8px] border border-dashed p-4 text-center ${dragging ? "border-[var(--accent)] bg-[var(--accent-soft)]" : "border-[var(--border-strong)] bg-[var(--bg-subtle)]"}`}
            >
              <Upload className="h-7 w-7 text-[var(--text-muted)]" />
              <span className="max-w-full truncate text-[12px] text-[var(--text-secondary)]">
                {zipFile?.name ||
                  nativeZipPath?.split(/[\\/]/).pop() ||
                  "拖入 ZIP 文件或选择文件"}
              </span>
              <span className="text-[10.5px] text-[var(--text-muted)]">
                ZIP ≤ 200 MiB · 本地目录 ≤ 512 MiB
              </span>
              <input
                type="file"
                aria-label="选择 ZIP 文件"
                accept=".zip,application/zip"
                disabled={busy}
                onChange={(event) => {
                  const file = event.target.files?.[0];
                  if (file) chooseZip(file);
                }}
                className="peer sr-only"
              />
              <span className="inline-flex h-7 items-center gap-1.5 rounded-[5px] border border-[var(--border)] bg-[var(--bg-card)] px-2.5 text-[11px] text-[var(--text-primary)] peer-focus-visible:outline-2 peer-focus-visible:outline-[var(--accent)]">
                <FolderInput className="h-3.5 w-3.5" />
                选择 ZIP 文件
              </span>
            </label>
          ) : (
            <label className="flex min-w-0 flex-col gap-2 text-[12px] text-[var(--text-secondary)]">
              技能目录路径
              <input
                value={importPath}
                disabled={busy}
                onChange={(event) => setImportPath(event.target.value)}
                placeholder="包含 SKILL.md 的目录"
                className={EXT_INPUT}
              />
            </label>
          )}
          <fieldset className="min-w-0">
            <legend className="mb-2 text-[12px] font-medium">同步端</legend>
            <ExtPlatformChoices
              value={importTargets}
              onChange={setImportTargets}
              disabled={busy}
            />
          </fieldset>
        </ExtSlideDrawer>
      )}
      {drawer === "repos" && (
        <ExtSlideDrawer
          title="GitHub 仓库源"
          busy={busy}
          onClose={() => setDrawer(null)}
        >
          <ExtError
            message={drawerError}
            onDismiss={() => setDrawerError(null)}
          />
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void addRepo();
            }}
            className="flex min-w-0 flex-col gap-3"
          >
            <label className="flex min-w-0 flex-col gap-1.5 text-[12px]">
              仓库
              <input
                value={repoInput}
                disabled={busy}
                onChange={(event) => setRepoInput(event.target.value)}
                placeholder="owner/repo"
                className={EXT_INPUT}
                required
              />
            </label>
            <label className="flex min-w-0 flex-col gap-1.5 text-[12px]">
              分支
              <input
                value={branchInput}
                disabled={busy}
                onChange={(event) => setBranchInput(event.target.value)}
                placeholder="main"
                className={EXT_INPUT}
              />
            </label>
            <button
              type="submit"
              disabled={busy || !repoInput.trim()}
              className={`${EXT_PRIMARY_BUTTON} self-end`}
            >
              <Plus className="h-3.5 w-3.5" />
              添加仓库
            </button>
          </form>
          {repoRows}
        </ExtSlideDrawer>
      )}
      {uninstalling && (
        <ExtSlideDrawer
          title="彻底卸载技能"
          busy={busy}
          onClose={() => setUninstalling(null)}
          footer={
            <>
              <button
                type="button"
                disabled={busy}
                onClick={() => setUninstalling(null)}
                className={EXT_BUTTON}
              >
                取消
              </button>
              <button
                type="button"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await applySnapshot(
                      api.uninstallSkill(uninstalling.skill.id),
                      "技能已彻底卸载",
                    );
                    setUninstalling(null);
                  }, true)
                }
                className={`${EXT_BUTTON} !border-[var(--danger)] !bg-[var(--danger-soft)] !text-[var(--danger)]`}
              >
                <Trash2 className="h-3.5 w-3.5" />
                彻底卸载
              </button>
            </>
          }
        >
          <ExtError
            message={drawerError}
            onDismiss={() => setDrawerError(null)}
          />
          <div className="flex items-start gap-2 text-[13px]">
            <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-[var(--warning)]" />
            <p className="min-w-0 break-words">
              将删除“{uninstalling.skill.name}
              ”的三端副本及主副本。仅停用时，请取消并关闭平台开关。
            </p>
          </div>
          <code className="min-w-0 break-all text-[11px] text-[var(--text-muted)]">
            {uninstalling.masterPath}
          </code>
        </ExtSlideDrawer>
      )}
    </>
  );
}
