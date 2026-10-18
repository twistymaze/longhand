// Machine-readable markers in the body of the upkeep tracking issue.
//
// The weekly routine writes the body (issue.ts), and the daily advisory
// workflow reads it (advisories.ts). The markers are HTML comments, so the
// issue shows only the text for people:
//
//   <!-- upkeep:last-run 2026-10-18 -->
//   <!-- upkeep:reported-advisories RUSTSEC-2026-0346 1241209 -->
//   <!-- upkeep:overdue-notified 2026-10-18 -->
//
// `last-run` is the date of the last successful weekly run (R21).
// `reported-advisories` lists the advisory IDs for which a notice was given,
// so each advisory gives one notice only (R2).
// `overdue-notified` is the `last-run` value (or `none`) at which the daily
// workflow gave the notice that the weekly run is overdue. So the overdue
// notice comes once for each missed run, not each day (KTD13).

export type Markers = {
  lastRun: string | null;
  reportedAdvisories: string[];
  // Undefined keeps the marker that the body has.
  overdueNotified?: string | null;
};

// The `overdue-notified` value when the body has no `last-run` marker.
export const NO_LAST_RUN = "none";

const LAST_RUN = /<!-- upkeep:last-run (\d{4}-\d{2}-\d{2}) -->/;
const REPORTED = /<!-- upkeep:reported-advisories ?([^>]*?) ?-->/;
const OVERDUE_NOTIFIED = /<!-- upkeep:overdue-notified (\S+) -->/;

// Reads the markers from an issue body. A missing marker gives null or an empty list.
export function readMarkers(body: string): Markers {
  const lastRun = body.match(LAST_RUN)?.[1] ?? null;
  const reported = body.match(REPORTED)?.[1]?.trim() ?? "";
  const overdueNotified = body.match(OVERDUE_NOTIFIED)?.[1] ?? null;
  return { lastRun, reportedAdvisories: reported === "" ? [] : reported.split(/\s+/), overdueNotified };
}

// Returns the body with the markers set. A marker that the body has is
// replaced in place; a missing one is added at the end.
export function writeMarkers(body: string, markers: Markers): string {
  let result = body;
  if (markers.lastRun !== null) {
    result = setMarker(result, LAST_RUN, `<!-- upkeep:last-run ${markers.lastRun} -->`);
  }
  const ids = [...new Set(markers.reportedAdvisories)].sort().join(" ");
  result = setMarker(result, REPORTED, `<!-- upkeep:reported-advisories ${ids} -->`);
  if (markers.overdueNotified) {
    result = setMarker(result, OVERDUE_NOTIFIED, `<!-- upkeep:overdue-notified ${markers.overdueNotified} -->`);
  }
  return result;
}

// The number of whole days from the last run to `now`, or null when the body has no last run.
export function daysSinceLastRun(body: string, now: Date): number | null {
  const { lastRun } = readMarkers(body);
  if (lastRun === null) return null;
  const last = Date.parse(`${lastRun}T00:00:00Z`);
  return Math.floor((now.getTime() - last) / 86_400_000);
}

function setMarker(body: string, pattern: RegExp, marker: string): string {
  if (pattern.test(body)) return body.replace(pattern, marker);
  return `${body.replace(/\s*$/, "")}\n\n${marker}\n`;
}
