import { describe, expect, test } from "bun:test";
import { formatChord, formatPosition, levelFraction } from "./timeline";

describe("formatPosition", () => {
  // Shared fixtures with session-core's format_position.
  test.each([
    [0, "00:00:00"],
    [999, "00:00:00"],
    [95_000, "00:01:35"],
    [3_600_000, "01:00:00"],
    [4 * 3_600_000 + 12 * 60_000 + 5_000, "04:12:05"],
  ])("%p ms reads %s", (ms, text) => {
    expect(formatPosition(ms)).toBe(text);
  });
});

describe("levelFraction", () => {
  test("silence is the floor and full scale the top", () => {
    expect(levelFraction(0)).toBe(0);
    expect(levelFraction(0.0001)).toBe(0);
    expect(levelFraction(1)).toBe(1);
  });
  test("ordinary speech sits well above the floor", () => {
    expect(levelFraction(0.1)).toBeCloseTo(2 / 3, 5);
  });
});

describe("formatChord", () => {
  test("Mac glyphs in Apple's order", () => {
    expect(formatChord("CommandOrControl+Alt+Shift+L", true)).toBe("⌥⇧⌘L");
  });
  test("names elsewhere", () => {
    expect(formatChord("CommandOrControl+Alt+Shift+L", false)).toBe("Ctrl+Alt+Shift+L");
  });
  test("a captured key code shows its key", () => {
    expect(formatChord("Shift+Super+KeyR", true)).toBe("⇧⌘R");
    expect(formatChord("Shift+Super+KeyR", false)).toBe("Win+Shift+R");
    expect(formatChord("Control+Digit1", true)).toBe("⌃1");
    expect(formatChord("Option+Space", true)).toBe("⌥Space");
    expect(formatChord("Control+Alt+Comma", false)).toBe("Ctrl+Alt+,");
  });
});
