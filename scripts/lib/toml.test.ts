import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { parseToml } from "./toml.ts";

const root = join(import.meta.dirname, "..", "..");

type Lock = {
  version: number;
  package: { name: string; version: string; source?: string; checksum?: string; dependencies?: string[] }[];
};

test("the repository's Cargo.lock gives version 4 and every package", () => {
  const text = readFileSync(join(root, "Cargo.lock"), "utf8");
  const lock = parseToml(text) as Lock;
  assert.equal(lock.version, 4);
  const headers = text.split("\n").filter((line) => line === "[[package]]").length;
  assert.equal(lock.package.length, headers);
  for (const p of lock.package) {
    assert.equal(typeof p.name, "string");
    assert.match(p.version, /^\d+\.\d+\.\d+/);
    if (p.source !== undefined) assert.equal(typeof p.source, "string");
    for (const d of p.dependencies ?? []) assert.equal(typeof d, "string");
  }
  assert.ok(lock.package.some((p) => p.source?.startsWith("registry+") && /^[0-9a-f]{64}$/.test(p.checksum ?? "")));
  assert.ok(lock.package.some((p) => p.source === undefined));
  assert.ok(lock.package.some((p) => (p.dependencies?.length ?? 0) > 1));
});

test("a multi-line array of basic strings with a trailing comma is a list", () => {
  assert.deepEqual(parseToml('[[package]]\nname = "a"\ndependencies = [\n "b",\n "c 1.0.0", # note\n]\n'), {
    package: [{ name: "a", dependencies: ["b", "c 1.0.0"] }],
  });
});

test("the repository's min-age-exceptions.toml parses, with no entries or only [[exception]] entries", () => {
  const parsed = parseToml(readFileSync(join(root, "min-age-exceptions.toml"), "utf8")) as { exception?: unknown[] };
  assert.deepEqual(Object.keys(parsed).filter((key) => key !== "exception"), []);
  for (const entry of parsed.exception ?? []) {
    assert.deepEqual(Object.keys(entry as object).sort(), ["ecosystem", "name", "reason", "version"]);
  }
});

test("two [[exception]] entries give a list of two tables", () => {
  const text = [
    "# Exceptions",
    "[[exception]]",
    'ecosystem = "crate"   # a comment',
    'name = "rustls"',
    'version = "0.23.31"',
    'reason = "RUSTSEC-2026-0400: the \\"fix\\" is only here"',
    "",
    "[[exception]]",
    'ecosystem = "package"',
    'name = "foo"',
    'version = "github:owner/foo#abc"',
    'reason = "fork with a fix"',
    "",
  ].join("\n");
  assert.deepEqual(parseToml(text), {
    exception: [
      { ecosystem: "crate", name: "rustls", version: "0.23.31", reason: 'RUSTSEC-2026-0400: the "fix" is only here' },
      { ecosystem: "package", name: "foo", version: "github:owner/foo#abc", reason: "fork with a fix" },
    ],
  });
});

test("a [table] header and boolean values give a nested table", () => {
  assert.deepEqual(parseToml("top = 1\n[deny]\nstrict = true\nloose = false\n"), {
    top: 1,
    deny: { strict: true, loose: false },
  });
});

test("an inline table is an error with its line number", () => {
  assert.throws(() => parseToml('name = "a"\ndep = { version = "1" }\n'), /line 2/);
});

test("a multi-line string is an error with its line number", () => {
  assert.throws(() => parseToml('name = "a"\n\nreason = """\ntext\n"""\n'), /line 3/);
});

test("a key that is given two times is an error with its line number", () => {
  assert.throws(() => parseToml('name = "a"\nname = "b"\n'), /line 2/);
});

test("\\u and \\U escapes give the character", () => {
  assert.deepEqual(parseToml('a = "caf\\u00e9"\nb = "\\U0001F600"\n'), { a: "café", b: "😀" });
});

test("an escape that TOML does not define is an error with its line number", () => {
  assert.throws(() => parseToml('a = "x"\nb = "\\q"\n'), /line 2/);
  assert.throws(() => parseToml('a = "\\u00g9"\n'), /line 1/);
});

test("an unterminated string or array is an error with its line number", () => {
  assert.throws(() => parseToml('a = "x"\nb = "open\n'), /line 2/);
  assert.throws(() => parseToml('a = [\n "b",\n'), /TOML line/);
});

test("a header with a missing bracket is an error with its line number", () => {
  assert.throws(() => parseToml('a = "x"\n[[package]\n'), /line 2/);
});

test("a [table] and an [[array]] with the same name are an error", () => {
  assert.throws(() => parseToml("[[a]]\n[a]\n"), /line 2/);
  assert.throws(() => parseToml("[a]\n[[a]]\n"), /line 2/);
});

test("a literal string is an error", () => {
  assert.throws(() => parseToml("a = 'x'\n"), /line 1/);
});

test("the key __proto__ is an own property and does not change the prototype", () => {
  const value = parseToml('__proto__ = "x"\n') as Record<string, unknown>;
  assert.equal(Object.getOwnPropertyDescriptor(value, "__proto__")?.value, "x");
  assert.equal(Object.getPrototypeOf(value), Object.prototype);
});
