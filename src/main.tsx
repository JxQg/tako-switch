import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

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
