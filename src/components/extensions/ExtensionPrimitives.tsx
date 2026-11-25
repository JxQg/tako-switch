import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import {
  AlertCircle,
  Check,
  ChevronDown,
  CircleHelp,
  Download,
  MoreHorizontal,
  Search,
  Upload,
  X,
  type LucideIcon,
} from "lucide-react";
import type { ExtPlatform } from "../../lib/api";

export const EXT_PLATFORMS: { id: ExtPlatform; label: string }[] = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "antigravity", label: "Antigravity" },
];

export const EXT_BUTTON =
  "ext-button inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-lg border border-[var(--border)] bg-[var(--bg-card)] px-2.5 text-[12px] font-medium text-[var(--text-secondary)] hover:bg-[var(--bg-subtle)] hover:text-[var(--text-primary)] focus-visible:outline-2 focus-visible:outline-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-50 cursor-pointer";
export const EXT_PRIMARY_BUTTON =
  "ext-button inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-lg border border-transparent bg-[var(--accent)] px-3 text-[12px] font-semibold text-[var(--on-accent)] hover:bg-[var(--accent-hover)] focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-50 cursor-pointer";
export const EXT_ICON_BUTTON =
  "inline-flex h-[26px] w-[26px] shrink-0 items-center justify-center rounded-md text-[var(--text-secondary)] hover:bg-[var(--bg-subtle)] hover:text-[var(--text-primary)] focus-visible:outline-2 focus-visible:outline-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-40 cursor-pointer";
export const EXT_INPUT =
  "min-w-0 w-full rounded-lg border border-[var(--border)] bg-[var(--bg-canvas)] px-2.5 py-2 text-[12px] text-[var(--text-primary)] outline-none placeholder:text-[var(--text-muted)] focus:border-[var(--accent)]";

export interface ExtensionSecondaryAction {
  label: string;
  icon: LucideIcon;
  testId?: string;
  onClick: () => void;
  disabled?: boolean;
  spinning?: boolean;
}

export interface ExtensionActions {
  secondary?: ExtensionSecondaryAction;
  more?: ExtensionMenuItem[];
  primary?: {
    label: string;
    icon: LucideIcon;
    testId: string;
    onClick: () => void;
    disabled?: boolean;
  };
  disabled?: boolean;
}

export type ExtensionNavigationGuard = (navigate: () => void) => void;

export interface ExtensionMenuItem {
  label: string;
  icon: LucideIcon;
  onClick: () => void;
  checked?: boolean;
  disabled?: boolean;
  danger?: boolean;
}

export function ExtSearchInput({
  value,
  onChange,
  placeholder,
  testId,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  testId: string;
}) {
  return (
    <label className="ext-button flex flex-1 min-w-[150px] max-w-[280px] items-center gap-2 rounded-lg border border-[var(--border)] bg-[var(--bg-card)] px-2.5 text-[var(--text-muted)] focus-within:border-[var(--accent)]">
      <Search className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
      <input
        data-testid={testId}
        aria-label={placeholder}
        placeholder={placeholder}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        className="min-w-0 w-full truncate bg-transparent text-[12px] text-[var(--text-primary)] outline-none placeholder:text-[var(--text-muted)]"
      />
      {value && (
        <button
          type="button"
          onClick={() => onChange("")}
          title="清空搜索"
          aria-label="清空搜索"
          className="shrink-0 cursor-pointer"
        >
          <X className="h-3 w-3" />
        </button>
      )}
    </label>
  );
}

function useDismiss(open: boolean, onClose: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    if (!open) return;
    const dismiss = (event: PointerEvent) => {
      if (
        ref.current &&
        event.target instanceof Node &&
        !ref.current.contains(event.target)
      ) {
        closeRef.current();
      }
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        event.stopImmediatePropagation();
        closeRef.current();
      }
    };
    document.addEventListener("pointerdown", dismiss);
    window.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open]);
  return ref;
}

export function ExtHelpPopover({
  label = "路径说明",
  children,
}: {
  label?: string;
  children: ReactNode;
}) {
  const [hovered, setHovered] = useState(false);
  const [pinned, setPinned] = useState(false);
  const open = hovered || pinned;
  const close = useCallback(() => {
    setHovered(false);
    setPinned(false);
  }, []);
  const ref = useDismiss(open, close);
  const id = useId();
  return (
    <div
      ref={ref}
      className="relative shrink-0"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <button
        type="button"
        title={label}
        aria-label={label}
        aria-expanded={open}
        aria-describedby={open ? id : undefined}
        onClick={() => {
          setPinned((prev) => !prev);
          setHovered(false);
        }}
        onFocus={(event) => {
          if (event.currentTarget.matches(":focus-visible")) {
            setPinned(true);
          }
        }}
        className="h-[26px] w-[26px] rounded-full border border-[var(--border)] bg-[var(--bg-card)] text-[var(--text-muted)] flex items-center justify-center cursor-pointer hover:text-[var(--text-primary)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
      >
        <CircleHelp className="h-3.5 w-3.5" />
      </button>
      {open && (
        <div
          id={id}
          role="tooltip"
          className="absolute right-0 top-full z-40 mt-1 w-[min(340px,calc(100vw-32px))] rounded-lg border border-[var(--border)] bg-[var(--bg-card)] p-3 text-[11.5px] leading-relaxed text-[var(--text-secondary)] shadow-[var(--shadow-modal)] break-words"
        >
          {children}
        </div>
      )}
    </div>
  );
}

