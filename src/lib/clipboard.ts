// Rich text on the clipboard with no colours or fonts, so a paste takes the
// styles of the receiving app (bold, lists and headings stay).
//
// On macOS, WebKit renders each HTML item that the page writes to the
// clipboard and adds the computed styles to each element. In dark mode the
// text colour is white, and a paste into a light email shows no text. Thus
// on macOS the app writes the clipboard through the native command
// `write_clipboard`. WebView2 on Windows keeps the HTML of a copy event as
// the page writes it.

import { commands } from "../bindings";

const isMac = () => navigator.userAgent.includes("Mac");

/// Write `text` and `html` to the clipboard as one item. When the native
/// write is not available, write through the webview; when the clipboard
/// refuses the item, write `text` only. False when the clipboard refuses all.
export async function copyText(text: string, html: string): Promise<boolean> {
  try {
    if (await commands.writeClipboard(text, html)) return true;
  } catch {
    // No native write (outside the app). The webview writes the clipboard.
  }
  try {
    // The first format is the one that a native app takes first.
    const item = new ClipboardItem({
      "text/html": new Blob([html], { type: "text/html" }),
      "text/plain": new Blob([text], { type: "text/plain" }),
    });
    await navigator.clipboard.write([item]);
    return true;
  } catch {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      return false;
    }
  }
}

/// Put `text` and `html` on the clipboard for a copy or cut `event`, in
/// place of what the webview writes.
export function writeCopyEvent(event: ClipboardEvent, text: string, html: string): void {
  event.preventDefault();
  if (isMac()) {
    void copyText(text, html);
    return;
  }
  event.clipboardData?.setData("text/html", html);
  event.clipboardData?.setData("text/plain", text);
}

// The elements that a copy keeps, with no attributes other than a link
// address. Each other element gives only its content.
const KEPT = new Set([
  "h1", "h2", "h3", "h4", "h5", "h6", "p", "ul", "ol", "li", "blockquote", "pre",
  "strong", "b", "em", "i", "code", "a", "s", "del", "mark", "br",
]);
const BLOCKS = new Set(["h1", "h2", "h3", "h4", "h5", "h6", "p", "ul", "ol", "li", "blockquote", "pre"]);
// The kept elements that style a part of a line.
const RUNS = new Set(["strong", "b", "em", "i", "code", "a", "s", "del", "mark"]);
// The containers that the page uses for layout. One that holds only text
// and runs becomes a paragraph, so that each line of the page stays a line.
const BOXES = new Set(["div", "section", "article", "header", "footer", "main"]);

function keptCopy(element: Element): HTMLElement {
  const tag = element.tagName.toLowerCase();
  const copy = document.createElement(tag);
  const href = tag === "a" ? element.getAttribute("href") : null;
  if (href !== null && /^(https?:|mailto:)/i.test(href)) copy.setAttribute("href", href);
  return copy;
}

function cleanInto(from: Node, to: Element): void {
  for (const child of Array.from(from.childNodes)) {
    if (child.nodeType === Node.TEXT_NODE) {
      to.appendChild(document.createTextNode(child.textContent ?? ""));
      continue;
    }
    if (child.nodeType !== Node.ELEMENT_NODE) continue;
    const element = child as Element;
    const tag = element.tagName.toLowerCase();
    if (KEPT.has(tag)) {
      const copy = keptCopy(element);
      cleanInto(element, copy);
      to.appendChild(copy);
    } else if (BOXES.has(tag)) {
      const box = document.createElement("div");
      cleanInto(element, box);
      const blocks = Array.from(box.children).some((c) => BLOCKS.has(c.tagName.toLowerCase()));
      if (blocks || (box.textContent ?? "").trim() === "") {
        to.append(...Array.from(box.childNodes));
      } else {
        const p = document.createElement("p");
        p.append(...Array.from(box.childNodes));
        to.appendChild(p);
      }
    } else {
      cleanInto(element, to);
    }
  }
}

/// The HTML of `node`'s content with only the kept elements, and no style,
/// class or other attribute.
export function cleanHtml(node: Node): string {
  const out = document.createElement("div");
  cleanInto(node, out);
  return out.innerHTML;
}

/// The HTML of `range` with no styles. A range in one run or in one list
/// gets that run or list around it, as the page shows it.
export function rangeHtml(range: Range): string {
  const out = document.createElement("div");
  cleanInto(range.cloneContents(), out);
  const listItems = Array.from(out.children).some((c) => c.tagName === "LI");
  let parent: Node | null = range.commonAncestorContainer;
  if (parent.nodeType !== Node.ELEMENT_NODE) parent = parent.parentNode;
  for (; parent !== null && parent.nodeType === Node.ELEMENT_NODE; parent = parent.parentNode) {
    const tag = (parent as Element).tagName.toLowerCase();
    if (BLOCKS.has(tag) && !(listItems && (tag === "ul" || tag === "ol"))) break;
    if (!RUNS.has(tag) && tag !== "ul" && tag !== "ol") continue;
    const wrap = keptCopy(parent as Element);
    wrap.append(...Array.from(out.childNodes));
    out.appendChild(wrap);
    if (tag === "ul" || tag === "ol") break;
  }
  return out.innerHTML;
}

/// For a copy `event` of the page's own text: put the selection on the
/// clipboard with no styles. A copy from a field or an editor that wrote
/// the clipboard itself is not changed.
export function copySelection(event: ClipboardEvent): void {
  if (event.defaultPrevented) return;
  const target = event.target as Element | null;
  if (target?.closest?.("input, textarea")) return;
  const selection = document.getSelection();
  if (selection === null || selection.isCollapsed || selection.rangeCount === 0) return;
  writeCopyEvent(event, selection.toString(), rangeHtml(selection.getRangeAt(0)));
}
