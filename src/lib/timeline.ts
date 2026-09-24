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
  const key = parts.pop() ?? "";
  const mods = new Set(parts.map((p) => p.toLowerCase()));
  if (mac) {
    let out = "";
    if (mods.has("control") || mods.has("ctrl")) out += "⌃";
    if (mods.has("alt") || mods.has("option")) out += "⌥";
    if (mods.has("shift")) out += "⇧";
    if (mods.has("commandorcontrol") || mods.has("command") || mods.has("cmd") || mods.has("super"))
      out += "⌘";
    return out + key.toUpperCase();
  }
  const names: string[] = [];
  if (mods.has("commandorcontrol") || mods.has("control") || mods.has("ctrl")) names.push("Ctrl");
  if (mods.has("alt")) names.push("Alt");
  if (mods.has("shift")) names.push("Shift");
  return [...names, key.toUpperCase()].join("+");
}
