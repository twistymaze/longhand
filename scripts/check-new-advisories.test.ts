import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, test } from "node:test";
import { auditCheckout, compare, legacyScans, scan, type RunResult, type ScanOutput } from "./check-new-advisories.ts";

// The tests give the script the text that cargo-deny and npm audit write, so
// no test runs cargo, npm or a registry, and no test reaches the network.

// Real `cargo deny --format json check advisories` lines (cargo-deny 0.20.2),
// shortened: the description, the dependency graph and most notes are removed.
const DENY_LINES = {
  rustls: `{"fields":{"advisory":{"id":"RUSTSEC-2026-0346","package":"rustls","title":"\`Debug\` impls for client session values expose TLS secrets","url":"https://github.com/rustls/rustls/pull/3299"},"code":"vulnerability","graphs":[{"Krate":{"name":"rustls","version":"0.23.45"}}],"message":"\`Debug\` impls for client session values expose TLS secrets","notes":["ID: RUSTSEC-2026-0346","Solution: Upgrade to >=0.23.46 (try \`cargo update -p rustls\`)"],"severity":"error"},"type":"diagnostic"}`,
  wasapi: `{"fields":{"advisory":{"id":"RUSTSEC-2026-0332","package":"wasapi","title":"\`WaveFormat::parse\` reads past the end of a \`&WAVEFORMATEX\`","url":"https://github.com/HEnquist/wasapi-rs/issues/65"},"code":"unsound","graphs":[{"Krate":{"name":"wasapi","version":"0.23.0"}}],"message":"\`WaveFormat::parse\` reads past the end of a \`&WAVEFORMATEX\`","notes":["ID: RUSTSEC-2026-0332","Solution: Upgrade to >=0.25.0 (try \`cargo update -p wasapi\`)"],"severity":"error"},"type":"diagnostic"}`,
  zip: `{"fields":{"advisory":{"id":"RUSTSEC-2026-0367","package":"zip","title":" Insecure Preservation of SUID/SGID Bits during ZIP Extraction Leading to Potential Privilege Escalation","url":null},"code":"vulnerability","graphs":[{"Krate":{"name":"zip","version":"4.6.1"}}],"message":" Insecure Preservation of SUID/SGID Bits during ZIP Extraction Leading to Potential Privilege Escalation","notes":["ID: RUSTSEC-2026-0367","Solution: Upgrade to >=9.0.0 (try \`cargo update -p zip\`)"],"severity":"error"},"type":"diagnostic"}`,
  yanked: `{"fields":{"code":"yanked","graphs":[{"Krate":{"name":"yoke-derive","version":"0.8.3"}}],"message":"detected yanked crate (try \`cargo update -p yoke-derive\`)","severity":"error"},"type":"diagnostic"}`,
};

function summary(errors: number): string {
  return `{"fields":{"advisories":{"errors":${errors},"helps":0,"notes":0,"warnings":0}},"type":"summary"}`;
}

/// cargo-deny's standard error for these diagnostic lines.
function denyOutput(lines: string[]): { exitCode: number; stderr: string } {
  return { exitCode: lines.length > 0 ? 1 : 0, stderr: [...lines, summary(lines.length)].join("\n") + "\n" };
}

/// A package-lock.json (lockfileVersion 3) with these keys and versions.
/// A key with two `node_modules/` parts is a nested copy.
function packageLock(nodes: Record<string, string>): string {
  const packages: Record<string, object> = { "": { name: "longhand" } };
  for (const [key, version] of Object.entries(nodes)) packages[key] = { version, resolved: `https://registry.npmjs.org/x/-/x-${version}.tgz`, integrity: "sha512-x" };
  return JSON.stringify({ name: "longhand", lockfileVersion: 3, requires: true, packages });
}

