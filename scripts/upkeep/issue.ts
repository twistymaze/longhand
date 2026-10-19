// Reads and writes the upkeep tracking issue (R18, R19, R22; KTD11).
//
//   bun scripts/upkeep/issue.ts read [--number <n>]
//     Prints JSON: the issue number, its body, its markers, and its comments.
//     Each comment has a trust level:
//       maintainer    the login in UPKEEP_MAINTAINER;
//       collaborator  another OWNER, MEMBER or COLLABORATOR;
//       data          all other accounts, and every bot.
//     Only a `maintainer` comment can defer or reject an advisory or an
//     upstream security finding. A `data` comment never changes a PR.
//
//   bun scripts/upkeep/issue.ts write <body file> [--number <n>] [--keep-last-run]
//     Replaces the issue body with the file. It sets the last run date
//     marker to today (UTC), or keeps the current date with
//     --keep-last-run (a run that failed). It keeps the reported advisories
//     and the overdue notice markers of the current body, which the daily
//     advisory workflow writes (issue-markers.ts).
//
// Without --number, the issue is the one open issue with the label `upkeep`.
// Before the first run there is no such issue: `read` then gives an empty
// issue with the number null, and `write` creates the issue. `write` also
// creates each upkeep label that does not exist.
// UPKEEP_MAINTAINER comes from the environment, or else from the repository
// variable of that name. The routine never comments on the issue (KTD11).

import { readFileSync } from "node:fs";
import { type Client, READY_LABEL, allPages, errorStatus, ghClient } from "./github";
import { type Markers, readMarkers, writeMarkers } from "./issue-markers";

export const ISSUE_LABEL = "upkeep";
export const ISSUE_TITLE = "Weekly upkeep";
/// The label that the routine adds to the issue when it has new items for
/// the maintainer. upkeep-notify.yml then posts the notice.
export const REVIEW_LABEL = "upkeep: needs review";
/// The labels of the upkeep. `issue.ts write` creates each one that does not exist.
export const LABELS = [
  { name: ISSUE_LABEL, color: "5319e7", description: "Weekly upkeep tracking issue" },
  { name: READY_LABEL, color: "0e8a16", description: "Upkeep PR is ready to merge" },
  { name: REVIEW_LABEL, color: "d93f0b", description: "Upkeep issue has new items for the maintainer to review" },
];
const COLLABORATOR = new Set(["OWNER", "MEMBER", "COLLABORATOR"]);
const MARKER_LINE = /^[ \t]*<!-- upkeep:(last-run|reported-advisories|overdue-notified)\b[^>]*-->[ \t]*\n?/gm;

export type Trust = "maintainer" | "collaborator" | "data";
export type Comment = { id: number; author: string; association: string; trust: Trust; createdAt: string; body: string };
/// `number` is null when no tracking issue exists yet (before the first run).
export type Issue = { number: number | null; body: string; markers: Markers; comments: Comment[] };

export function trustOf(author: { login: string; type: string; association: string }, maintainer: string): Trust {
  if (author.type === "Bot" || author.login.endsWith("[bot]")) return "data";
  if (author.login.toLowerCase() === maintainer.toLowerCase()) return "maintainer";
  if (COLLABORATOR.has(author.association)) return "collaborator";
  return "data";
}

/// The maintainer login: UPKEEP_MAINTAINER from the environment, or else the repository variable.
export function maintainerLogin(client: Client): string {
  const fromEnv = process.env.UPKEEP_MAINTAINER?.trim();
  if (fromEnv) return fromEnv;
  try {
    const value = (client.call({ path: `repos/${client.repo}/actions/variables/UPKEEP_MAINTAINER` }) as { value?: string }).value?.trim();
    if (value) return value;
  } catch (error) {
    if (errorStatus(error) !== 404) throw error;
  }
  throw new Error("UPKEEP_MAINTAINER is not set in the environment or as a repository variable");
}

