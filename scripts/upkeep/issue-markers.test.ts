import { describe, expect, test } from "bun:test";
import { daysSinceLastRun, readMarkers, writeMarkers } from "./issue-markers";

describe("issue markers", () => {
  test("a body with no markers gives no last run and no reported advisories", () => {
    expect(readMarkers("Nothing open.")).toEqual({ lastRun: null, reportedAdvisories: [], overdueNotified: null });
  });

  test("written markers read back the same", () => {
    const body = writeMarkers("Nothing open.", { lastRun: "2026-10-18", reportedAdvisories: ["RUSTSEC-2026-0346", "1241209"] });
    expect(body.startsWith("Nothing open.")).toBe(true);
    expect(readMarkers(body)).toEqual({ lastRun: "2026-10-18", reportedAdvisories: ["1241209", "RUSTSEC-2026-0346"], overdueNotified: null });
  });

  test("writing again replaces the markers and adds no second copy", () => {
    const first = writeMarkers("Text.", { lastRun: "2026-10-11", reportedAdvisories: ["A"] });
    const second = writeMarkers(first, { lastRun: "2026-10-18", reportedAdvisories: ["A", "B"] });
    expect(second.match(/upkeep:last-run/g)?.length).toBe(1);
    expect(second.match(/upkeep:reported-advisories/g)?.length).toBe(1);
    expect(readMarkers(second)).toEqual({ lastRun: "2026-10-18", reportedAdvisories: ["A", "B"], overdueNotified: null });
  });

  test("the overdue marker is kept when a write does not set it", () => {
    const first = writeMarkers("Text.", { lastRun: "2026-10-01", reportedAdvisories: [], overdueNotified: "2026-10-01" });
    const second = writeMarkers(first, { lastRun: "2026-10-18", reportedAdvisories: [] });
    expect(readMarkers(second).overdueNotified).toBe("2026-10-01");
    expect(readMarkers("Text.").overdueNotified).toBeNull();
  });

  test("days since the last run count whole days", () => {
    const body = writeMarkers("", { lastRun: "2026-10-10", reportedAdvisories: [] });
    expect(daysSinceLastRun(body, new Date("2026-10-19T08:00:00Z"))).toBe(9);
    expect(daysSinceLastRun("no marker", new Date("2026-10-19T08:00:00Z"))).toBeNull();
  });
});
