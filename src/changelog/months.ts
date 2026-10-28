// The changelog entries by month and by group (R11, KTD3 of the monthly
// changelog plan). `byMonth` comes from scripts/lib/changelog.ts, so the order
// is the order of CHANGELOG.md: months newest first; in a month, the groups in
// rank order; in a group, the newest line first. The update
// window (U6) uses `byMonth` for the changes that an update gives.

import type { ChangelogEntry } from "../bindings";
import { byMonth, PAGES_URL, REPOSITORY, type Month, type MonthGroup } from "../../scripts/lib/changelog";

export { byMonth, PAGES_URL };

/// The number of calendar months that Recent changes shows (R11).
export const RECENT_MONTHS = 6;

/// One group heading in a month, with its lines.
export type ChangelogGroup = MonthGroup<ChangelogEntry>;

/// One month, for example "2026-10" with the name "October 2026".
export type ChangelogMonth = Month<ChangelogEntry>;

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

/// The newest month in the entries and the 5 calendar months before it
/// (AE8). The window starts at the newest entry, not at today's date. A
/// calendar month with no entries is not in the result.
export function recentMonths(entries: ChangelogEntry[]): ChangelogMonth[] {
  if (entries.length === 0) return [];
  const newest = Math.max(...entries.map((e) => monthIndex(e.month)));
  return byMonth(entries.filter((e) => newest - monthIndex(e.month) < RECENT_MONTHS));
}
