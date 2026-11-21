import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { isOpenableHref } from "./links";

describe("links", () => {
  test("only web and mail addresses open", () => {
    for (const href of ["https://example.com", "http://example.com", "mailto:ana@example.com", " HTTPS://x.org"]) {
      expect(isOpenableHref(href), href).toBe(true);
    }
    for (const href of ["file:///etc/hosts", "javascript:alert(1)", "longhand://x", "example.com", ""]) {
      expect(isOpenableHref(href), href).toBe(false);
    }
  });

  // The opener scope of each window, by capability file.
  const dir = `${import.meta.dirname}/../../src-tauri/capabilities`;
  const openerScopes = Object.fromEntries(
    readdirSync(dir)
      .filter((f) => f.endsWith(".json"))
      .map((file) => {
        const capability = JSON.parse(readFileSync(`${dir}/${file}`, "utf8"));
        const opener = capability.permissions.find(
          (p: unknown) => typeof p === "object" && (p as { identifier: string }).identifier === "opener:allow-open-url",
        );
        return [file, opener ? (opener.allow.map((a: { url: string }) => a.url) as string[]) : []];
      }),
  );

  test("the main window's opener allows the same schemes as isOpenableHref, and no other", () => {
    expect([...openerScopes["default.json"]].sort()).toEqual(["http://*", "https://*", "mailto:*"]);
  });

  test("no window's opener allows a scheme that isOpenableHref refuses", () => {
    for (const [file, urls] of Object.entries(openerScopes)) {
      for (const url of urls) expect(isOpenableHref(url.replace(/\*/g, "x")), `${file}: opener scope ${url}`).toBe(true);
    }
  });
});
