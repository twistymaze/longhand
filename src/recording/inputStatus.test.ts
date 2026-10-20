import { expect, test } from "vitest";
import { inputStatus, QUIET_TICKS, STALL_AFTER_MS } from "./inputStatus";

const base = { recording: true, lostReason: null, startedAt: 0 };

test("a microphone being reopened reads as reconnecting, not stalled", () => {
  expect(inputStatus({ ...base, reconnecting: true, lastTickAt: 0, now: 60_000 })).toBe(
    "reconnecting",
  );
});

test("silence with frames still arriving is capturing", () => {
  // Level plays no part: a tick of zero peak is still a tick.
  expect(inputStatus({ ...base, lastTickAt: 10_000, now: 10_200 })).toBe("capturing");
});

test("frames stopping with no error is stalled after the threshold", () => {
  expect(inputStatus({ ...base, lastTickAt: 10_000, now: 10_000 + STALL_AFTER_MS - 1 })).toBe(
    "capturing",
  );
  expect(inputStatus({ ...base, lastTickAt: 10_000, now: 10_000 + STALL_AFTER_MS + 1 })).toBe(
    "stalled",
  );
});

test("a stream that never delivered stalls from its start", () => {
  expect(inputStatus({ ...base, lastTickAt: null, now: 500 })).toBe("starting");
  expect(inputStatus({ ...base, lastTickAt: null, now: STALL_AFTER_MS + 1 })).toBe("stalled");
});

test("a reported stream error is lost, whatever the ticks say", () => {
  expect(inputStatus({ ...base, lostReason: "microphone: mic_stream_ended", lastTickAt: 1, now: 2 })).toBe(
    "lost",
  );
});

test("nothing recording is idle", () => {
  expect(inputStatus({ ...base, recording: false, lastTickAt: null, now: 0 })).toBe("idle");
});

test("frames that carry almost no sound for a long time read as quiet", () => {
  const ticks = { ...base, lastTickAt: 100_000, now: 100_100 };
  const silent = Array.from({ length: QUIET_TICKS }, () => 0);
  expect(inputStatus({ ...ticks, levels: silent })).toBe("quiet");
  // A pause in the talk is shorter than the window.
  expect(inputStatus({ ...ticks, levels: silent.slice(1) })).toBe("capturing");
  // One tick with sound in the window is enough.
  expect(inputStatus({ ...ticks, levels: [...silent.slice(1), 0.05] })).toBe("capturing");
  // A stalled stream is stalled, not quiet.
  expect(inputStatus({ ...ticks, now: 100_000 + STALL_AFTER_MS + 1, levels: silent })).toBe("stalled");
});
