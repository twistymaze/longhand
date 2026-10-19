import { expect, test } from "bun:test";
import { compare, scan, type ScanOutput } from "./check-new-advisories";

// The tests give the script the text that cargo-deny and bun audit write, so
// no test runs cargo, bun or a registry, and no test reaches the network.

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

/// A bun.lock with these `name@version` packages. A key with a slash is a nested copy.
function bunLock(packages: Record<string, string>): string {
  const entries = Object.entries(packages).map(([key, spec]) => `    "${key}": ["${spec}", "", {}, "sha512-x"],`);
  return `{\n  "lockfileVersion": 1,\n  "packages": {\n${entries.join("\n")}\n  }\n}\n`;
}

// Real `bun audit --json` output (bun 1.4), shortened.
const SOURCE_MAP_AUDIT = {
  "source-map-js": [
    {
      id: 1241209,
      url: "https://github.com/advisories/GHSA-68fv-2mgg-jv7q",
      title: "source-map-js allows event-loop denial of service through indexed source-map section offsets",
      severity: "high",
      vulnerable_versions: ">=1.0.0 <1.2.2",
    },
  ],
};

function output(deny: string[], audit: object = {}, lock = bunLock({})): ScanOutput {
  const auditText = JSON.stringify(audit);
  return {
    deny: denyOutput(deny),
    audit: { exitCode: auditText === "{}" ? 0 : 1, stdout: auditText, stderr: "" },
    bunLock: lock,
  };
}

/// A cargo-deny line for advisory `id` on `name@version`.
function advisoryLine(id: string, name: string, version: string): string {
  return JSON.stringify({
    type: "diagnostic",
    fields: { code: "vulnerability", severity: "error", advisory: { id, package: name, title: `${id} title` }, graphs: [{ Krate: { name, version } }] },
  });
}

test("scan reads the advisories and the yanked crates of the real cargo-deny output", () => {
  const result = scan(output([DENY_LINES.rustls, DENY_LINES.wasapi, DENY_LINES.yanked]));
  expect(result.ok).toBe(true);
  if (!result.ok) return;
  expect(result.findings.map((f) => `${f.id} ${f.name} ${f.version}`)).toEqual([
    "RUSTSEC-2026-0346 rustls 0.23.45",
    "RUSTSEC-2026-0332 wasapi 0.23.0",
    "yanked:yoke-derive@0.8.3 yoke-derive 0.8.3",
  ]);
  expect(result.findings[0].title).toBe("`Debug` impls for client session values expose TLS secrets");
});

test("scan gives the bun.lock version that the bun advisory affects", () => {
  const lock = bunLock({ "source-map-js": "source-map-js@1.2.1", "other/source-map-js": "source-map-js@0.6.2" });
  const result = scan(output([], SOURCE_MAP_AUDIT, lock));
  expect(result.ok).toBe(true);
  if (!result.ok) return;
  expect(result.findings.map((f) => `${f.ecosystem} ${f.id} ${f.name} ${f.version}`)).toEqual(["package 1241209 source-map-js 1.2.1"]);
});

test("compare fails and names only the advisory that the head adds", () => {
  const base = output([DENY_LINES.rustls]);
  const head = output([DENY_LINES.rustls, DENY_LINES.zip]);
  const result = compare(scan(base), scan(head));
  expect(result.ok).toBe(false);
  expect(result.added.map((f) => f.id)).toEqual(["RUSTSEC-2026-0367"]);
  expect(result.message).toContain("RUSTSEC-2026-0367");
  expect(result.message).not.toContain("RUSTSEC-2026-0346");
});

test("compare passes when the base and the head have the same advisory", () => {
  const result = compare(scan(output([DENY_LINES.rustls])), scan(output([DENY_LINES.rustls])));
  expect(result.ok).toBe(true);
  expect(result.added).toEqual([]);
});