/// A legacy bun.lock with these `name@version` packages.
function bunLock(packages: Record<string, string>): string {
  const entries = Object.entries(packages).map(([key, spec]) => `    "${key}": ["${spec}", "", {}, "sha512-x"],`);
  return `{\n  "lockfileVersion": 1,\n  "packages": {\n${entries.join("\n")}\n  }\n}\n`;
}

// Real `npm audit --json --package-lock-only` output (npm 12.2.0), shortened:
// cwe, cvss, fixAvailable and metadata are removed.
const SOURCE_MAP_ADVISORY = {
  source: 1241209,
  name: "source-map-js",
  dependency: "source-map-js",
  title: "source-map-js allows event-loop denial of service through indexed source-map section offsets",
  url: "https://github.com/advisories/GHSA-68fv-2mgg-jv7q",
  severity: "high",
  range: ">=1.0.0 <1.2.2",
};

/// npm audit's report for these vulnerabilities.
function npmAudit(vulnerabilities: Record<string, { via: unknown[]; nodes: string[] }>): object {
  return { auditReportVersion: 2, vulnerabilities, metadata: {} };
}

/// The source-map-js advisory on these lockfile keys.
const sourceMapAudit = (...nodes: string[]) =>
  npmAudit({ "source-map-js": { via: [SOURCE_MAP_ADVISORY], nodes: nodes.length > 0 ? nodes : ["node_modules/source-map-js"] } });

function output(deny: string[], audit: object = npmAudit({}), lock = packageLock({})): ScanOutput {
  const empty = Object.keys((audit as { vulnerabilities: object }).vulnerabilities).length === 0;
  return {
    deny: denyOutput(deny),
    audit: { exitCode: empty ? 0 : 1, stdout: JSON.stringify(audit), stderr: "" },
    packageLock: lock,
  };
}

/// A cargo-deny line for advisory `id` on `name@version`.
function advisoryLine(id: string, name: string, version: string): string {
  return JSON.stringify({
    type: "diagnostic",
    fields: { code: "vulnerability", severity: "error", advisory: { id, package: name, title: `${id} title` }, graphs: [{ Krate: { name, version } }] },
  });
}

const ids = (findings: { ecosystem: string; id: string; name: string; version: string }[]) => findings.map((f) => `${f.ecosystem} ${f.id} ${f.name} ${f.version}`);

test("scan reads the advisories and the yanked crates of the real cargo-deny output", () => {
  const result = scan(output([DENY_LINES.rustls, DENY_LINES.wasapi, DENY_LINES.yanked]));
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.deepEqual(result.findings.map((f) => `${f.id} ${f.name} ${f.version}`), [
    "RUSTSEC-2026-0346 rustls 0.23.45",
    "RUSTSEC-2026-0332 wasapi 0.23.0",
    "yanked:yoke-derive@0.8.3 yoke-derive 0.8.3",
  ]);
  assert.equal(result.findings[0].title, "`Debug` impls for client session values expose TLS secrets");
});

test("scan gives the package-lock.json version of each node that the npm advisory affects", () => {
  const lock = packageLock({ "node_modules/source-map-js": "1.2.1", "node_modules/other/node_modules/source-map-js": "0.6.2" });
  const result = scan(output([], sourceMapAudit("node_modules/source-map-js", "node_modules/other/node_modules/source-map-js"), lock));
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.deepEqual(ids(result.findings), ["package 1241209 source-map-js 1.2.1"]);
  assert.equal(result.findings[0].title, SOURCE_MAP_ADVISORY.title);
});

test("a head lockfile with a new vulnerable version gives one finding with the advisory id, name and version", () => {
  const head = output([], sourceMapAudit(), packageLock({ "node_modules/source-map-js": "1.2.1" }));
  const result = compare(scan(output([])), scan(head));
  assert.equal(result.ok, false);
  assert.deepEqual(ids(result.added), ["package 1241209 source-map-js 1.2.1"]);
  assert.ok(result.message.includes("1241209 (source-map-js 1.2.1)"));
});

