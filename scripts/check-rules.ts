// Check the rule ledger in rules/, and prove that each check still catches its mistake.
//
// Each file rules/<family>.toml records one mistake family: its rule, its
// state, the command that holds the rule, and its negative examples. A
// negative example is a patch that puts the mistake back. The script applies
// each patch to the working tree, runs the command, and expects the command to
// fail with the expected text. Then it restores the tree. rules/README.md
// describes the fields.
//
// The script reads the committed tree, so it refuses to run when the working
// tree has changes. Commit first.
//
//   node scripts/check-rules.ts                  check the ledger and run every negative example
//   node scripts/check-rules.ts --validate       check the ledger only
//   node scripts/check-rules.ts --family <id>    check the ledger and run one family's negative examples
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { MAX_OUTPUT_BYTES } from "./lib/spawn.ts";
import { parseToml, type TomlTable, type TomlValue } from "./lib/toml.ts";

const root = resolve(import.meta.dirname, "..");
const rulesDir = join(root, "rules");
const states = ["enforced", "judgment"];
const familyKeys = ["family", "rule", "state", "enforced_by", "lessons", "judgment", "negative"];
const negativeKeys = ["apply", "run", "expect"];

type Negative = { apply: string; run: string; expect: string };
type Family = { id: string; file: string; state: string; negatives: Negative[] };

function isString(v: TomlValue | undefined): v is string {
  return typeof v === "string" && v.trim() !== "";
}

// Read and check one family file. Each problem goes into `errors` with the file and the field.
function readFamily(file: string, errors: string[]): Family | undefined {
  const where = `rules/${file}`;
  let table: TomlTable;
  try {
    table = parseToml(readFileSync(join(rulesDir, file), "utf8"));
  } catch (e) {
    errors.push(`${where}: ${(e as Error).message}`);
    return undefined;
  }
  const fail = (field: string, message: string) => errors.push(`${where}: ${field}: ${message}`);
  for (const key of Object.keys(table)) if (!familyKeys.includes(key)) fail(key, "unknown field");

  const id = file.replace(/\.toml$/, "");
  if (!isString(table.family)) fail("family", "missing");
  else if (table.family !== id) fail("family", `"${table.family}" must equal the file name "${id}"`);
  if (!isString(table.rule)) fail("rule", "missing");

  const state = table.state;
  if (!isString(state)) fail("state", "missing");
  else if (!states.includes(state)) fail("state", `"${state}" is not one of: ${states.join(", ")}`);

  if (table.lessons !== undefined) {
    if (!Array.isArray(table.lessons) || table.lessons.some((l) => typeof l !== "string")) fail("lessons", "must be an array of paths");
    else for (const path of table.lessons as string[]) if (!existsSync(join(root, path))) fail("lessons", `${path} does not exist`);
  }

  const negatives: Negative[] = [];
  const raw = table.negative;
  if (raw !== undefined && (!Array.isArray(raw) || raw.some((n) => typeof n !== "object"))) fail("negative", "must be [[negative]] tables");
  else
    for (const [i, n] of ((raw ?? []) as TomlTable[]).entries()) {
      const field = `negative ${i + 1}`;
      for (const key of Object.keys(n)) if (!negativeKeys.includes(key)) fail(field, `unknown field "${key}"`);
      if (!isString(n.apply)) fail(field, "apply is missing");
      else if (!existsSync(join(root, n.apply))) fail(field, `the patch ${n.apply} does not exist`);
      if (!isString(n.run)) fail(field, "run is missing");
      if (!isString(n.expect)) fail(field, "expect is missing");
      if (isString(n.apply) && isString(n.run) && isString(n.expect)) negatives.push({ apply: n.apply, run: n.run, expect: n.expect });
    }

  if (state === "enforced") {
    if (!isString(table.enforced_by)) fail("enforced_by", "an enforced family names the command that holds its rule");
    if (raw === undefined || (Array.isArray(raw) && raw.length === 0))
      fail("negative", "an enforced family has at least one negative example");
  }
  if (state === "judgment" && !isString(table.judgment)) fail("judgment", "a judgment family says why no check can hold its rule");

  return { id, file, state: isString(state) ? state : "", negatives };
}

function git(args: string[]) {
  return spawnSync("git", args, { cwd: root, encoding: "utf8", maxBuffer: MAX_OUTPUT_BYTES });
}

// A negative command that runs longer than this is stopped, so that the tree
// is restored.
const RUN_LIMIT_MS = 30 * 60 * 1000;

// The files that a patch changes or adds.
function patchPaths(patch: string): string[] {
  const r = git(["apply", "--numstat", "-z", patch]);
  return r.status === 0 ? r.stdout.split("\0").flatMap((entry) => entry.split("\t").slice(2)).filter((p) => p !== "") : [];
}

