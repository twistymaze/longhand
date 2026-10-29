// A macOS window around a screenshot scene (KTD4 of the landing page plan):
// the main window size, rounded corners, the window shadow and the three
// window buttons. Inside, the scene has the same shell as MainWindow in
// App.tsx, so the page draws its own 44-point title strip under the buttons.

import type { ReactNode } from "react";

/// The main window size in src-tauri/src/window.rs.
export const WINDOW = { width: 960, height: 680 };

/// The space around the window for its shadow.
export const MARGIN = 48;

const BUTTONS = ["#ff5f57", "#febc2e", "#28c840"];

/// The window edge and shadow. In dark mode, macOS draws a light edge on the
/// window, so the window stays visible on a dark page.
const SHADOW = {
  light:
    "0 0 0 0.5px rgb(0 0 0 / 0.22), 0 1px 2px rgb(0 0 0 / 0.08), 0 10px 22px rgb(0 0 0 / 0.14), 0 22px 44px rgb(0 0 0 / 0.16)",
  dark: "inset 0 0 0 0.5px rgb(255 255 255 / 0.16), 0 0 0 0.5px rgb(0 0 0 / 0.6), 0 1px 2px rgb(0 0 0 / 0.2), 0 10px 22px rgb(0 0 0 / 0.3), 0 22px 44px rgb(0 0 0 / 0.35)",
};

export function WindowFrame({ children }: { children: ReactNode }) {
  const dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  return (
    <div data-screenshot style={{ padding: MARGIN, width: "fit-content" }}>
      <div
        style={{
          position: "relative",
          width: WINDOW.width,
          height: WINDOW.height,
          borderRadius: 16,
          overflow: "hidden",
          boxShadow: dark ? SHADOW.dark : SHADOW.light,
        }}
      >
        <div className="flex h-full flex-col bg-window">
          <div className="title-strip" />
          <div className="min-h-0 flex-1">{children}</div>
        </div>
        {/* The window buttons, where a capture of the real window on macOS 26
            shows them: 18 points from the left edge, 12 from the top. */}
        <div style={{ position: "absolute", left: 18, top: 12, display: "flex", gap: 8 }}>
          {BUTTONS.map((colour) => (
            <span
              key={colour}
              style={{
                width: 12,
                height: 12,
                borderRadius: "50%",
                background: colour,
                boxShadow: "inset 0 0 0 0.5px rgb(0 0 0 / 0.18)",
              }}
            />
          ))}
        </div>
      </div>
    </div>
  );
}
