// Writes the website for GitHub Pages (U5 of the landing page plan): the
// landing page from site/ at the root, and the changelog page from a
// changelog.json at /changelog/.
//
//   node scripts/site.ts <changelog.json> <folder>
//
// The Pages job runs it with the changelog.json of the release it deploys,
// or of the latest release when only the page changed. Run it from any
// folder. It uses Node's built-in modules only, so it needs no `npm ci`.

import { cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { html, type Changelog } from "./lib/changelog.ts";

/// The landing page source.
export const SITE = join(import.meta.dirname, "..", "site");

/// Copies `site` into `out` and writes `out/changelog/index.html` from the
/// changelog file.
export function buildSite(site: string, changelogFile: string, out: string): void {
  if (!existsSync(changelogFile)) throw new Error(`site: no changelog file at ${changelogFile}`);
  const data = JSON.parse(readFileSync(changelogFile, "utf8")) as Changelog;
  mkdirSync(out, { recursive: true });
  cpSync(site, out, { recursive: true });
  mkdirSync(join(out, "changelog"), { recursive: true });
  writeFileSync(join(out, "changelog", "index.html"), html(data));
}

if (import.meta.main) {
  const [changelogFile, out, ...rest] = process.argv.slice(2);
  if (!changelogFile || !out || rest.length > 0) {
    console.error("usage: site.ts <changelog.json> <folder>");
    process.exit(2);
  }
  buildSite(SITE, changelogFile, out);
  console.log(`site: wrote ${out}`);
}
