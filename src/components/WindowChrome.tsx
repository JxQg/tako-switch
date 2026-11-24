import { useCallback, useEffect, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type ResizeDirection = Parameters<
  ReturnType<typeof getCurrentWindow>["startResizeDragging"]
>[0];

export interface NativeWindowState {
  isMaximized: boolean;
  isFocused: boolean;
  minimize: () => void;
  toggleMaximize: () => void;
  close: () => void;
  startResize: (direction: ResizeDirection) => void;
}

export function useNativeWindowState(enabled: boolean): NativeWindowState {
  const [isMaximized, setIsMaximized] = useState(false);
  const [isFocused, setIsFocused] = useState(true);

  useEffect(() => {
    if (!enabled || !isTauri()) return;
    const win = getCurrentWindow();
    let disposed = false;

    const syncMaximized = async () => {
      try {
        const max = await win.isMaximized();
        if (!disposed) setIsMaximized(max);
      } catch {
        // ignore window query error
      }
    };

    void syncMaximized();
    void win
      .isFocused()
      .then((focused) => {
        if (!disposed) setIsFocused(focused);
      })
      .catch(() => {});

    let unlistenResize: (() => void) | undefined;
    let unlistenFocus: (() => void) | undefined;

    void win
      .onResized(() => {
        void syncMaximized();
      })
      .then((fn) => {
        if (disposed) fn();
        else unlistenResize = fn;
      })
      .catch(() => {});

    void win
      .onFocusChanged(({ payload }) => {
        if (!disposed) setIsFocused(payload);
      })
      .then((fn) => {
        if (disposed) fn();
        else unlistenFocus = fn;
      })
      .catch(() => {});

    return () => {
      disposed = true;
      unlistenResize?.();
      unlistenFocus?.();
    };
  }, [enabled]);

  const minimize = useCallback(() => {
    if (!isTauri()) return;
    void getCurrentWindow()
      .minimize()
      .catch(() => {});
  }, []);

  const toggleMaximize = useCallback(() => {
    if (!isTauri()) return;
    const win = getCurrentWindow();
    void win
      .toggleMaximize()
      .then(() => win.isMaximized())
      .then((max) => setIsMaximized(max))
      .catch(() => {});
  }, []);

  const close = useCallback(() => {
    if (!isTauri()) return;
    void getCurrentWindow()
      .close()
      .catch(() => {});
  }, []);

  const startResize = useCallback((direction: ResizeDirection) => {
    if (!isTauri()) return;
    void getCurrentWindow()
      .startResizeDragging(direction)
      .catch(() => {});
  }, []);

  return {
    isMaximized,
    isFocused,
    minimize,
    toggleMaximize,
    close,
    startResize,
  };
}

/**
 * Transparent resize handles for undecorated Windows frame.
 * WebView2 covers the client area and intercepts top/corner mouse events;
 * these slim handles forward mousedown directly to Win32 startResizeDragging
 * and automatically unmount when maximized so top-right corner (max_x, 0)
 * obeys Fitts's Law for the Close button.
 */
export function WindowResizeHandles({
  enabled,
  onStartResize,
}: {
  enabled: boolean;
  onStartResize: (direction: ResizeDirection) => void;
}) {
  if (!enabled) return null;

  const bindEdge =
    (dir: ResizeDirection) => (e: React.MouseEvent<HTMLDivElement>) => {
      if (e.button !== 0) return;
      e.preventDefault();
      e.stopPropagation();
      onStartResize(dir);
    };

  return (
    <div aria-hidden="true" data-tauri-drag-region="false">
      {/* Top edge & top corners (primary gap where WebView2 shadows parent WM_NCHITTEST) */}
      <div
        onMouseDown={bindEdge("North")}
        className="fixed top-0 left-[10px] right-[10px] h-[5px] z-[60] cursor-ns-resize"
      />
      <div
        onMouseDown={bindEdge("NorthWest")}
        className="fixed top-0 left-0 w-[10px] h-[10px] z-[61] cursor-nwse-resize"
      />
      <div
        onMouseDown={bindEdge("NorthEast")}
        className="fixed top-0 right-0 w-[10px] h-[6px] z-[61] cursor-nesw-resize"
      />
      {/* Ultra-slim fallback edges for left/right/bottom in environments without DWM frame */}
      <div
        onMouseDown={bindEdge("West")}
        className="fixed top-[10px] bottom-[10px] left-0 w-[3px] z-[60] cursor-ew-resize"
      />
      <div
        onMouseDown={bindEdge("East")}
        className="fixed top-[6px] bottom-[10px] right-0 w-[3px] z-[60] cursor-ew-resize"
      />
      <div
        onMouseDown={bindEdge("South")}
        className="fixed bottom-0 left-[10px] right-[10px] h-[3px] z-[60] cursor-ns-resize"
      />
      <div
        onMouseDown={bindEdge("SouthWest")}
        className="fixed bottom-0 left-0 w-[10px] h-[10px] z-[61] cursor-nesw-resize"
      />
      <div
        onMouseDown={bindEdge("SouthEast")}
        className="fixed bottom-0 right-0 w-[10px] h-[10px] z-[61] cursor-nwse-resize"
      />
    </div>
  );
}

/**
 * Native-grade Windows 10/11 caption button group (Minimize / Maximize-Restore / Close)
 * aligned flush with the top-right window edge and synced with the app's theme tokens.
 */
export function WindowsCaptionControls({
  isMaximized,
  isFocused,
  onMinimize,
  onToggleMaximize,
  onClose,
}: {
  isMaximized: boolean;
  isFocused: boolean;
  onMinimize: () => void;
  onToggleMaximize: () => void;
  onClose: () => void;
}) {
  const idleTone = isFocused
    ? "text-[var(--text-secondary)]"
    : "text-[var(--text-muted)]/75";

  return (
    <div
      data-tauri-drag-region="false"
      className="flex items-stretch h-full shrink-0 ml-1"
    >
      <button
        type="button"
        data-testid="win-btn-minimize"
        tabIndex={-1}
        onClick={onMinimize}
        title="最小化"
        aria-label="最小化窗口"
        className={`w-[46px] h-full inline-flex items-center justify-center transition-colors duration-100 cursor-pointer ${idleTone} hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)] active:bg-[var(--border)]`}
      >
        <svg
          width="10"
          height="10"
          viewBox="0 0 10 10"
          fill="none"
          aria-hidden="true"
        >
          <path
            d="M1 5H9"
            stroke="currentColor"
            strokeWidth="1.1"
            strokeLinecap="round"
          />
        </svg>
      </button>

      <button
        type="button"
        data-testid="win-btn-maximize"
        tabIndex={-1}
        onClick={onToggleMaximize}
        title={isMaximized ? "向下还原" : "最大化"}
        aria-label={isMaximized ? "向下还原窗口" : "最大化窗口"}
        className={`w-[46px] h-full inline-flex items-center justify-center transition-colors duration-100 cursor-pointer ${idleTone} hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)] active:bg-[var(--border)]`}
      >
        {isMaximized ? (
          <svg
            width="10"
            height="10"
            viewBox="0 0 10 10"
            fill="none"
            aria-hidden="true"
          >
            <path
              d="M3 2.5V1.5C3 1.22386 3.22386 1 3.5 1H8.5C8.77614 1 9 1.22386 9 1.5V6.5C9 6.77614 8.77614 7 8.5 7H7.5"
              stroke="currentColor"
              strokeWidth="1.05"
            />
            <rect
              x="1"
              y="3"
              width="6"
              height="6"
              rx="0.7"
              stroke="currentColor"
              strokeWidth="1.05"
            />
          </svg>
        ) : (
          <svg
            width="10"
            height="10"
            viewBox="0 0 10 10"
            fill="none"
            aria-hidden="true"
          >
            <rect
              x="1.25"
              y="1.25"
              width="7.5"
              height="7.5"
              rx="0.9"
              stroke="currentColor"
              strokeWidth="1.1"
            />
          </svg>
        )}
      </button>

      <button
        type="button"
        data-testid="win-btn-close"
        tabIndex={-1}
        onClick={onClose}
        title="关闭"
        aria-label="关闭窗口"
        className={`w-[46px] h-full inline-flex items-center justify-center transition-colors duration-100 cursor-pointer ${idleTone} hover:bg-[#e81123] hover:text-white active:bg-[#c50f1f] active:text-white`}
      >
        <svg
          width="10"
          height="10"
          viewBox="0 0 10 10"
          fill="none"
          aria-hidden="true"
        >
          <path
            d="M1.5 1.5L8.5 8.5M8.5 1.5L1.5 8.5"
            stroke="currentColor"
            strokeWidth="1.15"
            strokeLinecap="round"
          />
        </svg>
      </button>
    </div>
  );
}
