import { expect, spyOn, test } from "bun:test";
import { act, renderHook } from "@testing-library/react";
import type { RecordingStatus } from "../bindings";
import { mockTauri } from "../test/tauri";
import { useCapture } from "./useCapture";

const idle: RecordingStatus = {
  session_id: null,
  state: null,
  recording: false,
  processing_waits: false,
  reconnecting: false,
  interruption: null,
  working_free_bytes: null,
  session_demand_bytes: null,
  processing_bytes_per_second: 0,
  reserve_bytes: 0,
  microphone: null,
  microphone_fallback: false,
  system_audio_silent: false,
  system_audio: false,
  system_audio_recording: false,
  system_audio_failed: false,
  system_audio_permission: null,
  system_audio_other_output: false,
  system_audio_mic_copy: false,
  microphone_choice: null,
};

test("recording starts and status polls do not check sound output; capture errors remain", async () => {
  const status = { ...idle, recording: true, session_id: "rec", state: "capturing" };
  const tauri = mockTauri({ recording_status: () => status });
  const timers = new Map<number, () => void>();
  const browserTimers: {
    setInterval(handler: TimerHandler, timeout?: number): number;
    clearInterval(id?: number): void;
  } = window;
  const interval = spyOn(browserTimers, "setInterval").mockImplementation((handler, timeout) => {
    timers.set(timeout!, handler as () => void);
    return timeout!;
  });
  const clear = spyOn(browserTimers, "clearInterval").mockImplementation(() => {});
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
