import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, statSync, utimesSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, test } from "node:test";
import { readChangelog, writeLocal } from "./changelog.ts";
import { releaseNotes } from "./lib/changelog.ts";

const CONFIG = join(import.meta.dirname, "..", "cliff.toml");
const SCRIPT = join(import.meta.dirname, "changelog.ts");
const folders: string[] = [];

after(() => {
  for (const folder of folders) rmSync(folder, { recursive: true, force: true });
});

function temporary(prefix: string): string {
  const folder = mkdtempSync(join(tmpdir(), prefix));
  folders.push(folder);
  return folder;
}

function git(repo: string, args: string[]): string {
  const result = spawnSync("git", ["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", ...args], { cwd: repo, encoding: "utf8" });
  if (result.status !== 0) throw new Error(`git ${args.join(" ")} failed: ${result.error?.message ?? result.stderr}`);
  return result.stdout;
}

/// Makes a temporary repository. Each step is a commit subject, or `tag:<name>`.
function repository(steps: string[]): string {
  const repo = temporary("changelog-");
  git(repo, ["init", "-q", "-b", "main"]);
  git(repo, ["config", "user.name", "Test"]);
  git(repo, ["config", "user.email", "test@example.test"]);
  for (const step of steps) {
    if (step.startsWith("tag:")) git(repo, ["tag", step.slice(4)]);
    else git(repo, ["commit", "-q", "--allow-empty", "-m", step]);
  }
  return repo;
}

// Real repositories and the real git-cliff.

test("the changelog holds the release commits after v1.0.0, with the head commit", () => {
  const repo = repository(["feat: before the first release", "tag:v1.0.0", "fix(ui): keep the timer visible (#3)", "docs: explain", "feat+: record audio (#4)"]);
  const data = readChangelog(repo, CONFIG);
  assert.ok(data.available);
  assert.deepEqual(
    data.entries.map((entry) => entry.line),
    ["ui: Keep the timer visible (#3)", "Record audio (#4)"],
  );
  assert.equal(data.head.id, git(repo, ["rev-parse", "HEAD"]).trim());
  assert.equal(data.entries[0].id.length, 40);
});

test("the changelog lines are the release notes lines of the same commits", () => {
  const repo = repository(["feat: start", "tag:v1.0.0", "fix: one (#1)", "feat(capture)+: two (#2)", "tag:v1.1.0", "build(deps): three"]);
  const data = readChangelog(repo, CONFIG);
  assert.ok(data.available);
  const context = spawnSync("git-cliff", ["--config", CONFIG, "--repository", repo, "--context", "v1.0.0..v1.1.0"], { encoding: "utf8" });
  const bullets = releaseNotes(JSON.parse(context.stdout))
    .split("\n")
    .filter((line) => line.startsWith("- "))
    .map((line) => line.slice(2));
  assert.deepEqual(bullets, ["capture: Two (#2)", "One (#1)"]);
  for (const bullet of bullets) assert.ok(data.entries.some((entry) => entry.line === bullet), bullet);
  assert.deepEqual(
    data.entries.map((entry) => entry.line),
    ["One (#1)", "capture: Two (#2)", "deps: Three"],
  );
});

test("write makes changelog.json, CHANGELOG.md and index.html", () => {
  const repo = repository(["feat: start", "tag:v1.0.0", "fix: one (#1)"]);
  const out = temporary("changelog-out-");
  const result = spawnSync(process.execPath, [SCRIPT, "write", out], { cwd: repo, encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(JSON.parse(readFileSync(join(out, "changelog.json"), "utf8")).entries[0].pr, 1);
  assert.ok(readFileSync(join(out, "CHANGELOG.md"), "utf8").includes("[#1](https://github.com/twistymaze/longhand/pull/1)"));
  assert.ok(readFileSync(join(out, "index.html"), "utf8").includes("<h3>Fixes</h3>"));
});

test("write fails when the history has no v1.0.0 tag", () => {
  const repo = repository(["feat: start", "fix: one"]);
  const result = spawnSync(process.execPath, [SCRIPT, "write", temporary("changelog-out-")], { cwd: repo, encoding: "utf8" });
  assert.notEqual(result.status, 0);
  assert.ok(result.stderr.includes("v1.0.0"));
});

test("local with no v1.0.0 tag writes the not-available file", () => {
  const repo = repository(["feat: start", "fix: one"]);
  const file = join(temporary("changelog-out-"), "changelog.json");
  assert.ok(writeLocal(repo, CONFIG, file));
  assert.deepEqual(JSON.parse(readFileSync(file, "utf8")), { format: 1, available: false });
});

test("local does not write the file again when its text is the same", () => {
  const repo = repository(["feat: start", "tag:v1.0.0", "fix: one (#1)"]);
  const file = join(temporary("changelog-out-"), "changelog.json");
  assert.equal(writeLocal(repo, CONFIG, file), null);
  // Set an old file time. A new write would set the time to now.
  const old = new Date("2020-01-01T00:00:00Z");
  utimesSync(file, old, old);
  assert.equal(writeLocal(repo, CONFIG, file), null);
  assert.equal(statSync(file).mtimeMs, old.getTime());
});

test("local with no git-cliff on the path writes the not-available file and ends without an error", () => {
  const repo = repository(["feat: start", "tag:v1.0.0", "fix: one"]);
  const file = join(temporary("changelog-out-"), "changelog.json");
  const result = spawnSync(process.execPath, [SCRIPT, "local", file], {
    cwd: repo,
    encoding: "utf8",
    env: { ...process.env, PATH: "/usr/bin:/bin", LONGHAND_CHANGELOG: "" },
  });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(JSON.parse(readFileSync(file, "utf8")), { format: 1, available: false });
  assert.ok(result.stdout.includes("not available"));
});

test("local writes nothing when LONGHAND_CHANGELOG is set", () => {
  const repo = repository(["feat: start", "tag:v1.0.0", "fix: one"]);
  const file = join(temporary("changelog-out-"), "changelog.json");
  const result = spawnSync(process.execPath, [SCRIPT, "local", file], {
    cwd: repo,
    encoding: "utf8",
    env: { ...process.env, LONGHAND_CHANGELOG: "/elsewhere/changelog.json" },
  });
  assert.equal(result.status, 0, result.stderr);
  assert.throws(() => readFileSync(file));
});
