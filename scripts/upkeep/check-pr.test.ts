import { describe, expect, test } from "bun:test";
import { type Registry, CHECK_END, CHECK_START, runCheckPr } from "./check-pr";
import type { CheckRun, Client, Kind, Request } from "./github";
import type { HeldList, Published } from "./select-updates";

// Each test gives the check a fake REST client and a fake registry, so no
// test reaches the network.

const NOW = new Date("2026-10-18T06:00:00Z");
const DAY = 24 * 60 * 60 * 1000;
const REQUIRED = ["Checks (macos-latest)", "Checks (windows-latest)", "cargo deny", "PR title"];
const REPO = "twistymaze/longhand";

const HELD: HeldList = {
  crates: [{ name: "cpal", reason: "Audio capture crate.", holdsDependencies: true }],
  packages: [{ name: "@tauri-apps/api", reason: "Paired with the Tauri core crates." }],
};

const green = (name: string): CheckRun => ({ name, status: "completed", conclusion: "success" });
const allGreen = (kind: Kind) => [...REQUIRED, ...(kind === "deps" ? ["New advisories on this PR"] : [])].map(green);

function cargoLock(packages: { name: string; version: string; source?: string | null; deps?: string[] }[]): string {
  const lines = ["version = 4", ""];
  for (const p of packages) {
    lines.push("[[package]]", `name = "${p.name}"`, `version = "${p.version}"`);
    if (p.source !== null) lines.push(`source = "${p.source ?? "registry+https://github.com/rust-lang/crates.io-index"}"`);
    if (p.deps) lines.push(`dependencies = [${p.deps.map((d) => `"${d}"`).join(", ")}]`);
    lines.push("");
  }
  return lines.join("\n");
}

const lockBase = (cpal = "0.15.0", serde = "1.0.0", coreaudio = "0.2.0") =>
  cargoLock([
    { name: "app", version: "0.1.0", source: null, deps: ["cpal", "serde"] },
    { name: "cpal", version: cpal, deps: ["coreaudio-sys"] },
    { name: "coreaudio-sys", version: coreaudio },
    { name: "serde", version: serde },
  ]);

function bunLock(packages: Record<string, string>, extra = ""): string {
  const entries = Object.entries(packages).map(
    ([name, spec]) => `    "${name}": ["${name}@${spec}", "", {}, "sha512-x"],`,
  );
  return `{\n  "lockfileVersion": 1,${extra}\n  "workspaces": {},\n  "packages": {\n${entries.join("\n")}\n  }\n}\n`;
}

function packageJson(deps: Record<string, string>, devDeps: Record<string, string> = {}): string {
  return JSON.stringify({ name: "longhand", private: true, dependencies: deps, devDependencies: devDeps }, null, 2);
}

const DENY_BASE = `[advisories]\nyanked = "deny"\nignore = [\n  { id = "RUSTSEC-2024-0429", reason = "glib, Linux only" },\n]\n\n[licenses]\nallow = ["MIT"]\n`;

const HANDY = "https://github.com/cjpais/Handy";
const ANARLOG = "https://github.com/fastrepl/anarlog";

function record(path: string, repository: string, revision: string): string {
  return `## \`${path}\`

- Repository: ${repository}
- Revision: ${revision}
- Source paths: \`src/x.rs\`
- Copyright: Copyright (c) 2025 Someone
- Licence: MIT, licences/sources/x.txt
- Status: modified
`;
}

const SUMMARIZE_PROVENANCE = `# Provenance\n\n${record("src/prompt.rs", ANARLOG, "1111111")}\n${record("src/parse.rs", HANDY, "2222222")}`;
const AUDIO_PROVENANCE = `# Provenance\n\n${record("src/config.rs", HANDY, "3333333")}`;

type FileChange = { base: string | null; head: string | null };

type Scenario = {
  kind: Kind;
  branch?: string;
  title?: string;
  runs?: CheckRun[];
  files: Record<string, FileChange>;
  /// Files that exist at the base and the head but do not change.
  unchanged?: Record<string, string>;
  labels?: string[];
  body?: string;
  /// The head SHA that a second read of the PR gives.
  headAfter?: string;
};

