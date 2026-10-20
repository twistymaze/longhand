import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, test } from "node:test";
import { decide, latestReleaseTag, releaseFacts, systemTools, type Tools } from "./release-version.ts";

const CONFIG = join(import.meta.dirname, "..", "cliff.toml");
const SCRIPT = join(import.meta.dirname, "release-version.ts");
const repos: string[] = [];

after(() => {
  for (const repo of repos) rmSync(repo, { recursive: true, force: true });
});

function git(repo: string, args: string[]): void {
  const result = spawnSync("git", ["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", ...args], { cwd: repo, encoding: "utf8" });
  if (result.status !== 0) throw new Error(`git ${args.join(" ")} failed: ${result.error?.message ?? result.stderr}`);
}

/// Makes a temporary repository. Each step is a commit subject, or `tag:<name>`
/// to tag the current commit.
function repository(steps: string[]): string {
  const repo = mkdtempSync(join(tmpdir(), "release-version-"));
  repos.push(repo);
  git(repo, ["init", "-q", "-b", "main"]);
  git(repo, ["config", "user.name", "Test"]);
  git(repo, ["config", "user.email", "test@example.test"]);
  for (const step of steps) {
    if (step.startsWith("tag:")) git(repo, ["tag", step.slice(4)]);
    else git(repo, ["commit", "-q", "--allow-empty", "-m", step]);
  }
  return repo;
}

function facts(steps: string[]) {
  return releaseFacts(systemTools(repository(steps), CONFIG));
}

// Real repositories and the real git-cliff.

test("a fix after v1.0.3 gives a due release 1.0.4", () => {
  const result = facts(["feat: start", "tag:v1.0.3", "fix: keep the recording timer visible"]);
  assert.equal(result.due, true);
  assert.equal(result.version, "1.0.4");
});

test("a feat and a fix after v1.0.4 give a due patch release 1.0.5", () => {
  const result = facts(["feat: start", "tag:v1.0.4", "feat: add note templates", "fix: keep the timer visible"]);
  assert.equal(result.due, true);
  assert.equal(result.version, "1.0.5");
});

test("a feat with a scope and a fix after v1.1.0 give 1.1.1 under two headings", () => {
  const result = facts(["feat: start", "tag:v1.1.0", "feat(about): show the version", "fix: typo"]);
  assert.equal(result.version, "1.1.1");
  const headings = (result.notes ?? "").split("\n").filter((line) => line.startsWith("### "));
  assert.deepEqual(headings, ["### New features and improvements", "### Fixes"]);
});

test("a feat+ commit with a scope after v1.1.0 gives a minor release 1.2.0 and notes without the +", () => {
  const result = facts(["feat: start", "tag:v1.1.0", "feat(capture)+: record system audio", "fix: typo"]);
  assert.equal(result.due, true);
  assert.equal(result.version, "1.2.0");
  assert.ok(result.notes!.includes("capture: Record system audio"));
  assert.ok(!result.notes!.includes(")+:"));
  assert.ok(!result.notes!.includes("audio+"));
});

test("a feat+ commit without a scope after v1.1.0 gives 1.2.0 and appears in the notes", () => {
  const result = facts(["feat: start", "tag:v1.1.0", "feat+: plain plus (#12)"]);
  assert.equal(result.version, "1.2.0");
  assert.ok(result.notes!.includes("Plain plus (#12)"));
});

test("a feat+ commit followed by fix and chore commits still gives 1.2.0", () => {
  const result = facts(["feat: start", "tag:v1.1.0", "feat(capture)+: record system audio", "fix: typo", "chore: tidy"]);
  assert.equal(result.version, "1.2.0");
});

test("a feat+ commit and a fix! commit after v1.4.2 give 2.0.0", () => {
  const result = facts(["feat: start", "tag:v1.4.2", "feat(capture)+: record system audio", "fix!: change the file names"]);
  assert.equal(result.version, "2.0.0");
});

test("a feat+ commit before v1.1.1 and only a fix after it give 1.1.2", () => {
  const result = facts(["feat: start", "tag:v1.1.0", "feat(capture)+: record system audio", "tag:v1.1.1", "fix: typo"]);
  assert.equal(result.version, "1.1.2");
});

