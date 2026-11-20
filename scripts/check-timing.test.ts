import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, test } from "node:test";

// Each test runs a copy of the script in a new folder, so the script's root is
// that folder. The folder holds a copy of each file that the exception list
// names, so the list matches. Each test then adds or changes one file.

const SCRIPT = join(import.meta.dirname, "check-timing.ts");
const REPO = join(import.meta.dirname, "..");
const HIT = "short time limit in a test";
const dirs: string[] = [];

afterEach(() => {
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

function check(root: string) {
  const r = spawnSync(process.execPath, ["scripts/check-timing.ts"], { cwd: root, encoding: "utf8" });
  return { status: r.status, out: `${r.stdout}${r.stderr}` };
}

function write(root: string, path: string, text: string) {
  mkdirSync(dirname(join(root, path)), { recursive: true });
  writeFileSync(join(root, path), text);
}

// A folder with the script and a copy of each file in its exception list. On
// an empty folder the script names each listed file with "remove the
// exception", so the test finds the files without a second copy of the list.
function tree(): string {
  const root = mkdtempSync(join(tmpdir(), "check-timing-test-"));
  dirs.push(root);
  cpSync(SCRIPT, join(root, "scripts/check-timing.ts"));
  const listed = [...check(root).out.matchAll(/^(\S+): .*remove the exception\.$/gm)].map((m) => m[1]);
  assert.ok(listed.length > 0);
  for (const file of new Set(listed)) write(root, file, readFileSync(join(REPO, file), "utf8"));
  return root;
}

function rustTest(body: string) {
  return `use std::time::{Duration, Instant};\n\n#[test]\nfn waits() {\n${body}\n}\n`;
}

test("the listed hits pass", () => {
  const r = check(tree());
  assert.equal(r.status, 0, r.out);
});

test("an elapsed() upper bound under 60 s fails, and one of 60 s passes", () => {
  const root = tree();
  write(root, "crates/demo/tests/t.rs", rustTest("    let start = Instant::now();\n    assert!(start.elapsed() < Duration::from_secs(2));"));
  const r = check(root);
  assert.equal(r.status, 1);
  assert.match(r.out, new RegExp(`crates/demo/tests/t.rs:6: ${HIT} \\(an elapsed\\(\\) upper bound`));
  assert.match(r.out, /tests-with-speed-limits-failed-on-a-loaded-runner\.md/);

  write(root, "crates/demo/tests/t.rs", rustTest("    let start = Instant::now();\n    assert!(start.elapsed() < Duration::from_secs(60));"));
  assert.equal(check(root).status, 0);
});

test("a recv_timeout under 60 s fails alone, and passes inside a loop with an outer limit", () => {
  const root = tree();
  write(root, "crates/demo/tests/t.rs", rustTest("    let (_tx, rx) = std::sync::mpsc::channel::<()>();\n    rx.recv_timeout(Duration::from_secs(1)).unwrap();"));
  const r = check(root);
  assert.equal(r.status, 1);
  assert.match(r.out, new RegExp(`crates/demo/tests/t.rs:6: ${HIT} \\(a recv_timeout`));

  write(
    root,
    "crates/demo/tests/t.rs",
    rustTest(`    let (_tx, rx) = std::sync::mpsc::channel::<()>();
    let limit = Instant::now() + Duration::from_secs(120);
    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(()) => break,
            Err(_) => assert!(Instant::now() < limit, "no event"),
        }
    }`),
  );
  const ok = check(root);
  assert.equal(ok.status, 0, ok.out);
});

test("a recv_timeout under 60 s in production code is not reported, and in a #[cfg(test)] module it is", () => {
  const root = tree();
  const production = `use std::time::Duration;

pub fn wait(rx: std::sync::mpsc::Receiver<()>) -> bool {
    rx.recv_timeout(Duration::from_secs(1)).is_ok()
}
`;
  write(root, "crates/demo/src/lib.rs", production);
  assert.equal(check(root).status, 0);

  write(
    root,
    "crates/demo/src/lib.rs",
    `${production}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits() {
        let (_tx, rx) = std::sync::mpsc::channel::<()>();
        rx.recv_timeout(Duration::from_millis(500)).unwrap();
    }
}
`,
  );
  const r = check(root);
  assert.equal(r.status, 1);
  assert.match(r.out, new RegExp(`crates/demo/src/lib.rs:14: ${HIT}`));
  assert.doesNotMatch(r.out, /crates\/demo\/src\/lib.rs:4:/);
});

test("a HANG_LIMIT other than 60 s fails", () => {
  const root = tree();
  write(root, "crates/demo/tests/t.rs", `use std::time::Duration;\n\nconst HANG_LIMIT: Duration = Duration::from_secs(10);\n`);
  const r = check(root);
  assert.equal(r.status, 1);
  assert.match(r.out, new RegExp(`crates/demo/tests/t.rs:3: ${HIT} \\(a HANG_LIMIT other than 60 s\\)`));
});

test("a waitFor timeout under 60000 ms fails, and a waitFor without a timeout passes", () => {
  const root = tree();
  write(root, "src/demo/x.test.ts", `await waitFor(() => {\n  expect(done).toBe(true);\n}, { timeout: 500 });\n`);
  const r = check(root);
  assert.equal(r.status, 1);
  assert.match(r.out, new RegExp(`src/demo/x.test.ts:3: ${HIT} \\(a waitFor timeout`));

  write(root, "src/demo/x.test.ts", `await waitFor(() => {\n  expect(done).toBe(true);\n});\n`);
  assert.equal(check(root).status, 0);
});

test("a new setTimeout sleep fails, and a listed sleep that is gone fails with remove the exception", () => {
  const root = tree();
  write(root, "src/demo/x.test.ts", `await new Promise((r) => setTimeout(r, 200));\n`);
  const r = check(root);
  assert.equal(r.status, 1);
  assert.match(r.out, new RegExp(`src/demo/x.test.ts:1: ${HIT} \\(a real setTimeout sleep\\)`));

  rmSync(join(root, "src/demo/x.test.ts"));
  const page = join(root, "src/session/SessionPage.test.tsx");
  writeFileSync(page, readFileSync(page, "utf8").replace(/setTimeout\(r, 1300\)/, "Promise.resolve()"));
  const gone = check(root);
  assert.equal(gone.status, 1);
  assert.match(gone.out, /src\/session\/SessionPage.test.tsx: the scan found no hit of this form \(a real setTimeout sleep\): remove the exception\./);
});
