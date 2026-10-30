// Links in notes and summaries open in the default browser. The app's window
// never shows another page: it has no way back to the app.

import { openUrl } from "@tauri-apps/plugin-opener";

const OPENABLE = /^(https?:|mailto:)/i;

/// True for the addresses that a link can open: web and mail addresses.
export function isOpenableHref(href: string): boolean {
  return OPENABLE.test(href.trim());
}

/// Open a web or mail address in the default browser. Other addresses do
/// nothing.
export function openLink(href: string) {
  if (isOpenableHref(href)) void openUrl(href.trim());
}

/// True when a click on a link in an editable note opens the link:
/// Command-click on macOS, Control-click on other systems.
export function isOpenLinkClick(event: MouseEvent | KeyboardEvent): boolean {
  return navigator.userAgent.includes("Mac") ? event.metaKey : event.ctrlKey;
}
