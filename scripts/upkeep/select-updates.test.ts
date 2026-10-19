import { afterEach, expect, test } from "bun:test";
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  heldReason,
  loadHeld,
  selectUpdates,
  systemBunTools,
  type BunTools,
  type CargoMetadata,
  type Published,
  type Tools,
} from "./select-updates";

// The tests give the script fake data in place of cargo, cargo-deny, bun and
// the registries, so no test reaches the network. The tests use the real
// held.json. One test runs the real bun steps with a fake `bun` executable in
// a temporary directory.

const NOW = new Date("2026-10-18T12:00:00Z");
const DAY = 24 * 60 * 60 * 1000;
const HELD = loadHeld();
const dirs: string[] = [];

afterEach(() => {
  for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
});

/// A version that the registry published `days` days before NOW.
function published(version: string, days: number, yanked = false): Published {
  return { version, published: new Date(NOW.getTime() - days * DAY), yanked };
}

/// Cargo metadata for a dependency graph. Each key is `name@version`, and the
/// value lists the `name@version` of its dependencies. `members` are the
/// workspace crates. `reqs` gives the requirement of a member's dependency
/// by name (the default is `^<version>`).
function metadata(graph: Record<string, string[]>, members: string[], reqs: Record<string, string> = {}): CargoMetadata {
  const id = (key: string) => (members.includes(key) ? `path+file:///repo#${key}` : `registry+https://github.com/rust-lang/crates.io-index#${key}`);
  const split = (key: string) => {
    const at = key.lastIndexOf("@");
    return { name: key.slice(0, at), version: key.slice(at + 1) };
  };
  return {
    workspace_members: members.map(id),
    packages: Object.entries(graph).map(([key, deps]) => ({
      id: id(key),
      ...split(key),
      dependencies: deps.map((dep) => ({ name: split(dep).name, req: reqs[split(dep).name] ?? `^${split(dep).version}` })),
    })),
    resolve: { nodes: Object.entries(graph).map(([key, deps]) => ({ id: id(key), deps: deps.map((dep) => ({ pkg: id(dep) })) })) },
  };
}

/// A cargo-deny diagnostic line for an advisory on `name@version`.
function advisory(id: string, name: string, version: string, solution: string | null): string {
  const notes = [`ID: ${id}`, solution ? `Solution: Upgrade to ${solution} (try \`cargo update -p ${name}\`)` : "Solution: No safe upgrade is available!"];
  return JSON.stringify({
    type: "diagnostic",
    fields: { code: "vulnerability", advisory: { id, package: name }, graphs: [{ Krate: { name, version } }], notes },
  });
}

const NO_BUN: BunTools = {
  lock: () => `{ "lockfileVersion": 1, "packages": {} }`,
  audit: () => ({}),
  update: () => ({ lock: `{ "lockfileVersion": 1, "packages": {} }`, audit: {} }),
};

type Fake = {
  graph: Record<string, string[]>;
  members?: string[];
  reqs?: Record<string, string>;
  /// The `cargo update --dry-run` output with no package.
  updates: string[];
  /// The `cargo update --dry-run` output for `-p <name@version>`.
  moves?: Record<string, string[]>;
  /// The `-p <name@version>` dry runs that fail.
  fails?: string[];
  /// The crates.io versions. A missing crate gives an API error.
  crates: Record<string, Published[]>;
  advisories?: string[];
  packageJson?: { dependencies?: Record<string, string>; devDependencies?: Record<string, string> };
  bun?: BunTools;
  npm?: Record<string, Published[]>;
};

function tools(fake: Fake): Tools {
  const members = fake.members ?? ["longhand@0.1.0"];
  return {
    now: () => NOW,
    cargoMetadata: () => metadata(fake.graph, members, fake.reqs),
    cargoDryRun: (args) => {
      const package_ = args[args.indexOf("-p") + 1];
      const lines = args.includes("-p") ? (fake.moves?.[package_] ?? []) : fake.updates;
      if (args.includes("-p") && fake.fails?.includes(package_)) return { ok: false, output: "error: failed to select a version" };
      return { ok: true, output: lines.join("\n") };
    },
    cargoDenyAdvisories: () => (fake.advisories ?? []).join("\n"),
    crateVersions: async (name) => {
      const versions = fake.crates[name];
      if (!versions) throw new Error(`crates.io returned 500 for ${name}`);
      return versions;
    },
    packageJson: () => fake.packageJson ?? {},
    bun: fake.bun ?? NO_BUN,
    npmVersions: async (name) => {
      const versions = fake.npm?.[name];
      if (!versions) throw new Error(`npm returned 500 for ${name}`);
      return versions;
    },
  };
}

