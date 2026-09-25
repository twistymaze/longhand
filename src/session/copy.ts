// The markdown each tab copies (R17): the summary body, the text of
// transcript.md, or the notes document. The copy is the file text, so it
// pastes into any markdown tool as the folder holds it.

import type { SessionDocument } from "../bindings";

export type CopyTab = "summary" | "transcript" | "notes";

/// The markdown of `tab`, or null when the tab has nothing to copy.
export function tabMarkdown(tab: CopyTab, doc: SessionDocument | null, notes: string | null): string | null {
  const text =
    tab === "notes" ? notes : tab === "transcript" ? (doc?.transcript_markdown ?? null) : (doc?.summary?.body ?? null);
  return text !== null && text.trim() !== "" ? text : null;
}

/// Write `text` to the clipboard. False when the clipboard refuses it.
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