/// The number of the one open issue with the label `upkeep`, or null when there is none.
export function findIssue(client: Client): number | null {
  const issues = allPages<any>(client, `repos/${client.repo}/issues?state=open&labels=${encodeURIComponent(ISSUE_LABEL)}`).filter(
    (item) => !item.pull_request,
  );
  if (issues.length > 1) throw new Error(`expected one open issue with the label ${ISSUE_LABEL}, found ${issues.length}`);
  return issues[0]?.number ?? null;
}

/// Creates each upkeep label that does not exist. GitHub answers 422 for a label that exists.
export function ensureLabels(client: Client): void {
  for (const label of LABELS) {
    try {
      client.call({ method: "POST", path: `repos/${client.repo}/labels`, body: label });
    } catch (error) {
      if (errorStatus(error) !== 422) throw error;
    }
  }
}

export function readIssue(client: Client, number = findIssue(client)): Issue {
  const maintainer = maintainerLogin(client);
  if (number === null) return { number: null, body: "", markers: readMarkers(""), comments: [] };
  const issue = client.call({ path: `repos/${client.repo}/issues/${number}` }) as any;
  const body: string = issue.body ?? "";
  const comments = allPages<any>(client, `repos/${client.repo}/issues/${number}/comments`).map((c) => ({
    id: c.id,
    author: c.user.login,
    association: c.author_association,
    trust: trustOf({ login: c.user.login, type: c.user.type, association: c.author_association }, maintainer),
    createdAt: c.created_at,
    body: c.body ?? "",
  }));
  return { number, body, markers: readMarkers(body), comments };
}

/// Replaces the issue body with `body`, and returns the body that it wrote.
/// `lastRun` is the date of this successful run, or null to keep the date of
/// the current body. The other markers come from the current body.
/// When no tracking issue exists, it creates the issue with the label `upkeep`.
export function writeIssue(client: Client, body: string, options: { lastRun: string | null; number?: number }): string {
  const number = options.number ?? findIssue(client);
  if (number === null) {
    const written = writeMarkers(body.replace(MARKER_LINE, ""), { lastRun: options.lastRun, reportedAdvisories: [] });
    client.call({ method: "POST", path: `repos/${client.repo}/issues`, body: { title: ISSUE_TITLE, body: written, labels: [ISSUE_LABEL] } });
    return written;
  }
  const current = readMarkers(((client.call({ path: `repos/${client.repo}/issues/${number}` }) as any).body ?? "") as string);
  const markers: Markers = {
    lastRun: options.lastRun ?? current.lastRun,
    reportedAdvisories: current.reportedAdvisories,
    overdueNotified: current.overdueNotified,
  };
  const written = writeMarkers(body.replace(MARKER_LINE, ""), markers);
  client.call({ method: "PATCH", path: `repos/${client.repo}/issues/${number}`, body: { body: written } });
  return written;
}

function usage(): never {
  console.error("usage: issue.ts read [--number <n>]\n       issue.ts write <body file> [--number <n>] [--keep-last-run]");
  process.exit(2);
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const command = args.shift();
  let number: number | undefined;
  let keepLastRun = false;
  let file: string | undefined;
  while (args.length > 0) {
    const arg = args.shift()!;
    if (arg === "--number") number = Number(args.shift());
    else if (arg === "--keep-last-run" && command === "write") keepLastRun = true;
    else if (command === "write" && file === undefined && !arg.startsWith("--")) file = arg;
    else usage();
  }
  if (number !== undefined && !Number.isInteger(number)) usage();
  try {
    const client = ghClient();
    if (command === "read") {
      console.log(JSON.stringify(readIssue(client, number), null, 2));
    } else if (command === "write" && file !== undefined) {
      const lastRun = keepLastRun ? null : new Date().toISOString().slice(0, 10);
      ensureLabels(client);
      writeIssue(client, readFileSync(file, "utf8"), { lastRun, number });
      console.log("issue body written");
    } else {
      usage();
    }
  } catch (error) {
    console.error((error as Error).message);
    process.exit(1);
  }
}
