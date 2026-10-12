import { expect, test } from "bun:test";
import { join } from "node:path";

const SCRIPT = join(import.meta.dir, "check-pr-title.sh");

function check(title: string) {
  const result = Bun.spawnSync(["bash", SCRIPT], { env: { ...process.env, PR_TITLE: title } });
  return { exitCode: result.exitCode, stderr: result.stderr.toString() };
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
    expect({ title, ...check(title) }).toEqual({ title, exitCode: 0, stderr: "" });
  }
});

test("a + on a type other than feat fails with a message about the +", () => {
  for (const title of ["fix+: typo", "chore(x)+: tidy"]) {
    const result = check(title);
    expect(result.exitCode).not.toBe(0);
    expect(result.stderr).toContain("`+`");
    expect(result.stderr).toContain("`feat`");
  }
});

test("a + together with a ! fails", () => {
  expect(check("feat+!: change the format").exitCode).not.toBe(0);
});

test("a ! on a type that makes no release fails with a message about the !", () => {
  for (const title of ["docs!: rewrite the guide", "ci!: change the runners", "chore(deps)!: x", "test!: x", "style!: x"]) {
    const result = check(title);
    expect(result.exitCode).not.toBe(0);
    expect(result.stderr).toContain("`!`");
  }
});

test("a title with shell characters is read as text", () => {
  const result = check('feat: show "$(touch /tmp/pwned)" and `id` in $HOME');
  expect(result).toEqual({ exitCode: 0, stderr: "" });
});
