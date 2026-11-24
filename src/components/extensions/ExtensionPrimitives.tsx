import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import {
  ChevronDown,
  CircleHelp,
  MoreHorizontal,
  Search,
  X,
  type LucideIcon,
} from "lucide-react";
import type { ExtPlatform } from "../../lib/api";

export const EXT_PLATFORMS: { id: ExtPlatform; label: string }[] = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "antigravity", label: "Antigravity" },
];

export const extButtonClass =
  "inline-flex h-[var(--density-ctrl-h)] shrink-0 cursor-pointer items-center justify-center gap-1.5 whitespace-nowrap rounded-[7px] border border-[var(--border)] bg-[var(--bg-card)] px-2.5 text-[12px] font-medium text-[var(--text-primary)] transition hover:bg-[var(--bg-hover)] focus-visible:outline-2 focus-visible:outline-[var(--accent)] disabled:cursor-default disabled:opacity-50";
export const extPrimaryClass =
  "inline-flex h-[var(--density-ctrl-h)] shrink-0 cursor-pointer items-center justify-center gap-1.5 whitespace-nowrap rounded-[7px] border border-transparent bg-[var(--accent)] px-3 text-[12px] font-semibold text-[var(--accent-fg)] transition hover:bg-[var(--accent-hover)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] disabled:cursor-default disabled:opacity-50";
export const extInputClass =
  "h-[var(--density-ctrl-h)] w-full min-w-0 rounded-[7px] border border-[var(--border)] bg-[var(--bg-card)] px-2.5 text-[12px] text-[var(--text-primary)] outline-none placeholder:text-[var(--text-muted)] focus:border-[var(--accent)]";

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
    <label className="flex h-[var(--density-ctrl-h)] min-w-[150px] max-w-[280px] flex-1 items-center gap-1.5 rounded-[7px] border border-[var(--border)] bg-[var(--bg-card)] px-2.5 text-[var(--text-muted)] focus-within:border-[var(--accent)]">
      <Search className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
      <input
        type="search"
        data-testid={testId}
        aria-label={placeholder}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        className="min-w-0 flex-1 truncate bg-transparent text-[12px] text-[var(--text-primary)] outline-none placeholder:text-[var(--text-muted)]"
      />
    </label>
  );
}

function usePopover(open: boolean, onClose: () => void) {
  const trigger = useRef<HTMLButtonElement>(null);
  const surface = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ top: 0, right: 8 });
  useLayoutEffect(() => {
    if (!open) return;
    const update = () => {
      const rect = trigger.current?.getBoundingClientRect();
      if (!rect) return;
      const height = surface.current?.offsetHeight ?? 0;
      const top =
        rect.bottom + height + 8 > window.innerHeight
          ? Math.max(8, rect.top - height - 6)
          : rect.bottom + 6;
      setPosition({
        top,
        right: Math.max(
          8,
          Math.min(window.innerWidth - rect.right, window.innerWidth - 288),
        ),
      });
    };
    update();
    const closeOutside = (event: PointerEvent) => {
      if (
        event.target instanceof Node &&
        !trigger.current?.contains(event.target) &&
        !surface.current?.contains(event.target)
      )
        onClose();
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        event.stopImmediatePropagation();
        onClose();
        trigger.current?.focus();
      }
    };
    window.addEventListener("pointerdown", closeOutside);
    window.addEventListener("keydown", closeOnEscape, true);
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.removeEventListener("pointerdown", closeOutside);
      window.removeEventListener("keydown", closeOnEscape, true);
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [open, onClose]);
  return { trigger, surface, position };
}

