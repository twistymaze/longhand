// Selects the dependency updates that the weekly upkeep PR may contain
// (R5, R6, R25; KTD1, KTD4, KTD5).
//
//   bun scripts/upkeep/select-updates.ts
//     Run from the repository root. Prints JSON to standard output:
//       apply       the updates, each with the command that applies it;
//       held        the dependencies that the PR does not update, each with
//                   a reason;
//       exceptions  the advisory fixes that are younger than 7 days.
//     The script changes no file in the repository. It reads Cargo.lock
//     with `cargo update --dry-run`, and it runs bun on a temporary copy of
//     package.json, bun.lock and bunfig.toml.
//
// held.json has two lists, `crates` and `packages`. Each entry has `name`
// (the exact name) or `prefix` (the start of the name), and `reason`. An
// entry with `holdsDependencies: true` also holds the crates that only such
// held crates use. The script also holds the exact pins in the Cargo.toml
// files (`=`) and in package.json (no `^` or `~`).
// Other scripts import `loadHeld` and `heldReason` to read the same list.
//
// A crate gets the newest version that is compatible with its current
// version, not newer than the version that cargo resolves, at least 7 days
// old and not yanked. The script reads the publish dates from the crates.io
// API, at most one request each second. bun applies the 7-day wait to
// JavaScript packages (bunfig.toml). For an advisory, the script takes the
// newest fixing version that is 7 days old. When no fixing version is that
// old, it takes the oldest fixing version and lists it in `exceptions`. A
// JavaScript exception gets a one-time `--minimum-release-age` that is equal
// to the age of that version, because bun has no option that excludes one
// package on the command line. The command updates only that package.
//
// The script tries each update alone. When an update moves a held
// dependency, the script holds the update.

import { cpSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

/// The 7-day wait in milliseconds. bunfig.toml has the same value in seconds.
export const MINIMUM_AGE = 7 * 24 * 60 * 60 * 1000;
const USER_AGENT = "longhand-upkeep (https://github.com/twistymaze/longhand)";
const HELD_FILE = join(import.meta.dir, "held.json");
const BUN_UPDATE = ["update", "--lockfile-only", "--ignore-scripts"];

/// `holdsDependencies` also holds the crates that only crates of such rules use.
export type HeldRule = { name?: string; prefix?: string; reason: string; holdsDependencies?: boolean };
export type HeldList = { crates: HeldRule[]; packages: HeldRule[] };

export type Published = { version: string; published: Date; yanked: boolean };

export type CargoMetadata = {
  workspace_members: string[];
  packages: { id: string; name: string; version: string; dependencies: { name: string; req: string }[] }[];
  resolve: { nodes: { id: string; deps: { pkg: string }[] }[] };
};

/// The `bun audit --json` output: the advisories of each package.
export type AuditReport = Record<string, { id: number | string; vulnerable_versions: string }[]>;

/// The bun steps. Each step works on a temporary copy of the lockfiles.
export type BunTools = {
  /// The text of bun.lock.
  lock(): string;
  /// The advisories for bun.lock.
  audit(): AuditReport;
  /// Runs `bun update --lockfile-only --ignore-scripts <args>`, and returns
  /// the new bun.lock and its advisories.
  update(args: string[]): { lock: string; audit: AuditReport };
};

/// The commands and registries that the script uses. Tests replace them.
export type Tools = {
  now(): Date;
  cargoMetadata(): CargoMetadata;
  /// Runs `cargo update --dry-run <args>`. The output has cargo's
  /// "Updating", "Adding" and "Removing" lines.
  cargoDryRun(args: string[]): { ok: boolean; output: string };
  /// The JSON lines of `cargo deny --format json check advisories`.
  cargoDenyAdvisories(): string;
  /// The versions of a crate on crates.io. Throws when the API fails.
  crateVersions(name: string): Promise<Published[]>;
  packageJson(): { dependencies?: Record<string, string>; devDependencies?: Record<string, string> };
  bun: BunTools;
  /// The versions of a package on the npm registry. Throws when it fails.
  npmVersions(name: string): Promise<Published[]>;
};

export type Ecosystem = "crate" | "package";
export type Update = { ecosystem: Ecosystem; name: string; from: string; to: string; command: string[]; advisories?: string[] };
export type Held = {
  ecosystem: Ecosystem;
  name: string;
  version: string;
  reason: string;
  via?: string;
  to?: string;
  moves?: string[];
  advisories?: string[];
};
export type Exception = { ecosystem: Ecosystem; name: string; version: string; published: string; advisories: string[] };
export type Selection = { apply: Update[]; held: Held[]; exceptions: Exception[] };

/// Reads held.json.
export function loadHeld(path = HELD_FILE): HeldList {
  return JSON.parse(readFileSync(path, "utf8"));
}

/// The reason of the first rule that matches `name`, or null.
export function heldReason(rules: HeldRule[], name: string): string | null {
  return heldRule(rules, name)?.reason ?? null;
}

/// The first rule that matches `name`, or undefined.
export function heldRule(rules: HeldRule[], name: string): HeldRule | undefined {
  return rules.find((rule) => rule.name === name || (rule.prefix !== undefined && name.startsWith(rule.prefix)));
}

function isPrerelease(version: string): boolean {
  return version.split("+")[0].includes("-");
}

/// Cargo's compatibility rule: the same major version, or for 0.x the same
/// minor version, or for 0.0.x the same patch version.
function compatible(current: string, candidate: string): boolean {
  const [a, b] = [current, candidate].map((version) => version.split(/[-+]/)[0].split(".").map(Number));
  if (a[0] !== b[0]) return false;
  if (a[0] > 0) return true;
  if (a[1] !== b[1]) return false;
  return a[1] > 0 || a[2] === b[2];
}

function newer(a: string, b: string): boolean {
  return Bun.semver.order(a, b) > 0;
}

function sortByVersion(versions: Published[]): Published[] {
  return [...versions].sort((a, b) => Bun.semver.order(a.version, b.version));
}

/// The versions that can replace `current` in its compatible range, oldest first.
function candidates(versions: Published[], current: string): Published[] {
  return sortByVersion(versions.filter((v) => !v.yanked && !isPrerelease(v.version) && newer(v.version, current) && compatible(current, v.version)));
}

function oldEnough(version: Published, now: Date): boolean {
  return now.getTime() - version.published.getTime() >= MINIMUM_AGE;
}

/// The fix choice for an advisory: the newest fixing version that is 7 days
/// old, or else the oldest fixing version as an exception.
function chooseFix(fixing: Published[], now: Date): { version: Published; exception: boolean } {
  const old = fixing.filter((v) => oldEnough(v, now));
  if (old.length > 0) return { version: old[old.length - 1], exception: false };
  return { version: fixing[0], exception: true };
}

/// Cargo's update lines: "Updating x v1 -> v2", "Adding x v1", "Removing x v1".
function cargoChanges(output: string): { name: string; from: string; to: string | null }[] {
  const changes = [];
  for (const line of output.split("\n")) {
    const match = line.match(/^\s*(Updating|Downgrading|Adding|Removing)\s+(\S+)\s+v(\S+)(?:\s+->\s+v(\S+))?/);
    if (match) changes.push({ name: match[2], from: match[3], to: match[4] ?? null });
  }
  return changes;
}

type CrateAdvisory = { name: string; version: string; ids: string[]; patched: (string | null)[] };

/// The items in cargo-deny's JSON lines. Other lines are ignored.
export function denyItems(text: string): any[] {
  return text
    .split("\n")
    .filter((line) => line.trim().startsWith("{"))
    .map((line) => JSON.parse(line));
}

/// The diagnostic items in cargo-deny's JSON lines.
export function denyDiagnostics(text: string): any[] {
  return denyItems(text).filter((item) => item.type === "diagnostic");
}

/// The advisories in cargo-deny's JSON lines, one entry for each crate version.
function crateAdvisories(text: string): CrateAdvisory[] {
  const byCrate = new Map<string, CrateAdvisory>();
  for (const item of denyDiagnostics(text)) {
    const advisory = item.fields?.advisory;
    if (!advisory) continue;
    const notes: string[] = item.fields.notes ?? [];
    // cargo-deny writes the patched ranges as "Upgrade to >=1.2.3, <1.3.0 OR >=1.3.4".
    const solution = notes.map((note) => note.match(/^Solution: Upgrade to (.+?)(?: \(try .*\))?$/)?.[1]).find(Boolean);
    const patched = solution ? solution.split(" OR ").map((range) => range.replaceAll(",", "")).join(" || ") : null;
    for (const graph of item.fields.graphs ?? []) {
      if (graph.Krate.name !== advisory.package) continue;
      const key = `${graph.Krate.name}@${graph.Krate.version}`;
      const entry: CrateAdvisory = byCrate.get(key) ?? { name: graph.Krate.name, version: graph.Krate.version, ids: [], patched: [] };
      if (!entry.ids.includes(advisory.id)) {
        entry.ids.push(advisory.id);
        entry.patched.push(patched);
      }
      byCrate.set(key, entry);
    }
  }
  return [...byCrate.values()];
}

/// The crates that only held crates with `holdsDependencies` use, by
/// `name@version`, with the reason of a held crate that uses them. Other
/// held crates (Rig, exact pins) count as users that are not held.
function onlyUsedByHeld(meta: CargoMetadata, rules: HeldRule[]): Map<string, { reason: string; via: string }> {
  const nodes = new Map(meta.resolve.nodes.map((node) => [node.id, node.deps.map((dep) => dep.pkg)]));
  const names = new Map(meta.packages.map((p) => [p.id, p]));
  const members = new Set(meta.workspace_members);
  const rootReason = (name: string) => {
    const rule = heldRule(rules, name);
    return rule?.holdsDependencies ? rule.reason : null;
  };
  const isHeld = (id: string) => rootReason(names.get(id)!.name) !== null;
  const walk = (starts: string[], enter: (id: string) => boolean): string[] => {
    const seen = new Set(starts);
    const queue = [...starts];
    while (queue.length > 0) {
      for (const dep of nodes.get(queue.shift()!) ?? []) {
        if (seen.has(dep) || !enter(dep)) continue;
        seen.add(dep);
        queue.push(dep);
      }
    }
    return [...seen];
  };
  const free = new Set(walk([...members], (id) => !isHeld(id)));
  const heldRoots = walk([...members], () => true)
    .filter((id) => !members.has(id) && isHeld(id))
    .sort((a, b) => names.get(a)!.name.localeCompare(names.get(b)!.name));
  const derived = new Map<string, { reason: string; via: string }>();
  for (const root of heldRoots) {
    const via = names.get(root)!.name;
    for (const id of walk([root], (dep) => !free.has(dep) && !members.has(dep) && !isHeld(dep))) {
      const p = names.get(id)!;
      const key = `${p.name}@${p.version}`;
      if (id !== root && !derived.has(key)) derived.set(key, { reason: rootReason(via)!, via });
    }
  }
  return derived;
}

/// The parsed bun.lock. bun.lock is JSON with trailing commas.
export function parseBunLock(text: string): any {
  return JSON.parse(text.replace(/,(\s*[}\]])/g, "$1"));
}

