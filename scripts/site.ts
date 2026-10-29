// Writes the website for GitHub Pages (U5 of the landing page plan): the
// pages in site/ with their shared partials, and the changelog page with
// the months of a changelog.json. site/vite.config.ts does the build.
//
//   node scripts/site.ts <changelog.json> <folder>
//
// The Pages job runs it with the changelog.json of the release it deploys,
// or of the latest release when only the page changed. It needs the npm
// packages (`npm ci`).

import { existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { build } from "vite";

/// The website source.
export const SITE = join(import.meta.dirname, "..", "site");

/// Builds the site in `site` into `out`, with the changelog in
/// `changelogFile`.
export async function buildSite(site: string, changelogFile: string, out: string): Promise<void> {
  if (!existsSync(changelogFile)) throw new Error(`site: no changelog file at ${changelogFile}`);
  const before = process.env.SITE_CHANGELOG;
  process.env.SITE_CHANGELOG = resolve(changelogFile);
  try {
    await build({
      configFile: join(site, "vite.config.ts"),
      logLevel: "warn",
      build: { outDir: resolve(out), emptyOutDir: true },
    });
  } finally {
    if (before === undefined) delete process.env.SITE_CHANGELOG;
    else process.env.SITE_CHANGELOG = before;
  }
}

if (import.meta.main) {
  const [changelogFile, out, ...rest] = process.argv.slice(2);
  if (!changelogFile || !out || rest.length > 0) {
    console.error("usage: site.ts <changelog.json> <folder>");
    process.exit(2);
  }
  await buildSite(SITE, changelogFile, out);
  console.log(`site: wrote ${out}`);
}
