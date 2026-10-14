// What each tab copies (R17): the summary body, the text of transcript.md, or
// the notes document, as markdown, plus the same content as HTML. The
// receiving app takes the format it can use: a rich-text app pastes headings
// and lists, and a plain-text app pastes the markdown as the folder holds it.

import MarkdownIt from "markdown-it";
import type { SessionDocument } from "../bindings";
import { highlightPlugin, strikethroughPlugin } from "../editor/markdown/parser";
import { type Block, type Inline, type ListBlock, parseMarkdown } from "./markdown";

export type CopyTab = "summary" | "transcript" | "notes";

/// The markdown of `tab`, or null when the tab has nothing to copy.
export function tabMarkdown(tab: CopyTab, doc: SessionDocument | null, notes: string | null): string | null {
  const text =
    tab === "notes" ? notes : tab === "transcript" ? (doc?.transcript_markdown ?? null) : (doc?.summary?.body ?? null);
  return text !== null && text.trim() !== "" ? text : null;
}

// Raw HTML stays text, and an image stays text, as in the editor. markdown-it
// removes javascript:, vbscript:, file: and data: link addresses.
const markdown = MarkdownIt("commonmark", { html: false }).disable(["image"]);
markdown.use(strikethroughPlugin);
markdown.use(highlightPlugin);

const escapeHtml = (text: string) =>
  text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

const runsHtml = (content: Inline[]) =>
  content
    .map((run) => {
      const text = escapeHtml(run.text);
      return run.kind === "text" ? text : `<${run.kind}>${text}</${run.kind}>`;
    })
    .join("");

function listHtml(list: ListBlock): string {
  const tag = list.ordered ? "ol" : "ul";
  const items = list.items.map((item) => `<li>${runsHtml(item.content)}${item.children.map(listHtml).join("")}</li>`);
  return `<${tag}>${items.join("")}</${tag}>`;
}

function blockHtml(block: Block): string {
  if (block.kind === "heading") return `<h${block.level}>${runsHtml(block.content)}</h${block.level}>`;
  if (block.kind === "list") return listHtml(block);
  return `<p>${runsHtml(block.content)}</p>`;
}

/// The HTML of `tab`'s markdown, with no styles, so a paste takes the styles
/// of the receiving app. The summary uses the Summary tab's parser, so the
/// paste has the structure that the tab shows.
export function tabHtml(tab: CopyTab, text: string): string {
  if (tab === "summary") return parseMarkdown(text).map((block) => `${blockHtml(block)}\n`).join("");
  return markdown.render(text);
}

/// Write `text` and `html` to the clipboard as one item. When the clipboard
/// refuses the item, write `text` only. False when the clipboard refuses both.
export async function copyText(text: string, html: string): Promise<boolean> {
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
