// Input status comes from frame arrival, not from level (U7): a flat trace
// is what both genuine silence and a dead stream look like.

/// Ticks arrive every 250 ms while frames flow; this many missed in a row
/// means the input has stalled.
export const STALL_AFTER_MS = 1500;

export type InputStatus = "idle" | "starting" | "capturing" | "stalled" | "lost";

export function inputStatus(opts: {
  recording: boolean;
  lostReason: string | null;
  lastTickAt: number | null;
  startedAt: number | null;
  now: number;
}): InputStatus {
  const { recording, lostReason, lastTickAt, startedAt, now } = opts;
  if (lostReason) return "lost";
  if (!recording) return "idle";
  const since = lastTickAt ?? startedAt;
  if (since === null) return "starting";
  if (now - since > STALL_AFTER_MS) return "stalled";
  return lastTickAt === null ? "starting" : "capturing";
}