// Put the files of the patch back to HEAD: undo edits and remove added files.
// Other files stay as they are, so a change that another process makes during
// the run is not lost.
function restore(paths: string[]) {
  if (paths.length === 0) return;
  git(["reset", "-q", "--", ...paths]);
  for (const path of paths) {
    if (git(["cat-file", "-e", `HEAD:${path}`]).status === 0) git(["checkout", "-q", "HEAD", "--", path]);
    else rmSync(join(root, path), { force: true });
  }
}

function lastLines(text: string, count: number): string {
  return text.trimEnd().split("\n").slice(-count).join("\n");
}

// Apply one negative example, run its command, and say why it failed (or undefined when it passed).
function runNegative(n: Negative): string | undefined {
  const paths = patchPaths(join(root, n.apply));
  try {
    // No --3way: CI has no history, so a 3-way merge cannot run there, and a
    // patch that applies only through it would pass locally and fail in CI.
    const applied = git(["apply", join(root, n.apply)]);
    if (applied.status !== 0) return `the patch ${n.apply} does not apply; regenerate the negative example\n${lastLines(applied.stderr, 10)}`;
    const r = spawnSync("bash", ["-c", n.run], { cwd: root, encoding: "utf8", maxBuffer: MAX_OUTPUT_BYTES, timeout: RUN_LIMIT_MS });
    const output = `${r.stdout ?? ""}${r.stderr ?? ""}`;
    if (r.error || r.signal) return `\`${n.run}\` did not finish (${r.error?.message ?? r.signal}); the tree was restored`;
    if (r.status === 0) return `\`${n.run}\` passed with ${n.apply} applied: the check no longer catches its mistake`;
    if (!output.includes(n.expect))
      return `\`${n.run}\` failed, but its output does not contain the expected text\n  expected: ${n.expect}\n  output (last lines):\n${lastLines(output, 40)}`;
    return undefined;
  } finally {
    restore(paths);
  }
}

// The working tree changes, or undefined when the tree is clean.
function treeChanges(): string | undefined {
  const status = git(["status", "--porcelain", "--untracked-files=all"]);
  return status.status !== 0 || status.stdout.trim() !== "" ? status.stdout.trimEnd() : undefined;
}

function main(args: string[]): number {
  const validateOnly = args.includes("--validate");
  const familyArg = args.indexOf("--family");
  const only = familyArg >= 0 ? args[familyArg + 1] : undefined;
  if (familyArg >= 0 && !only) {
    console.error("--family needs a family id");
    return 2;
  }

  const files = existsSync(rulesDir) ? readdirSync(rulesDir).filter((f) => f.endsWith(".toml")).sort() : [];
  const errors: string[] = [];
  const families = files.map((f) => readFamily(f, errors)).filter((f): f is Family => f !== undefined);
  if (only && !families.some((f) => f.id === only)) errors.push(`--family: no family "${only}" in rules/`);
  if (errors.length > 0) {
    for (const e of errors) console.error(e);
    console.error(`${errors.length} ledger problem(s).`);
    return 1;
  }
  if (validateOnly) {
    console.log(`${families.length} families, no ledger problems.`);
    return 0;
  }

  const selected = families.filter((f) => (only ? f.id === only : true) && f.negatives.length > 0);
  if (selected.length > 0) {
    const changes = treeChanges();
    if (changes !== undefined) {
      console.error("The working tree has changes. Commit or remove them first: the negative examples run on the committed tree, and the script restores the tree after each one.");
      console.error(changes);
      return 1;
    }
  }

  // On an interrupt, finish the current negative example (its command also
  // gets the signal), restore the tree, and stop.
  let stopped = false;
  const stop = () => {
    stopped = true;
  };
  process.on("SIGINT", stop);
  process.on("SIGTERM", stop);

  let failed = 0;
  for (const family of selected) {
    const reasons: string[] = [];
    for (const n of family.negatives) {
      const reason = runNegative(n);
      if (reason !== undefined) reasons.push(reason);
      const left = treeChanges();
      if (left !== undefined) {
        console.error(`fail ${family.id}: \`${n.run}\` left changes outside its patch. Remove them, then run the script again.\n${left}`);
        return 1;
      }
      if (stopped) {
        console.error("Stopped. The tree is restored.");
        return 130;
      }
    }
    if (reasons.length === 0) console.log(`pass ${family.id}`);
    else {
      failed++;
      for (const reason of reasons) console.error(`fail ${family.id}: ${reason}`);
    }
  }
  console.log(`${families.length} families, ${selected.length} with negative examples run, ${failed} failed.`);
  return failed > 0 ? 1 : 0;
}

process.exitCode = main(process.argv.slice(2));
