import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { test } from "node:test";

const SCRIPT = join(import.meta.dirname, "check-pr-title.sh");

function check(title: string) {
  const result = spawnSync("bash", [SCRIPT], { env: { ...process.env, PR_TITLE: title }, encoding: "utf8" });
  return { exitCode: result.status, stderr: result.stderr };
}

test("titles with correct markers or no markers pass", () => {
  for (const title of [
    "feat: add export",
    "feat(capture)+: record system audio",
    "feat+: add notes",
    "feat!: change the storage format",
    "fix!: change the file names",
    "perf!: x",
    "refactor(store)!: x",
    "revert!: x",
    "build!: x",
    "fix(capture): keep the timer visible",
    "docs: explain releases",
  ]) {
    assert.deepEqual({ title, ...check(title) }, { title, exitCode: 0, stderr: "" });
  }
});

test("a + on a type other than feat fails with a message about the +", () => {
  for (const title of ["fix+: typo", "chore(x)+: tidy"]) {
    const result = check(title);
    assert.notEqual(result.exitCode, 0);
    assert.ok(result.stderr.includes("`+`"));
    assert.ok(result.stderr.includes("`feat`"));
  }
});

test("a + together with a ! fails", () => {
  assert.notEqual(check("feat+!: change the format").exitCode, 0);
});

test("a ! on a type that makes no release fails with a message about the !", () => {
  for (const title of ["docs!: rewrite the guide", "ci!: change the runners", "chore(deps)!: x", "test!: x", "style!: x"]) {
    const result = check(title);
    assert.notEqual(result.exitCode, 0);
    assert.ok(result.stderr.includes("`!`"));
  }
});

test("a title with shell characters is read as text", () => {
  const result = check('feat: show "$(touch /tmp/pwned)" and `id` in $HOME');
  assert.deepEqual(result, { exitCode: 0, stderr: "" });
});
