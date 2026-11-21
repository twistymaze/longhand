// Helpers for the tests that scan the frontend source. This file has no tests,
// so a test file can import it and vitest does not run tests two times.
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = join(__dirname, "..", "..");

/// A source file: its path relative to the repository root, with "/", and its text.
export type SourceFile = { file: string; text: string };

function walk(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return walk(path);
    if (!/\.tsx?$/.test(entry.name) || /\.test\.tsx?$/.test(entry.name)) return [];
    return [path];
  });
}

let cache: SourceFile[] | undefined;

/// Each .ts and .tsx file in src/ that is not a test. Reads the files one time.
export function sourceFiles(): SourceFile[] {
  cache ??= walk(join(ROOT, "src")).map((path) => ({
    file: relative(ROOT, path).split("\\").join("/"),
    text: readFileSync(path, "utf8"),
  }));
  return cache;
}

/// The line number, from 1, of the character at `index`.
export function lineAt(text: string, index: number): number {
  return text.slice(0, index).split("\n").length;
}
