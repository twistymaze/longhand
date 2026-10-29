import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { commands } from "./bindings";
import { forwardErrorsToLog } from "./lib/log";
import "./index.css";

// Both windows load this page. Their errors go to the app log too.
forwardErrorsToLog();

// On macOS the main window has no title bar, and the page shows a strip in
// its place (`.title-strip` in index.css).
if (navigator.userAgent.includes("Mac")) document.documentElement.classList.add("mac");

// The recording widget window is transparent: only its pill shows
// (`.widget-window` in index.css).
if (new URLSearchParams(window.location.search).get("window") === "widget") {
  document.documentElement.classList.add("widget-window");
}

// On macOS the app menu holds Settings… (⌘,). On Windows only the main
// window has a menu bar, so Ctrl+, opens Settings from any window (KTD3).
if (!navigator.userAgent.includes("Mac")) {
  window.addEventListener("keydown", (event) => {
    if (event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey && event.key === ",") {
      event.preventDefault();
      void commands.openSettings(null);
    }
  });
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
