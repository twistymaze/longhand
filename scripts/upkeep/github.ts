// GitHub REST calls for the weekly upkeep routine (KTD3, R26).
//
//   bun scripts/upkeep/github.ts open-prs
//     Prints JSON: the open PRs on `upkeep/` branches, each with its kind,
//     check state (green, failed or pending), the `upkeep: ready` label and
//     the date of the run that opened it (`firstRun`).
//
//   bun scripts/upkeep/github.ts update-pr <branch> <commit> --body-file <file>
//     Rebuilds the open PR of an `upkeep/` branch in place: it removes the
//     `upkeep: ready` label, force-pushes <commit> to that branch only, and
//     replaces the PR body with the file. <commit> must have the current
//     `main` in its history. Run it from a clone whose `origin` is the
//     repository.
//
// The cloud proxy of the routine blocks GraphQL, so all calls use REST
// through `gh api`. `gh pr` and `gh issue` use GraphQL, so this file does
// not use them. The repository is UPKEEP_REPO, or twistymaze/longhand.

import { readFileSync } from "node:fs";

export const DEFAULT_REPO = "twistymaze/longhand";
export const READY_LABEL = "upkeep: ready";
export const BRANCH_PREFIX = "upkeep/";

/// The required checks of the main ruleset. A routine PR is ready only when
/// each one is successful on the head commit.
export const REQUIRED_CHECKS = ["Checks (macos-latest)", "Checks (windows-latest)", "cargo deny", "PR title"];
/// The check run of the advisory workflow (U3). A deps PR also needs it.
/// This is the only place that names it.
export const ADVISORY_CHECK = "New advisories on this PR";

export type Kind = "deps" | "port" | "docs";

/// One REST request. `raw` returns the response text, not parsed JSON.
export type Request = { method?: string; path: string; body?: unknown; raw?: boolean };
/// The REST client. `call` throws an error with `status` for an HTTP error.
export type Client = { repo: string; call(request: Request): unknown };

export type CheckRun = { name: string; status: string; conclusion: string | null };
export type CheckState = "green" | "failed" | "pending";

/// Conclusions that do not fail a PR. A skipped job, such as a release job
/// on a PR, is accepted.
const ACCEPTED = new Set(["success", "skipped", "neutral"]);

export function defaultRepo(): string {
  return process.env.UPKEEP_REPO || DEFAULT_REPO;
}

/// A client that runs `gh api` for each request.
export function ghClient(repo = defaultRepo()): Client {
  return {
    repo,
    call(request) {
      const accept = request.raw ? "application/vnd.github.raw+json" : "application/vnd.github+json";
      const args = ["gh", "api", "--method", request.method ?? "GET", "-H", `Accept: ${accept}`];
      if (request.body !== undefined) args.push("--input", "-");
      args.push(request.path);
      const result = Bun.spawnSync(args, {
        env: process.env,
        stdin: request.body !== undefined ? Buffer.from(JSON.stringify(request.body)) : "ignore",
        stdout: "pipe",
        stderr: "pipe",
      });
      const stdout = result.stdout.toString();
      if (result.exitCode !== 0) {
        const stderr = result.stderr.toString();
        const status = Number(stderr.match(/HTTP (\d{3})/)?.[1] ?? 0) || null;
        throw Object.assign(new Error(`gh api ${request.path} failed: ${stderr.trim()}`), { status });
      }
      if (request.raw) return stdout;
      return stdout.trim() === "" ? null : JSON.parse(stdout);
    },
  };
}

/// The HTTP status of an error from `Client.call`, or null.
export function errorStatus(error: unknown): number | null {
  return (error as { status?: number | null })?.status ?? null;
}

/// All items of a list endpoint. `pick` takes the list from a page.
export function allPages<T>(client: Client, path: string, pick: (page: any) => T[] = (page) => page): T[] {
  const items: T[] = [];
  const join = path.includes("?") ? "&" : "?";
  for (let page = 1; ; page++) {
    const list = pick(client.call({ path: `${path}${join}per_page=100&page=${page}` }));
    items.push(...list);
    if (list.length < 100) return items;
  }
}

