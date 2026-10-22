import { expect, test, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import type { RecordingStatus } from "../bindings";
import { mockTauri } from "../test/tauri";
import { useCapture } from "./useCapture";

const idle: RecordingStatus = {
  session_id: null,
  state: null,
  recording: false,
  reconnecting: false,
  interruption: null,
  working_free_bytes: null,
  session_demand_bytes: null,
  processing_bytes_per_second: 0,
  reserve_bytes: 0,
  microphone: null,
  microphone_fallback: false,
  note_apps: [],
  app_audio: null,
  microphone_choice: null,
  paused: false,
  recorded_ms: 0,
};

// A window that reloads while the note is paused gets no ticks, so the
// time comes from the status.
test("a window opened while the note is paused shows the recorded time and the paused input", async () => {
  mockTauri({
    recording_status: () => ({ ...idle, recording: true, paused: true, session_id: "rec", state: "capturing", recorded_ms: 312_000 }),
  });
  const hook = renderHook(() => useCapture());
  await act(async () => {});
  expect(hook.result.current.recordedMs).toBe(312_000);
  expect(hook.result.current.input).toBe("paused");
  hook.unmount();
});

// While a segment records, the status holds only the kept segments, so a
// poll does not move the ticked time back.
test("a status poll while the note records keeps the time from the ticks", async () => {
  const tauri = mockTauri({
    recording_status: () => ({ ...idle, recording: true, session_id: "rec", state: "capturing", recorded_ms: 60_000 }),
  });
  const hook = renderHook(() => useCapture());
  await act(async () => {});
  await act(async () => { await tauri.emit("capture-tick", { session_id: "rec", recorded_ms: 95_000, peak: 0.5 }); });
  await act(async () => { await hook.result.current.refresh(); });
  expect(hook.result.current.recordedMs).toBe(95_000);
  hook.unmount();
});

// An interrupted note gets no ticks, so the time comes from the status.
test("an interrupted note shows its recorded time from the status", async () => {
  mockTauri({
    recording_status: () => ({
      ...idle,
      session_id: "rec",
      state: "interrupted",
      interruption: { headline: "Microphone lost", disk: false },
      recorded_ms: 600_000,
    }),
  });
  const hook = renderHook(() => useCapture());
  await act(async () => {});
  expect(hook.result.current.recordedMs).toBe(600_000);
  hook.unmount();
});

test("a new recording note shows its own time before its first tick, not the time of the note before", async () => {
  const status = { current: { ...idle, recording: true, session_id: "a", state: "capturing", recorded_ms: 0 } as RecordingStatus };
  const tauri = mockTauri({ recording_status: () => status.current });
  const hook = renderHook(() => useCapture());
  await act(async () => {});
  await act(async () => { await tauri.emit("capture-tick", { session_id: "a", recorded_ms: 600_000, peak: 0.5 }); });
  expect(hook.result.current.recordedMs).toBe(600_000);

  status.current = { ...idle, recording: true, session_id: "b", state: "capturing", recorded_ms: 120_000 };
  await act(async () => { await tauri.emit("recording-changed", { session_id: "b", opened: true }); });
  await act(async () => { await hook.result.current.refresh(); });
  expect(hook.result.current.recordedMs).toBe(120_000);
  hook.unmount();
});

// Computer audio on or off, or a speaker stream that is lost or opens
// again, changes the recording but keeps the segment open. The status
// then holds only the kept segments, so it must not move the time back.
test("a recording change inside the open segment keeps the ticked time", async () => {
  const tauri = mockTauri({
    recording_status: () => ({ ...idle, recording: true, session_id: "rec", state: "capturing", recorded_ms: 0 }),
  });
  const hook = renderHook(() => useCapture());
  await act(async () => {});
  await act(async () => { await tauri.emit("capture-tick", { session_id: "rec", recorded_ms: 600_000, peak: 0.5 }); });
  await act(async () => { await tauri.emit("recording-changed", { session_id: "rec", opened: false }); });
  await act(async () => { await hook.result.current.refresh(); });
  expect(hook.result.current.recordedMs).toBe(600_000);
  hook.unmount();
});

test("recording starts and status polls do not check sound output; capture errors remain", async () => {
  const status = { ...idle, recording: true, session_id: "rec", state: "capturing" };
  const tauri = mockTauri({ recording_status: () => status });
  const timers = new Map<number, () => void>();
  const browserTimers: {
    setInterval(handler: TimerHandler, timeout?: number): number;
    clearInterval(id?: number): void;
  } = window;
  const interval = vi.spyOn(browserTimers, "setInterval").mockImplementation((handler, timeout) => {
    timers.set(timeout!, handler as () => void);
    return timeout!;
  });
  const clear = vi.spyOn(browserTimers, "clearInterval").mockImplementation(() => {});
  let unmount: (() => void) | undefined;
  try {
    const hook = renderHook(() => useCapture());
    unmount = hook.unmount;
    await act(async () => {});
    expect(hook.result.current.status?.recording).toBe(true);
    await act(async () => { await tauri.emit("recording-changed", { session_id: "rec" }); });
    await act(async () => { timers.get(5000)!(); });
    expect(tauri.calls.map((call) => call.cmd)).toEqual([
      "recording_status", "recording_status", "recording_status",
    ]);
    await act(async () => { await tauri.emit("capture-ended", { session_id: "rec", reason: "Microphone disconnected" }); });
    expect(hook.result.current.lostReason).toBe("Microphone disconnected");
  } finally {
    unmount?.();
    interval.mockRestore();
    clear.mockRestore();
  }
});
