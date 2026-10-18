import { afterEach, expect, test } from "bun:test";
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

// Each test runs a copy of the script in a new git repository, so the script's
// root is that repository. A fake `gh` is first on PATH in every test, so no
// test reaches the network or depends on where the real `gh` is.

const SCRIPT = join(import.meta.dir, "ideas.ts");
const dirs: string[] = [];

afterEach(() => {
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

type Gh =
  | { mode: "list"; branches: { headRefName: string; base: string }[] }
  | { mode: "fail" }
  | { mode: "exit127" }
  | { mode: "notJson" }
  | { mode: "absent" };

function git(root: string, ...args: string[]) {
  const r = Bun.spawnSync(["git", "-c", "user.name=t", "-c", "user.email=t@t", ...args], { cwd: root, stderr: "pipe" });
  if (r.exitCode !== 0) throw new Error(`git ${args.join(" ")}: ${r.stderr.toString()}`);
}

function write(root: string, path: string, text: string) {
  mkdirSync(dirname(join(root, path)), { recursive: true });
  writeFileSync(join(root, path), text);
}

function idea(slug: string, extra = "", body = "") {
  return `---\ntitle: ${slug}\ndescription: "An idea"\narea: interface\nadded: 2026-10-01\nsource: "a test"\n${extra}---\n\n# ${slug}\n${body}`;
}

function repo(files: Record<string, string>): string {
  const root = mkdtempSync(join(tmpdir(), "ideas-test-"));
  dirs.push(root);
  cpSync(SCRIPT, join(root, "scripts/ideas.ts"));
  write(root, ".gitignore", "/docs/ideas/**/*-screenshots/\n");
  write(root, "docs/ideas/README.md", "# Ideas\n");
  for (const folder of ["next", "someday", "parked", "done"]) mkdirSync(join(root, "docs/ideas", folder), { recursive: true });
  for (const [path, text] of Object.entries(files)) write(root, path, text);
  git(root, "init", "-q", "-b", "main");
  git(root, "add", "-A");
  git(root, "commit", "-q", "-m", "start");
  return root;
}

// A fake `gh`: it records its arguments, then lists the branches whose base is
// the `--base` it was given, fails, exits 127, or prints text that is not JSON.
// In "absent" mode, PATH has only `git`, so that no `gh` is found at all.
function fakeGh(root: string, gh: Gh): string {
  if (gh.mode === "absent") {
    const bin = join(root, ".nogh");
    mkdirSync(bin, { recursive: true });
    if (!existsSync(join(bin, "git"))) symlinkSync(Bun.which("git")!, join(bin, "git"));
    return bin;
  }
  const bin = join(root, ".bin");
  mkdirSync(bin, { recursive: true });
  const script =
    gh.mode === "exit127"
      ? `echo "gh: command not found" >&2; exit 127`
      : gh.mode === "fail"
        ? `echo "HTTP 401: Bad credentials" >&2; exit 1`
        : gh.mode === "notJson"
          ? `echo "not json"`
          : `printf '%s\\n' "$*" >> "${join(root, ".gh-args")}"
base=""; prev=""
for a in "$@"; do [ "$prev" = "--base" ] && base="$a"; prev="$a"; done
cat <<'JSON' | ${process.execPath} -e 'const b=process.argv[1]; const all=JSON.parse(require("fs").readFileSync(0,"utf8")); console.log(JSON.stringify(all.filter(p=>!b||p.base===b).map(p=>({headRefName:p.headRefName}))))' "$base"
${JSON.stringify(gh.branches)}
JSON`;
  writeFileSync(join(bin, "gh"), `#!/bin/sh\n${script}\n`);
  chmodSync(join(bin, "gh"), 0o755);
  return bin;
}

function run(root: string, args: string[], gh: Gh, ci = false) {
  const bin = fakeGh(root, gh);
  const path = gh.mode === "absent" ? bin : `${bin}:${process.env.PATH}`;
  const env: Record<string, string> = { ...process.env, PATH: path } as Record<string, string>;
  delete env.CI;
  delete env.GITHUB_ACTIONS;
  if (ci) env.CI = "true";
  const r = Bun.spawnSync([process.execPath, join(root, "scripts/ideas.ts"), ...args], { cwd: root, env, stderr: "pipe" });
  return { exitCode: r.exitCode, stdout: r.stdout.toString(), stderr: r.stderr.toString() };
}

const noPrs: Gh = { mode: "list", branches: [] };

// ---------------------------------------------------------------- move

test("move takes the screenshots folder of the idea with it", () => {
  const root = repo({
    "docs/ideas/next/x.md": idea("x", "", "\n![Shot](x-screenshots/a.png)\n"),
    "docs/notes.md": "See `docs/ideas/next/x-screenshots/a.png` and `docs/ideas/next/x-screenshots.png`.\n",
  });
  write(root, "docs/ideas/next/x-screenshots/a.png", "png");
  const r = run(root, ["move", "x", "done"], noPrs);
  expect({ exitCode: r.exitCode, stderr: r.stderr }).toEqual({ exitCode: 0, stderr: "" });
  expect(readFileSync(join(root, "docs/ideas/done/x-screenshots/a.png"), "utf8")).toBe("png");
  expect(existsSync(join(root, "docs/ideas/next/x-screenshots"))).toBe(false);
  expect(readFileSync(join(root, "docs/ideas/done/x.md"), "utf8")).toContain("](x-screenshots/a.png)");
  expect(readFileSync(join(root, "docs/notes.md"), "utf8")).toBe(
    "See `docs/ideas/done/x-screenshots/a.png` and `docs/ideas/next/x-screenshots.png`.\n",
  );
});

for (const withSource of [true, false])
  test(`move stops when the target folder already has a screenshots folder (${withSource ? "with" : "with no"} folder beside the idea)`, () => {
    const root = repo({ "docs/ideas/next/x.md": idea("x") });
    if (withSource) write(root, "docs/ideas/next/x-screenshots/a.png", "mine");
    write(root, "docs/ideas/done/x-screenshots/b.png", "other");
    const r = run(root, ["move", "x", "done"], noPrs);
    expect(r.exitCode).not.toBe(0);
    expect(r.stderr).toContain("x-screenshots");
    expect(existsSync(join(root, "docs/ideas/next/x.md"))).toBe(true);
    expect(existsSync(join(root, "docs/ideas/done/x.md"))).toBe(false);
    if (withSource) expect(readFileSync(join(root, "docs/ideas/next/x-screenshots/a.png"), "utf8")).toBe("mine");
  });

// ---------------------------------------------------------------- check

function shippedRepo() {
  return repo({
    "docs/plans/p.md": "---\ntopic: talk-to-notes\n---\n# Plan\n",
    "docs/ideas/next/x.md": idea("x", "plan: docs/plans/p.md\n"),
  });
}

const squashMerged: Gh = { mode: "list", branches: [{ headRefName: "worktree-talk-to-notes", base: "main" }] };

test("check reports an open idea whose plan branch merged into main by squash", () => {
  const root = shippedRepo();
  const r = run(root, ["check"], squashMerged);
  expect(r.exitCode).toBe(1);
  expect(r.stderr).toContain("docs/ideas/next/x.md: branch worktree-talk-to-notes of its plan merged");
  expect(readFileSync(join(root, ".gh-args"), "utf8")).toBe("pr list --state merged --base main --limit 1000 --json headRefName\n");
});

test("check reports an open idea whose codex plan branch merged", () => {
  const root = shippedRepo();
  const r = run(root, ["check"], { mode: "list", branches: [{ headRefName: "codex/talk-to-notes", base: "main" }] });
  expect(r.exitCode).toBe(1);
  expect(r.stderr).toContain("branch codex/talk-to-notes of its plan merged");
});

test("check says that it did not read all merged PRs when gh gives the full limit", () => {
  const root = shippedRepo();
  const branches = Array.from({ length: 1000 }, (_, n) => ({ headRefName: `b${n}`, base: "main" }));
  const r = run(root, ["check"], { mode: "list", branches });
  expect(r.stderr).toContain("Shipped-plan check: merged PRs not read (more than 1000 merged PRs)");
});

test("check does not count a PR that merged into another branch", () => {
  const root = shippedRepo();
  const r = run(root, ["check"], { mode: "list", branches: [{ headRefName: "worktree-talk-to-notes", base: "release" }] });
  expect(r.exitCode).toBe(0);
});

test("in CI, a shipped idea is a warning and does not fail the check", () => {
  const root = shippedRepo();
  const r = run(root, ["check"], squashMerged, true);
  expect(r.exitCode).toBe(0);
  expect(r.stdout + r.stderr).toContain("::warning file=docs/ideas/next/x.md::");
});

for (const [mode, reason] of [
  ["fail", "HTTP 401: Bad credentials"],
  ["exit127", "gh: command not found"],
  ["notJson", "gh gave output that is not JSON"],
  ["absent", "gh is not installed"],
] as const)
  test(`check says that it did not read merged PRs and passes when gh gives "${reason}"`, () => {
    const root = shippedRepo();
    const r = run(root, ["check"], { mode });
    expect(r.exitCode).toBe(0);
    expect(r.stderr).toContain(`Shipped-plan check: merged PRs not read (${reason}); merge commits only.`);
  });

test("in CI, check fails when it cannot read merged PRs", () => {
  const root = shippedRepo();
  const r = run(root, ["check"], { mode: "fail" }, true);
  expect(r.exitCode).toBe(1);
  expect(r.stderr).toContain("Shipped-plan check: merged PRs not read");
});

test("in CI, check fails when it cannot read merged PRs also with no open plan", () => {
  const root = repo({ "docs/ideas/someday/y.md": idea("y") });
  const r = run(root, ["check"], { mode: "absent" }, true);
  expect(r.exitCode).toBe(1);
  expect(r.stderr).toContain("Shipped-plan check: merged PRs not read");
  expect(run(root, ["check"], { mode: "absent" }).exitCode).toBe(0);
});

test("check still reports from a merge commit when gh is missing", () => {
  const root = shippedRepo();
  git(root, "checkout", "-q", "-b", "worktree-talk-to-notes");
  write(root, "docs/work.md", "work\n");
  git(root, "add", "-A");
  git(root, "commit", "-q", "-m", "work");
  git(root, "checkout", "-q", "main");
  git(root, "merge", "-q", "--no-ff", "-m", "Merge pull request #7 from owner/worktree-talk-to-notes", "worktree-talk-to-notes");
  const r = run(root, ["check"], { mode: "absent" });
  expect(r.exitCode).toBe(1);
  expect(r.stderr).toContain("branch worktree-talk-to-notes of its plan merged");
});