test("a crate with a 3-day and a 10-day patch release gets the 10-day version", async () => {
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": ["serde@1.0.1"], "serde@1.0.1": [] },
      updates: ["    Updating serde v1.0.1 -> v1.0.3"],
      crates: { serde: [published("1.0.1", 60), published("1.0.2", 10), published("1.0.3", 3)] },
    }),
    HELD,
  );
  expect(result.apply).toEqual([
    { ecosystem: "crate", name: "serde", from: "1.0.1", to: "1.0.2", command: ["cargo", "update", "-p", "serde@1.0.1", "--precise", "1.0.2"] },
  ]);
  expect(result.exceptions).toEqual([]);
});

test("a crate whose only newer version is yanked gets no update", async () => {
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": ["serde@1.0.1"], "serde@1.0.1": [] },
      updates: ["    Updating serde v1.0.1 -> v1.0.2"],
      crates: { serde: [published("1.0.1", 60), published("1.0.2", 10, true)] },
    }),
    HELD,
  );
  expect(result.apply).toEqual([]);
  expect(result.held).toEqual([]);
});

test("a crate that cpal and a Rig crate use is not held, and a crate that only cpal uses is held", async () => {
  const result = await selectUpdates(
    tools({
      graph: {
        "longhand@0.1.0": ["cpal@0.15.2", "rig-core@0.44.0"],
        "cpal@0.15.2": ["rustls@0.23.45", "coreaudio-sys@0.2.16"],
        "rig-core@0.44.0": ["rustls@0.23.45"],
        "rustls@0.23.45": [],
        "coreaudio-sys@0.2.16": [],
      },
      updates: ["    Updating coreaudio-sys v0.2.16 -> v0.2.17", "    Updating rustls v0.23.45 -> v0.23.47"],
      crates: { rustls: [published("0.23.45", 90), published("0.23.47", 30)] },
    }),
    HELD,
  );
  expect(result.apply.map((update) => `${update.name}@${update.to}`)).toEqual(["rustls@0.23.47"]);
  expect(result.held).toEqual([{ ecosystem: "crate", name: "coreaudio-sys", version: "0.2.16", reason: heldReason(HELD.crates, "cpal")!, via: "cpal" }]);
});

test("cpal and a crate that only cpal uses are both held with the cpal reason", async () => {
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": ["cpal@0.15.2"], "cpal@0.15.2": ["coreaudio-sys@0.2.16"], "coreaudio-sys@0.2.16": [] },
      updates: ["    Updating coreaudio-sys v0.2.16 -> v0.2.17", "    Updating cpal v0.15.2 -> v0.15.3"],
      crates: {},
    }),
    HELD,
  );
  const cpalReason = heldReason(HELD.crates, "cpal");
  expect(cpalReason).not.toBeNull();
  expect(result.apply).toEqual([]);
  expect(result.held).toEqual([
    { ecosystem: "crate", name: "coreaudio-sys", version: "0.2.16", reason: cpalReason!, via: "cpal" },
    { ecosystem: "crate", name: "cpal", version: "0.15.2", reason: cpalReason! },
  ]);
});

test("a crate that cpal and a crate that is not held both use is not held", async () => {
  const result = await selectUpdates(
    tools({
      graph: {
        "longhand@0.1.0": ["cpal@0.15.2", "serde@1.0.1"],
        "cpal@0.15.2": ["log@0.4.20"],
        "serde@1.0.1": ["log@0.4.20"],
        "log@0.4.20": [],
      },
      updates: ["    Updating log v0.4.20 -> v0.4.22"],
      crates: { log: [published("0.4.20", 90), published("0.4.22", 20)] },
    }),
    HELD,
  );
  expect(result.apply.map((update) => `${update.name}@${update.to}`)).toEqual(["log@0.4.22"]);
  expect(result.held).toEqual([]);
});

