// Fails a PR that adds a known vulnerability, an unsound or unmaintained
// crate, or a yanked crate (docs/development-workflow.md). The `cargo deny`
// CI job runs it on each PR.
//
//   node scripts/check-new-advisories.ts <base-dir> <head-dir>
//
// Runs cargo-deny (with the deny.toml of the base) and npm audit for the two
// checkouts. A PR adds an advisory when the head has an (ecosystem,
// advisory ID, package) that the base does not have, or more affected
// copies of that package than the base has. An advisory that is already on
// the base does not fail the PR, also when the PR moves the affected
// package to a version that is still affected. Fails with "audit could not
// run" when a tool cannot run.
//
// Both checkouts use the deny.toml and the .npmrc of the base. A PR that
// changes deny.toml or .npmrc then cannot hide its own advisory, and a
// stricter rule in the PR does not report an old finding as new. npm has no
// option that stops it from reading the .npmrc of its project folder, so
// npm audit runs in a new temporary folder that holds only copies of
// package.json, package-lock.json and the base .npmrc.

import { copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { packageLockEntries, readIfPresent, type Ecosystem } from "./lockfiles.ts";
import { MAX_OUTPUT_BYTES } from "./lib/spawn.ts";
import { satisfies } from "./lib/version.ts";

/// One affected package version.
export type Finding = { ecosystem: Ecosystem; id: string; name: string; version: string; title: string };

export type RunResult = { exitCode: number; stdout: string; stderr: string };

/// The output of cargo-deny and npm audit for one checkout, and its package-lock.json.
export type ScanOutput = {
  deny: { exitCode: number; stderr: string };
  audit: RunResult;
  packageLock: string;
};

export type Scan = { ok: true; findings: Finding[] } | { ok: false; error: string };

/// One advisory in `via` of an npm audit vulnerability. A string in `via`
/// is the name of another vulnerable package, not an advisory.
type AuditAdvisory = { source?: number | string; url?: string; title?: string; range?: string };

/// npm audit's JSON (auditReportVersion 2). `nodes` are package-lock.json keys.
type AuditReport = {
  vulnerabilities?: Record<string, { via?: (string | AuditAdvisory)[]; nodes?: string[] }>;
  message?: string;
};

/// The items in cargo-deny's JSON lines. Other lines are ignored.
function denyItems(text: string): any[] {
  return text
    .split("\n")
    .filter((line) => line.trim().startsWith("{"))
    .map((line) => JSON.parse(line));
}

/// The findings in cargo-deny's JSON lines. A yanked crate has no advisory
/// ID, so its ID is "yanked:<name>@<version>".
export function denyFindings(output: { exitCode: number; stderr: string }): Scan {
  const items = denyItems(output.stderr);
  const findings: Finding[] = [];
  for (const item of items.filter((i) => i.type === "diagnostic")) {
    if (item.fields?.severity !== "error") continue;
    const advisory = item.fields.advisory;
    const graphs: { Krate: { name: string; version: string } }[] = item.fields.graphs ?? [];
    if (advisory) {
      for (const { Krate } of graphs.filter((graph) => graph.Krate.name === advisory.package)) {
        findings.push({ ecosystem: "crate", id: advisory.id, name: Krate.name, version: Krate.version, title: String(advisory.title ?? "").trim() });
      }
    } else if (item.fields.code === "yanked") {
      for (const { Krate } of graphs) {
        findings.push({ ecosystem: "crate", id: `yanked:${Krate.name}@${Krate.version}`, name: Krate.name, version: Krate.version, title: "yanked version" });
      }
    }
  }
  // cargo-deny exits with 1 when it finds advisories. It always writes a
  // summary line when the check ran to the end.
  if (!items.some((item) => item.type === "summary") || ![0, 1].includes(output.exitCode)) {
    return { ok: false, error: `cargo-deny exited with ${output.exitCode}: ${lastLine(output.stderr)}` };
  }
  return { ok: true, findings };
}

/// True when the advisory range contains the version. A range or a version
/// outside the subset of scripts/lib/version.ts (for example a prerelease
/// version) counts as affected, so that the check does not miss a finding.
function inRange(version: string, range: string): boolean {
  try {
    return satisfies(version, range);
  } catch {
    return true;
  }
}

/// The findings in npm audit's JSON. Each node of a vulnerability is one
/// package-lock.json entry, and the script reads its version from the
/// lockfile. An advisory applies to a node only when the node's version is
/// in the advisory's range.
function auditFindings(output: RunResult, packageLock: string): Scan {
  let report: AuditReport;
  try {
    // npm audit exits with 1 when it finds advisories, and also when it
    // cannot reach the registry. Only the first case gives a
    // `vulnerabilities` object.
    report = JSON.parse(output.stdout);
    if (typeof report !== "object" || report === null || Array.isArray(report)) throw new Error("not an object");
    const vulnerabilities = report.vulnerabilities;
    if (typeof vulnerabilities !== "object" || vulnerabilities === null || Array.isArray(vulnerabilities)) throw new Error("no vulnerabilities");
    if (output.exitCode === 1 && Object.keys(vulnerabilities).length === 0) throw new Error("no advisories");
    if (![0, 1].includes(output.exitCode)) throw new Error("exit code");
  } catch {
    let detail = lastLine(output.stderr || output.stdout);
    try {
      detail = String((JSON.parse(output.stdout) as AuditReport).message ?? detail);
    } catch {
      // The output is not JSON. Keep the last line.
    }
    return { ok: false, error: `npm audit exited with ${output.exitCode}: ${detail}` };
  }
  let entries: ReturnType<typeof packageLockEntries>;
  try {
    entries = packageLockEntries(packageLock);
  } catch (error) {
    return { ok: false, error: `package-lock.json could not be read: ${(error as Error).message}` };
  }
  const findings: Finding[] = [];
  for (const [name, vulnerability] of Object.entries(report.vulnerabilities ?? {})) {
    const nodes = vulnerability.nodes ?? [];
    for (const advisory of vulnerability.via ?? []) {
      if (typeof advisory === "string") continue;
      const id = String(advisory.source ?? advisory.url ?? "unknown");
      const range = String(advisory.range ?? "*");
      const affected = nodes
        .map((node) => entries[node]?.version ?? "missing")
        .filter((version) => inRange(version, range));
      // One finding for each affected lockfile entry, so that two copies of
      // one version count as two.
      for (const version of affected.length > 0 ? affected : [range]) {
        findings.push({ ecosystem: "package", id, name, version, title: String(advisory.title ?? "").trim() });
      }
    }
  }
  return { ok: true, findings };
}

function lastLine(text: string): string {
  return text.trim().split("\n").pop() ?? "";
}

/// The findings of one checkout, or the reason that a tool could not run.
export function scan(output: ScanOutput): Scan {
  const deny = denyFindings(output.deny);
  const audit = auditFindings(output.audit, output.packageLock);
  if (!deny.ok || !audit.ok) {
    return { ok: false, error: [deny, audit].flatMap((result) => (result.ok ? [] : [result.error])).join("\n") };
  }
  return { ok: true, findings: [...deny.findings, ...audit.findings] };
}

const group = (f: Finding) => `${f.ecosystem} ${f.id} ${f.name}`;
const describe = (f: Finding) => `${f.id} (${f.name} ${f.version})${f.title ? `: ${f.title}` : ""}`;

/// The affected copies of each (ecosystem, advisory, package). Each finding
/// is one copy.
function copies(findings: Finding[]): Map<string, Finding[]> {
  const groups = new Map<string, Finding[]>();
  for (const f of findings) groups.set(group(f), [...(groups.get(group(f)) ?? []), f]);
  return groups;
}

/// The advisories that the head adds. For a group with more copies on the
/// head, the added findings are the head copies of each version beyond the
/// number of copies of that version on the base.
export function compare(base: Scan, head: Scan): { ok: boolean; added: Finding[]; message: string } {
  const failures = [base.ok ? null : `base: ${base.error}`, head.ok ? null : `head: ${head.error}`].filter(Boolean);
  if (!base.ok || !head.ok) {
    return { ok: false, added: [], message: `The audit could not run.\n${failures.join("\n")}` };
  }
  const known = copies(base.findings);
  const added: Finding[] = [];
  for (const [key, headCopies] of copies(head.findings)) {
    const baseCopies = known.get(key) ?? [];
    if (headCopies.length <= baseCopies.length) continue;
    const remaining = new Map<string, number>();
    for (const f of baseCopies) remaining.set(f.version, (remaining.get(f.version) ?? 0) + 1);
    for (const f of headCopies) {
      const count = remaining.get(f.version) ?? 0;
      if (count > 0) remaining.set(f.version, count - 1);
      else added.push(f);
    }
  }
  if (added.length === 0) return { ok: true, added, message: "The head adds no advisory." };
  return { ok: false, added, message: `The head adds these advisories:\n${added.map((f) => `- ${describe(f)}`).join("\n")}` };
}

/// A tool that runs longer than this is stopped, and the check fails with
/// "audit could not run".
const TOOL_TIMEOUT_MS = 10 * 60 * 1000;


export type Runner = (command: string[], cwd: string) => RunResult;

export const run: Runner = (command, cwd) => {
  const result = spawnSync(command[0], command.slice(1), { cwd, timeout: TOOL_TIMEOUT_MS, maxBuffer: MAX_OUTPUT_BYTES });
  // A stopped process has a null exit code. A process that did not start
  // has an error and no output.
  const stderr = [result.stderr?.toString() ?? "", result.error ? String(result.error.message) : ""].filter(Boolean).join("\n");
  return { exitCode: result.status ?? -1, stdout: result.stdout?.toString() ?? "", stderr };
};

/// Runs npm audit for the package-lock.json of `dir`, with the npm
/// settings of `baseDir` only. The audit runs in a new temporary folder
/// with a copy of package.json (without `devEngines`), a copy of
/// package-lock.json, and a copy of the base .npmrc when the base has one.
/// `devEngines` is removed because a PR can change it, and npm stops when
/// its own version is outside that range. The empty user config file and
/// `--registry` keep the user settings and another registry out. The
/// folder is removed after the run.
export function auditCheckout(dir: string, baseDir: string, runner: Runner = run): RunResult {
  let folder: string | undefined;
  try {
    folder = mkdtempSync(join(tmpdir(), "longhand-audit-"));
    const manifest = JSON.parse(readFileSync(join(dir, "package.json"), "utf8")) as Record<string, unknown>;
    delete manifest.devEngines;
    writeFileSync(join(folder, "package.json"), JSON.stringify(manifest, null, 2));
    copyFileSync(join(dir, "package-lock.json"), join(folder, "package-lock.json"));
    if (existsSync(join(baseDir, ".npmrc"))) copyFileSync(join(baseDir, ".npmrc"), join(folder, ".npmrc"));
    const userConfig = join(folder, "empty-user-config");
    writeFileSync(userConfig, "");
    return runner(
      ["npm", "audit", "--json", "--package-lock-only", "--ignore-scripts", "--registry=https://registry.npmjs.org/", `--userconfig=${userConfig}`],
      folder,
    );
  } catch (error) {
    return { exitCode: -1, stdout: "", stderr: `the audit folder could not be made: ${(error as Error).message}` };
  } finally {
    if (folder) rmSync(folder, { recursive: true, force: true });
  }
}

/// Runs cargo-deny in `dir` with the rules in `denyConfig`.
function denyDir(dir: string, denyConfig: string): { exitCode: number; stderr: string } {
  return run(["cargo", "deny", "--format", "json", "--config", resolve(denyConfig), "check", "advisories"], dir);
}

/// Runs cargo-deny and npm audit for `dir`, with the settings of `baseDir`.
function scanDir(dir: string, baseDir: string): ScanOutput {
  return {
    deny: denyDir(dir, join(baseDir, "deny.toml")),
    audit: auditCheckout(dir, baseDir),
    packageLock: readIfPresent(join(dir, "package-lock.json")),
  };
}

if (import.meta.main) {
  const [baseDir, headDir] = process.argv.slice(2);
  if (!baseDir || !headDir) {
    console.error("usage: check-new-advisories.ts <base-dir> <head-dir>");
    process.exit(2);
  }
  const result = compare(scan(scanDir(baseDir, baseDir)), scan(scanDir(headDir, baseDir)));
  console.log(result.message);
  process.exit(result.ok ? 0 : 1);
}
