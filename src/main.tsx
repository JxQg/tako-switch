import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import { api } from "./lib/api";

function installGlobalFrontendErrorReporter() {
  if (typeof window === "undefined") return;
  const seen = new Set<string>();
  let sentCount = 0;
  const MAX_REPORTS_PER_SESSION = 10;

  const sendOnce = (
    source: "frontend_error" | "unhandled_rejection",
    errorType: string,
    message: string,
    location: string,
    stack: string,
  ) => {
    const trimmedMsg = message.trim();
    if (!trimmedMsg) return;
    // Ignore benign browser layout observer noise.
    if (trimmedMsg.includes("ResizeObserver loop")) return;
    if (sentCount >= MAX_REPORTS_PER_SESSION) return;

    const key = `${source}|${errorType}|${location}|${trimmedMsg.slice(0, 80)}`;
    if (seen.has(key)) return;
    seen.add(key);
    sentCount += 1;

    void api.reportFrontendError({
      source,
      category: "ui",
      errorType: errorType || "FrontendError",
      message: trimmedMsg.slice(0, 500),
      location: location.slice(0, 180),
      stack: stack.slice(0, 2800),
      context: {
        href: window.location.pathname || "/",
      },
    });
  };

  window.addEventListener("error", (event) => {
    const err = event.error instanceof Error ? event.error : null;
    const msg = event.message || err?.message || "Unhandled window error";
    const errType = err?.name || "WindowError";
    const loc = event.filename
      ? `${event.filename}:${event.lineno || 0}:${event.colno || 0}`
      : "";
    const stack = err?.stack || "";
    sendOnce("frontend_error", errType, msg, loc, stack);
  });

  window.addEventListener("unhandledrejection", (event) => {
    const reason = event.reason;
    const err = reason instanceof Error ? reason : null;
    const msg =
      err?.message ||
      (typeof reason === "string"
        ? reason
        : (() => {
            try {
              return JSON.stringify(reason);
            } catch {
              return String(reason ?? "Unhandled promise rejection");
            }
          })());
    const errType = err?.name || "UnhandledRejection";
    const stack = err?.stack || "";
    sendOnce("unhandled_rejection", errType, msg, "", stack);
  });
}

installGlobalFrontendErrorReporter();

function bootstrapAppearance() {
  try {
    const root = document.documentElement;
    const savedTheme = localStorage.getItem("tako-switch-theme");
    const systemDark =
      typeof window !== "undefined" &&
      Boolean(window.matchMedia?.("(prefers-color-scheme: dark)").matches);
    const isDark =
      savedTheme === "dark" ||
      ((savedTheme === null || savedTheme === "system") && systemDark);
    root.setAttribute("data-theme", isDark ? "dark" : "light");

    const savedDensity = localStorage.getItem("tako-switch-density");
    root.setAttribute(
      "data-density",
      savedDensity === "comfortable" ? "comfortable" : "compact",
    );
  } catch {
    // ignore storage errors in restricted environments
  }
}

bootstrapAppearance();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
