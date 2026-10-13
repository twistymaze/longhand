import { describe, expect, test } from "bun:test";
import type { SessionEntry } from "../bindings";
import { sessionTitle } from "../processing/describe";
import { groupByDay, rowTitle } from "./group";

const entry = (id: string, started: Date, over: Partial<SessionEntry> = {}): SessionEntry => ({
  id,
  started: started.toISOString(),
  duration_ms: 0,
  state: "saved",
  stage: null,
  title: null,
  found_speakers: null,
  storage_unavailable: false,
  problem: null,
  remote: null,
  note_type: null,
  tags: [],
  ...over,
});

describe("groupByDay", () => {
  // Thursday 24 September 2026, 00:30 local time: 20 minutes after midnight.
  const now = new Date(2026, 8, 24, 0, 30);

  test("puts today, yesterday and last Monday into three groups, newest first", () => {
    const groups = groupByDay(
      [
        entry("monday", new Date(2026, 8, 21, 10, 0)),
        entry("before-midnight", new Date(2026, 8, 23, 23, 50)),
        entry("after-midnight", new Date(2026, 8, 24, 0, 10)),
        entry("yesterday-morning", new Date(2026, 8, 23, 9, 0)),
      ],
      now,
    );
    expect(groups.map((g) => g.label)).toEqual(["Today", "Yesterday", groups[2].label]);
    expect(groups[2].label).toContain("21");
    expect(groups.map((g) => g.entries.map((e) => e.id))).toEqual([
      ["after-midnight"],
      ["before-midnight", "yesterday-morning"],
      ["monday"],
    ]);
  });

  test("gives no groups for no sessions", () => {
    expect(groupByDay([], now)).toEqual([]);
  });
});

describe("rowTitle", () => {
  const now = new Date(2026, 9, 24, 18, 0);

  test("shows the start time for a session with no title", () => {
    const started = new Date(2026, 9, 24, 14, 5);
    expect(rowTitle(entry("s", started), now)).toBe(sessionTitle(started.toISOString(), now));
    expect(rowTitle(entry("s", started), now)).toStartWith("Today, ");
  });

  test("shows the title of a titled session", () => {
    expect(rowTitle(entry("s", new Date(2026, 9, 24, 14, 5), { title: "Budget review" }), now)).toBe(
      "Budget review",
    );
  });
});
