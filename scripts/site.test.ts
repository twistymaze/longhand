import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { changelog, type ContextRelease } from "./lib/changelog.ts";
import { buildSite } from "./site.ts";

const SITE = join(import.meta.dirname, "..", "site");

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

// Covers AE1: a page-only deploy keeps the release's changelog page.
test("the site has the landing page at its root and the changelog page from changelog.json", () => {
  const out = mkdtempSync(join(tmpdir(), "site-out-"));
  buildSite(SITE, sampleChangelog(), out);
  assert.equal(readFileSync(join(out, "index.html"), "utf8"), readFileSync(join(SITE, "index.html"), "utf8"));
  const page = readFileSync(join(out, "changelog", "index.html"), "utf8");
  assert.ok(page.includes("<h2>October 2026</h2>"));
  assert.ok(page.includes('<a href="../">Longhand</a>'));
});

test("the site has the stylesheet, the icon and the three screenshots", () => {
  const out = mkdtempSync(join(tmpdir(), "site-out-"));
  buildSite(SITE, sampleChangelog(), out);
  for (const file of ["style.css", "icon.png", "screenshots/recording.png", "screenshots/summary.png", "screenshots/library.png", "screenshots/recording-dark.png", "screenshots/summary-dark.png", "screenshots/library-dark.png"]) {
    assert.ok(existsSync(join(out, file)), file);
  }
});

test("a missing changelog.json stops the build with its path", () => {
  const out = mkdtempSync(join(tmpdir(), "site-out-"));
  assert.throws(() => buildSite(SITE, "/no/such/changelog.json", out), /\/no\/such\/changelog\.json/);
});

// Covers AE3: the page loads nothing from other sites.
test("every file that the landing page loads is in the site", () => {
  const page = readFileSync(join(SITE, "index.html"), "utf8");
  const loaded = [...page.matchAll(/<(?:img|script|link)\b[^>]*\b(?:src|href)="([^"]+)"/g)].map((m) => m[1]);
  assert.ok(loaded.length > 0);
  for (const path of loaded) {
    assert.ok(!/^[a-z]+:|^\/\//i.test(path), `loads ${path} from another site`);
    assert.ok(existsSync(join(SITE, path)), `${path} is not in the site`);
  }
  const css = readFileSync(join(SITE, "style.css"), "utf8");
  assert.ok(!/url\(|@import/.test(css), "the stylesheet loads another file");
});
