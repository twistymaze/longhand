// Home's list: sessions in groups of one local day, newest first (R5).

import type { SessionEntry } from "../bindings";
import { daysBefore, sessionTitle } from "../processing/describe";

export type DayGroup = { key: string; label: string; entries: SessionEntry[] };

const dayKey = (d: Date) => `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;

/// "Today", "Yesterday", "Monday 21 September", or with the year when it is
/// not this year.
export function dayLabel(date: Date, now: Date = new Date()): string {
  const days = daysBefore(date, now);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  return date.toLocaleDateString(undefined, {
    weekday: "long",
    day: "numeric",
    month: "long",
    year: date.getFullYear() === now.getFullYear() ? undefined : "numeric",
  });
}

/// Sessions by local day of their start, newest day first and newest session
/// first in each day.
export function groupByDay(entries: SessionEntry[], now: Date = new Date()): DayGroup[] {
  const sorted = [...entries].sort((a, b) => Date.parse(b.started) - Date.parse(a.started));
  const groups: DayGroup[] = [];
  for (const entry of sorted) {
    const started = new Date(entry.started);
    const key = dayKey(started);
    const last = groups[groups.length - 1];
    if (last?.key === key) last.entries.push(entry);
    else groups.push({ key, label: dayLabel(started, now), entries: [entry] });
  }
  return groups;
}

/// The title, or the start time until the session has one (R28).
export function rowTitle(entry: SessionEntry, now: Date = new Date()): string {
  return entry.title ?? sessionTitle(entry.started, now);
}

/// "3 speakers", "1 speaker", or nothing before processing counts them.
export function speakerCount(count: number | null): string | null {
  if (count === null) return null;
  return count === 1 ? "1 speaker" : `${count} speakers`;
}
