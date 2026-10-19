// Decides whether a routine PR is ready for the maintainer (KTD2, R7, R14).
//
//   bun scripts/upkeep/check-pr.ts <pr number> --kind deps|port|docs
//     Run from a checkout of the current `main`. Exits with 0 when the PR
//     passes, 1 when it does not, and 2 for a usage error. On a pass, it
//     adds the label `upkeep: ready`. On a failure, it removes that label and
//     prints each failed condition. In both cases it writes the result and
//     the checked head SHA in the PR body. It never merges.
//
// A PR passes only when all of these are true:
//   - the branch fits the kind (upkeep/deps, upkeep/port-<project>-<commit>,
//     upkeep/review-log), and the title type fits the kind, with no + or !;
//   - the required checks are successful on the head commit (a deps PR also
//     needs the advisory check), and no other check run failed or is still
//     running. Skipped and neutral check runs are accepted;
//   - no changed path is a refused path (KTD15);
//   - the change is allowed for the kind (see `depsProblems`,
//     `portProblems` and `docsProblems`).
//
// A deps PR may have a crate or package version that is less than 7 days
// old only when the current advisory output names that crate or package
// (R5, R25). The script runs cargo-deny and `bun audit` on the checkout for
// that output, and only when such a version is in the PR.

import { readFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import {
  type Client,
  type Kind,
  READY_LABEL,
  addLabel,
  changedFiles,
  checkProblems,
  checkRuns,
  fileAt,
  ghClient,
  mergeBase,
  prKind,
  pullRequest,
  removeLabel,
  requiredChecks,
  setPrBody,
} from "./github";
import {
  type HeldList,
  type HeldRule,
  type Published,
  MINIMUM_AGE,
  denyDiagnostics,
  heldRule,
  loadHeld,
  parseBunLock,
  splitSpec,
  systemTools,
} from "./select-updates";

const CRATES_IO = new Set(["registry+https://github.com/rust-lang/crates.io-index", "sparse+https://index.crates.io/"]);
/// The markers around the check result in the PR body.
export const CHECK_START = "<!-- upkeep:check -->";
export const CHECK_END = "<!-- /upkeep:check -->";

const TITLES: Record<Kind, { pattern: RegExp; text: string }> = {
  deps: { pattern: /^build\(deps\): \S/, text: "build(deps): ..." },
  port: { pattern: /^fix\(upstream\): \S/, text: "fix(upstream): ..." },
  docs: { pattern: /^docs\(upkeep\): \S/, text: "docs(upkeep): ..." },
};
const PORT_BRANCH = /^upkeep\/port-(.+)-[0-9a-f]{7,40}$/;
const REVIEW_LOG = "docs/reference/reference-source-reviews.md";
const TEST_FILES = ["**/*_test.rs", "**/tests/*.rs", "**/*.test.ts", "**/*.test.tsx"].map((p) => new Bun.Glob(p));
const SENSITIVE: Bun.Glob[] = JSON.parse(readFileSync(join(import.meta.dir, "sensitive-paths.json"), "utf8")).paths.map(
  (p: string) => new Bun.Glob(p),
);
const DEPENDENCY_LISTS = ["dependencies", "devDependencies", "optionalDependencies"];
/// A registry version range in package.json: an exact version, or one with ^ or ~.
const REGISTRY_RANGE = /^[\^~]?\d+(\.\d+){0,2}(-[0-9A-Za-z.-]+)?$/;
const SEMVER = /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$/;

/// The registries and the advisory output. Tests replace them.
export type Registry = {
  now(): Date;
  /// The versions of a crate on crates.io. Throws when the API fails.
  crateVersions(name: string): Promise<Published[]>;
  /// The versions of a package on the npm registry. Throws when it fails.
  npmVersions(name: string): Promise<Published[]>;
  /// The crates and packages that have an advisory in the current advisory
  /// output, as `crate:<name>` and `package:<name>`.
  advisoryPackages(): Promise<Set<string>>;
};

export type CheckResult = { ok: boolean; reasons: string[]; headSha: string };

type Version = { ecosystem: "crate" | "package"; name: string; version: string };
const describe = (v: Version) => `${v.ecosystem} ${v.name} ${v.version}`;

/// A path that no routine PR may change (KTD15). deny.toml is checked by kind.
export function refusedPath(path: string): boolean {
  return (
    path.startsWith("docs/upkeep/") ||
    path.startsWith("scripts/upkeep/") ||
    path.startsWith(".github/") ||
    path === "bunfig.toml" ||
    (path.startsWith("licences/") && !path.startsWith("licences/generated/"))
  );
}

export function sensitivePath(path: string): boolean {
  return SENSITIVE.some((glob) => glob.match(path));
}

function titleProblems(kind: Kind, title: string): string[] {
  if (/^[a-z]+(\([^)]*\))?[+!]:/.test(title)) return [`title has + or !: ${title}`];
  if (!TITLES[kind].pattern.test(title)) return [`title does not fit a ${kind} PR (${TITLES[kind].text}): ${title}`];
  return [];
}