/// The latest check run of each name on a commit.
export function checkRuns(client: Client, sha: string): CheckRun[] {
  return allPages(client, `repos/${client.repo}/commits/${sha}/check-runs`, (page) => page.check_runs).map((run: any) => ({
    name: run.name,
    status: run.status,
    conclusion: run.conclusion ?? null,
  }));
}

/// The required checks for a kind of PR.
export function requiredChecks(kind: Kind | null): string[] {
  return kind === "deps" ? [...REQUIRED_CHECKS, ADVISORY_CHECK] : REQUIRED_CHECKS;
}

/// The problems with the check runs of a head commit: a required check that
/// is missing or not successful, a failed check, or a check that is not
/// complete. An empty list means that the checks pass.
export function checkProblems(runs: CheckRun[], required: string[]): string[] {
  const problems: string[] = [];
  for (const name of required) {
    const run = runs.find((r) => r.name === name);
    if (!run) problems.push(`required check missing: ${name}`);
    else if (run.status === "completed" && run.conclusion !== "success") {
      problems.push(`required check not successful: ${name} (${run.conclusion})`);
    }
  }
  for (const run of runs) {
    if (run.status !== "completed") problems.push(`checks not complete: ${run.name} (${run.status})`);
    else if (!ACCEPTED.has(run.conclusion ?? "") && !required.includes(run.name)) {
      problems.push(`check failed: ${run.name} (${run.conclusion})`);
    }
  }
  return problems;
}

/// One word for the check runs of a head commit.
export function checkState(runs: CheckRun[], required: string[]): CheckState {
  const failed = runs.some((r) => r.status === "completed" && !ACCEPTED.has(r.conclusion ?? ""));
  const requiredFailed = required.some((name) => {
    const run = runs.find((r) => r.name === name);
    return run?.status === "completed" && run.conclusion !== "success";
  });
  if (failed || requiredFailed) return "failed";
  if (runs.some((r) => r.status !== "completed") || required.some((name) => !runs.some((r) => r.name === name))) return "pending";
  return "green";
}

/// The kind of a routine PR from its branch, or null for another branch.
/// The branch names are fixed: `upkeep/deps`, `upkeep/review-log` and
/// `upkeep/port-<project>-<short commit>`.
export function prKind(branch: string): Kind | null {
  if (branch === "upkeep/deps") return "deps";
  if (branch === "upkeep/review-log") return "docs";
  if (branch.startsWith("upkeep/port-")) return "port";
  return null;
}

export function pullRequest(client: Client, number: number): any {
  return client.call({ path: `repos/${client.repo}/pulls/${number}` });
}

/// The paths that a PR changes. A renamed file gives its old and its new path.
export function changedFiles(client: Client, number: number): { path: string; status: string }[] {
  const files = allPages<any>(client, `repos/${client.repo}/pulls/${number}/files`);
  const result: { path: string; status: string }[] = [];
  for (const file of files) {
    result.push({ path: file.filename, status: file.status });
    if (file.previous_filename) result.push({ path: file.previous_filename, status: "removed" });
  }
  return result;
}

/// The commit from which the head of a PR branches off its base.
export function mergeBase(client: Client, base: string, head: string): string {
  return (client.call({ path: `repos/${client.repo}/compare/${base}...${head}` }) as any).merge_base_commit.sha;
}

/// The text of a file at a commit, or null when the file does not exist there.
export function fileAt(client: Client, path: string, ref: string): string | null {
  try {
    return client.call({ path: `repos/${client.repo}/contents/${path.split("/").map(encodeURIComponent).join("/")}?ref=${ref}`, raw: true }) as string;
  } catch (error) {
    if (errorStatus(error) === 404) return null;
    throw error;
  }
}

export function addLabel(client: Client, number: number, label: string): void {
  client.call({ method: "POST", path: `repos/${client.repo}/issues/${number}/labels`, body: { labels: [label] } });
}

/// Removes a label. A label that the issue or PR does not have is not an error.
export function removeLabel(client: Client, number: number, label: string): void {
  try {
    client.call({ method: "DELETE", path: `repos/${client.repo}/issues/${number}/labels/${encodeURIComponent(label)}` });
  } catch (error) {
    if (errorStatus(error) !== 404) throw error;
  }
}