test("an = pin in a Cargo.toml and an exact version in package.json are held as exact pins", async () => {
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": ["specta@2.0.0-rc.22"], "specta@2.0.0-rc.22": [] },
      reqs: { specta: "=2.0.0-rc.22" },
      updates: [],
      crates: {},
      packageJson: { dependencies: { "prosemirror-view": "1.42.3", "markdown-it": "^14.2.0" }, devDependencies: { typescript: "~5.6.3" } },
    }),
    HELD,
  );
  expect(result.held).toEqual([
    { ecosystem: "crate", name: "specta", version: "2.0.0-rc.22", reason: "exact pin in a Cargo.toml" },
    { ecosystem: "package", name: "prosemirror-view", version: "1.42.3", reason: "exact version in package.json" },
  ]);
});

test("rig-core is held with the Rig reason", async () => {
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": ["rig-core@0.44.0"], "rig-core@0.44.0": [] },
      updates: ["    Updating rig-core v0.44.0 -> v0.44.1"],
      crates: {},
    }),
    HELD,
  );
  expect(result.apply).toEqual([]);
  expect(result.held).toEqual([{ ecosystem: "crate", name: "rig-core", version: "0.44.0", reason: expect.stringContaining("Rig") }]);
});

test("an advisory-fixing crate version 2 days old with no older fix is applied as an exception", async () => {
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": ["rustls@0.23.45"], "rustls@0.23.45": [] },
      updates: [],
      crates: { rustls: [published("0.23.44", 40), published("0.23.45", 20), published("0.23.46", 2)] },
      advisories: [advisory("RUSTSEC-2026-0346", "rustls", "0.23.45", ">=0.23.46")],
    }),
    HELD,
  );
  expect(result.apply).toEqual([
    {
      ecosystem: "crate",
      name: "rustls",
      from: "0.23.45",
      to: "0.23.46",
      command: ["cargo", "update", "-p", "rustls@0.23.45", "--precise", "0.23.46"],
      advisories: ["RUSTSEC-2026-0346"],
    },
  ]);
  expect(result.exceptions).toEqual([
    { ecosystem: "crate", name: "rustls", version: "0.23.46", published: published("0.23.46", 2).published.toISOString(), advisories: ["RUSTSEC-2026-0346"] },
  ]);
});

test("with fixing versions 10 days and 1 day old, the 10-day version is applied with no exception", async () => {
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": ["rustls@0.23.45"], "rustls@0.23.45": [] },
      updates: ["    Updating rustls v0.23.45 -> v0.23.47"],
      crates: { rustls: [published("0.23.45", 20), published("0.23.46", 10), published("0.23.47", 1)] },
      advisories: [advisory("RUSTSEC-2026-0346", "rustls", "0.23.45", ">=0.23.46"), advisory("RUSTSEC-2026-0347", "rustls", "0.23.45", ">=0.23.46")],
    }),
    HELD,
  );
  expect(result.apply.map((update) => [update.name, update.to, update.advisories])).toEqual([["rustls", "0.23.46", ["RUSTSEC-2026-0346", "RUSTSEC-2026-0347"]]]);
  expect(result.exceptions).toEqual([]);
});

test("an advisory that only a held crate or a major version fixes is held with the advisory", async () => {
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": ["wasapi@0.23.0", "zip@4.6.1"], "wasapi@0.23.0": [], "zip@4.6.1": [] },
      updates: [],
      crates: { zip: [published("4.6.1", 90), published("9.0.0", 30)] },
      advisories: [advisory("RUSTSEC-2026-0332", "wasapi", "0.23.0", ">=0.25.0"), advisory("RUSTSEC-2026-0367", "zip", "4.6.1", ">=9.0.0")],
    }),
    HELD,
  );
  expect(result.apply).toEqual([]);
  expect(result.held).toEqual([
    { ecosystem: "crate", name: "wasapi", version: "0.23.0", reason: `advisory: the fix needs a held update. ${heldReason(HELD.crates, "wasapi")}`, advisories: ["RUSTSEC-2026-0332"] },
    { ecosystem: "crate", name: "zip", version: "4.6.1", reason: "advisory: no fixing version within the current range", advisories: ["RUSTSEC-2026-0367"] },
  ]);
});

test("an update that makes the resolver move cpal is held", async () => {
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": ["cpal@0.15.2", "foo@1.0.0"], "cpal@0.15.2": [], "foo@1.0.0": [] },
      updates: ["    Updating foo v1.0.0 -> v1.1.0"],
      moves: { "foo@1.0.0": ["    Updating cpal v0.15.2 -> v0.15.3", "    Updating foo v1.0.0 -> v1.1.0"] },
      crates: { foo: [published("1.0.0", 90), published("1.1.0", 30)] },
    }),
    HELD,
  );
  expect(result.apply).toEqual([]);
  expect(result.held).toEqual([{ ecosystem: "crate", name: "foo", version: "1.0.0", to: "1.1.0", reason: "moves a held dependency", moves: ["cpal"] }]);
});

