import { afterEach, describe, expect, test } from "bun:test";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type CheckRun, type Client, type Request, checkState, ghClient, openPrs, updatePr } from "./github";

// The tests of the REST client use a fake `gh` first on PATH, so no test
// reaches the network. The other tests give the functions a fake client.

const dirs: string[] = [];
const originalPath = process.env.PATH;

afterEach(() => {
  process.env.PATH = originalPath;
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

function tempDir(): string {
  const dir = mkdtempSync(join(tmpdir(), "upkeep-github-test-"));
  dirs.push(dir);
  return dir;
}

// A fake `gh` that records its arguments and standard input, then prints
// `stdout`, writes `stderr` and exits with `code`.
function fakeGh(stdout: string, code = 0, stderr = ""): string {
  const dir = tempDir();
  const bin = join(dir, "bin");
  mkdirSync(bin);
  writeFileSync(join(dir, "stdout"), stdout);
  writeFileSync(join(dir, "stderr"), stderr);
  writeFileSync(
    join(bin, "gh"),
    `#!/bin/sh
printf '%s\\n' "$@" > "${join(dir, "args")}"
cat > "${join(dir, "stdin")}"
cat "${join(dir, "stdout")}"
cat "${join(dir, "stderr")}" >&2
exit ${code}
`,
  );
  chmodSync(join(bin, "gh"), 0o755);
  process.env.PATH = `${bin}:${originalPath}`;
  return dir;
}

type Route = { status?: number; body?: unknown };

// A fake client: it answers each request from `routes` by "METHOD path", and records each request.
function fakeClient(routes: Record<string, Route>): Client & { calls: Request[] } {
  const calls: Request[] = [];
  return {
    repo: "twistymaze/longhand",
    calls,
    call(request: Request) {
      calls.push(request);
      const key = `${request.method ?? "GET"} ${request.path}`;
      const route = routes[key] ?? routes[`${request.method ?? "GET"} ${request.path.split("?")[0]}`];
      if (!route) throw Object.assign(new Error(`HTTP 404: ${key}`), { status: 404 });
      if (route.status && route.status >= 400) throw Object.assign(new Error(`HTTP ${route.status}`), { status: route.status });
      return route.body ?? null;
    },
  };
}

function git(cwd: string, ...args: string[]): string {
  const r = Bun.spawnSync(["git", "-c", "user.name=t", "-c", "user.email=t@t", ...args], { cwd, stderr: "pipe" });
  if (r.exitCode !== 0) throw new Error(`git ${args.join(" ")}: ${r.stderr.toString()}`);
  return r.stdout.toString().trim();
}

describe("gh client", () => {
  test("a GET runs gh api with the path and parses the JSON", () => {
    const dir = fakeGh(`{"number": 5}`);
    const client = ghClient("owner/repo");
    expect(client.call({ path: "repos/owner/repo/pulls/5" })).toEqual({ number: 5 });
    const args = readFileSync(join(dir, "args"), "utf8").trim().split("\n");
    expect(args[0]).toBe("api");
    expect(args).toContain("repos/owner/repo/pulls/5");
    expect(args).toContain("GET");
    expect(args).not.toContain("graphql");
  });

  test("a body goes to gh on standard input as JSON", () => {
    const dir = fakeGh(`{}`);
    ghClient("owner/repo").call({ method: "PATCH", path: "repos/owner/repo/pulls/5", body: { body: "text" } });
    const args = readFileSync(join(dir, "args"), "utf8").trim().split("\n");
    expect(args).toContain("PATCH");
    expect(args).toContain("--input");
    expect(JSON.parse(readFileSync(join(dir, "stdin"), "utf8"))).toEqual({ body: "text" });
  });

  test("a raw request returns the text", () => {
    fakeGh("version = 4\n");
    expect(ghClient("owner/repo").call({ path: "repos/owner/repo/contents/Cargo.lock?ref=abc", raw: true })).toBe("version = 4\n");
  });

  test("an HTTP error gives the status", () => {
    fakeGh(`{"message":"Not Found"}`, 1, "gh: Not Found (HTTP 404)\n");
    let error: { status?: number } | null = null;
    try {
      ghClient("owner/repo").call({ path: "repos/owner/repo/pulls/99" });
    } catch (e) {
      error = e as { status?: number };
    }
    expect(error?.status).toBe(404);
  });

  test("the repository comes from UPKEEP_REPO, with twistymaze/longhand as the default", () => {
    const saved = process.env.UPKEEP_REPO;
    delete process.env.UPKEEP_REPO;
    expect(ghClient().repo).toBe("twistymaze/longhand");
    process.env.UPKEEP_REPO = "someone/fork";
    expect(ghClient().repo).toBe("someone/fork");
    if (saved === undefined) delete process.env.UPKEEP_REPO;
    else process.env.UPKEEP_REPO = saved;
  });
});

const green = (name: string): CheckRun => ({ name, status: "completed", conclusion: "success" });
const REQUIRED = ["Checks (macos-latest)", "Checks (windows-latest)", "cargo deny", "PR title"];

describe("check state", () => {
  test("all required checks green, and skipped jobs: green", () => {
    const runs = [...REQUIRED.map(green), { name: "Release build", status: "completed", conclusion: "skipped" }];
    expect(checkState(runs, REQUIRED)).toBe("green");
  });

  test("one failed check: failed", () => {
    const runs = [...REQUIRED.slice(1).map(green), { name: REQUIRED[0], status: "completed", conclusion: "failure" }];
    expect(checkState(runs, REQUIRED)).toBe("failed");
  });

  test("a check in progress or a missing required check: pending", () => {
    expect(checkState([...REQUIRED.slice(1).map(green), { name: REQUIRED[0], status: "in_progress", conclusion: null }], REQUIRED)).toBe("pending");
    expect(checkState(REQUIRED.slice(1).map(green), REQUIRED)).toBe("pending");
  });
});

function pull(number: number, branch: string, sha: string, created: string, labels: string[] = []) {
  return {
    number,
    title: `title ${number}`,
    html_url: `https://github.com/twistymaze/longhand/pull/${number}`,
    created_at: created,
    labels: labels.map((name) => ({ name })),
    head: { ref: branch, sha, repo: { full_name: "twistymaze/longhand" } },
    base: { ref: "main", sha: "base" },
  };
}

describe("open-prs", () => {
  test("lists the open upkeep PRs with kind, check state, ready label and first-run date", () => {
    const client = fakeClient({
      "GET repos/twistymaze/longhand/pulls": {
        body: [
          pull(10, "upkeep/deps", "aaa", "2026-10-09T06:00:00Z", ["upkeep: ready"]),
          pull(11, "upkeep/port-handy-1234abc", "bbb", "2026-10-16T06:00:00Z"),
          pull(12, "feature/x", "ccc", "2026-10-16T06:00:00Z"),
        ],
      },
      "GET repos/twistymaze/longhand/commits/aaa/check-runs": {
        body: { total_count: 5, check_runs: [...REQUIRED.map(green), green("New advisories on this PR")] },
      },
      "GET repos/twistymaze/longhand/commits/bbb/check-runs": {
        body: { total_count: 4, check_runs: [...REQUIRED.slice(1).map(green), { name: REQUIRED[0], status: "completed", conclusion: "failure" }] },
      },
    });
    expect(openPrs(client)).toEqual([
      {
        number: 10,
        branch: "upkeep/deps",
        kind: "deps",
        title: "title 10",
        url: "https://github.com/twistymaze/longhand/pull/10",
        headSha: "aaa",
        checks: "green",
        ready: true,
        firstRun: "2026-10-09",
      },
      {
        number: 11,
        branch: "upkeep/port-handy-1234abc",
        kind: "port",
        title: "title 11",
        url: "https://github.com/twistymaze/longhand/pull/11",
        headSha: "bbb",
        checks: "failed",
        ready: false,
        firstRun: "2026-10-16",
      },
    ]);
  });

  test("a deps PR without the advisory check is pending", () => {
    const client = fakeClient({
      "GET repos/twistymaze/longhand/pulls": { body: [pull(10, "upkeep/deps", "aaa", "2026-10-09T06:00:00Z")] },
      "GET repos/twistymaze/longhand/commits/aaa/check-runs": { body: { total_count: 4, check_runs: REQUIRED.map(green) } },
    });
    expect(openPrs(client)[0].checks).toBe("pending");
  });
});

// A bare origin with `main` and `upkeep/deps`, and a clone in which main has
// a new commit and a new deps commit is made on it.
function repositories() {
  const root = tempDir();
  const origin = join(root, "origin.git");
  const work = join(root, "work");
  git(root, "init", "-q", "--bare", "-b", "main", origin);
  git(root, "clone", "-q", origin, work);
  writeFileSync(join(work, "Cargo.lock"), "one\n");
  git(work, "add", "-A");
  git(work, "commit", "-q", "-m", "start");
  git(work, "push", "-q", "origin", "HEAD:refs/heads/main");
  git(work, "checkout", "-q", "-b", "upkeep/deps");
  writeFileSync(join(work, "Cargo.lock"), "old deps\n");
  git(work, "commit", "-q", "-am", "old deps");
  git(work, "push", "-q", "origin", "upkeep/deps");
  const oldHead = git(work, "rev-parse", "HEAD");
  git(work, "checkout", "-q", "main");
  writeFileSync(join(work, "README"), "new on main\n");
  git(work, "add", "-A");
  git(work, "commit", "-q", "-m", "main moves");
  git(work, "push", "-q", "origin", "main");
  const main = git(work, "rev-parse", "HEAD");
  git(work, "checkout", "-q", "-b", "rebuilt");
  writeFileSync(join(work, "Cargo.lock"), "new deps\n");
  git(work, "commit", "-q", "-am", "new deps");
  const commit = git(work, "rev-parse", "HEAD");
  return { origin, work, oldHead, main, commit };
}

describe("update-pr", () => {
  test("rebuilds a waiting deps PR on the current main, on the same branch and PR", () => {
    const { origin, work, oldHead, main, commit } = repositories();
    const client = fakeClient({
      "GET repos/twistymaze/longhand/pulls?state=open&head=twistymaze:upkeep/deps&per_page=100": {
        body: [pull(10, "upkeep/deps", oldHead, "2026-10-09T06:00:00Z", ["upkeep: ready"])],
      },
      "GET repos/twistymaze/longhand/git/ref/heads/main": { body: { object: { sha: main } } },
      "PATCH repos/twistymaze/longhand/pulls/10": { body: {} },
      "DELETE repos/twistymaze/longhand/issues/10/labels/upkeep%3A%20ready": { body: [] },
    });
    const number = updatePr(client, "upkeep/deps", commit, "Waiting since 2026-10-09.", work);
    expect(number).toBe(10);
    expect(git(origin, "rev-parse", "refs/heads/upkeep/deps")).toBe(commit);
    const methods = client.calls.map((c) => `${c.method ?? "GET"} ${c.path}`);
    expect(methods).toContain("DELETE repos/twistymaze/longhand/issues/10/labels/upkeep%3A%20ready");
    expect(client.calls.find((c) => c.method === "PATCH")?.body).toEqual({ body: "Waiting since 2026-10-09." });
    // No new PR is opened.
    expect(methods.some((m) => m.startsWith("POST repos/twistymaze/longhand/pulls"))).toBe(false);
  });

  test("refuses a commit that is not on the current main", () => {
    const { origin, work, oldHead, main } = repositories();
    const client = fakeClient({
      "GET repos/twistymaze/longhand/pulls?state=open&head=twistymaze:upkeep/deps&per_page=100": {
        body: [pull(10, "upkeep/deps", oldHead, "2026-10-09T06:00:00Z")],
      },
      "GET repos/twistymaze/longhand/git/ref/heads/main": { body: { object: { sha: main } } },
    });
    expect(() => updatePr(client, "upkeep/deps", oldHead, "body", work)).toThrow("not on the current main");
    expect(git(origin, "rev-parse", "refs/heads/upkeep/deps")).toBe(oldHead);
  });

  test("refuses a branch that does not start with upkeep/", () => {
    const client = fakeClient({});
    expect(() => updatePr(client, "fix/x", "abc", "body", tempDir())).toThrow("upkeep/");
    expect(client.calls).toEqual([]);
  });

  test("refuses a branch with no open PR", () => {
    const client = fakeClient({
      "GET repos/twistymaze/longhand/pulls?state=open&head=twistymaze:upkeep/deps&per_page=100": { body: [] },
    });
    expect(() => updatePr(client, "upkeep/deps", "abc", "body", tempDir())).toThrow("no open PR");
  });
});