function branchProblems(kind: Kind, branch: string): string[] {
  if (prKind(branch) !== kind || (kind === "port" && !PORT_BRANCH.test(branch))) {
    return [`branch ${branch} does not fit a ${kind} PR (upkeep/deps, upkeep/port-<project>-<commit> or upkeep/review-log)`];
  }
  return [];
}

type LockCrate = { name: string; version: string; source?: string; dependencies?: string[] };

function withoutKey(value: Record<string, unknown>, ...keys: string[]): Record<string, unknown> {
  const copy = { ...value };
  for (const key of keys) delete copy[key];
  return copy;
}

/// The held crates in a Cargo.lock, by `name@version`, with the reason. A
/// crate is held when a rule names it, or when only crates of rules with
/// `holdsDependencies` (and the crates that only they use) use it.
function heldCrates(crates: LockCrate[], rules: HeldRule[]): Map<string, string> {
  const id = (c: LockCrate) => `${c.name}@${c.version}`;
  const byName = new Map<string, string[]>();
  for (const c of crates) byName.set(c.name, [...(byName.get(c.name) ?? []), id(c)]);
  const users = new Map<string, string[]>();
  for (const c of crates) {
    for (const dep of c.dependencies ?? []) {
      const [name, version] = dep.split(" ");
      const target = version ? `${name}@${version}` : byName.get(name)?.[0];
      if (target) users.set(target, [...(users.get(target) ?? []), id(c)]);
    }
  }
  const held = new Map<string, string>();
  const holders = new Map<string, string>();
  for (const c of crates) {
    const rule = heldRule(rules, c.name);
    if (!rule) continue;
    held.set(id(c), rule.reason);
    if (rule.holdsDependencies) holders.set(id(c), rule.reason);
  }
  for (let changed = true; changed; ) {
    changed = false;
    for (const c of crates) {
      const key = id(c);
      if (held.has(key)) continue;
      const list = users.get(key) ?? [];
      if (list.length > 0 && list.every((user) => holders.has(user))) {
        const reason = holders.get(list[0])!;
        held.set(key, reason);
        holders.set(key, reason);
        changed = true;
      }
    }
  }
  return held;
}

function cargoLockProblems(base: string, head: string, held: HeldList, check: Version[]): string[] {
  const [a, b] = [Bun.TOML.parse(base) as any, Bun.TOML.parse(head) as any];
  const problems: string[] = [];
  if (!Bun.deepEquals(withoutKey(a, "package"), withoutKey(b, "package"))) problems.push("Cargo.lock: change outside the package list");
  const baseKeys = new Set((a.package as LockCrate[]).map((c) => `${c.name}@${c.version}@${c.source ?? ""}`));
  const heldIds = heldCrates(b.package, held.crates);
  for (const c of b.package as LockCrate[]) {
    if (baseKeys.has(`${c.name}@${c.version}@${c.source ?? ""}`)) continue;
    const version: Version = { ecosystem: "crate", name: c.name, version: c.version };
    const reason = heldIds.get(`${c.name}@${c.version}`);
    if (!c.source || !CRATES_IO.has(c.source)) problems.push(`not from the registry: ${describe(version)}`);
    else if (reason) problems.push(`held: ${describe(version)} (${reason})`);
    else check.push(version);
  }
  return problems;
}