test("an audit result whose via has only package names gives no finding of its own", () => {
  // npm audit lists a package that depends on a vulnerable package with the
  // name of that package in `via`.
  const audit = npmAudit({
    "source-map-js": { via: [SOURCE_MAP_ADVISORY], nodes: ["node_modules/source-map-js"] },
    postcss: { via: ["source-map-js"], nodes: ["node_modules/postcss"] },
  });
  const result = scan(output([], audit, packageLock({ "node_modules/source-map-js": "1.2.1", "node_modules/postcss": "8.5.6" })));
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.deepEqual(ids(result.findings), ["package 1241209 source-map-js 1.2.1"]);
});

test("each advisory applies only to the version in its range", () => {
  const audit = npmAudit({
    foo: {
      via: [
        { source: 1, name: "foo", title: "old fault", range: "<1.5.0" },
        { source: 2, name: "foo", title: "new fault", range: ">=2.0.0 <2.1.0" },
      ],
      nodes: ["node_modules/foo", "node_modules/bar/node_modules/foo"],
    },
  });
  const result = scan(output([], audit, packageLock({ "node_modules/foo": "1.4.0", "node_modules/bar/node_modules/foo": "2.0.3" })));
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.deepEqual(ids(result.findings), ["package 1 foo 1.4.0", "package 2 foo 2.0.3"]);
});

test("an advisory range or a version outside the supported subset applies to every node", () => {
  const audit = npmAudit({
    foo: { via: [{ source: 1, name: "foo", range: "^1.0.0" }], nodes: ["node_modules/foo", "node_modules/bar/node_modules/foo"] },
    baz: { via: [{ source: 2, name: "baz", range: "<1.0.0" }], nodes: ["node_modules/baz"] },
  });
  const lock = packageLock({ "node_modules/foo": "1.4.0", "node_modules/bar/node_modules/foo": "3.0.0", "node_modules/baz": "2.0.0-beta.1" });
  const result = scan(output([], audit, lock));
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.deepEqual(ids(result.findings), ["package 1 foo 1.4.0", "package 1 foo 3.0.0", "package 2 baz 2.0.0-beta.1"]);
});

test("compare fails and names only the advisory that the head adds", () => {
  const base = output([DENY_LINES.rustls]);
  const head = output([DENY_LINES.rustls, DENY_LINES.zip]);
  const result = compare(scan(base), scan(head));
  assert.equal(result.ok, false);
  assert.deepEqual(result.added.map((f) => f.id), ["RUSTSEC-2026-0367"]);
  assert.ok(result.message.includes("RUSTSEC-2026-0367"));
  assert.ok(!result.message.includes("RUSTSEC-2026-0346"));
});

test("compare passes when the base and the head have the same advisory", () => {
  const result = compare(scan(output([DENY_LINES.rustls])), scan(output([DENY_LINES.rustls])));
  assert.equal(result.ok, true);
  assert.deepEqual(result.added, []);
});

test("compare passes when the base and the head have the same npm finding", () => {
  const lock = packageLock({ "node_modules/source-map-js": "1.2.1" });
  const result = compare(scan(output([], sourceMapAudit(), lock)), scan(output([], sourceMapAudit(), lock)));
  assert.equal(result.ok, true);
  assert.deepEqual(result.added, []);
});

test("compare fails for a second affected version of a package with a known advisory", () => {
  const base = output([advisoryLine("RUSTSEC-2026-0001", "foo", "1.0.0")]);
  const head = output([advisoryLine("RUSTSEC-2026-0001", "foo", "1.0.0"), advisoryLine("RUSTSEC-2026-0001", "foo", "2.0.0")]);
  const result = compare(scan(base), scan(head));
  assert.equal(result.ok, false);
  assert.deepEqual(result.added.map((f) => `${f.name} ${f.version}`), ["foo 2.0.0"]);
  assert.ok(result.message.includes("foo 2.0.0"));
});