/// The name and version of a bun.lock package spec `name@version`.
export function splitSpec(spec: string): { name: string; version: string } {
  const at = spec.lastIndexOf("@");
  return { name: spec.slice(0, at), version: spec.slice(at + 1) };
}

/// The packages in bun.lock, by lock key, with their names and versions.
function lockPackages(text: string): Map<string, { name: string; version: string }> {
  const lock = parseBunLock(text);
  const packages = new Map<string, { name: string; version: string }>();
  for (const [key, value] of Object.entries(lock.packages ?? {})) packages.set(key, splitSpec((value as string[])[0]));
  return packages;
}

/// The lock keys whose package or version is different in `next`.
function lockChanges(base: Map<string, { name: string; version: string }>, next: Map<string, { name: string; version: string }>) {
  const changes: { key: string; name: string; from: string | null; to: string | null }[] = [];
  for (const key of new Set([...base.keys(), ...next.keys()])) {
    const [a, b] = [base.get(key), next.get(key)];
    if (a?.name === b?.name && a?.version === b?.version) continue;
    changes.push({ key, name: (b ?? a)!.name, from: a?.version ?? null, to: b?.version ?? null });
  }
  return changes;
}

async function selectCrates(tools: Tools, held: HeldList, now: Date, result: Selection): Promise<void> {
  const meta = tools.cargoMetadata();
  const members = new Set(meta.workspace_members);
  const pins = new Map<string, string>();
  for (const p of meta.packages.filter((p) => members.has(p.id))) {
    for (const dep of p.dependencies) if (dep.req.startsWith("=")) pins.set(dep.name, dep.req.slice(1).trim());
  }
  for (const [name, version] of pins) result.held.push({ ecosystem: "crate", name, version, reason: "exact pin in a Cargo.toml" });
  const heldName = (name: string) => heldReason(held.crates, name) ?? (pins.has(name) ? "exact pin in a Cargo.toml" : null);
  const derived = onlyUsedByHeld(meta, held.crates);
  const heldCrate = (name: string, version: string) => {
    const reason = heldName(name);
    return reason !== null ? { reason } : (derived.get(`${name}@${version}`) ?? null);
  };

  const planned: (Update & { exception?: Published })[] = [];
  const advisories = crateAdvisories(tools.cargoDenyAdvisories());
  for (const advisory of advisories) {
    const base = { ecosystem: "crate" as const, name: advisory.name, version: advisory.version, advisories: advisory.ids };
    const isHeld = heldCrate(advisory.name, advisory.version);
    if (isHeld) {
      result.held.push({ ...base, reason: `advisory: the fix needs a held update. ${isHeld.reason}` });
      continue;
    }
    if (advisory.patched.includes(null)) {
      result.held.push({ ...base, reason: "advisory: no fixed version is available" });
      continue;
    }
    let versions: Published[];
    try {
      versions = await tools.crateVersions(advisory.name);
    } catch {
      result.held.push({ ...base, reason: "publish date unknown" });
      continue;
    }
    const fixing = candidates(versions, advisory.version).filter((v) => advisory.patched.every((range) => Bun.semver.satisfies(v.version, range!)));
    if (fixing.length === 0) {
      result.held.push({ ...base, reason: "advisory: no fixing version within the current range" });
      continue;
    }
    const fix = chooseFix(fixing, now);
    planned.push({
      ecosystem: "crate",
      name: advisory.name,
      from: advisory.version,
      to: fix.version.version,
      command: [],
      advisories: advisory.ids,
      ...(fix.exception ? { exception: fix.version } : {}),
    });
  }

  const withAdvisory = new Set(advisories.map((a) => `${a.name}@${a.version}`));
  for (const change of cargoChanges(tools.cargoDryRun([]).output)) {
    if (change.to === null || withAdvisory.has(`${change.name}@${change.from}`)) continue;
    const isHeld = heldCrate(change.name, change.from);
    if (isHeld) {
      result.held.push({ ecosystem: "crate", name: change.name, version: change.from, ...isHeld });
      continue;
    }
    // A move to another major version is not within the current range.
    if (!compatible(change.from, change.to)) continue;
    let versions: Published[];
    try {
      versions = await tools.crateVersions(change.name);
    } catch {
      result.held.push({ ecosystem: "crate", name: change.name, version: change.from, reason: "publish date unknown" });
      continue;
    }
    const usable = candidates(versions, change.from).filter((v) => !newer(v.version, change.to!) && oldEnough(v, now));
    if (usable.length === 0) continue;
    planned.push({ ecosystem: "crate", name: change.name, from: change.from, to: usable[usable.length - 1].version, command: [] });
  }

  // Try each update alone, and hold it when it moves a held crate. Some
  // crates change only together with another crate (for example thiserror
  // and thiserror-impl). Cargo cannot update such a crate alone, but it is
  // not held when an applied update moves it.
  const failed: Held[] = [];
  const movedByApply = new Set<string>();
  for (const { exception, ...update } of planned) {
    const args = ["-p", `${update.name}@${update.from}`, "--precise", update.to];
    const extra = update.advisories ? { advisories: update.advisories } : {};
    const heldUpdate = { ecosystem: "crate" as const, name: update.name, version: update.from, to: update.to, ...extra };
    const run = tools.cargoDryRun(args);
    if (!run.ok) {
      const lastLine = run.output.trim().split("\n").pop() ?? "";
      failed.push({ ...heldUpdate, reason: `cargo cannot apply this version: ${lastLine}` });
      continue;
    }
    const changes = cargoChanges(run.output);
    const moves = changes
      .filter((change) => change.name !== update.name && heldCrate(change.name, change.from) !== null)
      .map((change) => change.name);
    if (moves.length > 0) {
      result.held.push({ ...heldUpdate, reason: "moves a held dependency", moves: [...new Set(moves)].sort() });
      continue;
    }
    result.apply.push({ ...update, command: ["cargo", "update", ...args] });
    for (const change of changes) movedByApply.add(`${change.name}@${change.from}`);
    if (exception) {
      result.exceptions.push({ ecosystem: "crate", name: update.name, version: update.to, published: exception.published.toISOString(), advisories: update.advisories! });
    }
  }
  result.held.push(...failed.filter((entry) => !movedByApply.has(`${entry.name}@${entry.version}`)));
}