export function ExtMoreDropdown({
  items,
  disabled = false,
  iconOnly = false,
  label = "更多",
}: {
  items: ExtensionMenuItem[];
  disabled?: boolean;
  iconOnly?: boolean;
  label?: string;
}) {
  const [open, setOpen] = useState(false);
  const close = useCallback(() => setOpen(false), []);
  const ref = useDismiss(open, close);
  return (
    <div className="relative shrink-0" ref={ref}>
      {iconOnly ? (
        <button
          type="button"
          title={label}
          aria-label={label}
          aria-haspopup="menu"
          aria-expanded={open}
          disabled={disabled}
          onClick={() => setOpen((value) => !value)}
          className={EXT_ICON_BUTTON}
        >
          <MoreHorizontal className="h-3.5 w-3.5" />
        </button>
      ) : (
        <button
          type="button"
          className={EXT_BUTTON}
          aria-label={label}
          aria-haspopup="menu"
          aria-expanded={open}
          disabled={disabled}
          onClick={() => setOpen((value) => !value)}
        >
          <span>{label}</span>
          <ChevronDown className="h-3 w-3" />
        </button>
      )}
      {open && (
        <div
          role="menu"
          className="absolute right-0 top-full z-40 mt-1 w-[min(220px,calc(100vw-24px))] rounded-lg border border-[var(--border)] bg-[var(--bg-card)] p-1 shadow-[var(--shadow-modal)]"
        >
          {items.map(
            ({
              label: itemLabel,
              icon: Icon,
              onClick,
              checked,
              disabled: itemDisabled,
              danger,
            }) => {
              const isRadio = checked !== undefined;
              return (
                <button
                  key={itemLabel}
                  role={isRadio ? "menuitemradio" : "menuitem"}
                  aria-checked={isRadio ? checked : undefined}
                  type="button"
                  disabled={itemDisabled}
                  onClick={() => {
                    close();
                    onClick();
                  }}
                  className={`flex w-full min-w-0 items-center gap-2 rounded-md px-2.5 py-2 text-left text-[12px] hover:bg-[var(--bg-subtle)] disabled:opacity-40 cursor-pointer ${danger ? "text-[var(--danger)]" : "text-[var(--text-secondary)]"}`}
                >
                  <Icon className="h-3.5 w-3.5 shrink-0" />
                  <span className="min-w-0 flex-1 truncate">{itemLabel}</span>
                  {checked && (
                    <Check className="h-3.5 w-3.5 shrink-0 text-[var(--accent)]" />
                  )}
                </button>
              );
            },
          )}
        </div>
      )}
    </div>
  );
}

export function ExtSlideDrawer({
  title,
  onClose,
  children,
  footer,
  busy = false,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  busy?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const id = useId();
  const closeRef = useRef(onClose);
  const busyRef = useRef(busy);
  closeRef.current = onClose;
  busyRef.current = busy;

  useEffect(() => {
    const previous =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const focusable = () =>
      Array.from(
        ref.current?.querySelectorAll<HTMLElement>(
          "button:not(:disabled), input:not(:disabled), textarea:not(:disabled), select:not(:disabled), [tabindex='0']",
        ) ?? [],
      );
    (
      ref.current?.querySelector<HTMLElement>(
        "input:not(:disabled):not([type='file']), textarea:not(:disabled), select:not(:disabled)",
      ) ?? focusable()[0]
    )?.focus();

    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        event.stopImmediatePropagation();
        if (!busyRef.current) {
          closeRef.current();
        }
      }
      if (event.key === "Tab") {
        const items = focusable();
        const first = items[0];
        const last = items[items.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last?.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first?.focus();
        }
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      previous?.focus();
    };
  }, []);

  return createPortal(
    <div
      className="fixed inset-0 z-50 modal-backdrop flex justify-end overflow-hidden"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !busy) {
          onClose();
        }
      }}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={id}
        className="flex h-full w-[min(500px,94vw)] min-w-0 flex-col border-l border-[var(--border)] bg-[var(--bg-card)] shadow-[var(--shadow-modal)]"
      >
        <header className="flex h-[48px] shrink-0 items-center justify-between gap-2 border-b border-[var(--border)] px-4">
          <h2
            id={id}
            className="min-w-0 truncate text-[14px] font-semibold text-[var(--text-primary)]"
          >
            {title}
          </h2>
          <button
            type="button"
            disabled={busy}
            onClick={onClose}
            title="关闭抽屉"
            aria-label="关闭抽屉"
            className={EXT_ICON_BUTTON}
          >
            <X className="h-4 w-4" />
          </button>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto subtle-scroll p-4 flex flex-col gap-3">
          {children}
        </div>
        {footer && (
          <footer className="flex shrink-0 items-center justify-end gap-2 border-t border-[var(--border)] px-4 py-3">
            {footer}
          </footer>
        )}
      </div>
    </div>,
    document.body,
  );
}

