import { describe, expect, test } from "bun:test";
import { chordFromKey, type KeyPress } from "./chord";

const press = (code: string, over: Partial<KeyPress> = {}): KeyPress => ({
  key: code,
  code,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  repeat: false,
  ...over,
});

describe("chordFromKey", () => {
  test("a key with modifiers held gives the chord, modifiers first", () => {
    expect(chordFromKey(press("KeyR", { key: "r", metaKey: true, shiftKey: true }))).toEqual({
      kind: "chord",
      chord: "Shift+Super+KeyR",
    });
    expect(chordFromKey(press("Digit1", { key: "1", ctrlKey: true, altKey: true }))).toEqual({
      kind: "chord",
      chord: "Control+Alt+Digit1",
    });
  });

  test("a modifier key alone gives nothing yet", () => {
    expect(chordFromKey(press("ShiftLeft", { key: "Shift", shiftKey: true }))).toEqual({ kind: "none" });
    expect(chordFromKey(press("MetaRight", { key: "Meta", metaKey: true }))).toEqual({ kind: "none" });
  });

  test("a repeated key-down gives nothing", () => {
    expect(chordFromKey(press("KeyR", { key: "r", metaKey: true, repeat: true }))).toEqual({ kind: "none" });
  });

  test("Escape cancels and Tab ends capture", () => {
    expect(chordFromKey(press("Escape", { key: "Escape" }))).toEqual({ kind: "cancel" });
    expect(chordFromKey(press("Tab", { key: "Tab" }))).toEqual({ kind: "end" });
  });

  test("a key with no modifier still gives a chord, which the backend refuses", () => {
    expect(chordFromKey(press("KeyR", { key: "r" }))).toEqual({ kind: "chord", chord: "KeyR" });
  });
});