function bunLockProblems(base: string, head: string, held: HeldList, check: Version[]): string[] {
  const [a, b] = [parseBunLock(base), parseBunLock(head)];
  const problems: string[] = [];
  if (!Bun.deepEquals(withoutKey(a, "packages", "workspaces"), withoutKey(b, "packages", "workspaces"))) {
    problems.push("bun.lock: change outside the package list");
  }
  const baseEntries = new Set(Object.values(a.packages ?? {}).map((entry) => JSON.stringify(entry)));
  const seen = new Set<string>();
  for (const entry of Object.values(b.packages ?? {}) as unknown[][]) {
    if (baseEntries.has(JSON.stringify(entry))) continue;
    const spec = String(entry[0]);
    const version: Version = { ecosystem: "package", ...splitSpec(spec) };
    if (seen.has(spec)) continue;
    seen.add(spec);
    const reason = heldRule(held.packages, version.name)?.reason;
    if (spec.lastIndexOf("@") <= 0 || !SEMVER.test(version.version) || entry[1] !== "") problems.push(`not from the registry: package ${spec}`);
    else if (reason) problems.push(`held: ${describe(version)} (${reason})`);
    else check.push(version);
  }
  return problems;
}

function packageJsonProblems(base: string, head: string, held: HeldList): string[] {
  const [a, b] = [JSON.parse(base), JSON.parse(head)];
  const problems: string[] = [];
  if (!Bun.deepEquals(withoutKey(a, ...DEPENDENCY_LISTS), withoutKey(b, ...DEPENDENCY_LISTS))) {
    problems.push("package.json: change outside the dependency lists");
  }
  for (const list of DEPENDENCY_LISTS) {
    const [from, to] = [a[list] ?? {}, b[list] ?? {}] as Record<string, string>[];
    if (!Bun.deepEquals(Object.keys(from).sort(), Object.keys(to).sort())) {
      problems.push(`package.json: the names in ${list} changed`);
      continue;
    }
    for (const name of Object.keys(to)) {
      if (from[name] === to[name]) continue;
      const reason = heldRule(held.packages, name)?.reason;
      if (!REGISTRY_RANGE.test(from[name]) || !REGISTRY_RANGE.test(to[name])) {
        problems.push(`package.json: not a registry version range: ${name} ${to[name]}`);
      } else if (/^\d/.test(from[name])) problems.push(`held: package ${name} is an exact pin`);
      else if (reason) problems.push(`held: package ${name} (${reason})`);
    }
  }
  return problems;
}

/// A deps PR may only remove advisory ignores from deny.toml (R3).
function denyProblems(base: string, head: string): string[] {
  const [a, b] = [Bun.TOML.parse(base) as any, Bun.TOML.parse(head) as any];
  const ignores = (t: any): unknown[] => t.advisories?.ignore ?? [];
  const rest = (t: any) => ({ ...t, advisories: withoutKey(t.advisories ?? {}, "ignore") });
  if (!Bun.deepEquals(rest(a), rest(b))) return ["deny.toml: change other than a removed advisory ignore"];
  if (!ignores(b).every((entry) => ignores(a).some((old) => Bun.deepEquals(old, entry)))) return ["deny.toml: adds an advisory ignore"];
  return [];
}

async function ageProblems(versions: Version[], registry: Registry): Promise<string[]> {
  const problems: string[] = [];
  const now = registry.now().getTime();
  let advisories: Set<string> | null = null;
  for (const v of versions) {
    let list: Published[];
    try {
      list = await (v.ecosystem === "crate" ? registry.crateVersions(v.name) : registry.npmVersions(v.name));
    } catch (error) {
      problems.push(`publish date unknown: ${describe(v)} (${(error as Error).message})`);
      continue;
    }
    const found = list.find((p) => p.version === v.version);
    if (!found) problems.push(`publish date unknown: ${describe(v)} (not on the registry)`);
    else if (found.yanked) problems.push(`yanked: ${describe(v)}`);
    else if (now - found.published.getTime() < MINIMUM_AGE) {
      advisories ??= await registry.advisoryPackages();
      if (!advisories.has(`${v.ecosystem}:${v.name}`)) problems.push(`less than 7 days old with no confirmed advisory: ${describe(v)}`);
    }
  }
  return problems;
}

