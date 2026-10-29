import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

// The script that finds the operating system of the visitor
// (site/public/platform.js). It runs here in a small stand-in for the page:
// the html element and the navigator object.
const SCRIPT = readFileSync(join(import.meta.dirname, "..", "site", "public", "platform.js"), "utf8");

const MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
const IPHONE = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
const ANDROID = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36";
const LINUX = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

/// The value of data-platform after the script runs. Without `navigator`,
/// the page has no navigator object.
function platform(navigator?: { userAgent: string; platform: string; maxTouchPoints: number }): string | undefined {
  const root = { dataset: {} as Record<string, string> };
  const context: Record<string, unknown> = { document: { documentElement: root } };
  if (navigator) context.navigator = navigator;
  runInNewContext(SCRIPT, context);
  return root.dataset.platform;
}

// Covers AE1.
test("a Mac with no touch input is a Mac", () => {
  assert.equal(platform({ userAgent: MAC, platform: "MacIntel", maxTouchPoints: 0 }), "mac");
});

test("an iPad, which names a Mac in its user agent, is not a Mac", () => {
  assert.equal(platform({ userAgent: MAC, platform: "MacIntel", maxTouchPoints: 5 }), undefined);
});

// Covers AE2.
test("Windows is Windows", () => {
  assert.equal(platform({ userAgent: WINDOWS, platform: "Win32", maxTouchPoints: 0 }), "windows");
});

// Covers AE3.
test("an iPhone, Android and Linux set no platform", () => {
  assert.equal(platform({ userAgent: IPHONE, platform: "iPhone", maxTouchPoints: 5 }), undefined);
  assert.equal(platform({ userAgent: ANDROID, platform: "Linux armv81", maxTouchPoints: 5 }), undefined);
  assert.equal(platform({ userAgent: LINUX, platform: "Linux x86_64", maxTouchPoints: 0 }), undefined);
});

test("a page with no navigator sets no platform", () => {
  assert.equal(platform(), undefined);
});
