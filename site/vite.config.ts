// The website build (landing page plan, U5). Each page in this folder is an
// HTML file. The pages share the files in partials/: a comment
// `<!-- include:<name> -->` gets the content of partials/<name>.html. The
// changelog page gets the months of the changelog at `<!-- changelog -->`.
//
//   npm run site                                   the dev server
//   node scripts/site.ts <changelog.json> <folder>   the build
//
// A build needs SITE_CHANGELOG, the path of a release's changelog.json.
// scripts/site.ts sets it. The dev server makes the changelog from the git
// history of this checkout (git-cliff) when SITE_CHANGELOG is not set.

import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { defineConfig, type Plugin } from "vite";
import { readChangelog } from "../scripts/changelog.ts";
import { htmlMonths, type Changelog } from "../scripts/lib/changelog.ts";

const SITE = import.meta.dirname;
const REPOSITORY = resolve(SITE, "..");

function changelogData(command: "build" | "serve"): Changelog {
  const file = process.env.SITE_CHANGELOG;
  if (file) return JSON.parse(readFileSync(file, "utf8")) as Changelog;
  if (command === "build") throw new Error("site: set SITE_CHANGELOG to the path of a changelog.json");
  return readChangelog(REPOSITORY, join(REPOSITORY, "cliff.toml"));
}

function pages(command: "build" | "serve"): Plugin {
  let months: string | null = null;
  return {
    name: "longhand-site-pages",
    // Vite does not see the partials as page files, so a change to one
    // reloads the open page.
    handleHotUpdate({ file, server }) {
      if (file.startsWith(join(SITE, "partials"))) server.ws.send({ type: "full-reload" });
    },
    transformIndexHtml: {
      order: "pre",
      handler(html) {
        const page = html.replace(/<!-- include:([a-z]+) -->\n?/g, (_, name: string) =>
          readFileSync(join(SITE, "partials", `${name}.html`), "utf8"),
        );
        if (!page.includes("<!-- changelog -->")) return page;
        months ??= htmlMonths(changelogData(command));
        return page.replace("<!-- changelog -->", months);
      },
    },
  };
}

export default defineConfig(({ command }) => ({
  root: SITE,
  publicDir: "public",
  // The landing page is at the root of longhand.md.
  base: "/",
  plugins: [pages(command)],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      input: { main: join(SITE, "index.html"), changelog: join(SITE, "changelog", "index.html") },
    },
  },
}));