test("compare fails for a second affected version of a package with a known advisory", () => {
  const base = output([advisoryLine("RUSTSEC-2026-0001", "foo", "1.0.0")]);
  const head = output([advisoryLine("RUSTSEC-2026-0001", "foo", "1.0.0"), advisoryLine("RUSTSEC-2026-0001", "foo", "2.0.0")]);
  const result = compare(scan(base), scan(head));
  expect(result.ok).toBe(false);
  expect(result.added.map((f) => `${f.name} ${f.version}`)).toEqual(["foo 2.0.0"]);
  expect(result.message).toContain("foo 2.0.0");
});

test("compare fails for a second affected copy of a JavaScript package", () => {
  const base = output([], SOURCE_MAP_AUDIT, bunLock({ "source-map-js": "source-map-js@1.2.1" }));
  const head = output([], SOURCE_MAP_AUDIT, bunLock({ "source-map-js": "source-map-js@1.2.1", "vite/source-map-js": "source-map-js@1.0.2" }));
  const result = compare(scan(base), scan(head));
  expect(result.ok).toBe(false);
  expect(result.added.map((f) => `${f.id} ${f.name} ${f.version}`)).toEqual(["1241209 source-map-js 1.0.2"]);
});

test("compare fails for a second affected copy at the same version", () => {
  const base = output([], SOURCE_MAP_AUDIT, bunLock({ "source-map-js": "source-map-js@1.2.1" }));
  const head = output([], SOURCE_MAP_AUDIT, bunLock({ "source-map-js": "source-map-js@1.2.1", "vite/source-map-js": "source-map-js@1.2.1" }));
  const result = compare(scan(base), scan(head));
  expect(result.ok).toBe(false);
  expect(result.added.map((f) => `${f.name} ${f.version}`)).toEqual(["source-map-js 1.2.1"]);
});

test("compare passes when the head removes an advisory", () => {
  const result = compare(scan(output([DENY_LINES.rustls], SOURCE_MAP_AUDIT, bunLock({ "source-map-js": "source-map-js@1.2.1" }))), scan(output([])));
  expect(result.ok).toBe(true);
});

test("compare passes when cargo-deny and bun audit find nothing", () => {
  const empty: ScanOutput = { deny: denyOutput([]), audit: { exitCode: 0, stdout: "{}", stderr: "" }, bunLock: bunLock({}) };
  const result = compare(scan(empty), scan(empty));
  expect(result.ok).toBe(true);
  expect(result.message).toBe("The head adds no advisory.");
});

test("compare fails with 'audit could not run' when bun audit cannot reach the registry", () => {
  const failed: ScanOutput = {
    deny: denyOutput([]),
    audit: { exitCode: 1, stdout: "", stderr: "error: POST https://registry.npmjs.org/-/npm/v1/security/advisories/bulk - ConnectionRefused" },
    bunLock: bunLock({}),
  };
  const result = compare(scan(output([])), scan(failed));
  expect(result.ok).toBe(false);
  expect(result.message).toContain("audit could not run");
  expect(result.message).toContain("ConnectionRefused");
  expect(result.added).toEqual([]);
  expect(result.message).not.toContain("The head adds");
});

test("compare fails with 'audit could not run' when cargo-deny writes no summary", () => {
  const failed: ScanOutput = { ...output([]), deny: { exitCode: 1, stderr: "error: failed to fetch advisory database\n" } };
  const result = compare(scan(failed), scan(output([])));
  expect(result.ok).toBe(false);
  expect(result.message).toContain("audit could not run");
});

test("compare passes when the PR moves an affected package to a version that is still affected", () => {
  const base = output([advisoryLine("RUSTSEC-2026-0001", "foo", "1.2.3")]);
  const head = output([advisoryLine("RUSTSEC-2026-0001", "foo", "1.2.4")]);
  const result = compare(scan(base), scan(head));
  expect(result.ok).toBe(true);
  expect(result.added).toEqual([]);
});
