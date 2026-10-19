// Finds known vulnerabilities in the Rust and JavaScript dependencies
// (R1, R2, R3, R21, R25; KTD6, KTD8, KTD9, KTD13). advisories.yml runs it.
//
//   bun scripts/upkeep/advisories.ts compare <base-dir> <head-dir>
//     Runs cargo-deny (with the deny.toml of the base) and bun audit in the
//     two checkouts. Fails when the head
//     has an affected package version (advisory ID, package and version) that
//     the base does not have. Fails with "audit could not run" when a tool
//     cannot run.
//
//   bun scripts/upkeep/advisories.ts ignores <base-dir> <head-dir>
//     Fails when the head adds an advisory ignore or a licence to deny.toml
//     or about.toml, or adds an `--ignore` to a bun audit command (R3).
//
//   bun scripts/upkeep/advisories.ts classify <issue-body-file> <out-dir>
//     Run from the repository root. Finds the advisories that need a
//     decision: the advisories that stay after the allowed updates of
//     select-updates.ts, and the advisories that only a held update fixes
//     (KTD9). The script applies the allowed updates to the lockfiles, scans
//     again, and then puts back the original Cargo.lock and bun.lock. It also
//     finds when the last weekly run is more than 8 days old (KTD13).
//     Writes <out-dir>/result.json, and <out-dir>/notice.md when the
//     maintainer must get a notice.
//
//   bun scripts/upkeep/advisories.ts mark <result-file> <issue-body-file>
//     Prints the issue body with the advisories of the notice added to the
//     reported-advisories marker (issue-markers.ts), so that each advisory
//     gives one notice only.
//
// Advisory text, such as a title, comes from other people. The workflow
// gives it to shell steps only in files and environment variables.

import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { daysSinceLastRun, NO_LAST_RUN, readMarkers, writeMarkers } from "./issue-markers";
import {
  denyDiagnostics,
  denyItems,
  loadHeld,
  parseBunLock,
  selectUpdates,
  splitSpec,
  systemTools,
  type AuditReport,
  type Selection,
  type Update,
} from "./select-updates";

/// A notice for a weekly run that is older than this number of days.
const OVERDUE_DAYS = 8;

/// One affected package version.
export type Finding = { ecosystem: "crate" | "package"; id: string; name: string; version: string; title: string };

/// The output of cargo-deny and bun audit for one checkout, and its bun.lock.
export type ScanOutput = {
  deny: { exitCode: number; stderr: string };
  audit: { exitCode: number; stdout: string; stderr: string };
  bunLock: string;
};

export type Scan = { ok: true; findings: Finding[] } | { ok: false; error: string };

