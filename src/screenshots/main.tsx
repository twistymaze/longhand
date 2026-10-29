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

// Only a scene of `scenes` is valid, so a wrong name stops the page with an
// error, which scripts/screenshots.ts reports.
// The name comes from the list of scenes, not from the query.
const requested = new URLSearchParams(window.location.search).get("scene") ?? "library";
const name = (Object.keys(scenes) as SceneName[]).find((key) => key === requested);
if (!name) throw new Error(`screenshots: no scene named ${requested}`);
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
