import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { HelpCircle, Search, X } from "lucide-react";
import type { ExtPlatform } from "../../lib/api";

export const EXT_PLATFORMS: { id: ExtPlatform; label: string }[] = [
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "antigravity", label: "Antigravity" },
];

export const EXT_BUTTON =
  "h-[var(--density-ctrl-h)] shrink-0 whitespace-nowrap rounded-lg border border-[var(--border)] bg-[var(--bg-card)] px-2.5 text-[12px] font-medium text-[var(--text-primary)] inline-flex items-center justify-center gap-1.5 hover:bg-[var(--bg-subtle)] focus-visible:outline-2 focus-visible:outline-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-45 cursor-pointer";
export const EXT_PRIMARY_BUTTON = `${EXT_BUTTON} border-[var(--accent-border)] bg-[var(--accent-soft)] text-[var(--accent)] hover:bg-[var(--accent-border)]`;

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
    <label className="flex flex-1 min-w-[150px] max-w-[280px] h-[var(--density-ctrl-h)] items-center gap-2 rounded-lg border border-[var(--border)] bg-[var(--bg-card)] px-2.5 focus-within:border-[var(--accent)]">
      <Search
        aria-hidden="true"
        className="h-3.5 w-3.5 shrink-0 text-[var(--text-muted)]"
      />
      <input
        type="search"
        aria-label={placeholder.replace(/\.\.\.$/, "")}
        data-testid={testId}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        className="min-w-0 w-full truncate bg-transparent text-[12px] text-[var(--text-primary)] placeholder:text-[var(--text-muted)] outline-none"
      />
    </label>
  );
}

export function ExtHelpPopover({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const id = useId();
  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopImmediatePropagation();
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", close);
    window.addEventListener("keydown", escape, true);
    return () => {
      document.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", escape, true);
    };
  }, [open]);
  return (
    <div
      className="relative shrink-0"
      ref={ref}
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => setOpen(false)}
    >
      <button
        type="button"
        aria-label={label}
        title={label}
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen(true)}
        onFocus={(event) => {
          if (event.currentTarget.matches(":focus-visible")) setOpen(true);
        }}
        className="h-[26px] w-[26px] rounded-md inline-flex items-center justify-center text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)] focus-visible:outline-2 focus-visible:outline-[var(--accent)] cursor-pointer"
      >
        <HelpCircle className="h-4 w-4" />
      </button>
      {open && (
        <div
          id={id}
          role="tooltip"
          className="absolute right-0 top-full z-40 mt-1 w-[min(320px,calc(100vw-40px))] rounded-lg border border-[var(--border)] bg-[var(--bg-card)] p-3 text-[12px] leading-relaxed text-[var(--text-secondary)] shadow-[var(--shadow-modal)] break-words"
        >
          {children}
        </div>
      )}
    </div>
  );
}

export function ExtSlideDrawer({
  title,
  onClose,
  children,
  busy = false,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  busy?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const id = useId();
  const busyRef = useRef(busy);
  const closeRef = useRef(onClose);
  busyRef.current = busy;
  closeRef.current = onClose;
  useEffect(() => {
    const previous =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const drawer = ref.current;
    (
      drawer?.querySelector<HTMLElement>("input") ??
      drawer?.querySelector<HTMLElement>("button")
    )?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        if (!busyRef.current) closeRef.current();
      }
      if (event.key === "Tab" && drawer) {
        const elements = Array.from(
          drawer.querySelectorAll<HTMLElement>(
            "button:not(:disabled), input:not(:disabled), [tabindex='0']",
          ),
        );
        const first = elements[0];
        const last = elements[elements.length - 1];
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
      className="fixed inset-0 z-50 modal-backdrop flex justify-end"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !busy) onClose();
      }}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={id}
        className="drawer-enter h-full w-[min(420px,94vw)] min-w-0 border-l border-[var(--border)] bg-[var(--bg-canvas)] shadow-[var(--shadow-modal)] flex flex-col"
      >
        <header className="flex shrink-0 items-center justify-between gap-2 border-b border-[var(--border)] bg-[var(--bg-card)] px-4 py-3">
          <h2 id={id} className="min-w-0 truncate text-[14px] font-semibold">
            {title}
          </h2>
          <button
            type="button"
            aria-label="关闭"
            title="关闭"
            disabled={busy}
            onClick={onClose}
            className="h-7 w-7 shrink-0 rounded-md flex items-center justify-center text-[var(--text-muted)] hover:bg-[var(--bg-hover)] cursor-pointer disabled:opacity-45"
          >
            <X className="h-4 w-4" />
          </button>
        </header>
        <div className="min-h-0 overflow-y-auto subtle-scroll p-4">
          {children}
        </div>
      </div>
    </div>,
    document.body,
  );
}