/// A deps PR may change only Cargo.lock, bun.lock, licences/generated/,
/// the version ranges of existing registry entries in package.json, and
/// remove ignores from deny.toml.
async function depsProblems(client: Client, files: string[], base: string, head: string, registry: Registry, held: HeldList) {
  const problems: string[] = [];
  const check: Version[] = [];
  const checkers: Record<string, (a: string, b: string) => string[]> = {
    "Cargo.lock": (a, b) => cargoLockProblems(a, b, held, check),
    "bun.lock": (a, b) => bunLockProblems(a, b, held, check),
    "package.json": (a, b) => packageJsonProblems(a, b, held),
    "deny.toml": denyProblems,
  };
  for (const path of files) {
    if (refusedPath(path) || path.startsWith("licences/generated/")) continue;
    const checker = checkers[path];
    if (!checker) {
      problems.push(`file not allowed in a deps PR: ${path}`);
      continue;
    }
    const [a, b] = [fileAt(client, path, base), fileAt(client, path, head)];
    if (a === null || b === null) {
      problems.push(`file added or removed: ${path}`);
      continue;
    }
    try {
      problems.push(...checker(a, b));
    } catch (error) {
      problems.push(`${path}: cannot read (${(error as Error).message})`);
    }
  }
  problems.push(...(await ageProblems(check, registry)));
  return problems;
}

type ProvenanceRecord = { path: string; repository: string; start: number; end: number };

