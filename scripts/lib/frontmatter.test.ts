import assert from "node:assert/strict";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { parseFrontmatter } from "./frontmatter.ts";

const ideasDir = join(import.meta.dirname, "..", "..", "docs", "ideas");

// The idea files in the status folders. docs/ideas/README.md is not an idea.
function ideaFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return ideaFiles(path);
    return name.endsWith(".md") && dir !== ideasDir ? [path] : [];
  });
}

test("the frontmatter of every file in docs/ideas reads, and each value is a string or a list of strings", () => {
  const files = ideaFiles(ideasDir);
  assert.ok(files.length > 0);
  for (const file of files) {
    const match = /^---\n([\s\S]*?)\n---\n?/.exec(readFileSync(file, "utf8"));
    assert.ok(match, file);
    const meta = parseFrontmatter(match[1]);
    assert.equal(typeof meta.title, "string", file);
    assert.match(String(meta.added), /^\d{4}-\d{2}-\d{2}$/, file);
    for (const value of Object.values(meta)) {
      assert.ok(typeof value === "string" || (Array.isArray(value) && value.every((v) => typeof v === "string")), file);
    }
  }
});

test("a date stays a string", () => {
  assert.deepEqual(parseFrontmatter("added: 2026-10-18"), { added: "2026-10-18" });
});

test("[a, b] is a list, [] is an empty list, and a quoted value with a colon is one string", () => {
  assert.deepEqual(parseFrontmatter('title: "x: y"\nrelated: [a, b]\nother: []\nsource: "say \\"hi\\""'), {
    title: "x: y",
    related: ["a", "b"],
    other: [],
    source: 'say "hi"',
  });
});

test("a nested mapping is an error with its line number", () => {
  assert.throws(() => parseFrontmatter("title: A\nmeta:\n  key: value"), /line 2/);
});