test("a crate that cargo cannot update alone is not held when an applied update moves it", async () => {
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": ["thiserror@2.0.20"], "thiserror@2.0.20": ["thiserror-impl@2.0.20"], "thiserror-impl@2.0.20": [] },
      updates: ["    Updating thiserror v2.0.20 -> v2.0.21", "    Updating thiserror-impl v2.0.20 -> v2.0.21"],
      moves: { "thiserror@2.0.20": ["    Updating thiserror v2.0.20 -> v2.0.21", "    Updating thiserror-impl v2.0.20 -> v2.0.21"] },
      fails: ["thiserror-impl@2.0.20"],
      crates: { thiserror: [published("2.0.20", 90), published("2.0.21", 30)], "thiserror-impl": [published("2.0.20", 90), published("2.0.21", 30)] },
    }),
    HELD,
  );
  expect(result.apply.map((update) => update.name)).toEqual(["thiserror"]);
  expect(result.held).toEqual([]);
});

test("an update that moves another crate to a version 2 days old is held", async () => {
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": ["foo@1.0.0", "bar@1.0.0"], "foo@1.0.0": ["bar@1.0.0"], "bar@1.0.0": [] },
      updates: ["    Updating foo v1.0.0 -> v1.1.0"],
      moves: { "foo@1.0.0": ["    Updating bar v1.0.0 -> v1.0.5", "    Updating foo v1.0.0 -> v1.1.0"] },
      crates: { foo: [published("1.0.0", 90), published("1.1.0", 30)], bar: [published("1.0.0", 90), published("1.0.5", 2)] },
    }),
    HELD,
  );
  expect(result.apply).toEqual([]);
  expect(result.held).toEqual([
    { ecosystem: "crate", name: "foo", version: "1.0.0", to: "1.1.0", reason: "brings bar 1.0.5, which is less than 7 days old" },
  ]);
});

test("an update that adds a crate version 2 days old is held", async () => {
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": ["foo@1.0.0"], "foo@1.0.0": [] },
      updates: ["    Updating foo v1.0.0 -> v1.1.0"],
      moves: { "foo@1.0.0": ["      Adding baz v0.3.1", "    Updating foo v1.0.0 -> v1.1.0"] },
      crates: { foo: [published("1.0.0", 90), published("1.1.0", 30)], baz: [published("0.3.1", 2)] },
    }),
    HELD,
  );
  expect(result.apply).toEqual([]);
  expect(result.held).toEqual([
    { ecosystem: "crate", name: "foo", version: "1.0.0", to: "1.1.0", reason: "brings baz 0.3.1, which is less than 7 days old" },
  ]);
});

test("an update that moves another crate to a version 10 days old is applied", async () => {
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": ["foo@1.0.0", "bar@1.0.0"], "foo@1.0.0": ["bar@1.0.0"], "bar@1.0.0": [] },
      updates: ["    Updating foo v1.0.0 -> v1.1.0"],
      moves: { "foo@1.0.0": ["    Updating bar v1.0.0 -> v1.0.5", "      Adding baz v0.3.1", "    Updating foo v1.0.0 -> v1.1.0"] },
      crates: {
        foo: [published("1.0.0", 90), published("1.1.0", 30)],
        bar: [published("1.0.0", 90), published("1.0.5", 10)],
        baz: [published("0.3.1", 10)],
      },
    }),
    HELD,
  );
  expect(result.apply.map((update) => `${update.name}@${update.to}`)).toEqual(["foo@1.1.0"]);
  expect(result.held).toEqual([]);
});

test("an advisory exception that moves another crate to a young version is applied", async () => {
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": ["rustls@0.23.45"], "rustls@0.23.45": ["ring@0.17.8"], "ring@0.17.8": [] },
      updates: [],
      moves: { "rustls@0.23.45": ["    Updating ring v0.17.8 -> v0.17.9", "    Updating rustls v0.23.45 -> v0.23.46"] },
      crates: { rustls: [published("0.23.45", 20), published("0.23.46", 2)], ring: [published("0.17.8", 90), published("0.17.9", 2)] },
      advisories: [advisory("RUSTSEC-2026-0346", "rustls", "0.23.45", ">=0.23.46")],
    }),
    HELD,
  );
  expect(result.apply.map((update) => `${update.name}@${update.to}`)).toEqual(["rustls@0.23.46"]);
  expect(result.exceptions.map((entry) => entry.name)).toEqual(["rustls"]);
});

