import assert from "node:assert/strict";
import { test } from "node:test";
import { checkAges, parseExceptions, type PublishDate } from "./check-dependency-age.ts";
import { addedVersions, registryTarball, type PackageLockEntry } from "./lockfiles.ts";

// The tests give the check fixed lockfiles and publish dates, so no test
// reaches crates.io or the npm registry.

const NOW = new Date("2026-10-20T12:00:00Z");
const daysAgo = (days: number) => new Date(NOW.getTime() - days * 86_400_000);
const CRATES = "registry+https://github.com/rust-lang/crates.io-index";

function cargoLock(crates: [string, string, string?][]): string {
  const blocks = crates.map(([name, version, source]) =>
    [`[[package]]`, `name = "${name}"`, `version = "${version}"`, ...(source === "" ? [] : [`source = "${source ?? CRATES}"`])].join("\n"),
  );
  return `version = 4\n\n${blocks.join("\n\n")}\n`;
}

/// A registry entry of package-lock.json for `name@version`.
function registryEntry(name: string, version: string): PackageLockEntry {
  return { version, resolved: registryTarball(name, version), integrity: "sha512-x" };
}

/// A package-lock.json (lockfileVersion 3) with these entries by key.
function packageLock(entries: Record<string, PackageLockEntry>): string {
  return JSON.stringify({ name: "longhand", lockfileVersion: 3, requires: true, packages: { "": { name: "longhand" }, ...entries } }, null, 2);
}

const dates = (table: Record<string, Date>): PublishDate => async (v) => {
  const date = table[`${v.ecosystem} ${v.name} ${v.version}`];
  if (!date) throw new Error("HTTP 404");
  return date;
};

const EMPTY = { cargo: cargoLock([]), npm: packageLock({}) };
const npmHead = (entries: Record<string, PackageLockEntry>) => ({ cargo: cargoLock([]), npm: packageLock(entries) });
const exception = (name: string, version: string) =>
  parseExceptions(`[[exception]]\necosystem = "package"\nname = "${name}"\nversion = "${version}"\nreason = "a reason"\n`);

test("a crate version published 2 days ago fails, with its name, version and date", async () => {
  const added = addedVersions(EMPTY, { cargo: cargoLock([["rustls", "0.23.31"]]), npm: packageLock({}) });
  const problems = await checkAges(added, [], NOW, dates({ "crate rustls 0.23.31": daysAgo(2) }));
  assert.equal(problems.length, 1);
  assert.ok(problems[0].includes("rustls 0.23.31"));
  assert.ok(problems[0].includes("2026-10-18"));
});

test("a crate version published 8 days ago passes", async () => {
  const added = addedVersions(EMPTY, { cargo: cargoLock([["serde", "1.0.230"]]), npm: packageLock({}) });
  assert.deepEqual(await checkAges(added, [], NOW, dates({ "crate serde 1.0.230": daysAgo(8) })), []);
});

test("a head that only removes or downgrades versions passes", async () => {
  const base = { cargo: cargoLock([["serde", "1.0.230"], ["log", "0.4.29"]]), npm: packageLock({}) };
  const head = { cargo: cargoLock([["serde", "1.0.229"]]), npm: packageLock({}) };
  const added = addedVersions(base, head);
  assert.deepEqual(added.map((v) => `${v.name} ${v.version}`), ["serde 1.0.229"]);
  assert.deepEqual(await checkAges(added, [], NOW, dates({ "crate serde 1.0.229": daysAgo(90) })), []);
});

test("an npm package in package-lock.json published yesterday fails", async () => {
  const head = npmHead({ "node_modules/source-map-js": registryEntry("source-map-js", "1.2.2") });
  const problems = await checkAges(addedVersions(EMPTY, head), [], NOW, dates({ "package source-map-js 1.2.2": daysAgo(1) }));
  assert.equal(problems.length, 1);
  assert.ok(problems[0].includes("source-map-js 1.2.2"));
});

test("AE2: a package version published 3 days ago fails, and passes with a matching exception", async () => {
  const head = npmHead({ "node_modules/left-pad": registryEntry("left-pad", "1.3.1") });
  const published = dates({ "package left-pad 1.3.1": daysAgo(3) });
  const problems = await checkAges(addedVersions(EMPTY, head), [], NOW, published);
  assert.equal(problems.length, 1);
  assert.ok(problems[0].includes("less than 7 days old: package left-pad 1.3.1"));
  assert.deepEqual(await checkAges(addedVersions(EMPTY, head), exception("left-pad", "1.3.1"), NOW, published), []);
});

test("a young version that the exception file lists with a reason passes", async () => {
  const exceptions = parseExceptions(`[[exception]]\necosystem = "crate"\nname = "rustls"\nversion = "0.23.31"\nreason = "RUSTSEC-2026-0400"\n`);
  const added = addedVersions(EMPTY, { cargo: cargoLock([["rustls", "0.23.31"]]), npm: packageLock({}) });
  assert.deepEqual(await checkAges(added, exceptions, NOW, dates({ "crate rustls 0.23.31": daysAgo(2) })), []);
});

test("an exception without a reason is a file error", () => {
  assert.throws(() => parseExceptions(`[[exception]]\necosystem = "crate"\nname = "rustls"\nversion = "0.23.31"\n`), /reason/);
});

