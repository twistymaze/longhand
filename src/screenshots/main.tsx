// The preview page for the website screenshots. It renders one scene,
// named in the query (`?scene=library`), inside a macOS window frame.
// scripts/screenshots.ts loads it in WebKit. `vite build` takes only the
// root index.html, so this page is never in the app.

import React from "react";
import ReactDOM from "react-dom/client";
import { ScopeTonesProvider } from "../lib/Labels";
import { mockTauri } from "../test/tauri";
import { type SceneName, scenes } from "./scenes";
import { WindowFrame } from "./WindowFrame";
import "../index.css";

const name = (new URLSearchParams(window.location.search).get("scene") ?? "library") as SceneName;
const scene = scenes[name]();
mockTauri(scene.handlers);

// The app draws its title strip on macOS only (main.tsx).
document.documentElement.classList.add("mac");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ScopeTonesProvider>
      <WindowFrame>{scene.view}</WindowFrame>
    </ScopeTonesProvider>
  </React.StrictMode>,
);