test("a crates.io API error holds that crate, and the other crates continue", async () => {
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": ["bar@1.0.0", "foo@1.0.0"], "bar@1.0.0": [], "foo@1.0.0": [] },
      updates: ["    Updating bar v1.0.0 -> v1.0.1", "    Updating foo v1.0.0 -> v1.0.1"],
      crates: { bar: [published("1.0.0", 90), published("1.0.1", 30)] },
    }),
    HELD,
  );
  expect(result.apply.map((update) => update.name)).toEqual(["bar"]);
  expect(result.held).toEqual([{ ecosystem: "crate", name: "foo", version: "1.0.0", reason: "publish date unknown" }]);
});

// A fake `bun` in a temporary repository. `update` with
// `--minimum-release-age` sets lodash to 4.17.21 in bun.lock, and `audit`
// reports the lodash advisory while bun.lock has 4.17.20.
function fakeBunRepository(): { repo: string; bun: string; log: string } {
  const repo = mkdtempSync(join(tmpdir(), "select-updates-"));
  dirs.push(repo);
  const log = join(repo, "bun-arguments.log");
  writeFileSync(join(repo, "package.json"), JSON.stringify({ dependencies: { express: "^4.21.0" } }));
  writeFileSync(
    join(repo, "bun.lock"),
    `{\n  "lockfileVersion": 1,\n  "packages": {\n    "express": ["express@4.21.0", "", { "dependencies": { "lodash": "^4.17.0" } }, "sha512-a"],\n    "lodash": ["lodash@4.17.20", "", {}, "sha512-b"],\n  }\n}\n`,
  );
  writeFileSync(join(repo, "bunfig.toml"), "[install]\nminimumReleaseAge = 604800\nminimumReleaseAgeExcludes = []\n");
  const bun = join(repo, "fake-bun");
  writeFileSync(
    bun,
    `#!/bin/sh
echo "$@" >> "${log}"
case "$1" in
  update)
    case "$*" in *--minimum-release-age=*)
      sed 's/lodash@4.17.20/lodash@4.17.21/' bun.lock > bun.lock.new && mv bun.lock.new bun.lock ;;
    esac ;;
  audit)
    if grep -q 'lodash@4.17.20' bun.lock; then
      echo '{"lodash":[{"id":1,"url":"https://example.test","title":"Prototype pollution","severity":"high","vulnerable_versions":"<4.17.21"}]}'
      exit 1
    fi
    echo '{}' ;;
esac
`,
  );
  chmodSync(bun, 0o755);
  return { repo, bun, log };
}

test("an advisory-fixing npm version 2 days old is applied with a one-time age setting, and bunfig.toml stays the same", async () => {
  const { repo, bun, log } = fakeBunRepository();
  const before = ["bunfig.toml", "bun.lock", "package.json"].map((file) => readFileSync(join(repo, file), "utf8"));
  const result = await selectUpdates(
    tools({
      graph: { "longhand@0.1.0": [] },
      updates: [],
      crates: {},
      packageJson: JSON.parse(readFileSync(join(repo, "package.json"), "utf8")),
      bun: systemBunTools(repo, bun),
      npm: { lodash: [published("4.17.20", 400), published("4.17.21", 2)] },
    }),
    HELD,
  );
  const age = Math.floor((2 * DAY) / 1000);
  const command = ["bun", "update", "--lockfile-only", "--ignore-scripts", `--minimum-release-age=${age}`, "lodash"];
  expect(result.apply).toEqual([{ ecosystem: "package", name: "lodash", from: "4.17.20", to: "4.17.21", command, advisories: ["1"] }]);
  expect(result.exceptions).toEqual([
    { ecosystem: "package", name: "lodash", version: "4.17.21", published: published("4.17.21", 2).published.toISOString(), advisories: ["1"] },
  ]);
  expect(readFileSync(log, "utf8")).toContain(`update --lockfile-only --ignore-scripts --minimum-release-age=${age} lodash`);
  expect(["bunfig.toml", "bun.lock", "package.json"].map((file) => readFileSync(join(repo, file), "utf8"))).toEqual(before);
});