export function ExtHelpPopover({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const close = useCallback(() => setOpen(false), []);
  const { trigger, surface, position } = usePopover(open, close);
  const id = useId();
  return (
    <>
      <button
        ref={trigger}
        type="button"
        title={label}
        aria-label={label}
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        onClick={() => setOpen(true)}
        onMouseEnter={() => setOpen(true)}
        onMouseLeave={(event) => {
          if (
            !event.relatedTarget ||
            !(event.relatedTarget instanceof Node) ||
            !surface.current?.contains(event.relatedTarget)
          )
            setOpen(false);
        }}
        onFocus={() => setOpen(true)}
        className="inline-flex h-[26px] w-[26px] shrink-0 cursor-pointer items-center justify-center rounded-full text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)] focus-visible:outline-2 focus-visible:outline-[var(--accent)]"
      >
        <CircleHelp className="h-4 w-4" />
      </button>
      {open &&
        createPortal(
          <div
            id={id}
            ref={surface}
            role="tooltip"
            style={position}
            onMouseLeave={() => setOpen(false)}
            className="fixed z-[80] w-[280px] max-w-[calc(100vw-16px)] rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] p-3 text-[12px] leading-relaxed text-[var(--text-secondary)] shadow-[var(--shadow-modal)] break-words"
          >
            {children}
          </div>,
          document.body,
        )}
    </>
  );
}

export interface ExtMenuItem {
  label: string;
  icon?: LucideIcon;
  onSelect: () => void;
  danger?: boolean;
  checked?: boolean;
  disabled?: boolean;
}

export function ExtMoreDropdown({
  items,
  disabled = false,
  iconOnly = false,
  label = "更多",
}: {
  items: ExtMenuItem[];
  disabled?: boolean;
  iconOnly?: boolean;
  label?: string;
}) {
  const [open, setOpen] = useState(false);
  const close = useCallback(() => setOpen(false), []);
  const { trigger, surface, position } = usePopover(open, close);
  const id = useId();
  return (
    <>
      <button
        ref={trigger}
        type="button"
        disabled={disabled}
        onClick={() => setOpen((value) => !value)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") {
            event.preventDefault();
            setOpen(true);
            window.setTimeout(
              () =>
                surface.current
                  ?.querySelector<HTMLButtonElement>("button:not(:disabled)")
                  ?.focus(),
              0,
            );
          }
        }}
        title={label}
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        className={
          iconOnly
            ? `${extButtonClass} !h-7 !w-7 !px-0 !border-transparent !bg-transparent`
            : extButtonClass
        }
      >
        {iconOnly ? (
          <MoreHorizontal className="h-4 w-4" />
        ) : (
          <>
            {label}
            <ChevronDown className="h-3 w-3" />
          </>
        )}
      </button>
      {open &&
        createPortal(
          <div
            id={id}
            ref={surface}
            role="menu"
            aria-label={label}
            style={position}
            onKeyDown={(event) => {
              if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key))
                return;
              event.preventDefault();
              const buttons = Array.from(
                event.currentTarget.querySelectorAll<HTMLButtonElement>(
                  "button:not(:disabled)",
                ),
              );
              const index = buttons.indexOf(
                document.activeElement as HTMLButtonElement,
              );
              const next =
                event.key === "Home"
                  ? 0
                  : event.key === "End"
                    ? buttons.length - 1
                    : (index +
                        (event.key === "ArrowDown" ? 1 : -1) +
                        buttons.length) %
                      buttons.length;
              buttons[next]?.focus();
            }}
            className="fixed z-[80] flex w-[244px] max-w-[calc(100vw-16px)] flex-col gap-0.5 rounded-[8px] border border-[var(--border)] bg-[var(--bg-card)] p-1 shadow-[var(--shadow-modal)]"
          >
            {items.map(
              ({
                label: text,
                icon: Icon,
                onSelect,
                danger,
                checked,
                disabled: itemDisabled,
              }) => (
                <button
                  key={text}
                  type="button"
                  role={checked === undefined ? "menuitem" : "menuitemradio"}
                  aria-checked={checked}
                  disabled={itemDisabled}
                  onClick={() => {
                    setOpen(false);
                    onSelect();
                  }}
                  className={`flex min-h-8 w-full cursor-pointer items-center gap-2 rounded-[5px] px-2 text-left text-[12px] hover:bg-[var(--bg-hover)] focus-visible:bg-[var(--bg-hover)] focus-visible:outline-none disabled:opacity-45 ${danger ? "text-[var(--danger)]" : "text-[var(--text-primary)]"}`}
                >
                  {Icon && <Icon className="h-3.5 w-3.5 shrink-0" />}
                  <span className="min-w-0 flex-1">{text}</span>
                  {checked !== undefined && (
                    <span
                      aria-hidden="true"
                      className={`h-2 w-2 shrink-0 rounded-full ${checked ? "bg-[var(--accent)]" : "border border-[var(--border-strong)]"}`}
                    />
                  )}
                </button>
              ),
            )}
          </div>,
          document.body,
        )}
    </>
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
  const closeRef = useRef(onClose);
  const busyRef = useRef(busy);
  closeRef.current = onClose;
  busyRef.current = busy;
  const id = useId();
  useEffect(() => {
    const previous = document.activeElement;
    ref.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (document.querySelector('[role="menu"]')) return;
        event.preventDefault();
        event.stopPropagation();
        if (!busyRef.current) closeRef.current();
      }
      if (event.key === "Tab") {
        const nodes = Array.from(
          ref.current?.querySelectorAll<HTMLElement>(
            'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex="0"]',
          ) ?? [],
        );
        const first = nodes[0];
        const last = nodes[nodes.length - 1];
        if (
          event.shiftKey &&
          (document.activeElement === first ||
            document.activeElement === ref.current)
        ) {
          event.preventDefault();
          last?.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first?.focus();
        }
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      if (previous instanceof HTMLElement) previous.focus();
    };
  }, []);
  return createPortal(
    <div
      className="fixed inset-0 z-50 flex justify-end bg-[var(--backdrop)]"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !busy) onClose();
      }}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={id}
        tabIndex={-1}
        className="flex h-full w-[min(460px,100vw)] min-w-0 flex-col border-l border-[var(--border)] bg-[var(--bg-canvas)] shadow-[var(--shadow-modal)] outline-none"
      >
        <div className="flex h-[52px] shrink-0 items-center justify-between gap-2 border-b border-[var(--border)] bg-[var(--bg-card)] px-4">
          <h2 id={id} className="min-w-0 truncate text-[14px] font-semibold">
            {title}
          </h2>
          <button
            type="button"
            title="关闭"
            aria-label="关闭抽屉"
            disabled={busy}
            onClick={onClose}
            className={`${extButtonClass} !h-7 !w-7 !border-transparent !px-0`}
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        <div className="subtle-scroll flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4">
          {children}
        </div>
        {footer && (
          <div className="flex shrink-0 items-center justify-end gap-2 border-t border-[var(--border)] bg-[var(--bg-card)] p-4">
            {footer}
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}
