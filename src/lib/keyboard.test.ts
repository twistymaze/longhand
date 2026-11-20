import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, test } from "vitest";

import { isImeComposing } from "./keyboard";

describe("isImeComposing", () => {
  test("a key during a composition is for the input method", () => {
    expect(isImeComposing(new KeyboardEvent("keydown", { key: "Enter", isComposing: true }))).toBe(true);
  });

  test("the Enter that ends a composition in WebKit is for the input method", () => {
    // WebKit sends this keydown after compositionend: isComposing is false,
    // and only keyCode 229 shows that the input method uses the key.
    const event = new KeyboardEvent("keydown", { key: "Enter", isComposing: false });
    Object.defineProperty(event, "keyCode", { value: 229 });
    expect(isImeComposing(event)).toBe(true);
  });

  test("a plain Enter is not for the input method", () => {
    const event = new KeyboardEvent("keydown", { key: "Enter" });
    Object.defineProperty(event, "keyCode", { value: 13 });
    expect(isImeComposing(event)).toBe(false);
  });

  test("a React keyboard event uses the values of its native event", () => {
    const nativeEvent = new KeyboardEvent("keydown", { key: "Enter", isComposing: true });
    expect(isImeComposing({ nativeEvent })).toBe(true);
    expect(isImeComposing({ nativeEvent: new KeyboardEvent("keydown", { key: "Enter" }) })).toBe(false);
  });
});

// The start of a keydown handler. A key check belongs to the nearest handler
// start above it.
const HANDLER_START = /onKeyDown(?:Capture)?=\{|["']keydown["']|handleKeyDown|\bKeyboardEvent\b/g;
// A check of the Enter key. The ProseMirror keymap (`keys["Enter"]`) is not
// matched: ProseMirror does not run its keymap during a composition.
const ENTER_CHECK = /\b(?:key|code)\s*[!=]==?\s*["'](?:Numpad)?Enter["']|["']Enter["']\s*[!=]==?|case\s+["'](?:Numpad)?Enter["']/g;
// A check of the Escape key, in the same forms as an Enter check.
const ESCAPE_CHECK = /\b(?:key|code)\s*[!=]==?\s*["']Escape["']|["']Escape["']\s*[!=]==?|case\s+["']Escape["']/g;
const GUARD = "isImeComposing(";

/// A file that has known key checks without the guard. `count` is the number
/// of these checks in the file. The list can only get shorter.
type Exception = { file: string; count: number; reason: string };

/// A key check without the guard: the file (relative to the repository root)
/// and the line.
type Hit = { file: string; line: number };

// An Escape that cancels a composition must not close or cancel. These
// handlers do not ignore a key for the input method yet. A separate PR fixes
// them.
const ESCAPE_EXCEPTIONS: Exception[] = [
  { file: "src/home/ImportSheet.tsx", count: 1, reason: "known defect" },
  { file: "src/lib/ConfirmDialog.tsx", count: 1, reason: "known defect" },
  { file: "src/session/SessionMenu.tsx", count: 1, reason: "known defect" },
  { file: "src/settings/chord.ts", count: 1, reason: "known defect" },
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

/// Each key check without a call of the guard between the start of its
/// handler (or the start of the file) and the check.
function unguardedChecks(check: RegExp): Hit[] {
  const found: Hit[] = [];
  for (const path of sourceFiles(join(ROOT, "src"))) {
    const text = readFileSync(path, "utf8");
    const starts = [...text.matchAll(HANDLER_START)].map((m) => m.index);
    for (const hit of text.matchAll(check)) {
      const start = Math.max(0, ...starts.filter((s) => s < hit.index));
      if (!text.slice(start, hit.index).includes(GUARD)) {
        const line = text.slice(0, hit.index).split("\n").length;
        found.push({ file: relative(ROOT, path).split("\\").join("/"), line });
      }
    }
  }
  return found;
}

/// Compares the hits with the exception list. Returns one problem for each
/// hit in a file that has more hits than its entry allows, and one for each
/// entry whose count is higher than the hits in its file.
function exceptionProblems(key: string, hits: Hit[], exceptions: Exception[]): string[] {
  const problems: string[] = [];
  for (const file of new Set(hits.map((h) => h.file))) {
    const inFile = hits.filter((h) => h.file === file);
    const allowed = exceptions.find((e) => e.file === file)?.count ?? 0;
    if (inFile.length <= allowed) continue;
    for (const h of inFile) problems.push(`${h.file}:${h.line}: an ${key} check without ${GUARD}) at the start of its keydown handler`);
  }
  for (const e of exceptions) {
    const count = hits.filter((h) => h.file === e.file).length;
    if (count === 0) problems.push(`${e.file}: no ${key} check without the guard is left: remove the exception`);
    else if (count < e.count) problems.push(`${e.file}: ${count} ${key} checks without the guard are left: lower the count to ${count}`);
  }
  return problems;
}

describe("Key checks", () => {
  test("each Enter check in the app ignores a key for the input method", () => {
    // An Enter that confirms a Japanese, Chinese or Korean composition must
    // not save, submit or move. Call isImeComposing at the start of the
    // keydown handler.
    expect(exceptionProblems("Enter", unguardedChecks(ENTER_CHECK), [])).toEqual([]);
  });

  test("each Escape check in the app ignores a key for the input method", () => {
    // An Escape that cancels a composition must not close or cancel. Call
    // isImeComposing at the start of the keydown handler.
    expect(exceptionProblems("Escape", unguardedChecks(ESCAPE_CHECK), ESCAPE_EXCEPTIONS)).toEqual([]);
  });
});

describe("exceptionProblems", () => {
  const exception = (file: string, count: number): Exception => ({ file, count, reason: "known defect" });

  test("a hit that no entry covers names its file, its line and the guard", () => {
    expect(exceptionProblems("Escape", [{ file: "src/a.tsx", line: 7 }], [])).toEqual([
      "src/a.tsx:7: an Escape check without isImeComposing() at the start of its keydown handler",
    ]);
  });

  test("an entry covers as many hits as its count, and no more", () => {
    const hits = [
      { file: "src/a.tsx", line: 3 },
      { file: "src/a.tsx", line: 9 },
    ];
    expect(exceptionProblems("Escape", hits, [exception("src/a.tsx", 2)])).toEqual([]);
    expect(exceptionProblems("Escape", hits, [exception("src/a.tsx", 1)])).toHaveLength(2);
  });

  test("an entry for a file with no hit must be removed", () => {
    expect(exceptionProblems("Escape", [], [exception("src/a.tsx", 1)])).toEqual([
      "src/a.tsx: no Escape check without the guard is left: remove the exception",
    ]);
  });

  test("an entry with a count higher than the hits must be lowered", () => {
    expect(exceptionProblems("Escape", [{ file: "src/a.tsx", line: 3 }], [exception("src/a.tsx", 2)])).toEqual([
      "src/a.tsx: 1 Escape checks without the guard are left: lower the count to 1",
    ]);
  });
});