function provenanceRecords(text: string): ProvenanceRecord[] {
  const lines = text.split("\n");
  const records: ProvenanceRecord[] = [];
  lines.forEach((line, i) => {
    const heading = line.match(/^## `(.*)`$/);
    if (heading) {
      if (records.length > 0) records[records.length - 1].end = i;
      records.push({ path: heading[1], repository: "", start: i, end: lines.length });
    } else if (records.length > 0 && line.startsWith("- Repository: ")) {
      records[records.length - 1].repository = line.slice("- Repository: ".length).trim();
    }
  });
  return records;
}

const projectOf = (repository: string) => repository.replace(/\/+$/, "").split("/").pop()!.toLowerCase();

/// The changed lines of a PROVENANCE.md may only be `- Revision:` lines of
/// records of the named project, for files that the PR changes (R15).
function provenanceProblems(path: string, base: string | null, head: string | null, project: string, changed: Set<string>): string[] {
  const problem = [`PROVENANCE change other than the Revision line of a changed ${project} file: ${path}`];
  if (base === null || head === null) return problem;
  const [a, b] = [base.split("\n"), head.split("\n")];
  if (a.length !== b.length) return problem;
  const records = provenanceRecords(base);
  const dir = dirname(path);
  for (let i = 0; i < a.length; i++) {
    if (a[i] === b[i]) continue;
    const record = records.find((r) => i > r.start && i < r.end);
    const allowed =
      a[i].startsWith("- Revision: ") &&
      b[i].startsWith("- Revision: ") &&
      record !== undefined &&
      projectOf(record.repository) === project &&
      changed.has(dir === "." ? record.path : `${dir}/${record.path}`);
    if (!allowed) return problem;
  }
  return [];
}

/// A port PR may change only files that a PROVENANCE record on the base
/// commit lists as borrowed from the named project, the Revision lines of
/// those records, and test files. build.rs and the sensitive paths are
/// refused (R14, R15).
function portProblems(client: Client, branch: string, files: string[], base: string, head: string): string[] {
  const project = branch.match(PORT_BRANCH)?.[1]?.toLowerCase();
  if (!project) return [];
  const problems: string[] = [];
  const changed = new Set(files);
  const records = new Map<string, ProvenanceRecord[]>();
  const recordsIn = (dir: string) => {
    if (!records.has(dir)) {
      const text = fileAt(client, dir === "." ? "PROVENANCE.md" : `${dir}/PROVENANCE.md`, base);
      records.set(dir, text === null ? [] : provenanceRecords(text));
    }
    return records.get(dir)!;
  };
  const borrowed = (path: string) => {
    for (let dir = dirname(path); ; dir = dirname(dir)) {
      const relative = dir === "." ? path : path.slice(dir.length + 1);
      if (recordsIn(dir).some((r) => r.path === relative && projectOf(r.repository) === project)) return true;
      if (dir === ".") return false;
    }
  };
  for (const path of files) {
    if (refusedPath(path)) continue;
    if (basename(path) === "build.rs") problems.push(`build.rs is refused in a port PR: ${path}`);
    else if (sensitivePath(path)) problems.push(`sensitive path: ${path}`);
    else if (basename(path) === "PROVENANCE.md") {
      problems.push(...provenanceProblems(path, fileAt(client, path, base), fileAt(client, path, head), project, changed));
    } else if (!TEST_FILES.some((glob) => glob.match(path)) && !borrowed(path)) {
      problems.push(`not borrowed from ${project}: ${path}`);
    }
  }
  return problems;
}

/// All failed conditions of a PR. An empty list means that the PR passes.
export async function evaluate(client: Client, pr: any, kind: Kind, registry: Registry, held: HeldList): Promise<string[]> {
  const branch: string = pr.head.ref;
  const head: string = pr.head.sha;
  const problems = [...branchProblems(kind, branch), ...titleProblems(kind, pr.title)];
  problems.push(...checkProblems(checkRuns(client, head), requiredChecks(kind)));
  const files = [...new Set(changedFiles(client, pr.number).map((f) => f.path))];
  for (const path of files) if (refusedPath(path)) problems.push(`refused path: ${path}`);
  const base = mergeBase(client, pr.base.sha, head);
  if (kind === "deps") problems.push(...(await depsProblems(client, files, base, head, registry, held)));
  if (kind === "port") problems.push(...portProblems(client, branch, files, base, head));
  if (kind === "docs") {
    for (const path of files) if (path !== REVIEW_LOG && !refusedPath(path)) problems.push(`file not allowed in a docs PR: ${path}`);
  }
  return problems;
}

/// Returns the body with the check result between the markers. A result
/// that the body has is replaced; else the result is added at the end.
export function withCheckRecord(body: string, text: string): string {
  const block = `${CHECK_START}\n${text}\n${CHECK_END}`;
  const pattern = new RegExp(`${CHECK_START}[\\s\\S]*?${CHECK_END}`);
  if (pattern.test(body)) return body.replace(pattern, block);
  return `${body.replace(/\s*$/, "")}\n\n${block}\n`;
}

/// Checks the PR, sets the ready label and writes the result in the PR body.
export async function runCheckPr(client: Client, number: number, kind: Kind, registry: Registry, held: HeldList): Promise<CheckResult> {
  const pr = pullRequest(client, number);
  const headSha: string = pr.head.sha;
  const reasons = await evaluate(client, pr, kind, registry, held);
  if (reasons.length === 0 && pullRequest(client, number).head.sha !== headSha) {
    reasons.push(`head changed during the check (checked ${headSha})`);
  }
  const ok = reasons.length === 0;
  const hasLabel = pr.labels.some((label: { name: string }) => label.name === READY_LABEL);
  if (ok) addLabel(client, number, READY_LABEL);
  else if (hasLabel) removeLabel(client, number, READY_LABEL);
  const date = registry.now().toISOString().slice(0, 10);
  const text = ok
    ? `Upkeep check (${kind}) passed at head ${headSha} on ${date}.`
    : `Upkeep check (${kind}) refused at head ${headSha} on ${date}:\n${reasons.map((r) => `- ${r}`).join("\n")}`;
  setPrBody(client, number, withCheckRecord(pr.body ?? "", text));
  return { ok, reasons, headSha };
}

/// The real registries, and the advisory output of the checkout in `repository`.
export function systemRegistry(repository: string): Registry {
  const tools = systemTools(repository);
  return {
    now: () => new Date(),
    crateVersions: tools.crateVersions,
    npmVersions: tools.npmVersions,
    advisoryPackages: async () => {
      const names = new Set<string>();
      for (const item of denyDiagnostics(tools.cargoDenyAdvisories())) {
        const name = item.fields?.advisory?.package;
        if (name) names.add(`crate:${name}`);
      }
      for (const name of Object.keys(tools.bun.audit())) names.add(`package:${name}`);
      return names;
    },
  };
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const number = Number(args[0]);
  const kind = args[1] === "--kind" ? (args[2] as Kind) : null;
  if (args.length !== 3 || !Number.isInteger(number) || !kind || !["deps", "port", "docs"].includes(kind)) {
    console.error("usage: check-pr.ts <pr number> --kind deps|port|docs");
    process.exit(2);
  }
  const result = await runCheckPr(ghClient(), number, kind, systemRegistry(process.cwd()), loadHeld());
  if (result.ok) {
    console.log(`passed: PR #${number} at ${result.headSha}`);
  } else {
    console.log(`refused: PR #${number} at ${result.headSha}`);
    for (const reason of result.reasons) console.log(`- ${reason}`);
    process.exit(1);
  }
}