function fakeClient(s: Scenario): Client & { calls: Request[] } {
  const calls: Request[] = [];
  const branch = s.branch ?? { deps: "upkeep/deps", port: "upkeep/port-anarlog-abc1234", docs: "upkeep/review-log" }[s.kind];
  const title = s.title ?? { deps: "build(deps): update dependencies", port: "fix(upstream): port anarlog fix", docs: "docs(upkeep): move baselines" }[s.kind];
  let prReads = 0;
  const files = { ...s.files };
  for (const [path, text] of Object.entries(s.unchanged ?? {})) files[path] = { base: text, head: text };
  return {
    repo: REPO,
    calls,
    call(request) {
      calls.push(request);
      const method = request.method ?? "GET";
      const path = request.path;
      const fail = (status: number) => {
        throw Object.assign(new Error(`HTTP ${status}: ${method} ${path}`), { status });
      };
      if (method === "GET" && path === `repos/${REPO}/pulls/7`) {
        prReads++;
        const sha = prReads > 1 && s.headAfter ? s.headAfter : "head1";
        return {
          number: 7,
          title,
          body: s.body ?? "Weekly updates.",
          labels: (s.labels ?? []).map((name) => ({ name })),
          head: { ref: branch, sha, repo: { full_name: REPO } },
          base: { ref: "main", sha: "base1" },
        };
      }
      if (method === "GET" && path.startsWith(`repos/${REPO}/commits/head1/check-runs`)) {
        const runs = s.runs ?? allGreen(s.kind);
        return { total_count: runs.length, check_runs: runs };
      }
      if (method === "GET" && path.startsWith(`repos/${REPO}/pulls/7/files`)) {
        return Object.entries(s.files).map(([filename, change]) => ({
          filename,
          status: change.base === null ? "added" : change.head === null ? "removed" : "modified",
        }));
      }
      if (method === "GET" && path === `repos/${REPO}/compare/base1...head1`) return { merge_base_commit: { sha: "mb" } };
      const content = path.match(/^repos\/[^/]+\/[^/]+\/contents\/(.+)\?ref=(\w+)$/);
      if (method === "GET" && content) {
        const file = files[decodeURIComponent(content[1])];
        const text = file ? (content[2] === "mb" ? file.base : file.head) : null;
        if (text === null || text === undefined) fail(404);
        return text;
      }
      if (method === "POST" && path === `repos/${REPO}/issues/7/labels`) return [];
      if (method === "DELETE" && path === `repos/${REPO}/issues/7/labels/upkeep%3A%20ready`) {
        if (!(s.labels ?? []).includes("upkeep: ready")) fail(404);
        return [];
      }
      if (method === "PATCH" && path === `repos/${REPO}/pulls/7`) return {};
      return fail(404);
    },
  };
}

function registry(options: { crates?: Record<string, Published[]>; packages?: Record<string, Published[]>; advisories?: string[]; failing?: string[] } = {}): Registry {
  const lookup = (map: Record<string, Published[]> | undefined, name: string) => {
    if (options.failing?.includes(name)) return Promise.reject(new Error("HTTP 500"));
    return Promise.resolve(map?.[name] ?? []);
  };
  return {
    now: () => NOW,
    crateVersions: (name) => lookup(options.crates, name),
    npmVersions: (name) => lookup(options.packages, name),
    advisoryPackages: () => Promise.resolve(new Set(options.advisories ?? [])),
  };
}

const published = (version: string, daysOld: number, yanked = false): Published => ({
  version,
  published: new Date(NOW.getTime() - daysOld * DAY),
  yanked,
});

const OLD_ENOUGH = registry({
  crates: { serde: [published("1.0.0", 100), published("1.0.1", 10)], cpal: [published("0.15.1", 10)], "coreaudio-sys": [published("0.2.1", 10)] },
  packages: { react: [published("19.2.9", 10)], tlds: [published("1.262.0", 10)] },
});

async function run(s: Scenario, reg = OLD_ENOUGH) {
  const client = fakeClient(s);
  const result = await runCheckPr(client, 7, s.kind, reg, HELD);
  const labelAdded = client.calls.some((c) => c.method === "POST" && c.path.endsWith("/issues/7/labels"));
  const labelRemoved = client.calls.some((c) => c.method === "DELETE" && c.path.includes("/labels/upkeep%3A%20ready"));
  const body = (client.calls.find((c) => c.method === "PATCH")?.body as { body: string } | undefined)?.body ?? null;
  const merged = client.calls.some((c) => c.path.includes("/merge"));
  expect(merged).toBe(false);
  return { ...result, labelAdded, labelRemoved, body, calls: client.calls };
}

const serdeUpdate = { "Cargo.lock": { base: lockBase(), head: lockBase("0.15.0", "1.0.1") } };

