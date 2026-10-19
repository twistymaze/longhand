import { expect, test } from "bun:test";
import {
  addedIgnores,
  classify,
  compare,
  markReported,
  noticeText,
  runClassify,
  scan,
  type Finding,
  type ScanOutput,
} from "./advisories";
import { readMarkers, writeMarkers } from "./issue-markers";
import type { Selection } from "./select-updates";

// The tests give the script the text that cargo-deny and bun audit write, so
// no test runs cargo, bun or a registry, and no test reaches the network.

const NOW = new Date("2026-10-18T06:00:00Z");

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
  expect(result.message).not.toContain("new advisory");
});

test("compare fails with 'audit could not run' when cargo-deny writes no summary", () => {
  const failed: ScanOutput = { ...output([]), deny: { exitCode: 1, stderr: "error: failed to fetch advisory database\n" } };
  const result = compare(scan(failed), scan(output([])));
  expect(result.ok).toBe(false);
  expect(result.message).toContain("audit could not run");
});

const finding = (id: string, name: string, version = "1.0.0"): Finding => ({ ecosystem: "crate", id, name, version, title: `${id} title` });
const NO_SELECTION: Selection = { apply: [], held: [], exceptions: [] };
const RECENT_BODY = writeMarkers("Nothing is open.", { lastRun: "2026-10-15", reportedAdvisories: [] });

test("classify gives no notice for an advisory that the allowed updates remove", () => {
  const selection: Selection = {
    apply: [{ ecosystem: "crate", name: "rustls", from: "0.23.45", to: "0.23.46", command: [], advisories: ["A"] }],
    held: [],
    exceptions: [],
  };
  const result = classify({ before: [finding("A", "rustls")], after: [], selection, issueBody: RECENT_BODY, now: NOW });
  expect(result.notices).toEqual([]);
  expect(result.overdue).toBe(false);
  expect(noticeText(result)).toBeNull();
});

test("classify gives one notice for an advisory that stays, and none the next day", () => {
  const first = classify({ before: [finding("B", "zip")], after: [finding("B", "zip")], selection: NO_SELECTION, issueBody: RECENT_BODY, now: NOW });
  expect(first.notices.map((n) => n.id)).toEqual(["B"]);
  expect(first.notices[0].reason).toBe("The next weekly update does not remove it.");
  expect(first.reportedAdvisories).toEqual(["B"]);

  const nextBody = markReported(RECENT_BODY, first);
  expect(readMarkers(nextBody).reportedAdvisories).toEqual(["B"]);
  expect(readMarkers(nextBody).lastRun).toBe("2026-10-15");
  const next = classify({ before: [finding("B", "zip")], after: [finding("B", "zip")], selection: NO_SELECTION, issueBody: nextBody, now: NOW });
  expect(next.notices).toEqual([]);
  expect(noticeText(next)).toBeNull();
});

test("classify gives a notice for an advisory that needs an update to a held crate", () => {
  const selection: Selection = {
    apply: [],
    held: [{ ecosystem: "crate", name: "wasapi", version: "0.23.0", advisories: ["C"], reason: "advisory: the fix needs a held update. Platform API crate." }],
    exceptions: [],
  };
  const result = classify({ before: [finding("C", "wasapi")], after: [finding("C", "wasapi")], selection, issueBody: RECENT_BODY, now: NOW });
  expect(result.notices.map((n) => n.id)).toEqual(["C"]);
  expect(result.notices[0].reason).toBe("advisory: the fix needs a held update. Platform API crate.");
  expect(noticeText(result)).toContain("C (wasapi 1.0.0)");
});

test("classify gives an overdue notice when the last run is 9 days old", () => {
  const body = writeMarkers("", { lastRun: "2026-10-09", reportedAdvisories: [] });
  const result = classify({ before: [], after: [], selection: NO_SELECTION, issueBody: body, now: NOW });
  expect(result.overdue).toBe(true);
  expect(result.daysSinceLastRun).toBe(9);
  expect(noticeText(result)).toContain("9 days ago");
});

