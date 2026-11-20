// The count of `map_err(|e| e.to_string())` in the Tauri commands. Each one
// sends the text of an internal error to the UI, so the user can get two
// messages or a message with no action
// (docs/solutions/ui-bugs/one-capture-failure-gave-two-messages.md). The count
// can only go down (rules/command-error-text.toml).
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

// The count on 2026-11-20. Lower it when a change removes some.
const ALLOWED = 63;
const PATTERN = "map_err(|e| e.to_string())";
const DIR = join(import.meta.dirname, "../src-tauri/src/commands");

function count(): number {
  return readdirSync(DIR, { recursive: true, encoding: "utf8" })
    .filter((f) => f.endsWith(".rs"))
    .reduce((sum, f) => sum + readFileSync(join(DIR, f), "utf8").split(PATTERN).length - 1, 0);
}

test("the commands do not add internal error text for the UI", () => {
  const found = count();
  assert.ok(
    found <= ALLOWED,
    `${found - ALLOWED} new ${PATTERN} in src-tauri/src/commands: an internal error text must not go to the UI. ` +
      "Map the error to a message that tells the user what to do.",
  );
  assert.equal(found, ALLOWED, `the count went down to ${found}: lower ALLOWED in scripts/command-errors.test.ts to ${found}`);
});