export function setPrBody(client: Client, number: number, body: string): void {
  client.call({ method: "PATCH", path: `repos/${client.repo}/pulls/${number}`, body: { body } });
}

export type OpenPr = {
  number: number;
  branch: string;
  kind: Kind | null;
  title: string;
  url: string;
  headSha: string;
  checks: CheckState;
  ready: boolean;
  /// The date that the PR was opened. An update in place keeps the PR, so
  /// this is the date of the first run that opened it.
  firstRun: string;
};

/// The open PRs on `upkeep/` branches of the repository itself.
export function openPrs(client: Client): OpenPr[] {
  const pulls = allPages<any>(client, `repos/${client.repo}/pulls?state=open`);
  return pulls
    .filter((pr) => pr.head.ref.startsWith(BRANCH_PREFIX) && pr.head.repo?.full_name === client.repo)
    .map((pr) => {
      const kind = prKind(pr.head.ref);
      return {
        number: pr.number,
        branch: pr.head.ref,
        kind,
        title: pr.title,
        url: pr.html_url,
        headSha: pr.head.sha,
        checks: checkState(checkRuns(client, pr.head.sha), requiredChecks(kind)),
        ready: pr.labels.some((label: { name: string }) => label.name === READY_LABEL),
        firstRun: pr.created_at.slice(0, 10),
      };
    });
}

function git(cwd: string, args: string[]): { ok: boolean; output: string } {
  const result = Bun.spawnSync(["git", ...args], { cwd, stdout: "pipe", stderr: "pipe" });
  return { ok: result.exitCode === 0, output: (result.stdout.toString() + result.stderr.toString()).trim() };
}

/// Rebuilds the open PR of `branch` in place with `commit` (R26), and
/// returns the PR number. `cwd` is a clone whose `origin` is the repository
/// and that has `commit`. The push replaces only that branch, and only when
/// the branch is still at the head that the PR showed.
export function updatePr(client: Client, branch: string, commit: string, body: string, cwd: string): number {
  if (!branch.startsWith(BRANCH_PREFIX)) throw new Error(`refused: ${branch} does not start with ${BRANCH_PREFIX}`);
  const owner = client.repo.split("/")[0];
  const pulls = client.call({ path: `repos/${client.repo}/pulls?state=open&head=${owner}:${branch}&per_page=100` }) as any[];
  if (pulls.length !== 1) throw new Error(`refused: no open PR for ${branch} (found ${pulls.length})`);
  const pr = pulls[0];
  const base = pr.base.ref;
  const baseSha = (client.call({ path: `repos/${client.repo}/git/ref/heads/${base}` }) as any).object.sha;
  if (!git(cwd, ["merge-base", "--is-ancestor", baseSha, commit]).ok) {
    throw new Error(`refused: ${commit} is not on the current main (${base} is at ${baseSha})`);
  }
  // Remove the ready mark first, so that it is never on a head that is not checked.
  removeLabel(client, pr.number, READY_LABEL);
  const push = git(cwd, ["push", "--quiet", `--force-with-lease=refs/heads/${branch}:${pr.head.sha}`, "origin", `${commit}:refs/heads/${branch}`]);
  if (!push.ok) throw new Error(`git push to ${branch} failed: ${push.output}`);
  setPrBody(client, pr.number, body);
  return pr.number;
}

function usage(): never {
  console.error("usage: github.ts open-prs\n       github.ts update-pr <branch> <commit> --body-file <file>");
  process.exit(2);
}

if (import.meta.main) {
  const [command, ...rest] = process.argv.slice(2);
  try {
    if (command === "open-prs" && rest.length === 0) {
      console.log(JSON.stringify(openPrs(ghClient()), null, 2));
    } else if (command === "update-pr" && rest.length === 4 && rest[2] === "--body-file") {
      const number = updatePr(ghClient(), rest[0], rest[1], readFileSync(rest[3], "utf8"), process.cwd());
      console.log(`updated PR #${number}`);
    } else {
      usage();
    }
  } catch (error) {
    console.error((error as Error).message);
    process.exit(1);
  }
}
