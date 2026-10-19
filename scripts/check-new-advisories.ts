// Fails a PR that adds a known vulnerability, an unsound or unmaintained
// crate, or a yanked crate (docs/development-workflow.md). The `cargo deny`
// CI job runs it on each PR.
//
//   bun scripts/check-new-advisories.ts <base-dir> <head-dir>
//
// Runs cargo-deny (with the deny.toml of the base) and bun audit in the two
// checkouts. A PR adds an advisory when the head has an (ecosystem,
// advisory ID, package) that the base does not have, or more affected
// copies of that package than the base has. An advisory that is already on
// the base does not fail the PR, also when the PR moves the affected
// package to a version that is still affected. Fails with "audit could not
// run" when a tool cannot run.
//
// Both checkouts use the deny.toml of the base. A PR that changes deny.toml
// then cannot hide its own advisory, and a stricter rule in the PR does not
// report an old finding as new.

import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { bunLockVersions } from "./lockfiles";

/// One affected package version.
export type Finding = { ecosystem: "crate" | "package"; id: string; name: string; version: string; title: string };

/// The output of cargo-deny and bun audit for one checkout, and its bun.lock.
export type ScanOutput = {
  deny: { exitCode: number; stderr: string };
  audit: { exitCode: number; stdout: string; stderr: string };
  bunLock: string;
};

export type Scan = { ok: true; findings: Finding[] } | { ok: false; error: string };

/// bun audit's JSON: the advisories of each package.
type AuditReport = Record<string, { id: number | string; vulnerable_versions: string; title?: string }[]>;

/// The items in cargo-deny's JSON lines. Other lines are ignored.
function denyItems(text: string): any[] {
  return text
    .split("\n")
    .filter((line) => line.trim().startsWith("{"))
    .map((line) => JSON.parse(line));
}

/// The findings in cargo-deny's JSON lines. A yanked crate has no advisory
/// ID, so its ID is "yanked:<name>@<version>".
function denyFindings(output: { exitCode: number; stderr: string }): Scan {
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

/// The findings in bun audit's JSON. bun audit gives no version, so the
/// script takes each bun.lock version of the package that the advisory affects.
function auditFindings(output: { exitCode: number; stdout: string; stderr: string }, bunLock: string): Scan {
  let report: AuditReport;
  try {
    // bun audit exits with 1 when it finds advisories, and also when it cannot
    // reach the registry. Only the first case gives a JSON object.
    report = output.stdout.trim() === "" && output.exitCode === 0 ? {} : JSON.parse(output.stdout);
    if (typeof report !== "object" || report === null || Array.isArray(report)) throw new Error("not an object");
    if (output.exitCode === 1 && Object.keys(report).length === 0) throw new Error("no advisories");
    if (![0, 1].includes(output.exitCode)) throw new Error("exit code");
  } catch {
    return { ok: false, error: `bun audit exited with ${output.exitCode}: ${lastLine(output.stderr || output.stdout)}` };
  }
  const versions = bunLockVersions(bunLock);
  const findings: Finding[] = [];
  for (const [name, entries] of Object.entries(report)) {
    for (const entry of entries) {
      const affected = (versions.get(name) ?? []).filter((version) => Bun.semver.satisfies(version, entry.vulnerable_versions));
      for (const version of new Set(affected.length > 0 ? affected : [entry.vulnerable_versions])) {
        findings.push({ ecosystem: "package", id: String(entry.id), name, version, title: String(entry.title ?? "").trim() });
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
  const audit = auditFindings(output.audit, output.bunLock);
  const errors = [deny, audit].flatMap((result) => (result.ok ? [] : [result.error]));
  if (errors.length > 0) return { ok: false, error: errors.join("\n") };
  return { ok: true, findings: [...(deny as { findings: Finding[] }).findings, ...(audit as { findings: Finding[] }).findings] };
}

const group = (f: Finding) => `${f.ecosystem} ${f.id} ${f.name}`;
const describe = (f: Finding) => `${f.id} (${f.name} ${f.version})${f.title ? `: ${f.title}` : ""}`;

/// The distinct affected versions of each (ecosystem, advisory, package).
function copies(findings: Finding[]): Map<string, Map<string, Finding>> {
  const groups = new Map<string, Map<string, Finding>>();
  for (const f of findings) {
    const versions = groups.get(group(f)) ?? new Map<string, Finding>();
    versions.set(f.version, f);
    groups.set(group(f), versions);
  }
  return groups;
}

/// The advisories that the head adds. For a group with more copies on the
/// head, the added findings are the head versions that the base does not have.
export function compare(base: Scan, head: Scan): { ok: boolean; added: Finding[]; message: string } {
  const failures = [base.ok ? null : `base: ${base.error}`, head.ok ? null : `head: ${head.error}`].filter(Boolean);
  if (!base.ok || !head.ok) {
    return { ok: false, added: [], message: `The audit could not run.\n${failures.join("\n")}` };
  }
  const known = copies(base.findings);
  const added: Finding[] = [];
  for (const [key, versions] of copies(head.findings)) {
    const before = known.get(key);
    if (before && versions.size <= before.size) continue;
    added.push(...[...versions.values()].filter((f) => !before?.has(f.version)));
  }
  if (added.length === 0) return { ok: true, added, message: "The head adds no advisory." };
  return { ok: false, added, message: `The head adds these advisories:\n${added.map((f) => `- ${describe(f)}`).join("\n")}` };
}

function run(command: string[], cwd: string): { exitCode: number; stdout: string; stderr: string } {
  const result = Bun.spawnSync(command, { cwd });
  return { exitCode: result.exitCode, stdout: result.stdout.toString(), stderr: result.stderr.toString() };
}

/// Runs cargo-deny and bun audit in `dir`, with the rules in `denyConfig`.
function scanDir(dir: string, denyConfig: string): ScanOutput {
  return {
    deny: run(["cargo", "deny", "--format", "json", "--config", resolve(denyConfig), "check", "advisories"], dir),
    audit: run([process.execPath, "audit", "--json"], dir),
    bunLock: readFileSync(join(dir, "bun.lock"), "utf8"),
  };
}

if (import.meta.main) {
  const [baseDir, headDir] = process.argv.slice(2);
  if (!baseDir || !headDir) {
    console.error("usage: check-new-advisories.ts <base-dir> <head-dir>");
    process.exit(2);
  }
  const baseConfig = join(baseDir, "deny.toml");
  const result = compare(scan(scanDir(baseDir, baseConfig)), scan(scanDir(headDir, baseConfig)));
  console.log(result.message);
  process.exit(result.ok ? 0 : 1);
}
