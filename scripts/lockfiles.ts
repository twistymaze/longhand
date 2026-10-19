// Reads Cargo.lock and bun.lock, and finds the package versions that a
// change adds. The dependency checks in the `cargo deny` CI job use it
// (check-dependency-age.ts, check-new-advisories.ts).

const CRATES_IO = new Set([
  "registry+https://github.com/rust-lang/crates.io-index",
  "sparse+https://index.crates.io/",
]);

export type Ecosystem = "crate" | "package";

/// The text of Cargo.lock and bun.lock. A missing lockfile is "".
export type Lockfiles = { cargo: string; bun: string };

/// One package version in a lockfile. `registry` is false for a crate or a
/// package that does not come from crates.io or the npm registry.
export type LockedVersion = { ecosystem: Ecosystem; name: string; version: string; registry: boolean; source: string };

/// The parsed bun.lock. bun.lock is JSON with trailing commas.
function parseBunLock(text: string): { packages?: Record<string, unknown[]> } {
  return JSON.parse(text.replace(/,(\s*[}\]])/g, "$1"));
}

/// The name and version of a bun.lock package spec `name@version`.
function splitSpec(spec: string): { name: string; version: string } {
  const at = spec.lastIndexOf("@");
  return { name: spec.slice(0, at), version: spec.slice(at + 1) };
}

/// The package versions in bun.lock, by name.
export function bunLockVersions(text: string): Map<string, string[]> {
  const versions = new Map<string, string[]>();
  for (const entry of Object.values(parseBunLock(text).packages ?? {})) {
    const { name, version } = splitSpec(String(entry[0]));
    versions.set(name, [...(versions.get(name) ?? []), version]);
  }
  return versions;
}

const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

function cargoVersions(text: string): LockedVersion[] {
  const lock = Bun.TOML.parse(text) as { package?: { name: string; version: string; source?: string }[] };
  return (lock.package ?? []).map((c) => ({
    ecosystem: "crate",
    name: c.name,
    version: c.version,
    source: c.source ?? "",
    registry: CRATES_IO.has(c.source ?? ""),
  }));
}

// A registry entry is ["name@version", "", {...}, "sha512-..."]. A git,
// GitHub, tarball or workspace entry has a source in the second element or
// a spec that is not `name@semver`. A workspace package is part of this
// repository, so it is not a dependency version.
function bunVersions(text: string): LockedVersion[] {
  return Object.values(parseBunLock(text).packages ?? {}).flatMap((entry): LockedVersion[] => {
    const spec = String(entry[0]);
    const { name, version } = splitSpec(spec);
    if (version.startsWith("workspace:")) return [];
    const registry = spec.lastIndexOf("@") > 0 && SEMVER.test(version) && entry[1] === "";
    return [{ ecosystem: "package", name, version, source: registry ? "" : String(entry[1] || spec), registry }];
  });
}

/// The versions in the head lockfiles that the base lockfiles do not have.
/// A missing lockfile counts as empty.
export function addedVersions(base: Lockfiles, head: Lockfiles): LockedVersion[] {
  const key = (v: LockedVersion) => `${v.ecosystem} ${v.name} ${v.version} ${v.source}`;
  const parse = (lock: Lockfiles) => [
    ...(lock.cargo ? cargoVersions(lock.cargo) : []),
    ...(lock.bun ? bunVersions(lock.bun) : []),
  ];
  const known = new Set(parse(base).map(key));
  const added = new Map<string, LockedVersion>();
  for (const v of parse(head)) if (!known.has(key(v))) added.set(key(v), v);
  return [...added.values()];
}
