import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { afterEach, test } from "node:test";

// Each test runs a copy of the hook in <tmp>/.claude/hooks/ of a new folder.
// The log is in that folder, and <tmp> is the checkout root of the copy.
const HOOK = join(import.meta.dirname, "rules.mjs");
const dirs = [];

afterEach(() => {
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

// Runs the hook. setup gets the checkout root of the copy and returns the
// hook input.
function hookIn(setup) {
  const root = mkdtempSync(join(tmpdir(), "rules-hook-"));
  dirs.push(root);
  const hooks = join(root, ".claude", "hooks");
  mkdirSync(hooks, { recursive: true });
  cpSync(HOOK, join(hooks, "rules.mjs"));
  const input = setup(root);
  const r = spawnSync(process.execPath, [join(hooks, "rules.mjs")], {
    input: typeof input === "string" ? input : JSON.stringify(input),
    encoding: "utf8",
  });
  const logPath = join(hooks, "rules.log");
  return { status: r.status, stdout: r.stdout, stderr: r.stderr, log: existsSync(logPath) ? readFileSync(logPath, "utf8") : "" };
}

const hook = (input) => hookIn(() => input);
const bash = (command) => hook({ tool_name: "Bash", tool_input: { command } });
// A Bash command that runs in folder cwd(root).
const bashIn = (cwd, command) => hookIn((root) => ({ tool_name: "Bash", cwd: cwd(root), tool_input: { command } }));

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

// The bun, yarn and pnpm rule is about this checkout's package manager. A
// command that runs in another repository next to it is allowed.
const outside = (root) => join(dirname(root), `${basename(root)}-other`);

test("bun in the checkout is denied, also through a symbolic link to it", () => {
  denied(bashIn((root) => root, "bun install"), "agent-tool-choice");
  denied(bashIn((root) => join(root, "src"), "bun install"), "agent-tool-choice");
  // tmpdir() on macOS is under /var, a symbolic link to /private/var.
  denied(bashIn((root) => (realpathSync(root) === root ? root : root.replace(/^\/private/, "")), "bun install"), "agent-tool-choice");
});

test("bun outside the checkout is allowed, also in a sibling whose name starts with the checkout's name", () => {
  allowed(bashIn(outside, "bun install"));
  allowed(bashIn(outside, "/Users/x/.bun/bin/bun scripts/a.ts"));
  allowed(bashIn((root) => dirname(root), "pnpm i"));
});

test("a cd before bun decides the folder", () => {
  allowed(bashIn((root) => root, `cd ${dirname(tmpdir())} && bun test`));
  allowed(bashIn((root) => root, "cd ../sibling && pnpm i"));
  denied(bashIn((root) => root, "cd src && pnpm i"), "agent-tool-choice");
  denied(bashIn((root) => root, `cd /elsewhere && cd ${"$"}OLDPWD && bun x`), "agent-tool-choice");
  denied(bashIn(outside, "cd ~/x; cd - && bun x"), "agent-tool-choice");
  denied(bashIn((root) => dirname(root), `cd ${"$"}(pwd) && yarn`), "agent-tool-choice");
});

test("a cd back into the checkout before bun is denied", () => {
  const r = hookIn((root) => ({ tool_name: "Bash", cwd: root, tool_input: { command: `cd /elsewhere && cd ${root} && bun x` } }));
  denied(r, "agent-tool-choice");
});

test("bun with no known folder is denied", () => {
  denied(bash("bun install"), "agent-tool-choice");
});

test("python and prettier stay denied outside the checkout", () => {
  denied(bashIn(outside, "python3 x.py"), "agent-tool-choice");
  denied(bashIn(outside, "npx prettier --write src"), "agent-tool-choice");
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
