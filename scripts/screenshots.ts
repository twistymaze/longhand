// Render the website screenshots (U3 of the landing page plan): each scene
// of src/screenshots in WebKit, the engine of the app's web view, at 2x
// with a transparent background, into site/public/screenshots/. Run it on a
// Mac, so the app's system font is the real one:
//
//   npm run screenshots
//
// It needs the WebKit build of playwright-core. If it is missing, run
// `npx playwright-core install webkit` once.

import { mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { webkit } from "playwright-core";
import { createServer } from "vite";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "site", "public", "screenshots");
const SCENES = ["recording", "summary", "library"] as const;

/// The text that shows when each scene is drawn. Each text comes from the
/// last command result that the scene loads, so the capture never shows a
/// loading state: the notes, the summary, and the summary preview.
const READY: Record<(typeof SCENES)[number], string> = {
  recording: "Weekly team meeting",
  summary: "Card update flow",
  library: "The team met without Ben.",
};

async function main() {
  mkdirSync(OUT, { recursive: true });
  // The preview page loads vite's dev server; port 0 takes a free port.
  const server = await createServer({ root: ROOT, server: { port: 0, strictPort: false }, logLevel: "error" });
  await server.listen();
  const address = server.resolvedUrls?.local[0];
  if (!address) throw new Error("the vite server did not report its address");
  const browser = await webkit.launch();
  try {
    // Each scene in light mode (<scene>.png) and dark mode (<scene>-dark.png),
    // so the page in dark mode shows the app in dark mode.
    for (const scheme of ["light", "dark"] as const) {
      const page = await browser.newPage({ viewport: { width: 1200, height: 900 }, deviceScaleFactor: 2, colorScheme: scheme });
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      page.on("console", (message) => {
        if (message.type() === "error") errors.push(message.text());
      });
      for (const scene of SCENES) {
        errors.length = 0;
        await page.goto(`${address}src/screenshots/index.html?scene=${scene}`);
        await page.getByText(READY[scene], { exact: false }).first().waitFor();
        // Fonts and images finish before the capture.
        await page.evaluate("document.fonts.ready");
        if (errors.length > 0) throw new Error(`scene ${scene} (${scheme}): ${errors.join("; ")}`);
        const file = join(OUT, scheme === "light" ? `${scene}.png` : `${scene}-dark.png`);
        await page.locator("[data-screenshot]").screenshot({ path: file, omitBackground: true });
        console.log(`wrote ${file}`);
      }
      await page.close();
    }
  } finally {
    await browser.close();
    await server.close();
  }
}

if (import.meta.main) await main();
