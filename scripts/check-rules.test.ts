import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, test } from "node:test";

// Each test runs a copy of the script in a new git repository, so the script's
// root is that repository. The repository has one checked file, src/a.txt, and
// one check, check.mjs, that fails with "BAD found" when the file contains BAD.

const SCRIPT = join(import.meta.dirname, "check-rules.ts");
const LIB = join(import.meta.dirname, "lib");
const dirs: string[] = [];

afterEach(() => {
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

function git(root: string, ...args: string[]): string {
  const r = spawnSync("git", ["-c", "user.name=t", "-c", "user.email=t@t", ...args], { cwd: root, encoding: "utf8" });
  if (r.status !== 0) throw new Error(`git ${args.join(" ")}: ${r.stderr}`);
  return r.stdout;
}

function write(root: string, path: string, text: string) {
  mkdirSync(dirname(join(root, path)), { recursive: true });
  writeFileSync(join(root, path), text);
}

const CHECK = `import { readFileSync } from "node:fs";
if (readFileSync("src/a.txt", "utf8").includes("BAD")) { console.error("BAD found in src/a.txt"); process.exit(1); }
console.log("ok");
`;
// "three" is in the middle, so the patch hunk is not at the start or the end
// of the file: git apply anchors such a hunk there.
const A = "a\nb\none\ntwo\nthree\nfour\nfive\nc\nd\n";

function family(id: string, extra = "") {
  return `family = "${id}"
rule = "Do not write BAD."
state = "enforced"
enforced_by = "node check.mjs"
lessons = ["docs/lesson.md"]
${extra}`;
}

function negative(patch: string, run = "node check.mjs", expect = "BAD found") {
  return `\n[[negative]]\napply = "${patch}"\nrun = "${run}"\nexpect = "${expect}"\n`;
}

// A repository with src/a.txt, check.mjs, a lesson, and a patch that writes BAD into src/a.txt.
function repo(rules: Record<string, string> = {}): string {
  const root = mkdtempSync(join(tmpdir(), "check-rules-test-"));
  dirs.push(root);
  cpSync(SCRIPT, join(root, "scripts/check-rules.ts"));
  cpSync(LIB, join(root, "scripts/lib"), { recursive: true });
  write(root, "src/a.txt", A);
  write(root, "check.mjs", CHECK);
  write(root, "docs/lesson.md", "# Lesson\n");
  git(root, "init", "-q", "-b", "main");
  git(root, "add", "-A");
  git(root, "commit", "-q", "-m", "start");
  write(root, "src/a.txt", A.replace("three", "BAD"));
  write(root, "rules/negatives/bad.patch", git(root, "diff"));
  git(root, "checkout", "--", "src/a.txt");
  for (const [path, text] of Object.entries(rules)) write(root, path, text);
  git(root, "add", "-A");
  git(root, "commit", "-q", "-m", "rules");
  return root;
}

function check(root: string, ...args: string[]) {
  const r = spawnSync(process.execPath, ["scripts/check-rules.ts", ...args], { cwd: root, encoding: "utf8" });
  return { status: r.status, out: `${r.stdout}${r.stderr}` };
}

function clean(root: string): boolean {
  return git(root, "status", "--porcelain", "--untracked-files=all").trim() === "";
}

test("an empty rules folder passes", () => {
  const root = repo();
  const r = check(root);
  assert.equal(r.status, 0, r.out);
});

test("a family whose patch makes its command fail with the expected text passes", () => {
  const root = repo({ "rules/no-bad.toml": family("no-bad", negative("rules/negatives/bad.patch")) });
  const r = check(root);
  assert.equal(r.status, 0, r.out);
  assert.match(r.out, /pass no-bad/);
  assert.ok(clean(root));
});

test("a family whose command passes with the patch applied fails", () => {
  const root = repo({ "rules/no-bad.toml": family("no-bad", negative("rules/negatives/bad.patch", "true")) });
  const r = check(root);
  assert.equal(r.status, 1);
  assert.match(r.out, /fail no-bad: .*no longer catches its mistake/);
});

test("a family whose command fails without the expected text fails and prints both", () => {
  const root = repo({ "rules/no-bad.toml": family("no-bad", negative("rules/negatives/bad.patch", "node check.mjs", "WRONG text")) });
  const r = check(root);
  assert.equal(r.status, 1);
  assert.match(r.out, /expected: WRONG text/);
  assert.match(r.out, /BAD found in src\/a.txt/);
});

test("a patch that does not apply fails with a request to regenerate it", () => {
  const root = repo({ "rules/no-bad.toml": family("no-bad", negative("rules/negatives/bad.patch")) });
  write(root, "src/a.txt", "a file with other text\n");
  git(root, "commit", "-q", "-am", "rewrite");
  const r = check(root);
  assert.equal(r.status, 1);
  assert.match(r.out, /regenerate the negative example/);
  assert.ok(clean(root));
});

// CI checks out one commit with no history, so a 3-way merge that needs the
// patch's old blobs cannot run there. A full clone has those blobs. The
// result must be the same in both, or a stale patch passes only locally.
test("a patch whose context lines changed fails, also when the repository has the old blobs", () => {
  const root = repo({ "rules/no-bad.toml": family("no-bad", negative("rules/negatives/bad.patch")) });
  write(root, "src/a.txt", A.replace("one", "one and a half"));
  git(root, "commit", "-q", "-am", "change a context line");
  const r = check(root);
  assert.equal(r.status, 1, r.out);
  assert.match(r.out, /regenerate the negative example/);
  assert.ok(clean(root));
});

test("a patch whose context moved by a few lines still applies", () => {
  const root = repo({ "rules/no-bad.toml": family("no-bad", negative("rules/negatives/bad.patch")) });
  write(root, "src/a.txt", `zero\nzero2\n${A}`);
  git(root, "commit", "-q", "-am", "add lines");
  const r = check(root);
  assert.equal(r.status, 0, r.out);
});

test("ledger problems fail with the file and the field", () => {
  const root = repo({
    "rules/no-field.toml": `family = "no-field"\nstate = "enforced"\nenforced_by = "x"\n${negative("rules/negatives/bad.patch")}`,
    "rules/bad-state.toml": `family = "bad-state"\nrule = "r"\nstate = "maybe"\n`,
    "rules/other-name.toml": `family = "wrong"\nrule = "r"\nstate = "judgment"\njudgment = "j"\n`,
    "rules/no-lesson.toml": family("no-lesson", negative("rules/negatives/bad.patch")).replace("docs/lesson.md", "docs/gone.md"),
    "rules/no-patch.toml": family("no-patch", negative("rules/negatives/gone.patch")),
    "rules/no-negative.toml": family("no-negative"),
    "rules/no-judgment.toml": `family = "no-judgment"\nrule = "r"\nstate = "judgment"\n`,
  });
  const r = check(root, "--validate");
  assert.equal(r.status, 1);
  assert.match(r.out, /rules\/no-field.toml: rule: missing/);
  assert.match(r.out, /rules\/bad-state.toml: state: "maybe" is not one of/);
  assert.match(r.out, /rules\/other-name.toml: family: "wrong" must equal the file name "other-name"/);
  assert.match(r.out, /rules\/no-lesson.toml: lessons: docs\/gone.md does not exist/);
  assert.match(r.out, /rules\/no-patch.toml: negative 1: the patch rules\/negatives\/gone.patch does not exist/);
  assert.match(r.out, /rules\/no-negative.toml: negative: an enforced family has at least one negative example/);
  assert.match(r.out, /rules\/no-judgment.toml: judgment: a judgment family says why/);
});

test("--validate does not run the negative examples", () => {
  const root = repo({ "rules/no-bad.toml": family("no-bad", negative("rules/negatives/bad.patch", "true")) });
  const r = check(root, "--validate");
  assert.equal(r.status, 0, r.out);
});

test("--family runs only that family", () => {
  const root = repo({
    "rules/no-bad.toml": family("no-bad", negative("rules/negatives/bad.patch")),
    "rules/broken.toml": family("broken", negative("rules/negatives/bad.patch", "true")),
  });
  const r = check(root, "--family", "no-bad");
  assert.equal(r.status, 0, r.out);
  assert.doesNotMatch(r.out, /broken/);
});

test("with changes in the working tree, the script refuses to run and changes nothing", () => {
  const root = repo({ "rules/no-bad.toml": family("no-bad", negative("rules/negatives/bad.patch")) });
  write(root, "src/a.txt", "work in progress\n");
  write(root, "new.txt", "new\n");
  const r = check(root);
  assert.equal(r.status, 1);
  assert.match(r.out, /working tree has changes/);
  assert.equal(readFileSync(join(root, "src/a.txt"), "utf8"), "work in progress\n");
  assert.equal(readFileSync(join(root, "new.txt"), "utf8"), "new\n");
});

test("a file outside the patch that changes during the run is kept, and the run fails", () => {
  const root = repo({
    "rules/no-bad.toml": family("no-bad", negative("rules/negatives/bad.patch", "echo later > other.txt; node check.mjs")),
  });
  const r = check(root);
  assert.equal(r.status, 1);
  assert.match(r.out, /left changes outside its patch/);
  assert.equal(readFileSync(join(root, "other.txt"), "utf8"), "later\n");
  assert.equal(readFileSync(join(root, "src/a.txt"), "utf8"), A);
});

test("after a failed run, the tree is clean: edited files are restored and added files are removed", () => {
  const root = repo();
  write(root, "src/a.txt", `${A}BAD\n`);
  write(root, "src/added.txt", "added\n");
  git(root, "add", "-A");
  const patch = git(root, "diff", "--cached");
  git(root, "reset", "-q", "--hard", "HEAD");
  write(root, "rules/negatives/add.patch", patch);
  write(root, "rules/no-bad.toml", family("no-bad", negative("rules/negatives/add.patch", "true")));
  git(root, "add", "-A");
  git(root, "commit", "-q", "-m", "rules");
  const r = check(root);
  assert.equal(r.status, 1);
  assert.ok(clean(root), git(root, "status", "--porcelain"));
  assert.equal(readFileSync(join(root, "src/a.txt"), "utf8"), A);
});