test("compare fails for a second affected copy of a JavaScript package", () => {
  const base = output([], sourceMapAudit(), packageLock({ "node_modules/source-map-js": "1.2.1" }));
  const head = output(
    [],
    sourceMapAudit("node_modules/source-map-js", "node_modules/vite/node_modules/source-map-js"),
    packageLock({ "node_modules/source-map-js": "1.2.1", "node_modules/vite/node_modules/source-map-js": "1.0.2" }),
  );
  const result = compare(scan(base), scan(head));
  assert.equal(result.ok, false);
  assert.deepEqual(result.added.map((f) => `${f.id} ${f.name} ${f.version}`), ["1241209 source-map-js 1.0.2"]);
});

test("compare fails for a second affected copy at the same version", () => {
  const base = output([], sourceMapAudit(), packageLock({ "node_modules/source-map-js": "1.2.1" }));
  const head = output(
    [],
    sourceMapAudit("node_modules/source-map-js", "node_modules/vite/node_modules/source-map-js"),
    packageLock({ "node_modules/source-map-js": "1.2.1", "node_modules/vite/node_modules/source-map-js": "1.2.1" }),
  );
  const result = compare(scan(base), scan(head));
  assert.equal(result.ok, false);
  assert.deepEqual(result.added.map((f) => `${f.name} ${f.version}`), ["source-map-js 1.2.1"]);
});

test("compare passes when the head removes an advisory", () => {
  const base = output([DENY_LINES.rustls], sourceMapAudit(), packageLock({ "node_modules/source-map-js": "1.2.1" }));
  assert.equal(compare(scan(base), scan(output([]))).ok, true);
});

test("compare passes when cargo-deny and npm audit find nothing", () => {
  const empty = output([]);
  const result = compare(scan(empty), scan(empty));
  assert.equal(result.ok, true);
  assert.equal(result.message, "The head adds no advisory.");
});

test("compare fails with 'audit could not run' when npm audit cannot reach the registry", () => {
  // Real npm 12.2.0 output when the registry refuses the connection.
  const failed: ScanOutput = {
    deny: denyOutput([]),
    audit: {
      exitCode: 1,
      stdout: JSON.stringify({
        message: "request to https://registry.npmjs.org/-/npm/v1/security/advisories/bulk failed, reason: connect ECONNREFUSED 127.0.0.1:443",
        error: { summary: "", detail: "" },
      }),
      stderr: "npm error audit endpoint returned an error\n",
    },
    packageLock: packageLock({}),
  };
  const result = compare(scan(output([])), scan(failed));
  assert.equal(result.ok, false);
  assert.ok(result.message.includes("audit could not run"));
  assert.ok(result.message.includes("ECONNREFUSED"));
  assert.deepEqual(result.added, []);
  assert.ok(!result.message.includes("The head adds"));
});

test("compare fails with 'audit could not run' when npm audit exits non-zero with no JSON", () => {
  const failed: ScanOutput = { ...output([]), audit: { exitCode: 1, stdout: "", stderr: "npm error code ENOLOCK\n" } };
  const result = compare(scan(output([])), scan(failed));
  assert.equal(result.ok, false);
  assert.ok(result.message.includes("audit could not run"));
  assert.ok(result.message.includes("ENOLOCK"));
});

test("compare fails with 'audit could not run' when cargo-deny writes no summary", () => {
  const failed: ScanOutput = { ...output([]), deny: { exitCode: 1, stderr: "error: failed to fetch advisory database\n" } };
  const result = compare(scan(failed), scan(output([])));
  assert.equal(result.ok, false);
  assert.ok(result.message.includes("audit could not run"));
});

test("compare passes when the PR moves an affected package to a version that is still affected", () => {
  const base = output([advisoryLine("RUSTSEC-2026-0001", "foo", "1.2.3")]);
  const head = output([advisoryLine("RUSTSEC-2026-0001", "foo", "1.2.4")]);
  const result = compare(scan(base), scan(head));
  assert.equal(result.ok, true);
  assert.deepEqual(result.added, []);
});

