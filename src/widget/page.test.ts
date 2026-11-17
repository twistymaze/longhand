import { describe, expect, test } from "vitest";

import { type WidgetWindow, setUpWidgetPage } from "./page";

/// A separate window, so that a listener does not stay on the test window.
function newWindow(): WidgetWindow & EventTarget {
  const target = new EventTarget();
  return Object.assign(target, { document: document.implementation.createHTMLDocument() });
}

/// Whether the page prevents a right-click on the widget. A prevented
/// event does not open the menu of the WebView (Reload, Save as, Print).
function rightClickPrevented(win: EventTarget): boolean {
  const event = new MouseEvent("contextmenu", { cancelable: true, button: 2 });
  win.dispatchEvent(event);
  return event.defaultPrevented;
}

describe("setUpWidgetPage", () => {
  test("a release build prevents the WebView menu on a right-click", () => {
    const win = newWindow();
    setUpWidgetPage(win, { release: true });
    expect(rightClickPrevented(win)).toBe(true);
    expect(win.document.documentElement.classList.contains("widget-window")).toBe(true);
  });

  test("a development build keeps the menu, for Inspect Element", () => {
    const win = newWindow();
    setUpWidgetPage(win, { release: false });
    expect(rightClickPrevented(win)).toBe(false);
    expect(win.document.documentElement.classList.contains("widget-window")).toBe(true);
  });

  test("a window that is not set up keeps the menu", () => {
    expect(rightClickPrevented(newWindow())).toBe(false);
  });
});
