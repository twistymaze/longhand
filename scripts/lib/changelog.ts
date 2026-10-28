// The one renderer for the release notes and the changelog (KTD1 of the
// monthly changelog plan). git-cliff applies the rules in cliff.toml and gives
// its result as JSON (`git-cliff --context`). This module writes every text
// from that JSON, so the release notes and the changelog always agree:
//
// - the release notes of one release (by version, as before),
// - `changelog.json`, the data that the app and the update window read (KTD2),
// - `CHANGELOG.md`, the full changelog by month,
// - `index.html`, the GitHub Pages page.
//
// The module uses only Node built-ins, because the release jobs run no
// package install.

export const REPOSITORY = "twistymaze/longhand";
/// The changelog page. Builds keep this address, so it stays on github.io:
/// GitHub sends it on to the custom domain that the Pages settings name. A
/// change of that domain needs no change here (docs/releases.md, setup step 5).
export const PAGES_URL = "https://twistymaze.github.io/longhand/changelog/";

/// The format of `changelog.json`. A reader that gets another number treats
/// the changelog as not available. Change it only with the readers in
/// src-tauri/src/commands/changelog.rs.
export const FORMAT = 1;

/// One commit in the JSON of `git-cliff --context`, after the cliff.toml
/// rules. Only the fields that this module reads.
export type ContextCommit = {
  id: string;
  message: string;
  group?: string | null;
  scope?: string | null;
  committer: { timestamp: number };
};

/// One release in the JSON of `git-cliff --context`.
export type ContextRelease = { commits: ContextCommit[] };

/// One change in `changelog.json`.
export type Entry = {
  /// The full commit id on main.
  id: string;
  /// The commit time on main, in seconds since 1970 (UTC).
  time: number;
  /// The month of the change, `YYYY-MM`, in UTC.
  month: string;
  /// The heading of the group, for example "Fixes".
  group: string;
  /// The position of the group heading: 0 is the first heading.
  rank: number;
  /// The line text, the same as in the release notes.
  line: string;
  /// The PR number from a trailing `(#NN)`, or null.
  pr: number | null;
};

/// The commit that the changelog was made from.
export type Head = { id: string; time: number };

export type Changelog =
  | { format: typeof FORMAT; available: true; head: Head; entries: Entry[] }
  | { format: typeof FORMAT; available: false };

export const NOT_AVAILABLE: Changelog = { format: FORMAT, available: false };

const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];