test("a revert title that quotes a feat+ title gives a patch release", () => {
  const result = facts(["feat: start", "tag:v1.1.0", "revert: feat(capture)+: record system audio"]);
  assert.equal(result.version, "1.1.1");
});

test("a fix+ title gives a patch release", () => {
  const result = facts(["feat: start", "tag:v1.1.0", "fix+: typo"]);
  assert.equal(result.version, "1.1.1");
});

test("only a docs! commit after v1.1.0 gives no due release", () => {
  const result = facts(["feat: start", "tag:v1.1.0", "docs!: rewrite the guide"]);
  assert.equal(result.due, false);
  assert.equal(result.version, "1.1.0");
});

test("only test, docs and ci commits after v1.0.4 give no due release", () => {
  const result = facts(["feat: start", "tag:v1.0.4", "test: cover the folder watcher", "docs: explain releases", "ci: cache bun"]);
  assert.equal(result.due, false);
  assert.equal(result.version, "1.0.4");
  assert.equal(result.notes, null);
});

test("the notes list the fix and not the earlier test commit", () => {
  const result = facts(["feat: start", "tag:v1.0.4", "test: cover the folder watcher", "fix: keep the timer visible"]);
  assert.equal(result.due, true);
  assert.equal(result.version, "1.0.5");
  assert.ok(result.notes!.includes("Keep the timer visible"));
  assert.ok(!result.notes!.includes("folder watcher"));
  assert.ok(!result.notes!.includes("start"));
});

test("a feat! commit after v1.4.2 gives a due release 2.0.0", () => {
  const result = facts(["feat: start", "tag:v1.4.2", "feat!: change the storage format"]);
  assert.equal(result.due, true);
  assert.equal(result.version, "2.0.0");
});

test("a feat commit with an empty body gives a patch release", () => {
  const result = facts(["feat: start", "tag:v1.4.2", "feat: add note templates (#12)"]);
  assert.equal(result.version, "1.4.3");
});

test("mixed and unconventional history with no tag gives a due release 1.0.0", () => {
  const result = facts(["Initial commit", "feat: add recording", "feat+: add notes", "fix: stop the crash", "Merge branch x", "feat!: new format"]);
  assert.equal(result.due, true);
  assert.equal(result.version, "1.0.0");
  assert.ok(result.notes!.includes("Add recording"));
  assert.ok(!result.notes!.includes("Initial commit"));
});

test("the notes put the commits under three headings in a fixed order", () => {
  const result = facts([
    "feat: start",
    "tag:v1.0.0",
    "build(deps): update tauri",
    "perf: load the note list faster",
    "fix: keep the timer visible",
    "feat: add note templates",
  ]);
  const notes = result.notes ?? "";
  const headings = notes.split("\n").filter((line) => line.startsWith("### "));
  assert.deepEqual(headings, ["### New features and improvements", "### Fixes", "### Other changes"]);
  const fixes = notes.slice(notes.indexOf("### Fixes"), notes.indexOf("### Other changes"));
  assert.ok(fixes.includes("Load the note list faster"));
  assert.ok(fixes.includes("Keep the timer visible"));
  assert.ok(!notes.includes("Bug fixes"));
});

test("a build(deps) commit gives a patch release", () => {
  assert.equal(facts(["feat: start", "tag:v1.0.0", "build(deps): update tauri"]).version, "1.0.1");
});

test("a refactor commit gives a patch release", () => {
  assert.equal(facts(["feat: start", "tag:v1.0.0", "refactor: split the capture module"]).version, "1.0.1");
});

test("a preview tag does not count as a release tag", () => {
  const result = facts(["feat: start", "tag:v1.0.0", "fix: one", "tag:v0.1.0-preview.1", "fix: two"]);
  assert.equal(result.due, true);
  assert.equal(result.version, "1.0.1");
  const first = facts(["feat: start", "tag:v0.1.0-preview.1", "fix: one"]);
  assert.equal(first.version, "1.0.0");
});

test("the CLI prints due and version and writes the notes file", () => {
  const repo = repository(["feat: start", "tag:v1.0.3", "fix: keep the timer visible"]);
  const notes = join(repo, "notes.md");
  const result = spawnSync(process.execPath, [SCRIPT, notes], { cwd: repo, encoding: "utf8" });
  assert.equal(result.status, 0);
  assert.equal(result.stdout, "due=true\nversion=1.0.4\n");
  assert.ok(readFileSync(notes, "utf8").includes("Keep the timer visible"));
});

