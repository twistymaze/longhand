/// The parts of the widget window that the setup uses.
export type WidgetWindow = Pick<Window, "document" | "addEventListener">;

/// Sets up the page of the recording widget window.
export function setUpWidgetPage(win: WidgetWindow, { release }: { release: boolean }) {
  // The window is transparent: only its pill shows (`.widget-window` in
  // index.css).
  win.document.documentElement.classList.add("widget-window");
  // The widget floats over other apps, so a right-click on it must not open
  // the menu of the WebView (Reload, Save as, Print). A development build
  // keeps the menu for Inspect Element.
  if (release) win.addEventListener("contextmenu", (event) => event.preventDefault());
}
