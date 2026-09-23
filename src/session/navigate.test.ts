import { describe, expect, test } from "bun:test";
import type { DocLine, NoteView } from "../bindings";
import { targetLine } from "./navigate";

const hour = 3_600_000;
const lines: DocLine[] = [
  { kind: "turn", position_ms: 0, speaker: 1, text: "Welcome back." },
  { kind: "turn", position_ms: hour + 30 * 60_000, speaker: 2, text: "The rules changed." },
  { kind: "turn", position_ms: hour + 35 * 60_000, speaker: 1, text: "Roll initiative." },
  { kind: "gap", at_ms: 2 * hour, duration_ms: 600_000 },
  { kind: "turn", position_ms: 2 * hour + 600_000, speaker: 1, text: "We're back." },
];

const note = (position_ms: number | null, gap_anchored = false): NoteView => ({
  id: 1,
  text: "n",
  position_ms,
  gap_anchored,
});

describe("following a note into the transcript", () => {
  test("an anchor at 01:34:12 finds the turn spanning it", () => {
    expect(targetLine(lines, note(hour + 34 * 60_000 + 12_000))).toBe(1);
  });

  test("an anchor exactly on a turn's start finds that turn", () => {
    expect(targetLine(lines, note(hour + 35 * 60_000))).toBe(2);
  });

  test("an anchor before the first turn goes to the first turn", () => {
    const later: DocLine[] = [{ kind: "turn", position_ms: 5_000, speaker: null, text: "hi" }];
    expect(targetLine(later, note(1_000))).toBe(0);
  });

  test("a gap-anchored note goes to the gap marker at its segment end", () => {
    expect(targetLine(lines, note(2 * hour, true))).toBe(3);
  });

  test("a gap-anchored note after the last segment goes to the last turn", () => {
    expect(targetLine(lines, note(3 * hour, true))).toBe(4);
  });

  test("an un-anchored note offers nowhere to go", () => {
    expect(targetLine(lines, note(null))).toBeNull();
  });

  test("an empty transcript offers nowhere to go", () => {
    expect(targetLine([], note(1_000))).toBeNull();
  });
});
