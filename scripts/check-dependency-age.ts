// Fails a PR that adds a dependency version that is less than 7 days old
// (docs/development-workflow.md). The `cargo deny` CI job runs it on each PR.
//
//   node scripts/check-dependency-age.ts <base-dir> <head-dir>
//
// It compares Cargo.lock and package-lock.json of the base and the head.
//
// For each version that the head adds, it reads the publish date: a crate
// from the crates.io sparse index (`pubtime`), a JavaScript package from
// the npm registry (`time`). A version that `min-age-exceptions.toml` of
// the head lists, with a reason, passes. A package-lock.json entry that
// does not come from the npm registry fails unless the exception file
// lists it. An entry comes from the npm registry only when its `resolved`
// URL is the registry.npmjs.org tarball of its name and version, it has an
// `integrity` value, and it is not an alias for another package. A crate
// from git or a path is not checked: deny.toml (`unknown-git = "deny"`)
// allows only the git sources that it names.

import { join } from "node:path";
import { addedVersions, readIfPresent, type Ecosystem, type LockedVersion, type Lockfiles } from "./lockfiles.ts";
import { parseToml } from "./lib/toml.ts";

export const MINIMUM_AGE_DAYS = 7;
const DAY = 86_400_000;
const USER_AGENT = "longhand-dependency-age-check (https://github.com/twistymaze/longhand)";

export type Exception = { ecosystem: Ecosystem; name: string; version: string; reason: string };
export type PublishDate = (version: LockedVersion) => Promise<Date>;

/// The entries of min-age-exceptions.toml. Each entry needs ecosystem,
/// name, version and reason.
export function parseExceptions(text: string): Exception[] {
  const entries = (parseToml(text) as unknown as { exception?: Partial<Exception>[] }).exception ?? [];
  return entries.map((e, i) => {
    for (const field of ["ecosystem", "name", "version", "reason"] as const) {
      if (typeof e[field] !== "string" || e[field]!.trim() === "") {
        throw new Error(`min-age-exceptions.toml: entry ${i + 1} has no ${field}`);
      }
    }
    if (e.ecosystem !== "crate" && e.ecosystem !== "package") {
      throw new Error(`min-age-exceptions.toml: entry ${i + 1} has ecosystem "${e.ecosystem}"; use "crate" or "package"`);
    }
    return e as Exception;
  });
}

const describe = (v: LockedVersion) => `${v.ecosystem} ${v.name} ${v.version}`;

/// The problems with the added versions. A version that is not checked
/// (a crate from git or a path) goes into `notes`.
export async function checkAges(
  added: LockedVersion[],
  exceptions: Exception[],
  now: Date,
  publishDate: PublishDate,
  notes: string[] = [],
): Promise<string[]> {
  const excepted = (v: LockedVersion) =>
    exceptions.some((e) => e.ecosystem === v.ecosystem && e.name === v.name && e.version === v.version);
  const problems: string[] = [];
  for (const v of added) {
    if (excepted(v)) continue;
    if (!v.registry) {
      if (v.ecosystem === "crate") notes.push(`not checked: ${describe(v)} (source ${v.source || "path"})`);
      else problems.push(`not from the npm registry: ${describe(v)} (source ${v.source}). Add it to min-age-exceptions.toml with a reason, or use a registry version.`);
      continue;
    }
    let published: Date;
    try {
      published = await publishDate(v);
    } catch (error) {
      problems.push(`publish date unknown: ${describe(v)} (${(error as Error).message}). This is a registry fault: run the check again.`);
      continue;
    }
    if (now.getTime() - published.getTime() < MINIMUM_AGE_DAYS * DAY) {
      problems.push(
        `less than ${MINIMUM_AGE_DAYS} days old: ${describe(v)}, published ${published.toISOString().slice(0, 10)}. ` +
          "Wait, or add it to min-age-exceptions.toml with a reason.",
      );
    }
  }
  return problems;
}

// The path of a crate in the sparse index (https://doc.rust-lang.org/cargo/reference/registry-index.html).
function indexPath(name: string): string {
  const n = name.toLowerCase();
  if (n.length <= 2) return `${n.length}/${n}`;
  if (n.length === 3) return `3/${n[0]}/${n}`;
  return `${n.slice(0, 2)}/${n.slice(2, 4)}/${n}`;
}

async function fetchText(url: string): Promise<string> {
  const response = await fetch(url, { headers: { "User-Agent": USER_AGENT }, signal: AbortSignal.timeout(60_000) });
  if (!response.ok) throw new Error(`HTTP ${response.status} from ${url}`);
  return response.text();
}

/// Publish dates from crates.io and the npm registry, one request per name.
export function registryPublishDate(): PublishDate {
  const pages = new Map<string, Promise<string>>();
  const page = (url: string) => {
    if (!pages.has(url)) pages.set(url, fetchText(url));
    return pages.get(url)!;
  };
  return async (v) => {
    let date: string | undefined;
    if (v.ecosystem === "crate") {
      for (const line of (await page(`https://index.crates.io/${indexPath(v.name)}`)).split("\n")) {
        if (line.trim() === "") continue;
        const entry = JSON.parse(line) as { vers: string; pubtime?: string };
        if (entry.vers === v.version) date = entry.pubtime;
      }
    } else {
      const doc = JSON.parse(await page(`https://registry.npmjs.org/${v.name.replaceAll("/", "%2f")}`)) as { time?: Record<string, string> };
      date = doc.time?.[v.version];
    }
    if (!date) throw new Error("the registry gives no publish date for this version");
    return new Date(date);
  };
}

if (import.meta.main) {
  const [baseDir, headDir] = process.argv.slice(2);
  if (!baseDir || !headDir) {
    console.error("usage: check-dependency-age.ts <base-dir> <head-dir>");
    process.exit(2);
  }
  const lockfiles = (dir: string): Lockfiles => ({
    cargo: readIfPresent(join(dir, "Cargo.lock")),
    npm: readIfPresent(join(dir, "package-lock.json")),
  });
  const base = lockfiles(baseDir);
  const head = lockfiles(headDir);
  let exceptions: Exception[];
  try {
    exceptions = parseExceptions(readIfPresent(join(headDir, "min-age-exceptions.toml")));
  } catch (error) {
    console.error((error as Error).message);
    process.exit(1);
  }
  const added = addedVersions(base, head);
  const notes: string[] = [];
  const problems = await checkAges(added, exceptions, new Date(), registryPublishDate(), notes);
  for (const note of notes) console.log(note);
  if (problems.length > 0) {
    console.log(`The head adds dependency versions that fail the ${MINIMUM_AGE_DAYS}-day rule:\n${problems.map((p) => `- ${p}`).join("\n")}`);
    process.exit(1);
  }
  console.log(`Checked ${added.length} added dependency versions: each is at least ${MINIMUM_AGE_DAYS} days old or listed as an exception.`);
}
