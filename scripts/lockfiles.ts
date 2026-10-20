// Reads Cargo.lock and package-lock.json, and finds the package versions
// that a change adds. The dependency checks in the `cargo deny` CI job use
// it (check-dependency-age.ts, check-new-advisories.ts).

import { readFileSync } from "node:fs";
import { parseToml } from "./lib/toml.ts";

const CRATES_IO = new Set([
  "registry+https://github.com/rust-lang/crates.io-index",
  "sparse+https://index.crates.io/",
]);

const NPM_REGISTRY = "https://registry.npmjs.org/";

export type Ecosystem = "crate" | "package";

/// The text of Cargo.lock and package-lock.json. A missing lockfile is "".
export type Lockfiles = { cargo: string; npm: string };

/// One package version in a lockfile. `registry` is false for a crate or a
/// package that does not come from crates.io or the npm registry.
export type LockedVersion = { ecosystem: Ecosystem; name: string; version: string; registry: boolean; source: string };

function cargoVersions(text: string): LockedVersion[] {
  const lock = parseToml(text) as unknown as { package?: { name: string; version: string; source?: string }[] };
  return (lock.package ?? []).map((c) => ({
    ecosystem: "crate",
    name: c.name,
    version: c.version,
    source: c.source ?? "",
    registry: CRATES_IO.has(c.source ?? ""),
  }));
}

/// One entry of the `packages` object of package-lock.json.
export type PackageLockEntry = { name?: string; version?: string; resolved?: string; integrity?: string; link?: boolean };

/// The `packages` object of package-lock.json (lockfileVersion 3).
export function packageLockEntries(text: string): Record<string, PackageLockEntry> {
  const lock = JSON.parse(text) as { packages?: Record<string, PackageLockEntry> };
  if (typeof lock !== "object" || lock === null || typeof lock.packages !== "object" || lock.packages === null) {
    throw new Error("package-lock.json has no packages object");
  }
  return lock.packages;
}

/// The package name of a package-lock.json key: the part after the last
/// `node_modules/`. A key with no `node_modules/` is a folder in this
/// repository, so the key itself is its name.
export function packageLockName(key: string): string {
  const marker = "node_modules/";
  const at = key.lastIndexOf(marker);
  return at < 0 ? key : key.slice(at + marker.length);
}

/// The npm registry tarball URL of a package version.
/// `@scope/name` 1.2.3 gives https://registry.npmjs.org/@scope/name/-/name-1.2.3.tgz.
export function registryTarball(name: string, version: string): string {
  const basename = name.slice(name.lastIndexOf("/") + 1);
  return `${NPM_REGISTRY}${name}/-/${basename}-${version}.tgz`;
}

// An entry is a registry entry only when its `resolved` URL is the
// registry tarball of the name and version that the age check looks up, it
// has an `integrity` value, and its `name` (when present) is the name in
// its key. An entry with a different `name` is an alias for another
// package. A `link` entry points to a folder, so it is not a registry
// entry. The root entry "" is this repository, so it is not a dependency.
function npmVersions(text: string): LockedVersion[] {
  return Object.entries(packageLockEntries(text)).flatMap(([key, entry]): LockedVersion[] => {
    if (key === "") return [];
    const name = packageLockName(key);
    const version = String(entry.version ?? "missing");
    const registry =
      key.includes("node_modules/") &&
      entry.link !== true &&
      typeof entry.version === "string" &&
      entry.resolved === registryTarball(name, entry.version) &&
      typeof entry.integrity === "string" &&
      entry.integrity !== "" &&
      (entry.name === undefined || entry.name === name);
    return [{ ecosystem: "package", name, version, source: registry ? "" : String(entry.resolved ?? "missing"), registry }];
  });
}

/// The versions in the head lockfiles that the base lockfiles do not have.
/// A missing lockfile counts as empty.
export function addedVersions(base: Lockfiles, head: Lockfiles): LockedVersion[] {
  const key = (v: LockedVersion) => `${v.ecosystem} ${v.name} ${v.version} ${v.source}`;
  const parse = (lock: Lockfiles) => [
    ...(lock.cargo ? cargoVersions(lock.cargo) : []),
    ...(lock.npm ? npmVersions(lock.npm) : []),
  ];
  const known = new Set(parse(base).map(key));
  const added = new Map<string, LockedVersion>();
  for (const v of parse(head)) if (!known.has(key(v))) added.set(key(v), v);
  return [...added.values()];
}

/// The text of the file at `path`, or "" when the file does not exist.
export function readIfPresent(path: string): string {
  try {
    return readFileSync(path, "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return "";
    throw error;
  }
}
