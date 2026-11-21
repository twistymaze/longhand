import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test } from "node:test";

// Each test runs a copy of the hook in a new folder, so that the log is in
// that folder.
const HOOK = join(import.meta.dirname, "rules.mjs");
const dirs = [];

afterEach(() => {
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

function hook(input) {
  const dir = mkdtempSync(join(tmpdir(), "rules-hook-"));
  dirs.push(dir);
  cpSync(HOOK, join(dir, "rules.mjs"));
  const r = spawnSync(process.execPath, [join(dir, "rules.mjs")], {
    input: typeof input === "string" ? input : JSON.stringify(input),
    encoding: "utf8",
  });
  const logPath = join(dir, "rules.log");
  return { status: r.status, stdout: r.stdout, stderr: r.stderr, log: existsSync(logPath) ? readFileSync(logPath, "utf8") : "" };
}

const bash = (command) => hook({ tool_name: "Bash", tool_input: { command } });

function denied(r, family) {
  assert.equal(r.status, 2, r.stderr);
  assert.match(r.stderr, new RegExp(`^${family}: `));
}

function allowed(r) {
  assert.equal(r.status, 0, r.stderr);
  assert.equal(r.stdout, "");
}

test("a feat PR title asks with the patch-or-minor question", () => {
  const r = bash('gh pr create -R twistymaze/longhand --title "feat: x" --body "y"');
  assert.equal(r.status, 0);
  const out = JSON.parse(r.stdout).hookSpecificOutput;
  assert.equal(out.permissionDecision, "ask");
  assert.match(out.permissionDecisionReason, /patch or minor\?/);
});

test("a PR title with ! is denied", () => {
  denied(bash('gh pr create --title "fix!: x"'), "agent-release-title");
  denied(bash('gh pr edit 12 --title="feat(capture)!: x"'), "agent-release-title");
});

test("Compound Engineering branding in a PR body is denied", () => {
  denied(bash('gh pr create --title "fix: x" --body "Made with Compound Engineering"'), "agent-pr-branding");
});

test("a CI re-run asks", () => {
  const r = bash("gh run rerun 123 --failed");
  assert.equal(JSON.parse(r.stdout).hookSpecificOutput.permissionDecision, "ask");
});

test("a git mv of an idea file is denied and names the ideas script", () => {
  const r = bash("git mv docs/ideas/someday/a.md docs/ideas/next/a.md");
  denied(r, "agent-ideas-move");
  assert.match(r.stderr, /node scripts\/ideas\.ts move/);
});

test("uv run python is allowed, and python3 is denied", () => {
  allowed(bash("uv run --no-project python x.py"));
  denied(bash("python3 x.py"), "agent-tool-choice");
  denied(bash("cd scripts && pip install x"), "agent-tool-choice");
});

test("bun, yarn, pnpm and prettier are denied", () => {
  denied(bash("bun install"), "agent-tool-choice");
  denied(bash("npx prettier --write src"), "agent-tool-choice");
  denied(bash("LANG=C yarn"), "agent-tool-choice");
});

test("a commit message or a PR body that names a tool or BREAKING CHANGE is allowed", () => {
  allowed(bash('git commit -m "no pip or prettier here"'));
  allowed(bash('gh pr create --title "chore: x" --body "no BREAKING CHANGE"'));
});

test("a here-document body is text, and the command after it is checked", () => {
  allowed(bash("cat > x.sh <<EOF\npython3 a.py\nEOF"));
  allowed(bash("cat > x.sh <<-'END'\n\tpip install x\n\tEND\necho done"));
  denied(bash("cat > x.sh <<EOF\nok\nEOF\npython3 a.py"), "agent-tool-choice");
});

test("an edit in reference-source is denied, and an edit elsewhere is allowed", () => {
  denied(hook({ tool_name: "Edit", tool_input: { file_path: "/repo/reference-source/handy/README.md" } }), "agent-reference-source");
  allowed(hook({ tool_name: "Write", tool_input: { file_path: "/repo/src/a.ts" } }));
});

test("a forced git add of a private path is denied", () => {
  denied(bash("git add -f manual-tests/notes.md"), "agent-private-paths");
  allowed(bash("git add src/a.ts"));
});

test("a copy into /Applications is denied", () => {
  denied(bash("ditto Longhand.app /Applications/Longhand.app"), "agent-test-build-location");
});

test("a deny adds one line to the log, and an allowed command adds none", () => {
  const r = bash("pnpm i");
  assert.match(r.log, /^\S+\tagent-tool-choice\tdeny\n$/);
  assert.equal(bash("ls").log, "");
});

test("malformed hook input prints a warning and allows the command", () => {
  const r = hook("not json");
  assert.equal(r.status, 0);
  assert.match(r.stderr, /the command is not checked/);
});
