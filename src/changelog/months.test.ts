import { describe, expect, test } from "vitest";
import type { ChangelogEntry } from "../bindings";
import { byMonth, prAddress, recentMonths } from "./months";

let next = 0;
/// An entry in the month `month`. The group rank follows the order of
/// cliff.toml: New is 0, Fixes is 1.
function entry(month: string, line: string, group = "New", pr: number | null = null): ChangelogEntry {
  next += 1;
  return { id: `id-${next}`, time: next, month, group, rank: group === "New" ? 0 : 1, line, pr };
}

const names = (entries: ChangelogEntry[]) => recentMonths(entries).map((m) => m.name);

describe("recentMonths", () => {
  // Covers AE8.
  test("entries from August 2026 to January 2027 give 6 months, January 2027 first", () => {
    const entries = ["2026-08", "2026-09", "2026-10", "2026-11", "2026-12", "2027-01"].map((m) => entry(m, `Change ${m}`));
    expect(names(entries)).toEqual([
      "January 2027",
      "December 2026",
      "November 2026",
      "October 2026",
      "September 2026",
      "August 2026",
    ]);
  });

  test("entries over 8 calendar months give only the newest 6 calendar months", () => {
    const entries = ["2026-03", "2026-04", "2026-05", "2026-06", "2026-07", "2026-08", "2026-09", "2026-10"].map((m) =>
      entry(m, `Change ${m}`),
    );
    expect(names(entries)).toEqual([
      "October 2026",
      "September 2026",
      "August 2026",
      "July 2026",
      "June 2026",
      "May 2026",
    ]);
  });

  test("a calendar month with no entries gets no heading, and still counts", () => {
    const entries = [entry("2026-04", "Old"), entry("2026-10", "October"), entry("2027-01", "January")];
    expect(names(entries)).toEqual(["January 2027", "October 2026"]);
  });

  test("no entries give no months", () => {
    expect(recentMonths([])).toEqual([]);
  });
});

describe("byMonth", () => {
  test("puts the groups in rank order and the lines newest first", () => {
    const entries = [
      entry("2026-10", "First fix", "Fixes"),
      entry("2026-10", "First new"),
      entry("2026-10", "Second fix", "Fixes"),
      entry("2026-09", "September new"),
    ];
    const months = byMonth(entries);
    expect(months.map((m) => m.month)).toEqual(["2026-10", "2026-09"]);
    expect(months[0].groups.map((g) => [g.group, g.entries.map((e) => e.line)])).toEqual([
      ["New", ["First new"]],
      ["Fixes", ["Second fix", "First fix"]],
    ]);
  });

  test("a month with fixes only has only the Fixes group", () => {
    const months = byMonth([entry("2026-10", "A fix", "Fixes")]);
    expect(months[0].groups.map((g) => g.group)).toEqual(["Fixes"]);
  });
});

describe("prAddress", () => {
  test("builds the PR address from the number", () => {
    expect(prAddress(72)).toBe("https://github.com/twistymaze/longhand/pull/72");
  });
});