test("with a legacy base, a head finding for a name and version in bun.lock is not reported, and a new version is", () => {
  const headLock = packageLock({ "node_modules/source-map-js": "1.2.1", "node_modules/vite/node_modules/source-map-js": "1.0.2" });
  const head = scan(output([DENY_LINES.rustls], sourceMapAudit("node_modules/source-map-js", "node_modules/vite/node_modules/source-map-js"), headLock));
  const legacy = legacyScans(denyOutput([DENY_LINES.rustls]), head, bunLock({ "source-map-js": "source-map-js@1.2.1" }));
  const result = compare(legacy.base, legacy.head);
  assert.equal(result.ok, false);
  assert.deepEqual(ids(result.added), ["package 1241209 source-map-js 1.0.2"]);
});

test("with a legacy base, a head with only known findings passes", () => {
  const head = scan(output([], sourceMapAudit(), packageLock({ "node_modules/source-map-js": "1.2.1" })));
  const legacy = legacyScans(denyOutput([]), head, bunLock({ "source-map-js": "source-map-js@1.2.1" }));
  assert.equal(compare(legacy.base, legacy.head).ok, true);
});

test("with a legacy base, a head that adds a second copy of a known version reports the added copy", () => {
  const headLock = packageLock({ "node_modules/source-map-js": "1.2.1", "node_modules/vite/node_modules/source-map-js": "1.2.1" });
  const head = scan(output([], sourceMapAudit("node_modules/source-map-js", "node_modules/vite/node_modules/source-map-js"), headLock));
  const legacy = legacyScans(denyOutput([]), head, bunLock({ "source-map-js": "source-map-js@1.2.1" }));
  const result = compare(legacy.base, legacy.head);
  assert.equal(result.ok, false);
  assert.deepEqual(ids(result.added), ["package 1241209 source-map-js 1.2.1"]);
});

test("with a legacy base, two copies in bun.lock cover two head copies", () => {
  const headLock = packageLock({ "node_modules/source-map-js": "1.2.1", "node_modules/vite/node_modules/source-map-js": "1.2.1" });
  const head = scan(output([], sourceMapAudit("node_modules/source-map-js", "node_modules/vite/node_modules/source-map-js"), headLock));
  const legacy = legacyScans(denyOutput([]), head, bunLock({ "source-map-js": "source-map-js@1.2.1", "vite/source-map-js": "source-map-js@1.2.1" }));
  assert.equal(compare(legacy.base, legacy.head).ok, true);
});

// Checkouts on disk for auditCheckout. Each test gets new folders.
const scratch = mkdtempSync(join(tmpdir(), "longhand-advisories-test-"));
after(() => rmSync(scratch, { recursive: true, force: true }));
let checkouts = 0;

function checkout(files: Record<string, string>): string {
  const dir = join(scratch, `checkout-${checkouts++}`);
  mkdirSync(dir);
  for (const [name, text] of Object.entries(files)) writeFileSync(join(dir, name), text);
  return dir;
}

const MANIFEST = JSON.stringify({
  name: "longhand",
  devEngines: { packageManager: { name: "npm", version: ">=99.0.0", onFail: "error" } },
  dependencies: { "source-map-js": "1.2.1" },
});

/// What the stub runner saw in the audit folder when npm audit would run.
type Seen = { command: string[]; cwd: string; files: string[]; npmrc: string | null; manifest: Record<string, unknown>; userConfig: string };

function stubRunner(result: RunResult, seen: Seen[]) {
  return (command: string[], cwd: string): RunResult => {
    const userConfig = command.find((arg) => arg.startsWith("--userconfig="))?.slice("--userconfig=".length) ?? "";
    seen.push({
      command,
      cwd,
      files: readdirSync(cwd).sort(),
      npmrc: existsSync(join(cwd, ".npmrc")) ? readFileSync(join(cwd, ".npmrc"), "utf8") : null,
      manifest: JSON.parse(readFileSync(join(cwd, "package.json"), "utf8")),
      userConfig: userConfig ? readFileSync(userConfig, "utf8") : "missing",
    });
    return result;
  };
}

