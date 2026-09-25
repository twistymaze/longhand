// Input status comes from frame arrival, not from level (U7): a flat trace
// is what both genuine silence and a dead stream look like. Only a long run
// of near-silent frames counts as quiet input (R12).

/// Ticks arrive every 250 ms while frames flow; this many missed in a row
/// means the input has stalled.
export const STALL_AFTER_MS = 1500;

/// 20 s of ticks, at one tick per 250 ms. A pause in a conversation is
/// shorter than this.
export const QUIET_TICKS = 80;

/// A peak below -60 dBFS: the level meter shows nothing.
export const QUIET_PEAK = 0.001;

export type InputStatus =
  | "idle"
  | "starting"
  | "capturing"
  | "quiet"
  | "stalled"
  | "reconnecting"
  | "lost";

export function inputStatus(opts: {
  recording: boolean;
  /// The microphone went away and is being reopened (U14).
  reconnecting?: boolean;
  lostReason: string | null;
  lastTickAt: number | null;
  startedAt: number | null;
  now: number;
  /// The peaks of the latest ticks, oldest first.
  levels?: number[];
}): InputStatus {
  const { recording, reconnecting, lostReason, lastTickAt, startedAt, now, levels = [] } = opts;
  if (reconnecting) return "reconnecting";
  if (lostReason) return "lost";
  if (!recording) return "idle";
  const since = lastTickAt ?? startedAt;
  if (since === null) return "starting";
  if (now - since > STALL_AFTER_MS) return "stalled";
  if (lastTickAt === null) return "starting";
  const recent = levels.slice(-QUIET_TICKS);
  if (recent.length === QUIET_TICKS && recent.every((peak) => peak < QUIET_PEAK)) return "quiet";
  return "capturing";
}
