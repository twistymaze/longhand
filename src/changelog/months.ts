// The changelog entries by month and by group (R11, KTD3 of the monthly
// changelog plan). The order is the order of CHANGELOG.md
// (scripts/lib/changelog.ts): months newest first; in a month, the groups in
// rank order; in a group, the lines in file order (oldest first). The update
// window (U6) uses `byMonth` for the changes that an update gives.

import type { ChangelogEntry } from "../bindings";
import { monthName, PAGES_URL, REPOSITORY } from "../../scripts/lib/changelog";

export { PAGES_URL };

/// The number of calendar months that Recent changes shows (R11).
export const RECENT_MONTHS = 6;

/// One group heading in a month, with its lines.
export type ChangelogGroup = { group: string; rank: number; entries: ChangelogEntry[] };

/// One month, for example "2026-10" with the name "October 2026".
export type ChangelogMonth = { month: string; name: string; groups: ChangelogGroup[] };

/// The PR page of a PR number. The address uses only the integer, never
/// text from the changelog (R7).
export function prAddress(pr: number): string {
  return `https://github.com/${REPOSITORY}/pull/${Math.trunc(pr)}`;
}

/// The count of calendar months from year 0 to a month key `YYYY-MM`.
function monthIndex(month: string): number {
  const [year, number] = month.split("-").map(Number);
  return year * 12 + number - 1;
}

/// The entries by month, newest month first, then by group in rank order.
/// The lines of a group stay in the order of `entries`.
export function byMonth(entries: ChangelogEntry[]): ChangelogMonth[] {
  const keys = [...new Set(entries.map((e) => e.month))].sort().reverse();
  return keys.map((month) => {
    const inMonth = entries.filter((e) => e.month === month);
    const ranks = [...new Set(inMonth.map((e) => e.rank))].sort((a, b) => a - b);
    const groups = ranks.map((rank) => {
      const lines = inMonth.filter((e) => e.rank === rank);
      return { group: lines[0].group, rank, entries: lines };
    });
    return { month, name: monthName(month), groups };
  });
}

/// The newest month in the entries and the 5 calendar months before it
/// (AE8). The window starts at the newest entry, not at today's date. A
/// calendar month with no entries is not in the result.
export function recentMonths(entries: ChangelogEntry[]): ChangelogMonth[] {
  if (entries.length === 0) return [];
  const newest = Math.max(...entries.map((e) => monthIndex(e.month)));
  return byMonth(entries.filter((e) => newest - monthIndex(e.month) < RECENT_MONTHS));
}