test("classify gives the overdue notice once for each last run date", () => {
  const body = writeMarkers("", { lastRun: "2026-10-09", reportedAdvisories: [] });
  const first = classify({ before: [], after: [], selection: NO_SELECTION, issueBody: body, now: NOW });
  expect(first.overdueNotice).toBe(true);

  const nextBody = markReported(body, first);
  const next = classify({ before: [], after: [], selection: NO_SELECTION, issueBody: nextBody, now: NOW });
  expect(next.overdue).toBe(true);
  expect(next.overdueNotice).toBe(false);
  expect(noticeText(next)).toBeNull();

  // A new weekly run on 2026-10-12 that is overdue again 9 days later gives a new notice.
  const later = writeMarkers(nextBody, { lastRun: "2026-10-12", reportedAdvisories: [] });
  const laterNow = new Date("2026-10-21T06:00:00Z");
  expect(classify({ before: [], after: [], selection: NO_SELECTION, issueBody: later, now: laterNow }).overdueNotice).toBe(true);
});

test("classify gives no overdue notice when the last run is 3 days old", () => {
  const result = classify({ before: [], after: [], selection: NO_SELECTION, issueBody: RECENT_BODY, now: NOW });
  expect(result.overdue).toBe(false);
  expect(noticeText(result)).toBeNull();
});

test("classify gives an overdue notice when the issue has no last run marker", () => {
  const result = classify({ before: [], after: [], selection: NO_SELECTION, issueBody: "Nothing is open.", now: NOW });
  expect(result.overdue).toBe(true);
  expect(result.daysSinceLastRun).toBeNull();
  expect(noticeText(result)).toContain("no date of the last weekly run");
});

test("runClassify scans, applies the allowed updates, scans again and restores the lockfiles", async () => {
  const steps: string[] = [];
  const selection: Selection = {
    apply: [{ ecosystem: "crate", name: "rustls", from: "0.23.45", to: "0.23.46", command: ["cargo", "update"], advisories: ["RUSTSEC-2026-0346"] }],
    held: [],
    exceptions: [],
  };
  let applied = false;
  const result = await runClassify(
    {
      scan: () => {
        steps.push("scan");
        return output(applied ? [DENY_LINES.zip] : [DENY_LINES.rustls, DENY_LINES.zip]);
      },
      select: async () => {
        steps.push("select");
        return selection;
      },
      apply: (updates) => {
        steps.push(`apply ${updates.length}`);
        applied = true;
      },
      restore: () => {
        steps.push("restore");
        applied = false;
      },
    },
    RECENT_BODY,
    NOW,
  );
  expect(steps).toEqual(["scan", "select", "apply 1", "scan", "restore"]);
  expect(result.notices.map((n) => n.id)).toEqual(["RUSTSEC-2026-0367"]);
});

test("runClassify stops when the scan cannot run", async () => {
  const failed: ScanOutput = { ...output([]), audit: { exitCode: 1, stdout: "", stderr: "error: ConnectionRefused" } };
  let restored = false;
  const tools = {
    scan: () => failed,
    select: async () => NO_SELECTION,
    apply: () => {},
    restore: () => {
      restored = true;
    },
  };
  await expect(runClassify(tools, RECENT_BODY, NOW)).rejects.toThrow("audit could not run");
  expect(restored).toBe(true);
});

test("addedIgnores names an advisory ignore and a licence that the head adds", () => {
  const baseDeny = `[advisories]\nignore = []\n[licenses]\nallow = ["MIT"]\nexceptions = []\n`;
  const headDeny = `[advisories]\nignore = [{ id = "RUSTSEC-2026-0332", reason = "x" }]\n[licenses]\nallow = ["MIT", "MPL-2.0"]\nexceptions = [{ crate = "foo", allow = ["GPL-3.0"] }]\n`;
  const base = { denyToml: baseDeny, aboutToml: `accepted = ["MIT"]\n`, auditCommands: "" };
  const head = { denyToml: headDeny, aboutToml: `accepted = ["MIT", "MPL-2.0"]\n`, auditCommands: `bun audit --json --ignore=GHSA-68fv-2mgg-jv7q` };
  expect(addedIgnores(base, head)).toEqual([
    "deny.toml advisory ignore: RUSTSEC-2026-0332",
    "deny.toml licence: MPL-2.0",
    "deny.toml licence exception: foo GPL-3.0",
    "about.toml licence: MPL-2.0",
    "bun audit ignore: GHSA-68fv-2mgg-jv7q",
  ]);
});

test("addedIgnores names nothing when the head removes an ignore", () => {
  const base = { denyToml: `[advisories]\nignore = ["RUSTSEC-2026-0332"]\n`, aboutToml: "", auditCommands: "" };
  const head = { denyToml: `[advisories]\nignore = []\n`, aboutToml: "", auditCommands: "" };
  expect(addedIgnores(base, head)).toEqual([]);
});