test("an exception for another version of the same crate does not pass the young version", async () => {
  const exceptions = parseExceptions(`[[exception]]\necosystem = "crate"\nname = "rustls"\nversion = "0.23.30"\nreason = "RUSTSEC-2026-0400"\n`);
  const added = addedVersions(EMPTY, { cargo: cargoLock([["rustls", "0.23.31"]]), npm: packageLock({}) });
  assert.equal((await checkAges(added, exceptions, NOW, dates({ "crate rustls 0.23.31": daysAgo(2) }))).length, 1);
});

test("a failed publish-date lookup fails and says to run the check again", async () => {
  const added = addedVersions(EMPTY, { cargo: cargoLock([["serde", "1.0.230"]]), npm: packageLock({}) });
  const problems = await checkAges(added, [], NOW, dates({}));
  assert.equal(problems.length, 1);
  assert.ok(problems[0].includes("run the check again"));
});

test("a git-sourced crate is reported as not checked and does not fail", async () => {
  const head = { cargo: cargoLock([["transcribe", "0.2.4", "git+https://github.com/handy-computer/transcribe.cpp#abc"]]), npm: packageLock({}) };
  const notes: string[] = [];
  assert.deepEqual(await checkAges(addedVersions(EMPTY, head), [], NOW, dates({}), notes), []);
  assert.ok(notes[0].includes("not checked"));
});

test("a package-lock.json entry from a git source fails unless the exception file lists it", async () => {
  const head = npmHead({ "node_modules/foo": { version: "1.0.0", resolved: "git+ssh://git@github.com/owner/foo.git#abc" } });
  const problems = await checkAges(addedVersions(EMPTY, head), [], NOW, dates({}));
  assert.equal(problems.length, 1);
  assert.ok(problems[0].includes("not from the npm registry"));
  assert.deepEqual(await checkAges(addedVersions(EMPTY, head), exception("foo", "1.0.0"), NOW, dates({})), []);
});

test("AE6: an old name and version downloaded from another host fails unless the exception file lists it", async () => {
  const base = npmHead({ "node_modules/left-pad": registryEntry("left-pad", "1.3.0") });
  const head = npmHead({
    "node_modules/left-pad": { version: "1.3.0", resolved: "https://evil.example/left-pad/-/left-pad-1.3.0.tgz", integrity: "sha512-x" },
  });
  const published = dates({ "package left-pad 1.3.0": daysAgo(400) });
  const problems = await checkAges(addedVersions(base, head), [], NOW, published);
  assert.equal(problems.length, 1);
  assert.ok(problems[0].includes("not from the npm registry: package left-pad 1.3.0"));
  assert.ok(problems[0].includes("evil.example"));
  assert.deepEqual(await checkAges(addedVersions(base, head), exception("left-pad", "1.3.0"), NOW, published), []);
});

test("an alias entry is not a registry entry", async () => {
  // `npm install x@npm:y@1.0.0` writes the key node_modules/x with name "y".
  const head = npmHead({ "node_modules/x": { name: "y", ...registryEntry("x", "1.0.0") } });
  const [v] = addedVersions(EMPTY, head);
  assert.equal(v.registry, false);
  const problems = await checkAges([v], [], NOW, dates({ "package x 1.0.0": daysAgo(400) }));
  assert.equal(problems.length, 1);
  assert.ok(problems[0].includes("not from the npm registry"));
});

test("an entry with no integrity is not a registry entry", async () => {
  const { integrity: _, ...entry } = registryEntry("left-pad", "1.3.0");
  const [v] = addedVersions(EMPTY, npmHead({ "node_modules/left-pad": entry }));
  assert.equal(v.registry, false);
  assert.equal((await checkAges([v], [], NOW, dates({ "package left-pad 1.3.0": daysAgo(400) }))).length, 1);
});

test("a link entry is not a registry entry", () => {
  const [v] = addedVersions(EMPTY, npmHead({ "node_modules/local": { resolved: "packages/local", link: true } }));
  assert.equal(v.registry, false);
});

test("a scoped package with the correct tarball URL is a registry entry", async () => {
  const head = npmHead({
    "node_modules/@scope/name": { version: "1.2.3", resolved: "https://registry.npmjs.org/@scope/name/-/name-1.2.3.tgz", integrity: "sha512-x" },
  });
  const added = addedVersions(EMPTY, head);
  assert.deepEqual(added.map((v) => `${v.name} ${v.version} ${v.registry}`), ["@scope/name 1.2.3 true"]);
  assert.deepEqual(await checkAges(added, [], NOW, dates({ "package @scope/name 1.2.3": daysAgo(30) })), []);
});

test("a version in the lockfile at two paths is checked once", async () => {
  const head = npmHead({
    "node_modules/source-map-js": registryEntry("source-map-js", "1.2.2"),
    "node_modules/vite/node_modules/source-map-js": registryEntry("source-map-js", "1.2.2"),
  });
  const added = addedVersions(EMPTY, head);
  assert.deepEqual(added.map((v) => `${v.name} ${v.version}`), ["source-map-js 1.2.2"]);
  let lookups = 0;
  const problems = await checkAges(added, [], NOW, async () => {
    lookups++;
    return daysAgo(1);
  });
  assert.equal(lookups, 1);
  assert.equal(problems.length, 1);
});
