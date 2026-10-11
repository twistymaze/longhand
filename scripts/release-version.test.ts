import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { decide, latestReleaseTag, releaseFacts, systemTools, type Tools } from "./release-version";

const CONFIG = join(import.meta.dir, "..", "cliff.toml");
const SCRIPT = join(import.meta.dir, "release-version.ts");
const repos: string[] = [];

afterAll(() => {
  for (const repo of repos) rmSync(repo, { recursive: true, force: true });
});

function git(repo: string, args: string[]): void {
  const result = Bun.spawnSync(["git", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", ...args], { cwd: repo });
  if (result.exitCode !== 0) throw new Error(`git ${args.join(" ")} failed: ${result.stderr.toString()}`);
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
  expect(result.due).toBe(true);
  expect(result.version).toBe("1.0.4");
});

test("a feat and a fix after v1.0.4 give a due release 1.1.0", () => {
  const result = facts(["feat: start", "tag:v1.0.4", "feat: add note templates", "fix: keep the timer visible"]);
  expect(result.due).toBe(true);
  expect(result.version).toBe("1.1.0");
});

test("only test, docs and ci commits after v1.0.4 give no due release", () => {
  const result = facts(["feat: start", "tag:v1.0.4", "test: cover the folder watcher", "docs: explain releases", "ci: cache bun"]);
  expect(result.due).toBe(false);
  expect(result.version).toBe("1.0.4");
  expect(result.notes).toBeNull();
});

test("the notes list the fix and not the earlier test commit", () => {
  const result = facts(["feat: start", "tag:v1.0.4", "test: cover the folder watcher", "fix: keep the timer visible"]);
  expect(result.due).toBe(true);
  expect(result.version).toBe("1.0.5");
  expect(result.notes).toContain("Keep the timer visible");
  expect(result.notes).not.toContain("folder watcher");
  expect(result.notes).not.toContain("start");
});

test("a feat! commit after v1.4.2 gives a due release 2.0.0", () => {
  const result = facts(["feat: start", "tag:v1.4.2", "feat!: change the storage format"]);
  expect(result.due).toBe(true);
  expect(result.version).toBe("2.0.0");
});

test("a feat commit with an empty body gives a minor release", () => {
  const result = facts(["feat: start", "tag:v1.4.2", "feat: add note templates (#12)"]);
  expect(result.version).toBe("1.5.0");
});

test("mixed and unconventional history with no tag gives a due release 1.0.0", () => {
  const result = facts(["Initial commit", "feat: add recording", "fix: stop the crash", "Merge branch x", "feat!: new format"]);
  expect(result.due).toBe(true);
  expect(result.version).toBe("1.0.0");
  expect(result.notes).toContain("Add recording");
  expect(result.notes).not.toContain("Initial commit");
});

test("a build(deps) commit gives a patch release", () => {
  expect(facts(["feat: start", "tag:v1.0.0", "build(deps): update tauri"]).version).toBe("1.0.1");
});

test("a refactor commit gives a patch release", () => {
  expect(facts(["feat: start", "tag:v1.0.0", "refactor: split the capture module"]).version).toBe("1.0.1");
});

test("a preview tag does not count as a release tag", () => {
  const result = facts(["feat: start", "tag:v1.0.0", "fix: one", "tag:v0.1.0-preview.1", "fix: two"]);
  expect(result.due).toBe(true);
  expect(result.version).toBe("1.0.1");
  const first = facts(["feat: start", "tag:v0.1.0-preview.1", "fix: one"]);
  expect(first.version).toBe("1.0.0");
});

test("the CLI prints due and version and writes the notes file", () => {
  const repo = repository(["feat: start", "tag:v1.0.3", "fix: keep the timer visible"]);
  const notes = join(repo, "notes.md");
  const result = Bun.spawnSync([process.execPath, SCRIPT, notes], { cwd: repo });
  expect(result.exitCode).toBe(0);
  expect(result.stdout.toString()).toBe("due=true\nversion=1.0.4\n");
  expect(readFileSync(notes, "utf8")).toContain("Keep the timer visible");
});

test("the CLI fails with a clear message when git-cliff is missing", () => {
  const repo = repository(["feat: start", "tag:v1.0.3", "fix: keep the timer visible"]);
  const result = Bun.spawnSync([process.execPath, SCRIPT, join(repo, "notes.md")], {
    cwd: repo,
    env: { ...process.env, PATH: "/usr/bin:/bin" },
  });
  expect(result.exitCode).not.toBe(0);
  expect(result.stderr.toString()).toContain("git-cliff");
  expect(result.stdout.toString()).not.toContain("due=");
});

// Pure logic with a fake git-cliff.

function fakeTools(tags: string[], bumped: string, calls: string[][] = []): Tools {
  return {
    tags: () => tags,
    cliff: (args) => {
      calls.push(args);
      return args.includes("--bumped-version") ? `${bumped}\n` : "### Bug fixes\n- Fix one\n";
    },
  };
}

test("the latest release tag is the highest v<major>.<minor>.<patch> tag", () => {
  expect(latestReleaseTag(["v1.0.9", "v1.0.10", "v0.1.0-preview.1", "preview-20261010", "v2.0"])).toBe("v1.0.10");
  expect(latestReleaseTag(["v0.1.0-preview.1"])).toBeNull();
  expect(latestReleaseTag([])).toBeNull();
});

test("no release tag makes a release due with the bumped version", () => {
  expect(decide(null, "v1.0.0")).toEqual({ due: true, version: "1.0.0" });
});

test("a bumped version equal to the latest tag is not due, and a different one is due", () => {
  expect(decide("v1.0.4", "v1.0.4")).toEqual({ due: false, version: "1.0.4" });
  expect(decide("v1.0.4", "v1.1.0")).toEqual({ due: true, version: "1.1.0" });
});

test("a bumped version that is not X.Y.Z fails", () => {
  expect(() => decide("v1.0.4", "")).toThrow(/not a version/);
  expect(() => decide(null, "v1.0.0-rc.1")).toThrow(/not a version/);
});

test("a due release requests the notes for its tag with all headers and footers removed", () => {
  const calls: string[][] = [];
  const result = releaseFacts(fakeTools(["v1.0.3"], "v1.0.4", calls));
  expect(result).toEqual({ due: true, version: "1.0.4", notes: "### Bug fixes\n- Fix one\n" });
  expect(calls).toEqual([["--bumped-version"], ["--unreleased", "--tag", "v1.0.4", "--strip", "all"]]);
});

test("a release that is not due requests no notes", () => {
  const calls: string[][] = [];
  expect(releaseFacts(fakeTools(["v1.0.3"], "v1.0.3", calls))).toEqual({ due: false, version: "1.0.3", notes: null });
  expect(calls).toEqual([["--bumped-version"]]);
});