async function selectPackages(tools: Tools, held: HeldList, now: Date, result: Selection): Promise<void> {
  const manifest = tools.packageJson();
  const direct = { ...manifest.dependencies, ...manifest.devDependencies };
  const exact = new Map(Object.entries(direct).filter(([, spec]) => /^\d/.test(spec)));
  for (const [name, version] of exact) result.held.push({ ecosystem: "package", name, version, reason: "exact version in package.json" });
  const heldName = (name: string) => heldReason(held.packages, name) ?? (exact.has(name) ? "exact version in package.json" : null);
  for (const name of Object.keys(direct)) {
    const reason = heldReason(held.packages, name);
    if (reason !== null && !exact.has(name)) result.held.push({ ecosystem: "package", name, version: direct[name], reason });
  }

  const base = lockPackages(tools.bun.lock());
  const versionIn = (lock: Map<string, { name: string; version: string }>, name: string) =>
    lock.get(name)?.name === name ? lock.get(name)!.version : ([...lock.values()].find((p) => p.name === name)?.version ?? null);
  /// Runs one update and returns its new version of `name` and the held packages that it moves.
  const tryUpdate = (name: string, args: string[]) => {
    const run = tools.bun.update(args);
    const next = lockPackages(run.lock);
    const moves = lockChanges(base, next).filter((change) => change.name !== name && heldName(change.name) !== null);
    return { to: versionIn(next, name), moves: [...new Set(moves.map((change) => change.name))].sort(), audit: run.audit };
  };

  const audit = tools.bun.audit();
  for (const [name, entries] of Object.entries(audit)) {
    const ids = entries.map((entry) => String(entry.id));
    const current = versionIn(base, name);
    if (current === null) continue;
    const heldEntry = { ecosystem: "package" as const, name, version: current, advisories: ids };
    const reason = heldName(name);
    if (reason !== null) {
      result.held.push({ ...heldEntry, reason: `advisory: the fix needs a held update. ${reason}` });
      continue;
    }
    let versions: Published[];
    try {
      versions = await tools.npmVersions(name);
    } catch {
      result.held.push({ ...heldEntry, reason: "publish date unknown" });
      continue;
    }
    const fixing = candidates(versions, current).filter((v) => entries.every((entry) => !Bun.semver.satisfies(v.version, entry.vulnerable_versions)));
    if (fixing.length === 0) {
      result.held.push({ ...heldEntry, reason: "advisory: no fixing version within the current range" });
      continue;
    }
    const fix = chooseFix(fixing, now);
    const age = Math.floor((now.getTime() - fix.version.published.getTime()) / 1000);
    const args = fix.exception ? [`--minimum-release-age=${age}`, name] : [name];
    const run = tryUpdate(name, args);
    const fixed = run.to !== null && (fix.exception ? run.to === fix.version.version : !newer(fix.version.version, run.to)) && !(name in run.audit);
    if (!fixed) {
      result.held.push({ ...heldEntry, to: fix.version.version, reason: "advisory: the update did not remove the advisory" });
      continue;
    }
    if (run.moves.length > 0) {
      result.held.push({ ...heldEntry, to: run.to!, reason: "moves a held dependency", moves: run.moves });
      continue;
    }
    result.apply.push({ ecosystem: "package", name, from: current, to: run.to!, command: ["bun", ...BUN_UPDATE, ...args], advisories: ids });
    if (fix.exception) {
      result.exceptions.push({ ecosystem: "package", name, version: run.to!, published: fix.version.published.toISOString(), advisories: ids });
    }
  }

  // bun applies the 7-day wait. An update of all packages that are not held
  // gives the packages that can change. Then each one is tried alone.
  const exclusions = Object.keys(direct).filter((name) => heldName(name) !== null).map((name) => `!${name}`);
  const all = lockPackages(tools.bun.update(exclusions).lock);
  const names = new Set(lockChanges(base, all).map((change) => change.name));
  for (const name of [...names].sort()) {
    if (heldName(name) !== null || name in audit) continue;
    const from = versionIn(base, name);
    const run = tryUpdate(name, [name]);
    if (from === null || run.to === null || run.to === from) continue;
    if (run.moves.length > 0) {
      result.held.push({ ecosystem: "package", name, version: from, to: run.to, reason: "moves a held dependency", moves: run.moves });
      continue;
    }
    result.apply.push({ ecosystem: "package", name, from, to: run.to, command: ["bun", ...BUN_UPDATE, name] });
  }
}

