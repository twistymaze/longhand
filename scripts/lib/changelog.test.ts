import assert from "node:assert/strict";
import { test } from "node:test";
import { changelog, html, lineText, markdown, monthKey, monthName, prNumber, releaseNotes, type ContextRelease } from "./changelog.ts";

const FEAT = "<!-- 0 -->New features and improvements";
const FIX = "<!-- 1 -->Fixes";
const OTHER = "<!-- 2 -->Other changes";

/// 31 October 2026 23:30 UTC and 1 November 2026 00:10 UTC.
const OCT_31_2330 = Date.UTC(2026, 9, 31, 23, 30) / 1000;
const NOV_1_0010 = Date.UTC(2026, 10, 1, 0, 10) / 1000;

function commit(id: string, group: string, message: string, time: number, scope: string | null = null) {
  return { id, group, message, scope, committer: { timestamp: time } };
}

test("a commit at 23:30 UTC on 31 October is in October, and one at 00:10 UTC on 1 November is in November", () => {
  assert.equal(monthKey(OCT_31_2330), "2026-10");
  assert.equal(monthKey(NOV_1_0010), "2026-11");
  assert.equal(monthName("2026-11"), "November 2026");
});

test("a line is scope, colon and the message with an upper-case first letter", () => {
  assert.equal(lineText(commit("a", FEAT, "record system audio (#4)", 0, "capture")), "capture: Record system audio (#4)");
  assert.equal(lineText(commit("a", FIX, "two", 0)), "Two");
});

test("a trailing (#72) gives PR 72, and a line with no reference gives none", () => {
  assert.equal(prNumber("import: Add a notes field (#72)"), 72);
  assert.equal(prNumber("Faster builds"), null);
  assert.equal(prNumber("Fix #3 in the middle"), null);
});

test("the changelog has the head, the format and the entries oldest first in the order of main", () => {
  // git-cliff gives the newest release first, and its commits oldest first.
  const releases: ContextRelease[] = [
    { commits: [commit("c3", FIX, "three", 300)] },
    { commits: [commit("c1", FEAT, "one (#1)", 100), commit("c2", OTHER, "two", 200, "deps")] },
  ];
  const data = changelog(releases, { id: "head", time: 400 });
  assert.ok(data.available);
  assert.equal(data.format, 1);
  assert.deepEqual(data.head, { id: "head", time: 400 });
  assert.deepEqual(
    data.entries.map((entry) => [entry.id, entry.group, entry.rank, entry.line, entry.pr]),
    [
      ["c1", "New features and improvements", 0, "One (#1)", 1],
      ["c2", "Other changes", 2, "deps: Two", null],
      ["c3", "Fixes", 1, "Three", null],
    ],
  );
});

test("commits with no group do not appear", () => {
  const data = changelog([{ commits: [{ id: "x", message: "skipped", group: null, committer: { timestamp: 0 } }] }], { id: "h", time: 0 });
  assert.ok(data.available);
  assert.deepEqual(data.entries, []);
});

test("the Markdown puts months newest first, headings in order and the PR as a link", () => {
  const releases: ContextRelease[] = [
    {
      commits: [
        commit("a", FIX, "keep the timer visible (#3)", OCT_31_2330, "ui"),
        commit("b", FEAT, "add note templates", OCT_31_2330),
        commit("c", FIX, "load faster", NOV_1_0010),
      ],
    },
  ];
  const text = markdown(changelog(releases, { id: "h", time: NOV_1_0010 }));
  assert.ok(text.indexOf("## November 2026") < text.indexOf("## October 2026"));
  const october = text.slice(text.indexOf("## October 2026"));
  assert.ok(october.indexOf("### New features and improvements") < october.indexOf("### Fixes"));
  assert.ok(october.includes("- ui: Keep the timer visible ([#3](https://github.com/twistymaze/longhand/pull/3))"));
  // November has only a fix, so it has no features heading.
  const november = text.slice(text.indexOf("## November 2026"), text.indexOf("## October 2026"));
  assert.ok(!november.includes("### New features"));
});

test("the page escapes commit text and links the PR, and the Markdown and the data keep the text", () => {
  const releases: ContextRelease[] = [{ commits: [commit("a", FIX, "stop <script> & \"quotes\" (#9)", OCT_31_2330)] }];
  const data = changelog(releases, { id: "h", time: 0 });
  const page = html(data);
  assert.ok(page.includes("Stop &lt;script&gt; &amp; &quot;quotes&quot; (<a href=\"https://github.com/twistymaze/longhand/pull/9\">#9</a>)"));
  assert.ok(!page.includes("<script>"));
  assert.ok(markdown(data).includes("Stop <script> & \"quotes\""));
  assert.ok(data.available && data.entries[0].line === "Stop <script> & \"quotes\" (#9)");
});

test("the release notes put each group under its heading with a blank line between", () => {
  const releases: ContextRelease[] = [
    { commits: [commit("a", OTHER, "update tauri", 0, "deps"), commit("b", FEAT, "add notes", 0), commit("c", FEAT, "more", 0)] },
  ];
  assert.equal(
    releaseNotes(releases),
    "### New features and improvements\n\n- Add notes\n- More\n\n### Other changes\n\n- deps: Update tauri\n",
  );
});
