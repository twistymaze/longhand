// Positions on the session timeline, formatted as session-core's
// `format_position` writes them into notes.md and transcript.md.

export function formatPosition(ms: number): string {
  const secs = Math.floor(Math.max(0, ms) / 1000);
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${pad(Math.floor(secs / 3600))}:${pad(Math.floor(secs / 60) % 60)}:${pad(secs % 60)}`;
}

/// Level on a perceptual scale: -60 dBFS and below is the floor, 0 dBFS the
/// top. A linear peak would leave ordinary speech as a sliver.
export function levelFraction(peak: number): number {
  if (peak <= 0) return 0;
  const db = 20 * Math.log10(peak);
  return Math.min(1, Math.max(0, (db + 60) / 60));
}

/// How a chord reads on this platform, e.g. ⌥⇧⌘L on a Mac.
export function formatChord(chord: string, mac: boolean): string {
  const parts = chord.split("+");
  const key = keyName(parts.pop() ?? "");
  const mods = new Set(parts.map((p) => p.toLowerCase()));
  const control = mods.has("control") || mods.has("ctrl");
  const alt = mods.has("alt") || mods.has("option");
  const command = mods.has("command") || mods.has("cmd") || mods.has("super") || mods.has("meta");
  const commandOrControl = mods.has("commandorcontrol") || mods.has("cmdorctrl");
  if (mac) {
    let out = "";
    if (control) out += "⌃";
    if (alt) out += "⌥";
    if (mods.has("shift")) out += "⇧";
    if (command || commandOrControl) out += "⌘";
    return out + key;
  }
  const names: string[] = [];
  if (command) names.push("Win");
  if (control || commandOrControl) names.push("Ctrl");
  if (alt) names.push("Alt");
  if (mods.has("shift")) names.push("Shift");
  return [...names, key].join("+");
}

const KEY_NAMES: Record<string, string> = {
  Comma: ",",
  Period: ".",
  Slash: "/",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  BracketLeft: "[",
  BracketRight: "]",
  Minus: "-",
  Equal: "=",
  Backquote: "`",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
};

/// The key of a chord as a person names it: `KeyR` is R, `Digit1` is 1.
function keyName(code: string): string {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  return KEY_NAMES[code] ?? (code.length === 1 ? code.toUpperCase() : code);
}
