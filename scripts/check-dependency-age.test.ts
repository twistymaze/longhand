import { expect, test } from "bun:test";
import { checkAges, parseExceptions, type PublishDate } from "./check-dependency-age";
import { addedVersions } from "./lockfiles";

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

function bunLock(entries: Record<string, unknown[]>): string {
  return JSON.stringify({ lockfileVersion: 1, workspaces: { "": { name: "longhand" } }, packages: entries }, null, 2).replace(/\n(\s*)}/g, ",\n$1}");
}

const dates = (table: Record<string, Date>): PublishDate => async (v) => {
  const date = table[`${v.ecosystem} ${v.name} ${v.version}`];
  if (!date) throw new Error("HTTP 404");
  return date;
};

const EMPTY = { cargo: cargoLock([]), bun: bunLock({}) };

test("a crate version published 2 days ago fails, with its name, version and date", async () => {
  const added = addedVersions(EMPTY, { cargo: cargoLock([["rustls", "0.23.31"]]), bun: bunLock({}) });
  const problems = await checkAges(added, [], NOW, dates({ "crate rustls 0.23.31": daysAgo(2) }));
  expect(problems).toHaveLength(1);
  expect(problems[0]).toContain("rustls 0.23.31");
  expect(problems[0]).toContain("2026-10-18");
});

test("a crate version published 8 days ago passes", async () => {
  const added = addedVersions(EMPTY, { cargo: cargoLock([["serde", "1.0.230"]]), bun: bunLock({}) });
  expect(await checkAges(added, [], NOW, dates({ "crate serde 1.0.230": daysAgo(8) }))).toEqual([]);
});

test("a head that only removes or downgrades versions passes", async () => {
  const base = { cargo: cargoLock([["serde", "1.0.230"], ["log", "0.4.29"]]), bun: bunLock({}) };
  const head = { cargo: cargoLock([["serde", "1.0.229"]]), bun: bunLock({}) };
  const added = addedVersions(base, head);
  expect(added.map((v) => `${v.name} ${v.version}`)).toEqual(["serde 1.0.229"]);
  expect(await checkAges(added, [], NOW, dates({ "crate serde 1.0.229": daysAgo(90) }))).toEqual([]);
});

test("an npm package in bun.lock published yesterday fails", async () => {
  const head = { cargo: cargoLock([]), bun: bunLock({ "source-map-js": ["source-map-js@1.2.2", "", {}, "sha512-x"] }) };
  const problems = await checkAges(addedVersions(EMPTY, head), [], NOW, dates({ "package source-map-js 1.2.2": daysAgo(1) }));
  expect(problems).toHaveLength(1);
  expect(problems[0]).toContain("source-map-js 1.2.2");
});

test("a young version that the exception file lists with a reason passes", async () => {
  const exceptions = parseExceptions(`[[exception]]\necosystem = "crate"\nname = "rustls"\nversion = "0.23.31"\nreason = "RUSTSEC-2026-0400"\n`);
  const added = addedVersions(EMPTY, { cargo: cargoLock([["rustls", "0.23.31"]]), bun: bunLock({}) });
  expect(await checkAges(added, exceptions, NOW, dates({ "crate rustls 0.23.31": daysAgo(2) }))).toEqual([]);
});

test("an exception without a reason is a file error", () => {
  expect(() => parseExceptions(`[[exception]]\necosystem = "crate"\nname = "rustls"\nversion = "0.23.31"\n`)).toThrow("reason");
});

test("an exception for another version of the same crate does not pass the young version", async () => {
  const exceptions = parseExceptions(`[[exception]]\necosystem = "crate"\nname = "rustls"\nversion = "0.23.30"\nreason = "RUSTSEC-2026-0400"\n`);
  const added = addedVersions(EMPTY, { cargo: cargoLock([["rustls", "0.23.31"]]), bun: bunLock({}) });
  expect(await checkAges(added, exceptions, NOW, dates({ "crate rustls 0.23.31": daysAgo(2) }))).toHaveLength(1);
});

test("a failed publish-date lookup fails and says to run the check again", async () => {
  const added = addedVersions(EMPTY, { cargo: cargoLock([["serde", "1.0.230"]]), bun: bunLock({}) });
  const problems = await checkAges(added, [], NOW, dates({}));
  expect(problems).toHaveLength(1);
  expect(problems[0]).toContain("run the check again");
});

test("a git-sourced crate is reported as not checked and does not fail", async () => {
  const head = { cargo: cargoLock([["transcribe", "0.2.4", "git+https://github.com/handy-computer/transcribe.cpp#abc"]]), bun: bunLock({}) };
  const notes: string[] = [];
  expect(await checkAges(addedVersions(EMPTY, head), [], NOW, dates({}), notes)).toEqual([]);
  expect(notes[0]).toContain("not checked");
});

test("a bun.lock entry from a GitHub or tarball source fails unless the exception file lists it", async () => {
  const head = { cargo: cargoLock([]), bun: bunLock({ foo: ["foo@github:owner/foo#abc", {}, "owner-foo-abc"] }) };
  const problems = await checkAges(addedVersions(EMPTY, head), [], NOW, dates({}));
  expect(problems).toHaveLength(1);
  expect(problems[0]).toContain("not from the npm registry");
  const exceptions = parseExceptions(`[[exception]]\necosystem = "package"\nname = "foo"\nversion = "github:owner/foo#abc"\nreason = "fork with a fix"\n`);
  expect(await checkAges(addedVersions(EMPTY, head), exceptions, NOW, dates({}))).toEqual([]);
});
