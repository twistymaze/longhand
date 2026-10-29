import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

// The light and dark mode script of the website (site/public/theme.js). It
// runs here in a small stand-in for the page: the html element, the button
// and its label, localStorage and the system mode.
const SCRIPT = readFileSync(join(import.meta.dirname, "..", "site", "public", "theme.js"), "utf8");

function page(options: { systemDark?: boolean; stored?: string | null; storage?: "works" | "fails" } = {}) {
  const store = new Map<string, string>();
  if (options.stored) store.set("longhand-theme", options.stored);
  const fails = options.storage === "fails";
  const localStorage = {
    getItem: (key: string) => {
      if (fails) throw new Error("storage is blocked");
      return store.get(key) ?? null;
    },
    setItem: (key: string, value: string) => {
      if (fails) throw new Error("storage is blocked");
      store.set(key, value);
    },
  };
  const label = { textContent: "" };
  const clicks: (() => void)[] = [];
  const button = {
    hidden: true,
    title: "",
    querySelector: () => label,
    addEventListener: (_: string, listener: () => void) => clicks.push(listener),
  };
  const ready: (() => void)[] = [];
  const root = { dataset: {} as Record<string, string> };
  const document = {
    documentElement: root,
    querySelector: () => button,
    addEventListener: (_: string, listener: () => void) => ready.push(listener),
  };
  const matchMedia = () => ({ matches: options.systemDark ?? false, addEventListener: () => {} });
  runInNewContext(SCRIPT, { document, localStorage, window: { matchMedia } });
  return {
    root,
    button,
    label,
    store,
    load: () => ready.forEach((listener) => listener()),
    click: () => clicks.forEach((listener) => listener()),
  };
}

test("the page follows the system mode until the visitor chooses one", () => {
  const p = page({ systemDark: true });
  p.load();
  assert.equal(p.root.dataset.theme, undefined);
  assert.equal(p.label.textContent, "Light mode");
  assert.equal(p.button.hidden, false);
});

test("a kept mode applies before the page loads", () => {
  const p = page({ stored: "dark" });
  assert.equal(p.root.dataset.theme, "dark");
});

test("a click changes to the other mode and keeps it", () => {
  const p = page();
  p.load();
  p.click();
  assert.equal(p.root.dataset.theme, "dark");
  assert.equal(p.store.get("longhand-theme"), "dark");
  assert.equal(p.label.textContent, "Light mode");
  p.click();
  assert.equal(p.root.dataset.theme, "light");
  assert.equal(p.store.get("longhand-theme"), "light");
});

test("blocked storage does not stop the button", () => {
  const p = page({ storage: "fails" });
  p.load();
  p.click();
  assert.equal(p.root.dataset.theme, "dark");
});

test("a stored value that is not a mode is ignored", () => {
  const p = page({ stored: "blue" });
  assert.equal(p.root.dataset.theme, undefined);
});
