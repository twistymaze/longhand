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

// The start of a keydown handler. An Enter check belongs to the nearest
// handler start above it.
const HANDLER_START = /onKeyDown(?:Capture)?=\{|["']keydown["']|handleKeyDown|\bKeyboardEvent\b/g;
// A check of the Enter key. The ProseMirror keymap (`keys["Enter"]`) is not
// matched: ProseMirror does not run its keymap during a composition.
const ENTER_CHECK = /\b(?:key|code)\s*[!=]==?\s*["'](?:Numpad)?Enter["']|["']Enter["']\s*[!=]==?|case\s+["'](?:Numpad)?Enter["']/g;
const GUARD = "isImeComposing(";

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    if (!/\.tsx?$/.test(entry.name) || /\.test\.tsx?$/.test(entry.name)) return [];
    return [path];
  });
}

/// Each Enter check without a call of the guard between the start of its
/// handler (or the start of the file) and the check, as "file:line".
function unguardedEnterChecks(root: string): string[] {
  const found: string[] = [];
  for (const file of sourceFiles(root)) {
    const text = readFileSync(file, "utf8");
    const starts = [...text.matchAll(HANDLER_START)].map((m) => m.index);
    for (const check of text.matchAll(ENTER_CHECK)) {
      const start = Math.max(0, ...starts.filter((s) => s < check.index));
      if (!text.slice(start, check.index).includes(GUARD)) {
        const line = text.slice(0, check.index).split("\n").length;
        found.push(`${relative(root, file)}:${line}`);
      }
    }
  }
  return found;
}

describe("Enter checks", () => {
  test("each Enter check in the app ignores a key for the input method", () => {
    // An Enter that confirms a Japanese, Chinese or Korean composition must
    // not save, submit or move. Call isImeComposing at the start of the
    // keydown handler.
    expect(unguardedEnterChecks(join(__dirname, ".."))).toEqual([]);
  });
});
