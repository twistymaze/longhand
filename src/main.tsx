import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { commands } from "./bindings";
import "./index.css";

// On macOS the app menu holds Settings… (⌘,). Windows has no app menu, so
// Ctrl+, opens Settings from any window (KTD3).
if (!navigator.userAgent.includes("Mac")) {
  window.addEventListener("keydown", (event) => {
    if (event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey && event.key === ",") {
      event.preventDefault();
      void commands.openSettings();
    }
  });
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
