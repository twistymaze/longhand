// Reads Cargo.lock and bun.lock, and finds the package versions that a
// change adds. The dependency checks in the `cargo deny` CI job use it
// (check-dependency-age.ts, check-new-advisories.ts).

export const CRATES_IO = new Set([
  "registry+https://github.com/rust-lang/crates.io-index",
  "sparse+https://index.crates.io/",
]);

/// One package version in a lockfile. `registry` is false for a crate or a
/// package that does not come from crates.io or the npm registry.
export type LockedVersion = { ecosystem: "crate" | "package"; name: string; version: string; registry: boolean; source: string };

/// The parsed bun.lock. bun.lock is JSON with trailing commas.
export function parseBunLock(text: string): any {
  return JSON.parse(text.replace(/,(\s*[}\]])/g, "$1"));
}

/// The name and version of a bun.lock package spec `name@version`.
export function splitSpec(spec: string): { name: string; version: string } {
  const at = spec.lastIndexOf("@");
  return { name: spec.slice(0, at), version: spec.slice(at + 1) };
}

/// The package versions in bun.lock, by name.
export function bunLockVersions(text: string): Map<string, string[]> {
  const versions = new Map<string, string[]>();
  for (const entry of Object.values(parseBunLock(text).packages ?? {})) {
    const { name, version } = splitSpec(String((entry as unknown[])[0]));
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
  const entries = Object.values(parseBunLock(text).packages ?? {}) as unknown[][];
  return entries.filter((entry) => !splitSpec(String(entry[0])).version.startsWith("workspace:")).map((entry) => {
    const spec = String(entry[0]);
    const { name, version } = splitSpec(spec);
    const registry = spec.lastIndexOf("@") > 0 && SEMVER.test(version) && entry[1] === "";
    return { ecosystem: "package", name, version, source: registry ? "" : String(entry[1] || spec), registry };
  });
}

/// The versions in the head lockfiles that the base lockfiles do not have.
/// A missing lockfile counts as empty.
export function addedVersions(base: { cargo: string; bun: string }, head: { cargo: string; bun: string }): LockedVersion[] {
  const key = (v: LockedVersion) => `${v.ecosystem} ${v.name} ${v.version} ${v.source}`;
  const parse = (lock: { cargo: string; bun: string }) => [
    ...(lock.cargo ? cargoVersions(lock.cargo) : []),
    ...(lock.bun ? bunVersions(lock.bun) : []),
  ];
  const known = new Set(parse(base).map(key));
  const added = new Map<string, LockedVersion>();
  for (const v of parse(head)) if (!known.has(key(v))) added.set(key(v), v);
  return [...added.values()];
}
