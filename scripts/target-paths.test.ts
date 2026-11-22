import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { test } from "node:test";

// CI and scripts must not use the build folder layout of Rust 1.95 and older.
// Since Rust 1.100, Cargo keeps each build script run in
// target/<profile>/build/<package>/<hash>/, with out/, run/ and fingerprint/
// in that folder. A glob for build/<package>-*/ or a .fingerprint/ path finds
// nothing, and the step that uses it fails or does nothing
// (docs/solutions/build-errors/cargo-1-100-build-script-folder-layout.md).

const root = join(import.meta.dirname, "..");
const scanned = [".github", "scripts"];
const extensions = [".yml", ".yaml", ".sh", ".ps1", ".ts", ".mjs", ".js", ".py"];
const self = relative(root, import.meta.filename);

const oldLayout = [
  { pattern: /build\/[A-Za-z0-9_.-]+-\*/, fix: "write build/<package>/*/ (Cargo 1.100 keeps each build script run in build/<package>/<hash>/)" },
  { pattern: /\.fingerprint\b/, fix: "the fingerprint is in build/<package>/<hash>/fingerprint since Cargo 1.100; remove build/<package>/<hash>/ to run the build script again" },
];

function files(dir: string): string[] {
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === "node_modules" ? [] : files(path);
    return extensions.some((e) => entry.name.endsWith(e)) ? [path] : [];
  });
}

test("CI and scripts use the build folder layout of Cargo 1.100", () => {
  const hits: string[] = [];
  for (const file of scanned.flatMap((d) => files(join(root, d)))) {
    const path = relative(root, file);
    if (path === self) continue;
    for (const [i, line] of readFileSync(file, "utf8").split("\n").entries())
      for (const { pattern, fix } of oldLayout) if (pattern.test(line)) hits.push(`${path}:${i + 1}: old Cargo build folder layout: ${fix}`);
  }
  assert.deepEqual(hits, []);
});