export async function selectUpdates(tools: Tools, held: HeldList): Promise<Selection> {
  const now = tools.now();
  const result: Selection = { apply: [], held: [], exceptions: [] };
  await selectCrates(tools, held, now, result);
  await selectPackages(tools, held, now, result);
  const order = (a: { ecosystem: string; name: string }, b: { ecosystem: string; name: string }) =>
    a.ecosystem.localeCompare(b.ecosystem) || a.name.localeCompare(b.name);
  result.apply.sort(order);
  result.held.sort(order);
  result.exceptions.sort(order);
  return result;
}

function run(command: string[], cwd: string, okCodes = [0]): { stdout: string; stderr: string; exitCode: number } {
  const result = Bun.spawnSync(command, { cwd });
  const output = { stdout: result.stdout.toString(), stderr: result.stderr.toString(), exitCode: result.exitCode };
  if (!okCodes.includes(result.exitCode)) {
    throw new Error(`${command.join(" ")} failed with exit code ${result.exitCode}: ${output.stderr}`);
  }
  return output;
}

/// The bun steps on temporary copies of the lockfiles in `repository`.
/// `bun` is the bun executable.
export function systemBunTools(repository: string, bun = process.execPath): BunTools {
  const inCopy = <T>(work: (dir: string) => T): T => {
    const dir = mkdtempSync(join(tmpdir(), "upkeep-bun-"));
    try {
      for (const file of ["package.json", "bun.lock", "bunfig.toml"]) cpSync(join(repository, file), join(dir, file));
      return work(dir);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  };
  // bun audit exits with 1 when it finds advisories.
  const audit = (dir: string): AuditReport => JSON.parse(run([bun, "audit", "--json"], dir, [0, 1]).stdout || "{}");
  return {
    lock: () => readFileSync(join(repository, "bun.lock"), "utf8"),
    audit: () => inCopy(audit),
    update: (args) =>
      inCopy((dir) => {
        run([bun, ...BUN_UPDATE, ...args], dir);
        return { lock: readFileSync(join(dir, "bun.lock"), "utf8"), audit: audit(dir) };
      }),
  };
}

/// The real cargo, cargo-deny, bun and registries for `repository`.
export function systemTools(repository: string): Tools {
  // crates.io allows at most one request each second. The npm registry
  // needs no delay.
  const spacing: Record<string, number> = { "crates.io": 1000 };
  const lastRequest = new Map<string, number>();
  const getJson = async (url: string) => {
    const host = new URL(url).host;
    const wait = (lastRequest.get(host) ?? 0) + (spacing[host] ?? 0) - Date.now();
    if (wait > 0) await Bun.sleep(wait);
    lastRequest.set(host, Date.now());
    const response = await fetch(url, { headers: { "User-Agent": USER_AGENT, Accept: "application/json" } });
    if (!response.ok) throw new Error(`${url} returned ${response.status}`);
    return response.json();
  };
  /// Keeps one lookup for each name. A failed lookup is not kept.
  const cached = (lookup: (name: string) => Promise<Published[]>) => {
    const results = new Map<string, Promise<Published[]>>();
    return (name: string): Promise<Published[]> => {
      let result = results.get(name);
      if (result === undefined) {
        result = lookup(name);
        results.set(name, result);
        const kept = result;
        kept.catch(() => {
          if (results.get(name) === kept) results.delete(name);
        });
      }
      return result;
    };
  };
  return {
    now: () => new Date(),
    cargoMetadata: () => JSON.parse(run(["cargo", "metadata", "--format-version", "1"], repository).stdout),
    cargoDryRun: (args) => {
      const result = run(["cargo", "update", "--dry-run", ...args], repository, [0, 101]);
      return { ok: result.exitCode === 0, output: result.stderr + result.stdout };
    },
    // cargo-deny writes its JSON lines to standard error, and exits with 1
    // when it finds advisories.
    cargoDenyAdvisories: () => run(["cargo", "deny", "--format", "json", "check", "advisories"], repository, [0, 1]).stderr,
    crateVersions: cached(async (name) => {
      const versions: Published[] = [];
      let page: string | null = "";
      while (page !== null) {
        const data = await getJson(`https://crates.io/api/v1/crates/${name}/versions${page}`);
        for (const v of data.versions) versions.push({ version: v.num, published: new Date(v.created_at), yanked: v.yanked });
        page = data.meta?.next_page ?? null;
      }
      return versions;
    }),
    packageJson: () => JSON.parse(readFileSync(join(repository, "package.json"), "utf8")),
    bun: systemBunTools(repository),
    npmVersions: cached(async (name) => {
      const data = await getJson(`https://registry.npmjs.org/${name.replace("/", "%2f")}`);
      return Object.keys(data.versions ?? {}).map((version) => ({ version, published: new Date(data.time[version]), yanked: false }));
    }),
  };
}

if (import.meta.main) {
  if (process.argv.length > 2) {
    console.error("usage: select-updates.ts");
    process.exit(2);
  }
  const selection = await selectUpdates(systemTools(process.cwd()), loadHeld());
  console.log(JSON.stringify(selection, null, 2));
  console.error(`apply=${selection.apply.length} held=${selection.held.length} exceptions=${selection.exceptions.length}`);
}