test("AE5: the audit folder holds only the base .npmrc, and npm audit gets the registry and an empty user config", () => {
  const lock = packageLock({ "node_modules/source-map-js": "1.2.1" });
  const base = checkout({ "package.json": MANIFEST, "package-lock.json": packageLock({}), ".npmrc": "min-release-age=7\n" });
  const head = checkout({ "package.json": MANIFEST, "package-lock.json": lock, ".npmrc": "registry=https://evil.example/\n" });
  const seen: Seen[] = [];
  const audit = auditCheckout(head, base, stubRunner({ exitCode: 1, stdout: JSON.stringify(sourceMapAudit()), stderr: "" }, seen));
  assert.equal(seen.length, 1);
  assert.equal(seen[0].npmrc, "min-release-age=7\n");
  assert.deepEqual(seen[0].files, [".npmrc", "empty-user-config", "package-lock.json", "package.json"]);
  assert.ok(seen[0].command.includes("--registry=https://registry.npmjs.org/"));
  assert.ok(seen[0].command.some((arg) => arg.startsWith("--userconfig=")));
  assert.equal(seen[0].userConfig, "");
  assert.deepEqual(seen[0].command.slice(0, 5), ["npm", "audit", "--json", "--package-lock-only", "--ignore-scripts"]);
  // The audit runs outside the head checkout, and the folder is removed after the run.
  const { cwd } = seen[0];
  assert.ok(!cwd.startsWith(head));
  assert.equal(existsSync(cwd), false);
  // The head advisory still fails the check.
  const result = compare(scan(output([])), scan({ deny: denyOutput([]), audit, packageLock: lock }));
  assert.equal(result.ok, false);
  assert.deepEqual(ids(result.added), ["package 1241209 source-map-js 1.2.1"]);
});

test("the audit folder has no .npmrc when the base has none, also when the head has one", () => {
  const base = checkout({ "package.json": MANIFEST, "package-lock.json": packageLock({}) });
  const head = checkout({ "package.json": MANIFEST, "package-lock.json": packageLock({}), ".npmrc": "registry=https://evil.example/\n" });
  const seen: Seen[] = [];
  auditCheckout(head, base, stubRunner({ exitCode: 0, stdout: JSON.stringify(npmAudit({})), stderr: "" }, seen));
  assert.equal(seen[0].npmrc, null);
});

test("a head package.json whose devEngines needs a newer npm still gives a normal audit result", () => {
  const lock = packageLock({ "node_modules/source-map-js": "1.2.1" });
  const base = checkout({ "package.json": MANIFEST, "package-lock.json": lock });
  const seen: Seen[] = [];
  const audit = auditCheckout(base, base, stubRunner({ exitCode: 1, stdout: JSON.stringify(sourceMapAudit()), stderr: "" }, seen));
  assert.equal("devEngines" in seen[0].manifest, false);
  assert.deepEqual(seen[0].manifest.dependencies, { "source-map-js": "1.2.1" });
  const result = scan({ deny: denyOutput([]), audit, packageLock: lock });
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.deepEqual(ids(result.findings), ["package 1241209 source-map-js 1.2.1"]);
});

test("a checkout with no package-lock.json gives 'audit could not run'", () => {
  const dir = checkout({ "package.json": MANIFEST });
  const seen: Seen[] = [];
  const audit = auditCheckout(dir, dir, stubRunner({ exitCode: 0, stdout: "{}", stderr: "" }, seen));
  assert.equal(seen.length, 0);
  const result = compare(scan(output([])), scan({ deny: denyOutput([]), audit, packageLock: "" }));
  assert.equal(result.ok, false);
  assert.ok(result.message.includes("audit could not run"));
});
