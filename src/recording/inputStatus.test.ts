import { expect, test } from "bun:test";
import { inputStatus, STALL_AFTER_MS } from "./inputStatus";

const base = { recording: true, lostReason: null, startedAt: 0 };

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