describe("deps PR", () => {
  test("green checks and only lockfile changes: passes, adds the ready label and records the head SHA", async () => {
    const r = await run({ kind: "deps", files: serdeUpdate });
    expect(r.reasons).toEqual([]);
    expect(r.ok).toBe(true);
    expect(r.labelAdded).toBe(true);
    expect(r.body).toContain("head1");
    expect(r.body).toContain(CHECK_START);
    expect(r.body).toContain(CHECK_END);
    expect(r.body?.startsWith("Weekly updates.")).toBe(true);
  });

  test("a second check replaces the record in the body and adds no second copy", async () => {
    const first = await run({ kind: "deps", files: serdeUpdate });
    const second = await run({ kind: "deps", files: serdeUpdate, body: first.body! });
    expect(second.body?.split(CHECK_START).length).toBe(2);
  });

  test("one check still in progress: refuses with checks not complete", async () => {
    const runs = [...allGreen("deps").slice(1), { name: REQUIRED[0], status: "in_progress", conclusion: null }];
    const r = await run({ kind: "deps", files: serdeUpdate, runs });
    expect(r.ok).toBe(false);
    expect(r.reasons.join("\n")).toContain("checks not complete");
    expect(r.labelAdded).toBe(false);
  });

  test("the release jobs and the soak job skipped, all others green: passes", async () => {
    const skipped = ["Release build", "Release publish", "Recorder soak"].map((name) => ({ name, status: "completed", conclusion: "skipped" }));
    const r = await run({ kind: "deps", files: serdeUpdate, runs: [...allGreen("deps"), ...skipped] });
    expect(r.reasons).toEqual([]);
  });

  test("one check failed: refuses", async () => {
    const runs = [...allGreen("deps"), { name: "Lint", status: "completed", conclusion: "failure" }];
    const r = await run({ kind: "deps", files: serdeUpdate, runs });
    expect(r.ok).toBe(false);
    expect(r.reasons.join("\n")).toContain("check failed: Lint");
  });

  test("cancelled and timed out checks: refuses", async () => {
    const runs = [
      ...allGreen("deps"),
      { name: "A", status: "completed", conclusion: "cancelled" },
      { name: "B", status: "completed", conclusion: "timed_out" },
    ];
    const r = await run({ kind: "deps", files: serdeUpdate, runs });
    expect(r.reasons).toEqual(["check failed: A (cancelled)", "check failed: B (timed_out)"]);
  });

  test("a required check missing from the head: refuses", async () => {
    const r = await run({ kind: "deps", files: serdeUpdate, runs: allGreen("deps").slice(1) });
    expect(r.ok).toBe(false);
    expect(r.reasons).toContain(`required check missing: ${REQUIRED[0]}`);
  });

  test("the advisory check missing on a deps PR: refuses", async () => {
    const r = await run({ kind: "deps", files: serdeUpdate, runs: REQUIRED.map(green) });
    expect(r.reasons).toContain("required check missing: New advisories on this PR");
  });

  test("an ignore added to deny.toml: refuses with the path", async () => {
    const head = DENY_BASE.replace("ignore = [\n", `ignore = [\n  { id = "RUSTSEC-2026-0001", reason = "x" },\n`);
    const r = await run({ kind: "deps", files: { ...serdeUpdate, "deny.toml": { base: DENY_BASE, head } } });
    expect(r.ok).toBe(false);
    expect(r.reasons.join("\n")).toContain("deny.toml");
  });

  test("an ignore removed from deny.toml: allowed", async () => {
    const head = DENY_BASE.replace(/  \{ id = "RUSTSEC-2024-0429".*\n/, "");
    const r = await run({ kind: "deps", files: { ...serdeUpdate, "deny.toml": { base: DENY_BASE, head } } });
    expect(r.reasons).toEqual([]);
  });

  test("another change to deny.toml: refuses", async () => {
    const head = DENY_BASE.replace(`allow = ["MIT"]`, `allow = ["MIT", "GPL-3.0"]`);
    const r = await run({ kind: "deps", files: { ...serdeUpdate, "deny.toml": { base: DENY_BASE, head } } });
    expect(r.reasons.join("\n")).toContain("deny.toml");
  });

  test("Cargo.lock changes cpal: refuses with held", async () => {
    const r = await run({ kind: "deps", files: { "Cargo.lock": { base: lockBase(), head: lockBase("0.15.1") } } });
    expect(r.ok).toBe(false);
    expect(r.reasons.join("\n")).toContain("held: crate cpal 0.15.1");
  });

  test("a crate that only cpal uses: refuses with held", async () => {
    const r = await run({ kind: "deps", files: { "Cargo.lock": { base: lockBase(), head: lockBase("0.15.0", "1.0.0", "0.2.1") } } });
    expect(r.reasons.join("\n")).toContain("held: crate coreaudio-sys 0.2.1");
  });

  test("a crate version that is yanked or less than 7 days old: refuses", async () => {
    const yanked = registry({ crates: { serde: [published("1.0.1", 10, true)] } });
    expect((await run({ kind: "deps", files: serdeUpdate }, yanked)).reasons.join("\n")).toContain("yanked: crate serde 1.0.1");
    const young = registry({ crates: { serde: [published("1.0.1", 2)] } });
    expect((await run({ kind: "deps", files: serdeUpdate }, young)).reasons.join("\n")).toContain("less than 7 days old");
  });

  test("a young crate version with an advisory that the current advisory output confirms: passes", async () => {
    const young = registry({ crates: { serde: [published("1.0.1", 2)] }, advisories: ["crate:serde"] });
    expect((await run({ kind: "deps", files: serdeUpdate }, young)).reasons).toEqual([]);
  });

  test("the registry fails for a crate: refuses with publish date unknown", async () => {
    const failing = registry({ failing: ["serde"] });
    expect((await run({ kind: "deps", files: serdeUpdate }, failing)).reasons.join("\n")).toContain("publish date unknown: crate serde");
  });

  test("a crate that is not from the registry: refuses", async () => {
    const head = lockBase().replace(
      `name = "serde"\nversion = "1.0.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"`,
      `name = "serde"\nversion = "1.0.1"\nsource = "git+https://github.com/someone/serde#abc"`,
    );
    const r = await run({ kind: "deps", files: { "Cargo.lock": { base: lockBase(), head } } });
    expect(r.reasons.join("\n")).toContain("not from the registry: crate serde 1.0.1");
  });

  test("bun.lock has a version 2 days old with no confirmed advisory: refuses", async () => {
    const reg = registry({ packages: { react: [published("19.2.9", 2)] } });
    const r = await run({ kind: "deps", files: { "bun.lock": { base: bunLock({ react: "19.2.8" }), head: bunLock({ react: "19.2.9" }) } } }, reg);
    expect(r.ok).toBe(false);
    expect(r.reasons.join("\n")).toContain("less than 7 days old with no confirmed advisory: package react 19.2.9");
  });

  test("bun.lock with a 10-day-old version: passes", async () => {
    const r = await run({ kind: "deps", files: { "bun.lock": { base: bunLock({ react: "19.2.8" }), head: bunLock({ react: "19.2.9" }) } } });
    expect(r.reasons).toEqual([]);
  });

  test("bun.lock changes a held package: refuses with held", async () => {
    const reg = registry({ packages: { "@tauri-apps/api": [published("2.12.0", 10)] } });
    const r = await run(
      { kind: "deps", files: { "bun.lock": { base: bunLock({ "@tauri-apps/api": "2.11.0" }), head: bunLock({ "@tauri-apps/api": "2.12.0" }) } } },
      reg,
    );
    expect(r.reasons.join("\n")).toContain("held: package @tauri-apps/api 2.12.0");
  });

  test("bun.lock with a package from GitHub, or a change outside the package list: refuses", async () => {
    const github = await run({ kind: "deps", files: { "bun.lock": { base: bunLock({ react: "19.2.8" }), head: bunLock({ react: "github:evil/react#abc" }) } } });
    expect(github.reasons.join("\n")).toContain("not from the registry: package react");
    const trusted = await run({
      kind: "deps",
      files: { "bun.lock": { base: bunLock({ react: "19.2.8" }), head: bunLock({ react: "19.2.8" }, `\n  "trustedDependencies": ["react"],`) } },
    });
    expect(trusted.reasons.join("\n")).toContain("bun.lock");
  });

  test("package.json changes an entry to a github: or file: source: refuses", async () => {
    for (const source of ["github:evil/tlds", "file:../tlds"]) {
      const r = await run({ kind: "deps", files: { "package.json": { base: packageJson({ tlds: "^1.261.0" }), head: packageJson({ tlds: source }) } } });
      expect(r.ok).toBe(false);
      expect(r.reasons.join("\n")).toContain("package.json");
    }
  });

  test("package.json changes a version range: allowed", async () => {
    const r = await run({ kind: "deps", files: { "package.json": { base: packageJson({ tlds: "^1.261.0" }), head: packageJson({ tlds: "^1.262.0" }) } } });
    expect(r.reasons).toEqual([]);
  });

  test("package.json changes a dependency name, not only a version: refuses", async () => {
    const r = await run({ kind: "deps", files: { "package.json": { base: packageJson({ tlds: "^1.261.0" }), head: packageJson({ "tlds-evil": "^1.261.0" }) } } });
    expect(r.ok).toBe(false);
    expect(r.reasons.join("\n")).toContain("package.json");
  });

  test("package.json changes an exact version: refuses with held", async () => {
    const r = await run({ kind: "deps", files: { "package.json": { base: packageJson({ "react-reconciler": "0.33.0" }), head: packageJson({ "react-reconciler": "0.34.0" }) } } });
    expect(r.reasons.join("\n")).toContain("held: package react-reconciler is an exact pin");
  });

  test("package.json changes outside the dependency lists: refuses", async () => {
    const head = JSON.stringify({ ...JSON.parse(packageJson({ tlds: "^1.261.0" })), scripts: { postinstall: "curl evil" } });
    const r = await run({ kind: "deps", files: { "package.json": { base: packageJson({ tlds: "^1.261.0" }), head } } });
    expect(r.reasons.join("\n")).toContain("package.json");
  });

  test("licences/generated is allowed; another file is not", async () => {
    const ok = await run({ kind: "deps", files: { ...serdeUpdate, "licences/generated/rust.json": { base: "a", head: "b" } } });
    expect(ok.reasons).toEqual([]);
    const other = await run({ kind: "deps", files: { ...serdeUpdate, "src/main.ts": { base: "a", head: "b" } } });
    expect(other.reasons.join("\n")).toContain("src/main.ts");
  });
});

describe("refused paths, branch and title", () => {
  test("changes to the refused paths of KTD15: refuses", async () => {
    for (const path of ["docs/upkeep/weekly-routine.md", "scripts/upkeep/held.json", ".github/workflows/ci.yml", "bunfig.toml", "licences/manifest.json"]) {
      const r = await run({ kind: "deps", files: { ...serdeUpdate, [path]: { base: "a", head: "b" } } });
      expect(r.reasons.join("\n")).toContain(`refused path: ${path}`);
    }
  });

  test("a port PR that changes .github/workflows/ci.yml: refuses", async () => {
    const r = await run({ kind: "port", files: { ".github/workflows/ci.yml": { base: "a", head: "b" } } });
    expect(r.ok).toBe(false);
    expect(r.reasons.join("\n")).toContain("refused path: .github/workflows/ci.yml");
  });

  test("branch fix/x instead of upkeep/...: refuses", async () => {
    const r = await run({ kind: "deps", branch: "fix/x", files: serdeUpdate });
    expect(r.ok).toBe(false);
    expect(r.reasons.join("\n")).toContain("branch");
  });

  test("title build(deps)+: refuses", async () => {
    const r = await run({ kind: "deps", title: "build(deps)+: update dependencies", files: serdeUpdate });
    expect(r.ok).toBe(false);
    expect(r.reasons.join("\n")).toContain("title");
  });

  test("a title with ! or a type that does not fit the kind: refuses", async () => {
    expect((await run({ kind: "deps", title: "build(deps)!: x", files: serdeUpdate })).ok).toBe(false);
    expect((await run({ kind: "deps", title: "feat: x", files: serdeUpdate })).ok).toBe(false);
  });
});

describe("port PR", () => {
  const borrowed = {
    "crates/summarize/src/prompt.rs": { base: "old", head: "new" },
    "crates/summarize/PROVENANCE.md": { base: SUMMARIZE_PROVENANCE, head: SUMMARIZE_PROVENANCE.replace("1111111", "9999999") },
    "crates/summarize/tests/prompt.rs": { base: null, head: "test" },
  };

  test("a borrowed file, its Revision line and a test: passes", async () => {
    const r = await run({ kind: "port", files: borrowed });
    expect(r.reasons).toEqual([]);
    expect(r.labelAdded).toBe(true);
  });

  test("a file in crates/audio-cpal that PROVENANCE lists as borrowed from Handy: refuses (AE2)", async () => {
    const r = await run({
      kind: "port",
      branch: "upkeep/port-handy-abc1234",
      title: "fix(upstream): port Handy fix",
      files: { "crates/audio-cpal/src/config.rs": { base: "a", head: "b" } },
      unchanged: { "crates/audio-cpal/PROVENANCE.md": AUDIO_PROVENANCE },
    });
    expect(r.ok).toBe(false);
    expect(r.reasons.join("\n")).toContain("sensitive path: crates/audio-cpal/src/config.rs");
  });

  test("a PROVENANCE record that adds a new path: refuses", async () => {
    const head = `${SUMMARIZE_PROVENANCE}\n${record("src/new.rs", ANARLOG, "9999999")}`;
    const r = await run({
      kind: "port",
      files: {
        "crates/summarize/src/new.rs": { base: null, head: "new" },
        "crates/summarize/PROVENANCE.md": { base: SUMMARIZE_PROVENANCE, head },
      },
    });
    expect(r.ok).toBe(false);
    expect(r.reasons.join("\n")).toContain("not borrowed from anarlog: crates/summarize/src/new.rs");
    expect(r.reasons.join("\n")).toContain("crates/summarize/PROVENANCE.md");
  });

  test("a PROVENANCE change other than a Revision line: refuses", async () => {
    const r = await run({
      kind: "port",
      files: {
        "crates/summarize/src/prompt.rs": { base: "old", head: "new" },
        "crates/summarize/PROVENANCE.md": { base: SUMMARIZE_PROVENANCE, head: SUMMARIZE_PROVENANCE.replace("- Status: modified", "- Status: changed") },
      },
    });
    expect(r.reasons.join("\n")).toContain("crates/summarize/PROVENANCE.md");
  });

  test("a build.rs change: refuses", async () => {
    const provenance = `# Provenance\n\n${record("build.rs", ANARLOG, "1111111")}`;
    const r = await run({
      kind: "port",
      files: { "crates/summarize/build.rs": { base: "a", head: "b" } },
      unchanged: { "crates/summarize/PROVENANCE.md": provenance },
    });
    expect(r.ok).toBe(false);
    expect(r.reasons.join("\n")).toContain("build.rs");
  });

  test("a file that no PROVENANCE record lists for that project: refuses", async () => {
    const r = await run({
      kind: "port",
      files: { "crates/summarize/src/parse.rs": { base: "a", head: "b" } },
      unchanged: { "crates/summarize/PROVENANCE.md": SUMMARIZE_PROVENANCE },
    });
    expect(r.ok).toBe(false);
    expect(r.reasons.join("\n")).toContain("not borrowed from anarlog: crates/summarize/src/parse.rs");
  });

  test("a file with no PROVENANCE record at all: refuses", async () => {
    const r = await run({ kind: "port", files: { "src/app.ts": { base: "a", head: "b" } } });
    expect(r.reasons.join("\n")).toContain("not borrowed from anarlog: src/app.ts");
  });
});

describe("docs PR", () => {
  test("only the review log: passes", async () => {
    const r = await run({ kind: "docs", files: { "docs/reference/reference-source-reviews.md": { base: "a", head: "b" } } });
    expect(r.reasons).toEqual([]);
  });

  test("another file: refuses", async () => {
    const r = await run({ kind: "docs", files: { "docs/reference/reference-source-reviews.md": { base: "a", head: "b" }, "README.md": { base: "a", head: "b" } } });
    expect(r.reasons.join("\n")).toContain("README.md");
  });
});

describe("ready label", () => {
  test("a ready PR whose head changed after the check: the label is removed until the check passes again", async () => {
    const runs = [...allGreen("deps").slice(1), { name: REQUIRED[0], status: "queued", conclusion: null }];
    const body = `Weekly updates.\n\n${CHECK_START}\nUpkeep check passed at old0.\n${CHECK_END}\n`;
    const r = await run({ kind: "deps", files: serdeUpdate, runs, labels: ["upkeep: ready"], body });
    expect(r.ok).toBe(false);
    expect(r.labelRemoved).toBe(true);
    expect(r.labelAdded).toBe(false);
    expect(r.body).not.toContain("passed at old0");
    expect(r.body).toContain("head1");
    const again = await run({ kind: "deps", files: serdeUpdate, labels: [], body: r.body! });
    expect(again.ok).toBe(true);
    expect(again.labelAdded).toBe(true);
  });

  test("the head changes during the check: refuses and adds no label", async () => {
    const r = await run({ kind: "deps", files: serdeUpdate, headAfter: "head2" });
    expect(r.ok).toBe(false);
    expect(r.reasons.join("\n")).toContain("head changed");
    expect(r.labelAdded).toBe(false);
  });
});
