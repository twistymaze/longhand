// The pure part of the shortcut control (KTD2): what one key-down means
// while the control captures keys. After anarlog's dictation-shortcut
// control: the chord is complete at the first key that is not a modifier,
// and it is written with `KeyboardEvent.code`, which the global-shortcut
// plugin parses and which does not change with the keyboard layout. The
// backend decides whether the chord is valid (R8), so this does not.

export type KeyPress = Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey" | "repeat">;

export type KeyResult =
  | { kind: "none" }
  | { kind: "cancel" }
  | { kind: "end" }
  | { kind: "chord"; chord: string };

const MODIFIER_KEYS = new Set(["Control", "Alt", "Shift", "Meta", "Fn", "OS", "AltGraph"]);

export function chordFromKey(e: KeyPress): KeyResult {
  if (e.key === "Escape") return { kind: "cancel" };
  if (e.key === "Tab") return { kind: "end" };
  if (e.repeat || MODIFIER_KEYS.has(e.key)) return { kind: "none" };
  const modifiers = [
    e.ctrlKey && "Control",
    e.altKey && "Alt",
    e.shiftKey && "Shift",
    e.metaKey && "Super",
  ].filter((m): m is string => Boolean(m));
  return { kind: "chord", chord: [...modifiers, e.code].join("+") };
}
