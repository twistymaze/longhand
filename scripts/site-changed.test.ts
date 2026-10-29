import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

const SCRIPT = join(import.meta.dirname, "site-changed.sh");

/// A repository with the script and one commit. Returns the folder and a
/// function that commits a file and returns the commit before it.
function repository() {
  const dir = mkdtempSync(join(tmpdir(), "site-changed-"));
  const git = (...args: string[]) =>
    execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false", ...args], {
      cwd: dir,
      encoding: "utf8",
    }).trim();
  git("init", "--quiet");
  mkdirSync(join(dir, "scripts"));
  copyFileSync(SCRIPT, join(dir, "scripts", "site-changed.sh"));
  git("add", ".");
  git("commit", "--quiet", "-m", "start");
  const change = (path: string) => {
    const before = git("rev-parse", "HEAD");
    mkdirSync(join(dir, path, ".."), { recursive: true });
    writeFileSync(join(dir, path), `${Math.random()}\n`);
    git("add", ".");
    git("commit", "--quiet", "-m", `change ${path}`);
    return before;
  };
  const run = (base: string) => execFileSync("bash", [join(dir, "scripts", "site-changed.sh"), base], { cwd: dir, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
  return { change, run };
}

// Covers AE4: a push that changes only the app does not deploy the site.
test("a change only under src/ prints changed=false", () => {
  const repo = repository();
  assert.equal(repo.run(repo.change("src/App.tsx")), "changed=false");
});

test("a change to a file that builds the site prints changed=true", () => {
  for (const path of ["site/index.html", "scripts/site.ts", "scripts/lib/changelog.ts"]) {
    const repo = repository();
    assert.equal(repo.run(repo.change(path)), "changed=true", path);
  }
});

test("no base commit prints changed=true", () => {
  const repo = repository();
  assert.equal(repo.run(""), "changed=true");
});