test("the CLI fails with a clear message when git-cliff is missing", () => {
  const repo = repository(["feat: start", "tag:v1.0.3", "fix: keep the timer visible"]);
  const result = spawnSync(process.execPath, [SCRIPT, join(repo, "notes.md")], {
    cwd: repo,
    encoding: "utf8",
    env: { ...process.env, PATH: "/usr/bin:/bin" },
  });
  assert.notEqual(result.status, 0);
  assert.ok(result.stderr.includes("git-cliff"));
  assert.ok(!result.stdout.includes("due="));
});

// Pure logic with a fake git-cliff.

function fakeTools(tags: string[], bumped: string, calls: string[][] = [], subjects: string[] = []): Tools {
  return {
    tags: () => tags,
    cliff: (args) => {
      calls.push(args);
      return args.includes("--bumped-version") ? `${bumped}\n` : "### Fixes\n- Fix one\n";
    },
    subjects: (since) => {
      calls.push(["subjects", since]);
      return subjects;
    },
  };
}

test("the latest release tag is the highest v<major>.<minor>.<patch> tag", () => {
  assert.equal(latestReleaseTag(["v1.0.9", "v1.0.10", "v0.1.0-preview.1", "preview-20261010", "v2.0"]), "v1.0.10");
  assert.equal(latestReleaseTag(["v0.1.0-preview.1"]), null);
  assert.equal(latestReleaseTag([]), null);
});

test("no release tag makes a release due with the bumped version", () => {
  assert.deepEqual(decide(null, "v1.0.0"), { due: true, version: "1.0.0" });
});

test("a bumped version equal to the latest tag is not due, and a different one is due", () => {
  assert.deepEqual(decide("v1.0.4", "v1.0.4"), { due: false, version: "1.0.4" });
  assert.deepEqual(decide("v1.0.4", "v1.1.0"), { due: true, version: "1.1.0" });
});

test("a bumped version that is not X.Y.Z fails", () => {
  assert.throws(() => decide("v1.0.4", ""), /not a version/);
  assert.throws(() => decide(null, "v1.0.0-rc.1"), /not a version/);
});

test("a due release requests the notes for its tag with all headers and footers removed", () => {
  const calls: string[][] = [];
  const result = releaseFacts(fakeTools(["v1.0.3"], "v1.0.4", calls));
  assert.deepEqual(result, { due: true, version: "1.0.4", notes: "### Fixes\n- Fix one\n" });
  assert.deepEqual(calls, [["--bumped-version"], ["subjects", "v1.0.3"], ["--unreleased", "--tag", "v1.0.4", "--strip", "all"]]);
});

test("a feat+ subject raises the release to the next minor, and the notes use that tag", () => {
  const calls: string[][] = [];
  const result = releaseFacts(fakeTools(["v1.1.0"], "v1.1.1", calls, ["fix: typo", "feat(capture)+: record system audio"]));
  assert.equal(result.version, "1.2.0");
  assert.deepEqual(calls.at(-1), ["--unreleased", "--tag", "v1.2.0", "--strip", "all"]);
});

test("a git-cliff version that is not the next patch fails", () => {
  assert.throws(() => releaseFacts(fakeTools(["v1.1.0"], "v1.2.0", [], ["feat(capture)+: record system audio"])), /next patch/);
});

test("a failed subjects read fails the release decision", () => {
  const tools = fakeTools(["v1.1.0"], "v1.1.1");
  tools.subjects = () => {
    throw new Error("git log failed");
  };
  assert.throws(() => releaseFacts(tools), /git log failed/);
});

test("with no release tag the release decision reads no subjects", () => {
  const calls: string[][] = [];
  assert.equal(releaseFacts(fakeTools([], "v1.0.0", calls, ["feat+: add notes"])).version, "1.0.0");
  assert.equal(calls.some((call) => call[0] === "subjects"), false);
});

test("a release that is not due requests no notes", () => {
  const calls: string[][] = [];
  assert.deepEqual(releaseFacts(fakeTools(["v1.0.3"], "v1.0.3", calls)), { due: false, version: "1.0.3", notes: null });
  assert.deepEqual(calls, [["--bumped-version"]]);
});
