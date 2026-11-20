import { describe, expect, test } from "vitest";
import { lineAt, sourceFiles } from "./sourceScan";

// Each rule names a pattern that only its owner files can use. Other files
// call the shared function of the owner.
type Owner = { name: string; pattern: RegExp; owners: string[]; use: string };

const OWNERS: Owner[] = [
  {
    name: "markdown-it",
    pattern: /from\s+["']markdown-it["']/g,
    owners: ["src/editor/markdown/parser.ts", "src/session/copy.ts"],
    use: "do not import markdown-it here: call parseMarkdown (src/session/markdown.ts)",
  },
  {
    name: "close listener",
    pattern: /onCloseRequested/g,
    owners: ["src/settings/closeSaves.ts"],
    use: "do not call onCloseRequested here: call registerCloseSave (src/settings/closeSaves.ts)",
  },
  {
    name: "clipboard",
    pattern: /navigator\.clipboard|execCommand\(/g,
    owners: ["src/lib/clipboard.ts"],
    use: "do not use the clipboard here: call copyText, writeCopyEvent or copySelection (src/lib/clipboard.ts)",
  },
  {
    name: "note editor",
    pattern: /<NoteEditor\b/g,
    owners: ["src/editor/NoteEditor.tsx", "src/settings/MarkdownField.tsx", "src/session/NotesTab.tsx", "src/home/ImportSheet.tsx"],
    use: "do not put NoteEditor here: the editor needs the px-3 frame that MarkdownField, NotesTab and ImportSheet give it",
  },
];

/// Each use of the pattern outside the owner files, as "file:line: message".
function usesOutsideOwners(rule: Owner): string[] {
  const found: string[] = [];
  for (const { file, text } of sourceFiles()) {
    if (rule.owners.includes(file)) continue;
    for (const hit of text.matchAll(rule.pattern)) found.push(`${file}:${lineAt(text, hit.index)}: ${rule.use}`);
  }
  return found;
}

describe("Single owners", () => {
  for (const rule of OWNERS) {
    test(`only the owner files use the ${rule.name}`, () => {
      expect(usesOutsideOwners(rule)).toEqual([]);
    });
  }
});