export function ExtPlatformChoices({
  value,
  onChange,
  disabled = false,
  platforms = EXT_PLATFORMS,
}: {
  value: ExtPlatform[];
  onChange: (targets: ExtPlatform[]) => void;
  disabled?: boolean;
  platforms?: { id: ExtPlatform; label: string }[];
}) {
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
      {platforms.map((platform) => (
        <label
          key={platform.id}
          className="inline-flex shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap text-[12px] text-[var(--text-secondary)]"
        >
          <input
            type="checkbox"
            checked={value.includes(platform.id)}
            disabled={disabled}
            onChange={(event) =>
              onChange(
                event.target.checked
                  ? [...value, platform.id]
                  : value.filter((target) => target !== platform.id),
              )
            }
            className="h-3.5 w-3.5 accent-[var(--accent)]"
          />
          {platform.label}
        </label>
      ))}
    </div>
  );
}

export function ExtMatrixHeader({
  columns = "minmax(0, 1fr) 70px 64px 76px 86px",
  leadingLabel = "名称 / 配置",
  trailingLabel = "操作",
}: {
  columns?: string;
  leadingLabel?: string;
  trailingLabel?: string;
}) {
  return (
    <div
      className="grid min-w-0 items-center gap-1 border-b border-[var(--border)] bg-[var(--bg-subtle)] px-3 py-2 text-[11px] text-[var(--text-muted)]"
      style={{ gridTemplateColumns: columns }}
    >
      <span className="min-w-0 truncate">{leadingLabel}</span>
      {EXT_PLATFORMS.map((platform) => (
        <span key={platform.id} className="text-center whitespace-nowrap">
          {platform.label}
        </span>
      ))}
      <span className="text-right whitespace-nowrap">{trailingLabel}</span>
    </div>
  );
}

export function ExtError({
  message,
  onDismiss,
}: {
  message: string | null;
  onDismiss?: () => void;
}) {
  if (!message) return null;
  return (
    <div
      role="alert"
      className="flex min-w-0 items-start gap-2 rounded-lg border border-[var(--danger)]/30 bg-[var(--danger-soft)] px-3 py-2 text-[12px] text-[var(--danger)]"
    >
      <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
      <span className="min-w-0 flex-1 break-words [overflow-wrap:anywhere]">
        {message}
      </span>
      {onDismiss && (
        <button
          type="button"
          aria-label="关闭错误提示"
          title="关闭错误提示"
          onClick={onDismiss}
          className="shrink-0 cursor-pointer"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  );
}

export function useExtensionBackup(
  exportBackup: () => Promise<string>,
  importBackup: (backup: string) => Promise<void>,
) {
  const [open, setOpen] = useState(false);
  const [mode, setMode] = useState<"export" | "import">("export");
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const close = useCallback(() => setOpen(false), []);
  const begin = useCallback(
    async (next: "export" | "import") => {
      setMode(next);
      setError(null);
      setDraft("");
      setOpen(true);
      if (next === "export") {
        setBusy(true);
        try {
          setDraft(await exportBackup());
        } catch (err) {
          setError(String(err));
        } finally {
          setBusy(false);
        }
      }
    },
    [exportBackup],
  );
  const download = () => {
    const url = URL.createObjectURL(
      new Blob([draft], { type: "application/json" }),
    );
    const link = document.createElement("a");
    link.href = url;
    link.download = "tako-extensions-backup.json";
    link.click();
    window.setTimeout(() => URL.revokeObjectURL(url), 1000);
  };
  const restore = async () => {
    setBusy(true);
    setError(null);
    try {
      await importBackup(draft);
      close();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };
  const drawer = open && (
    <ExtSlideDrawer
      title={mode === "export" ? "导出 JSON 备份" : "导入 JSON 备份"}
      busy={busy}
      onClose={close}
      footer={
        <button
          type="button"
          className={EXT_PRIMARY_BUTTON}
          disabled={busy || !draft.trim()}
          onClick={mode === "export" ? download : () => void restore()}
        >
          {mode === "export" ? (
            <Download className="h-3.5 w-3.5" />
          ) : (
            <Upload className="h-3.5 w-3.5" />
          )}
          {busy ? "处理中..." : mode === "export" ? "下载备份" : "导入并恢复"}
        </button>
      }
    >
      <ExtError message={error} />
      {mode === "import" && (
        <input
          type="file"
          accept="application/json,.json"
          aria-label="选择 JSON 备份文件"
          className="min-w-0 max-w-full text-[12px] text-[var(--text-secondary)]"
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (file)
              void file
                .text()
                .then(setDraft)
                .catch((err: unknown) => setError(String(err)));
          }}
        />
      )}
      <textarea
        aria-label="JSON 备份内容"
        readOnly={mode === "export"}
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        className={`${EXT_INPUT} min-h-[300px] resize-y font-mono`}
      />
    </ExtSlideDrawer>
  );
  return { begin, drawer };
}