/// The findings in cargo-deny's JSON lines. A yanked crate has no advisory
/// ID, so its ID is "yanked:<name>@<version>".
function denyFindings(output: { exitCode: number; stderr: string }): Scan {
  const findings: Finding[] = [];
  const summary = denyItems(output.stderr).some((item) => item.type === "summary");
  for (const item of denyDiagnostics(output.stderr)) {
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
  if (!summary || ![0, 1].includes(output.exitCode)) {
    return { ok: false, error: `cargo-deny exited with ${output.exitCode}: ${lastLine(output.stderr)}` };
  }
  return { ok: true, findings };
}

/// The package versions in bun.lock, by name.
function lockVersions(text: string): Map<string, string[]> {
  const lock = parseBunLock(text);
  const versions = new Map<string, string[]>();
  for (const value of Object.values(lock.packages ?? {})) {
    const { name, version } = splitSpec((value as string[])[0]);
    versions.set(name, [...(versions.get(name) ?? []), version]);
  }
  return versions;
}

/// The findings in bun audit's JSON. bun audit gives no version, so the
/// script takes each bun.lock version of the package that the advisory affects.
function auditFindings(output: { exitCode: number; stdout: string; stderr: string }, bunLock: string): Scan {
  let report: AuditReport & Record<string, { title?: string }[]>;
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
  const versions = lockVersions(bunLock);
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

const key = (f: Finding) => `${f.ecosystem} ${f.id} ${f.name} ${f.version}`;
const describe = (f: Finding) => `${f.id} (${f.name} ${f.version})${f.title ? `: ${f.title}` : ""}`;

/// The affected package versions that the head has and the base does not have (KTD8).
export function compare(base: Scan, head: Scan): { ok: boolean; added: Finding[]; message: string } {
  const failures = [base.ok ? null : `base: ${base.error}`, head.ok ? null : `head: ${head.error}`].filter(Boolean);
  if (!base.ok || !head.ok) {
    return { ok: false, added: [], message: `The audit could not run.\n${failures.join("\n")}` };
  }
  const known = new Set(base.findings.map(key));
  const added = [...new Map(head.findings.filter((f) => !known.has(key(f))).map((f) => [key(f), f])).values()];
  if (added.length === 0) return { ok: true, added, message: "The head adds no advisory." };
  return { ok: false, added, message: `The head adds these advisories:\n${added.map((f) => `- ${describe(f)}`).join("\n")}` };
}

export type Notice = { id: string; findings: Finding[]; reason: string };
export type Classification = {
  notices: Notice[];
  overdue: boolean;
  /// True when the run is overdue and no overdue notice was given for this last run date.
  overdueNotice: boolean;
  daysSinceLastRun: number | null;
  lastRun: string | null;
  /// The reported advisories after this notice: the issue's list and the new notices.
  reportedAdvisories: string[];
};

/// Applies KTD9 and KTD13. `before` is the scan of the current lockfiles,
/// `after` the scan after the allowed updates of `selection`.
export function classify(input: { before: Finding[]; after: Finding[]; selection: Selection; issueBody: string; now: Date }): Classification {
  const markers = readMarkers(input.issueBody);
  const heldReasons = new Map<string, string>();
  for (const held of input.selection.held) for (const id of held.advisories ?? []) if (!heldReasons.has(id)) heldReasons.set(id, held.reason);

  const open = new Map<string, Finding[]>();
  for (const f of [...input.after, ...input.before.filter((f) => heldReasons.has(f.id))]) {
    const list = open.get(f.id) ?? [];
    if (!list.some((other) => key(other) === key(f))) list.push(f);
    open.set(f.id, list);
  }
  const reported = new Set(markers.reportedAdvisories);
  const notices = [...open.entries()]
    .filter(([id]) => !reported.has(id))
    .map(([id, findings]) => ({ id, findings, reason: heldReasons.get(id) ?? "The next weekly update does not remove it." }))
    .sort((a, b) => a.id.localeCompare(b.id));

  const days = daysSinceLastRun(input.issueBody, input.now);
  const overdue = days === null || days > OVERDUE_DAYS;
  return {
    notices,
    overdue,
    overdueNotice: overdue && markers.overdueNotified !== (markers.lastRun ?? NO_LAST_RUN),
    daysSinceLastRun: days,
    lastRun: markers.lastRun,
    reportedAdvisories: [...new Set([...markers.reportedAdvisories, ...notices.map((n) => n.id)])].sort(),
  };
}

/// The text of the notice comment, without the mention, or null when the
/// maintainer gets no notice.
export function noticeText(result: Classification): string | null {
  const parts: string[] = [];
  if (result.notices.length > 0) {
    const lines = result.notices.map((notice) => {
      const versions = notice.findings.map((f) => `${f.name} ${f.version}`).join(", ");
      const title = notice.findings.find((f) => f.title)?.title;
      return `- ${notice.id} (${versions})${title ? `: ${title}` : ""}\n  ${notice.reason}`;
    });
    parts.push(`The daily advisory check found advisories that the weekly update cannot fix. Each one needs your decision.\n\n${lines.join("\n")}`);
  }
  if (result.overdueNotice) {
    parts.push(
      result.daysSinceLastRun === null
        ? "The tracking issue has no date of the last weekly run. Make sure that the weekly upkeep routine runs."
        : `The last weekly upkeep run was ${result.daysSinceLastRun} days ago (${result.lastRun}). Make sure that the weekly upkeep routine runs.`,
    );
  }
  return parts.length > 0 ? parts.join("\n\n") : null;
}

/// The issue body with the reported advisories of `result`, and the overdue
/// marker when `result` gave the overdue notice.
export function markReported(body: string, result: Classification): string {
  const markers = readMarkers(body);
  return writeMarkers(body, {
    lastRun: markers.lastRun,
    reportedAdvisories: [...markers.reportedAdvisories, ...result.reportedAdvisories],
    overdueNotified: result.overdueNotice ? (markers.lastRun ?? NO_LAST_RUN) : undefined,
  });
}

/// The steps of classify. Tests replace them.
export type ClassifyTools = {
  /// cargo-deny and bun audit on the current lockfiles.
  scan(): ScanOutput;
  /// The selection of select-updates.ts.
  select(): Promise<Selection>;
  /// Applies the updates to the lockfiles.
  apply(updates: Update[]): void;
  /// Puts back the original lockfiles.
  restore(): void;
};

export async function runClassify(tools: ClassifyTools, issueBody: string, now: Date): Promise<Classification> {
  try {
    const before = scan(tools.scan());
    if (!before.ok) throw new Error(`The audit could not run.\n${before.error}`);
    const selection = await tools.select();
    tools.apply(selection.apply);
    const after = scan(tools.scan());
    if (!after.ok) throw new Error(`The audit could not run after the updates.\n${after.error}`);
    return classify({ before: before.findings, after: after.findings, selection, issueBody, now });
  } finally {
    tools.restore();
  }
}

/// The settings that can hide an advisory or allow a licence.
export type IgnoreFiles = { denyToml: string; aboutToml: string; auditCommands: string };

/// The ignores and licences that `head` adds (R3). bun has no setting for
/// audit ignores, so the script reads the arguments of the bun audit commands.
export function addedIgnores(base: IgnoreFiles, head: IgnoreFiles): string[] {
  const read = (files: IgnoreFiles) => {
    const deny = Bun.TOML.parse(files.denyToml || "") as {
      advisories?: { ignore?: (string | { id: string })[] };
      licenses?: { allow?: string[]; exceptions?: { crate: string; allow: string[] }[] };
    };
    const about = Bun.TOML.parse(files.aboutToml || "") as { accepted?: string[] };
    const ignoreArgument = /--ignore[= ]+["']?([\w-]+)/g;
    const commands = files.auditCommands.split("\n").filter((line) => line.includes("audit"));
    return {
      "deny.toml advisory ignore": (deny.advisories?.ignore ?? []).map((entry) => (typeof entry === "string" ? entry : entry.id)),
      "deny.toml licence": deny.licenses?.allow ?? [],
      "deny.toml licence exception": (deny.licenses?.exceptions ?? []).flatMap((entry) => entry.allow.map((licence) => `${entry.crate} ${licence}`)),
      "about.toml licence": about.accepted ?? [],
      "bun audit ignore": commands.flatMap((line) => [...line.matchAll(ignoreArgument)].map((match) => match[1])),
    };
  };
  const [before, after] = [read(base), read(head)];
  const added: string[] = [];
  for (const [kind, values] of Object.entries(after)) {
    const known = new Set(before[kind as keyof typeof before]);
    for (const value of new Set(values)) if (!known.has(value)) added.push(`${kind}: ${value}`);
  }
  return added;
}

function run(command: string[], cwd: string): { exitCode: number; stdout: string; stderr: string } {
  const result = Bun.spawnSync(command, { cwd });
  return { exitCode: result.exitCode, stdout: result.stdout.toString(), stderr: result.stderr.toString() };
}

/// Runs cargo-deny and bun audit in `dir`. cargo-deny uses the rules in
/// `denyConfig`, by default the deny.toml of `dir`.
function scanDir(dir: string, denyConfig = join(dir, "deny.toml")): ScanOutput {
  return {
    deny: run(["cargo", "deny", "--format", "json", "--config", resolve(denyConfig), "check", "advisories"], dir),
    audit: run([process.execPath, "audit", "--json"], dir),
    bunLock: readFileSync(join(dir, "bun.lock"), "utf8"),
  };
}

function readOrEmpty(path: string): string {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return "";
  }
}

/// The files of `dir` that the ignores check reads.
function ignoreFiles(dir: string): IgnoreFiles {
  const files = [join(dir, "package.json")];
  for (const [folder, suffix] of [
    [".github/workflows", ".yml"],
    ["scripts/upkeep", ".ts"],
  ]) {
    const names = (() => {
      try {
        return readdirSync(join(dir, folder));
      } catch {
        return [];
      }
    })();
    for (const name of names) if (name.endsWith(suffix) && !name.endsWith(".test.ts")) files.push(join(dir, folder, name));
  }
  return {
    denyToml: readOrEmpty(join(dir, "deny.toml")),
    aboutToml: readOrEmpty(join(dir, "about.toml")),
    auditCommands: files.map(readOrEmpty).join("\n"),
  };
}

/// The classify steps for the repository in `dir`. Each update runs on the
/// lockfiles of `dir`; `restore` writes back the original lockfiles.
function systemClassifyTools(dir: string): ClassifyTools {
  const lockfiles = ["Cargo.lock", "bun.lock"].map((name) => ({ path: join(dir, name), text: readFileSync(join(dir, name), "utf8") }));
  return {
    scan: () => scanDir(dir),
    select: () => selectUpdates(systemTools(dir), loadHeld()),
    apply: (updates) => {
      for (const update of updates) {
        const command = update.command[0] === "bun" ? [process.execPath, ...update.command.slice(1)] : update.command;
        const result = run(command, dir);
        // A failed update leaves its advisory in the second scan, so the
        // advisory gets a notice.
        if (result.exitCode !== 0) console.error(`${update.command.join(" ")} failed: ${lastLine(result.stderr)}`);
      }
    },
    restore: () => {
      for (const file of lockfiles) writeFileSync(file.path, file.text);
    },
  };
}

if (import.meta.main) {
  const [command, first, second] = process.argv.slice(2);
  const usage = () => {
    console.error("usage: advisories.ts compare|ignores <base-dir> <head-dir>");
    console.error("       advisories.ts classify <issue-body-file> <out-dir>");
    console.error("       advisories.ts mark <result-file> <issue-body-file>");
    process.exit(2);
  };
  if (!first || !second) usage();
  if (command === "compare") {
    // Both checkouts use the rules of the base. A PR that changes deny.toml
    // then cannot hide its own advisory, and a stricter rule in the PR does
    // not report an old finding as new.
    const baseConfig = join(first, "deny.toml");
    const result = compare(scan(scanDir(first, baseConfig)), scan(scanDir(second, baseConfig)));
    console.log(result.message);
    process.exit(result.ok ? 0 : 1);
  } else if (command === "ignores") {
    const added = addedIgnores(ignoreFiles(first), ignoreFiles(second));
    if (added.length === 0) {
      console.log("The head adds no ignore and no licence.");
    } else {
      console.log(`An upkeep PR must not add an ignore or a licence. Only the maintainer adds one (R3).\n${added.map((line) => `- ${line}`).join("\n")}`);
      process.exit(1);
    }
  } else if (command === "classify") {
    const result = await runClassify(systemClassifyTools(process.cwd()), readFileSync(first, "utf8"), new Date());
    mkdirSync(second, { recursive: true });
    writeFileSync(join(second, "result.json"), `${JSON.stringify(result, null, 2)}\n`);
    const text = noticeText(result);
    if (text !== null) writeFileSync(join(second, "notice.md"), `${text}\n`);
    console.log(`notices=${result.notices.length} overdue=${result.overdue} days-since-last-run=${result.daysSinceLastRun ?? "unknown"}`);
  } else if (command === "mark") {
    const result: Classification = JSON.parse(readFileSync(first, "utf8"));
    process.stdout.write(markReported(readFileSync(second, "utf8"), result));
  } else {
    usage();
  }
}