/// A trailing `(#NN)` PR reference in a line.
const TRAILING_PR = /\(#(\d+)\)\s*$/;

/// The heading text of a git-cliff group: the text after the `<!-- N -->`
/// order prefix that cliff.toml puts first. This is not a sanitizer: `html`
/// escapes the heading.
function heading(group: string): string {
  const end = group.indexOf("-->");
  return (group.startsWith("<!--") && end >= 0 ? group.slice(end + 3) : group).trim();
}

/// The first character in upper case. The other characters do not change.
function upperFirst(text: string): string {
  return text.length === 0 ? text : text[0].toUpperCase() + text.slice(1);
}

/// The text of one line: `scope: Message`, as the release notes write it.
export function lineText(commit: ContextCommit): string {
  const message = upperFirst(commit.message);
  return commit.scope ? `${commit.scope}: ${message}` : message;
}

/// The PR number of a trailing `(#NN)`, or null.
export function prNumber(line: string): number | null {
  const match = TRAILING_PR.exec(line);
  return match ? Number(match[1]) : null;
}

/// The month key `YYYY-MM` of a time in seconds, in UTC.
export function monthKey(time: number): string {
  const date = new Date(time * 1000);
  return `${date.getUTCFullYear()}-${String(date.getUTCMonth() + 1).padStart(2, "0")}`;
}

/// The name of a month key, for example "October 2026".
export function monthName(month: string): string {
  const [year, number] = month.split("-");
  return `${MONTHS[Number(number) - 1]} ${year}`;
}

/// The commits that have a group, in their order. git-cliff gives the
/// releases newest first and the commits of a release oldest first
/// (`sort_commits = "oldest"`), so the result is oldest first.
function grouped(releases: ContextRelease[]): ContextCommit[] {
  return [...releases].reverse().flatMap((release) => release.commits.filter((commit) => commit.group));
}

/// The groups in heading order. git-cliff's `group_by` sorts the groups by
/// their text, and the `<!-- N -->` comment in cliff.toml sets that order.
function groupOrder(commits: ContextCommit[]): string[] {
  return [...new Set(commits.map((commit) => commit.group!))].sort();
}

/// The entries from the git-cliff JSON, oldest first.
function entriesOf(releases: ContextRelease[]): Entry[] {
  const commits = grouped(releases);
  const order = groupOrder(commits);
  return commits.map((commit) => {
    const line = lineText(commit);
    return {
      id: commit.id,
      time: commit.committer.timestamp,
      month: monthKey(commit.committer.timestamp),
      group: heading(commit.group!),
      rank: order.indexOf(commit.group!),
      line,
      pr: prNumber(line),
    };
  });
}

/// The changelog data from the git-cliff JSON and the head commit.
export function changelog(releases: ContextRelease[], head: Head): Changelog {
  return { format: FORMAT, available: true, head, entries: entriesOf(releases) };
}

/// The fields of a change that `byMonth` reads. Both `Entry` and the
/// frontend's `ChangelogEntry` have them.
type Groupable = { month: string; group: string; rank: number };

/// One group heading in a month, with its lines.
export type MonthGroup<E extends Groupable> = { group: string; rank: number; entries: E[] };

/// One month, for example "2026-10" with the name "October 2026".
export type Month<E extends Groupable> = { month: string; name: string; groups: MonthGroup<E>[] };

/// The entries under their group headings, in rank order. Each group takes
/// its name from its first line. The lines of a group stay in the order of
/// `entries`.
function byGroup<E extends Groupable>(entries: E[]): MonthGroup<E>[] {
  const ranks = [...new Set(entries.map((entry) => entry.rank))].sort((a, b) => a - b);
  return ranks.map((rank) => {
    const lines = entries.filter((entry) => entry.rank === rank);
    return { group: lines[0].group, rank, entries: lines };
  });
}

/// The entries by month, newest month first, then by group (`byGroup`). In a
/// group, the newest line is first. `entries` is oldest first. The release
/// notes keep their lines oldest first.
export function byMonth<E extends Groupable>(entries: E[]): Month<E>[] {
  const keys = [...new Set(entries.map((entry) => entry.month))].sort().reverse();
  return keys.map((month) => ({
    month,
    name: monthName(month),
    groups: byGroup(entries.filter((entry) => entry.month === month).reverse()),
  }));
}

/// The release notes of one release: the commits under their headings.
export function releaseNotes(releases: ContextRelease[]): string {
  return (
    byGroup(entriesOf(releases))
      .map((group) => `### ${group.group}\n\n${group.entries.map((entry) => `- ${entry.line}`).join("\n")}`)
      .join("\n\n") + "\n"
  );
}

function pullUrl(pr: number): string {
  return `https://github.com/${REPOSITORY}/pull/${pr}`;
}

/// A line for Markdown: the trailing `(#NN)` links to the PR.
function markdownLine(entry: Entry): string {
  if (entry.pr === null) return entry.line;
  return entry.line.replace(TRAILING_PR, `([#${entry.pr}](${pullUrl(entry.pr)}))`);
}

/// The full changelog as Markdown, newest month first.
export function markdown(data: Changelog): string {
  const parts = ["# Changelog", `The changes to Longhand by month. The latest version is also at ${PAGES_URL}.`];
  if (data.available) {
    for (const { name, groups } of byMonth(data.entries)) {
      parts.push(`## ${name}`);
      for (const group of groups) {
        parts.push(`### ${group.group}`, group.entries.map((entry) => `- ${markdownLine(entry)}`).join("\n"));
      }
    }
  }
  return parts.join("\n\n") + "\n";
}

function escape(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

/// A line for HTML: the trailing `(#NN)` links to the PR.
function htmlLine(entry: Entry): string {
  if (entry.pr === null) return escape(entry.line);
  const text = entry.line.replace(TRAILING_PR, "");
  return `${escape(text)}(<a href="${pullUrl(entry.pr)}">#${entry.pr}</a>)`;
}

/// The GitHub Pages page: one static file with no script.
export function html(data: Changelog): string {
  const body: string[] = [];
  if (data.available) {
    for (const { name, groups } of byMonth(data.entries)) {
      body.push(`<h2>${name}</h2>`);
      for (const group of groups) {
        body.push(`<h3>${escape(group.group)}</h3>`, "<ul>");
        for (const entry of group.entries) body.push(`<li>${htmlLine(entry)}</li>`);
        body.push("</ul>");
      }
    }
  }
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Longhand changelog</title>
<style>
:root { color-scheme: light dark; --text: #1d1d1f; --muted: #6e6e73; --link: #0066cc; --background: #ffffff; }
@media (prefers-color-scheme: dark) { :root { --text: #f5f5f7; --muted: #a1a1a6; --link: #4da3ff; --background: #1d1d1f; } }
body { margin: 0 auto; max-width: 44rem; padding: 2rem 1rem 4rem; font: 16px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; color: var(--text); background: var(--background); }
h1 { font-size: 1.75rem; margin: 0 0 0.25rem; }
h2 { font-size: 1.3rem; margin: 2.25rem 0 0.5rem; }
h3 { font-size: 0.95rem; margin: 1.25rem 0 0.25rem; }
ul { margin: 0; padding-left: 1.25rem; }
li { margin: 0.2rem 0; }
a { color: var(--link); }
p { color: var(--muted); margin: 0; }
</style>
</head>
<body>
<h1>Longhand changelog</h1>
<p>The changes to <a href="../">Longhand</a> by month.</p>
${body.join("\n")}
</body>
</html>
`;
}
