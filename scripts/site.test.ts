import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { before, test } from "node:test";
import { changelog, type ContextRelease } from "./lib/changelog.ts";
import { buildSite, SITE } from "./site.ts";

function sampleChangelog(): string {
  const releases: ContextRelease[] = [
    {
      commits: [
        {
          id: "a",
          group: "<!-- 1 -->Fixes",
          message: "keep the timer visible (#3)",
          scope: null,
          committer: { timestamp: Date.UTC(2026, 9, 12) / 1000 },
        },
      ],
    },
  ];
  const file = join(mkdtempSync(join(tmpdir(), "site-test-")), "changelog.json");
  writeFileSync(file, JSON.stringify(changelog(releases, { id: "h", time: 0 })));
  return file;
}

// One build for the tests that read its files.
let out = "";
before(async () => {
  out = mkdtempSync(join(tmpdir(), "site-out-"));
  await buildSite(SITE, sampleChangelog(), out);
});

const read = (path: string) => readFileSync(join(out, path), "utf8");

/// The part of a page from `start` to `end`, both included.
function part(page: string, start: string, end: string): string {
  const from = page.indexOf(start);
  assert.ok(from >= 0, `the page has no ${start}`);
  return page.slice(from, page.indexOf(end, from) + end.length);
}

// Covers AE1: a page-only deploy keeps the release's changelog page.
test("the site has the landing page at its root and the changelog page from changelog.json", () => {
  assert.ok(read("index.html").includes("Take part in the meeting."));
  const page = read("changelog/index.html");
  assert.ok(page.includes("<h2>October 2026</h2>"));
  assert.ok(page.includes("<h3>Fixes</h3>"));
});

test("the two pages share the head files, the bar and the footer", () => {
  const landing = read("index.html");
  const changes = read("changelog/index.html");
  for (const [start, end] of [['<header class="bar">', "</header>"], ["<footer>", "</footer>"]]) {
    assert.equal(part(changes, start, end), part(landing, start, end));
  }
  for (const page of [landing, changes]) {
    assert.ok(page.includes('<script src="/theme.js"></script>'));
    assert.match(page, /<link rel="stylesheet"[^>]*href="\/assets\/style-[^"]+\.css">/);
    assert.ok(!page.includes("<!-- include:"), "a partial is not in the page");
  }
});

test("the site has the icon, the mode script and the six screenshots", () => {
  for (const file of ["theme.js", "icon.png"]) assert.ok(existsSync(join(out, file)), file);
  const screenshots = readdirSync(join(out, "screenshots")).sort();
  assert.deepEqual(screenshots, ["library-dark.png", "library.png", "recording-dark.png", "recording.png", "summary-dark.png", "summary.png"]);
});

test("a missing changelog.json stops the build with its path", async () => {
  const other = mkdtempSync(join(tmpdir(), "site-out-"));
  await assert.rejects(buildSite(SITE, "/no/such/changelog.json", other), /\/no\/such\/changelog\.json/);
});

// Covers AE3: the pages load nothing from other sites.
test("every file that a page loads is in the site", () => {
  for (const name of ["index.html", "changelog/index.html"]) {
    const page = read(name);
    const loaded = [...page.matchAll(/<(?:img|script|link)\b[^>]*\b(?:src|href)="([^"]+)"/g)].map((m) => m[1]);
    assert.ok(loaded.length > 0);
    for (const path of loaded) {
      assert.ok(!/^[a-z]+:|^\/\//i.test(path), `${name} loads ${path} from another site`);
      assert.ok(existsSync(join(out, path)), `${name} loads ${path}, which is not in the site`);
    }
  }
  const css = readFileSync(join(SITE, "style.css"), "utf8");
  assert.ok(!/url\(|@import/.test(css), "the stylesheet loads another file");
});

const MAC_URL = "https://github.com/twistymaze/longhand/releases/latest/download/Longhand_aarch64.dmg";
const WINDOWS_URL = "https://github.com/twistymaze/longhand/releases/latest/download/Longhand_x64-setup.exe";

/// The download links of one group, in page order.
function links(group: string): string[] {
  return [...group.matchAll(/<a class="download" href="([^"]+)">/g)].map((m) => m[1]);
}

test("the landing page loads the platform script before the page shows", () => {
  assert.ok(read("index.html").includes('<script src="/platform.js"></script>'));
  assert.ok(existsSync(join(out, "platform.js")));
});

// Covers AE1, AE2 and AE3: each download place has the same two links, in
// a Mac-first group and in a Windows-first group.
test("both download places link to the installers of the latest release", () => {
  const page = read("index.html");
  assert.ok(!/coming soon/i.test(page), "the page says Coming soon");
  for (const [start, end] of [['<div class="get">', "</p>"], ['<section class="last">', "</section>"]]) {
    const place = part(page, start, end);
    const macFirst = part(place, '<div class="downloads mac-first">', "</div>");
    const windowsFirst = part(place, '<div class="downloads windows-first">', "</div>");
    assert.deepEqual(links(macFirst), [MAC_URL, WINDOWS_URL], `${start} Mac-first group`);
    assert.deepEqual(links(windowsFirst), [WINDOWS_URL, MAC_URL], `${start} Windows-first group`);
  }
});
