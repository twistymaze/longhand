import { readFileSync } from "node:fs";
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

  test("the main window's opener allows the same schemes as isOpenableHref, and no other", () => {
    const capability = JSON.parse(readFileSync(`${import.meta.dirname}/../../src-tauri/capabilities/default.json`, "utf8"));
    const opener = capability.permissions.find(
      (p: unknown) => typeof p === "object" && (p as { identifier: string }).identifier === "opener:allow-open-url",
    );
    expect(opener.allow.map((a: { url: string }) => a.url).sort()).toEqual(["http://*", "https://*", "mailto:*"]);
  });
});
