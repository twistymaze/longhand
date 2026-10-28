import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Changelog, ChangelogEntry } from "../bindings";
import { mockTauri } from "../test/tauri";
import { PAGES_URL, prAddress } from "./months";
import { RecentChangesWindow } from "./RecentChangesWindow";

let next = 0;
function entry(month: string, line: string, group = "New", pr: number | null = null): ChangelogEntry {
  next += 1;
  return { id: `id-${next}`, time: next, month, group, rank: group === "New" ? 0 : 1, line, pr };
}

const available = (entries: ChangelogEntry[]): Changelog => ({
  available: true,
  head: { id: "head", time: 0 },
  entries,
});

function mock(changelog: Changelog) {
  return mockTauri({ changelog: () => changelog, "plugin:opener|open_url": () => null });
}

const opened = (tauri: ReturnType<typeof mock>) =>
  tauri.calls.filter((c) => c.cmd === "plugin:opener|open_url").map((c) => c.args.url);

const headings = (level: number) => screen.queryAllByRole("heading", { level }).map((h) => h.textContent);

describe("RecentChangesWindow", () => {
  // Covers AE8: the months come from the data, not from today's date.
  test("entries from August 2026 to January 2027 show 6 months, January 2027 first", async () => {
    const months = ["2026-08", "2026-09", "2026-10", "2026-11", "2026-12", "2027-01"];
    mock(available(months.map((m) => entry(m, `Change ${m}`))));
    render(<RecentChangesWindow />);
    await screen.findByRole("heading", { name: "January 2027" });
    expect(headings(2)).toEqual([
      "January 2027",
      "December 2026",
      "November 2026",
      "October 2026",
      "September 2026",
      "August 2026",
    ]);
  });

  test("entries over 8 calendar months show the newest 6 and the full-changelog link", async () => {
    const months = ["2026-03", "2026-04", "2026-05", "2026-06", "2026-07", "2026-08", "2026-09", "2026-10"];
    mock(available(months.map((m) => entry(m, `Change ${m}`))));
    render(<RecentChangesWindow />);
    await screen.findByRole("heading", { name: "October 2026" });
    expect(headings(2)).toHaveLength(6);
    expect(screen.queryByText("Change 2026-04")).toBeNull();
    expect(screen.getByRole("button", { name: /Full changelog/ })).toBeTruthy();
  });

  test("entries in January 2027 and October 2026 show both months and no heading between them", async () => {
    mock(available([entry("2026-10", "October change"), entry("2027-01", "January change")]));
    render(<RecentChangesWindow />);
    await screen.findByRole("heading", { name: "January 2027" });
    expect(headings(2)).toEqual(["January 2027", "October 2026"]);
  });

  test("a month with fixes only shows only the Fixes heading", async () => {
    mock(available([entry("2026-10", "A fix", "Fixes")]));
    render(<RecentChangesWindow />);
    await screen.findByRole("heading", { name: "October 2026" });
    expect(headings(3)).toEqual(["Fixes"]);
  });

  test("a click on a line with PR 72 opens the PR once", async () => {
    const tauri = mock(available([entry("2026-10", "capture: Fix the timer (#72)", "Fixes", 72)]));
    render(<RecentChangesWindow />);
    await userEvent.setup().click(await screen.findByRole("button", { name: /Fix the timer/ }));
    await waitFor(() => expect(opened(tauri)).toEqual(["https://github.com/twistymaze/longhand/pull/72"]));
  });

  test("a line with no PR is plain text, with no link and no button", async () => {
    mock(available([entry("2026-10", "Add tags")]));
    render(<RecentChangesWindow />);
    const line = (await screen.findByText("Add tags")).closest("li") as HTMLElement;
    expect(within(line).queryByRole("link")).toBeNull();
    expect(within(line).queryByRole("button")).toBeNull();
  });

  // Covers AE7.
  test("a changelog that is not available shows only the full-changelog link", async () => {
    const tauri = mock({ available: false, head: null, entries: [] });
    render(<RecentChangesWindow />);
    const link = await screen.findByRole("button", { name: /Full changelog/ });
    expect(screen.queryAllByRole("heading")).toEqual([]);
    expect(screen.getAllByRole("button")).toHaveLength(1);
    await userEvent.setup().click(link);
    await waitFor(() => expect(opened(tauri)).toEqual(["https://twistymaze.github.io/longhand/"]));
  });

  test("the window shows no version number", async () => {
    mock(available([entry("2026-10", "Add tags"), entry("2026-10", "capture: Fix the timer (#72)", "Fixes", 72)]));
    render(<RecentChangesWindow />);
    await screen.findByText("Add tags");
    expect(document.body.textContent).not.toMatch(/\bv?\d+\.\d+\.\d+\b/);
    expect(document.body.textContent).not.toMatch(/version/i);
  });

  test("the window opener allows only the PR pages and the full changelog page", () => {
    const capability = JSON.parse(
      readFileSync(`${import.meta.dirname}/../../src-tauri/capabilities/changelog.json`, "utf8"),
    );
    expect(capability.windows).toEqual(["changelog"]);
    const opener = capability.permissions.find(
      (p: unknown) => typeof p === "object" && (p as { identifier: string }).identifier === "opener:allow-open-url",
    );
    const allowed: string[] = opener.allow.map((a: { url: string }) => a.url).sort();
    expect(allowed).toEqual([
      "https://github.com/twistymaze/longhand/pull/*",
      "https://twistymaze.github.io/longhand/*",
    ]);
    // The scope is a glob where * also matches an empty text and /. Each
    // address that the window opens must match one pattern.
    const matches = (url: string) =>
      allowed.some((pattern) => new RegExp(`^${pattern.replace(/[.?+^$()|[\]\\]/g, "\\$&").replace(/\*/g, ".*")}$`).test(url));
    expect(matches(PAGES_URL)).toBe(true);
    expect(matches(prAddress(72))).toBe(true);
  });
});
