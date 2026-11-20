import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, test } from "vitest";

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

const ROOT = join(__dirname, "..", "..");

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    if (!/\.tsx?$/.test(entry.name) || /\.test\.tsx?$/.test(entry.name)) return [];
    return [path];
  });
}

/// Each use of the pattern outside the owner files, as "file:line: message".
function usesOutsideOwners(rule: Owner): string[] {
  const found: string[] = [];
  for (const path of sourceFiles(join(ROOT, "src"))) {
    const file = relative(ROOT, path).split("\\").join("/");
    if (rule.owners.includes(file)) continue;
    const text = readFileSync(path, "utf8");
    for (const hit of text.matchAll(rule.pattern)) {
      const line = text.slice(0, hit.index).split("\n").length;
      found.push(`${file}:${line}: ${rule.use}`);
    }
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
