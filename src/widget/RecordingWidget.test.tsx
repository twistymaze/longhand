import { describe, expect, test } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockWindows } from "@tauri-apps/api/mocks";
import type { RecordingStatus } from "../bindings";
import { mockTauri } from "../test/tauri";
import { RecordingWidget } from "./RecordingWidget";

const idle: RecordingStatus = {
  note_id: null,
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

const recording: RecordingStatus = { ...idle, note_id: "s1", state: "capturing", recording: true, recorded_ms: 0 };
const paused: RecordingStatus = { ...recording, paused: true, recorded_ms: 754_000 };
const interrupted: RecordingStatus = {
  ...recording,
  recording: false,
  state: "interrupted",
  recorded_ms: 61_000,
  interruption: { headline: "The microphone stopped.", disk: false },
};

/// Mock the commands of the pill. `status` gives the current status, so a
/// test can change it between reads.
function open(status: () => RecordingStatus, handlers: Record<string, () => unknown> = {}) {
  const tauri = mockTauri({
    recording_status: status,
    pause_recording: () => null,
    continue_recording: () => null,
    stop_recording: () => "stopped",
    widget_hide: () => null,
    widget_open_note: () => null,
    "plugin:window|start_dragging": () => null,
    ...handlers,
  });
  mockWindows("widget");
  render(<RecordingWidget />);
  return { tauri, user: userEvent.setup() };
}

const called = (tauri: { calls: { cmd: string }[] }, cmd: string) => tauri.calls.filter((c) => c.cmd === cmd).length;

describe("the pill", () => {
  test("a recording note shows the dot, the meter, the time, Pause and Stop", async () => {
    open(() => recording);
    await screen.findByRole("button", { name: "Pause" });
    expect(screen.getByRole("button", { name: "Stop" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Continue" })).toBeNull();
    expect(document.querySelector("[data-meter]")).not.toBeNull();
    expect(document.querySelector("[data-dot]")).not.toBeNull();
    expect(screen.getByText("00:00:00")).toBeTruthy();
  });

  // AE1.
  test("a paused note shows Paused, the time, Continue and Stop, and no meter", async () => {
    open(() => paused);
    await screen.findByText("Paused");
    expect(screen.getByText("00:12:34")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Continue" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Stop" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
    expect(document.querySelector("[data-meter]")).toBeNull();
  });

  // AE7.
  test("an interrupted note shows Interrupted and Stop only", async () => {
    open(() => interrupted);
    await screen.findByText("Interrupted");
    expect(screen.getByText("00:01:01")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Stop" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Continue" })).toBeNull();
  });

  test("a tick moves the time and the meter while the note records", async () => {
    const { tauri } = open(() => recording);
    await screen.findByRole("button", { name: "Pause" });
    await act(async () => {
      await tauri.emit("capture-tick", { note_id: "s1", peak: 0.8, speaker_peak: null, recorded_ms: 95_000 });
    });
    expect(screen.getByText("00:01:35")).toBeTruthy();
    const bars = [...document.querySelectorAll<HTMLElement>("[data-bar]")];
    expect(bars[bars.length - 1]?.style.transform).not.toBe("scaleY(0.2)");
  });

  test("the pill reads the status again when it shows, with no tick", async () => {
    let status = paused;
    const { tauri } = open(() => status);
    await screen.findByText("Paused");
    status = { ...recording, recorded_ms: 800_000 };
    await act(async () => {
      await tauri.emit("widget-shown", null);
    });
    await screen.findByRole("button", { name: "Pause" });
    expect(screen.queryByText("Paused")).toBeNull();
  });

  test("Pause pauses the note and reads the status again", async () => {
    let status = recording;
    const { tauri, user } = open(() => status, {
      pause_recording: () => {
        status = paused;
        return null;
      },
    });
    await user.click(await screen.findByRole("button", { name: "Pause" }));
    await screen.findByText("Paused");
    expect(called(tauri, "pause_recording")).toBe(1);
    expect(called(tauri, "widget_open_note")).toBe(0);
  });

  test("Continue shows the recording mode after the status is read, before any recording-changed event", async () => {
    let status = paused;
    const { tauri, user } = open(() => status, {
      continue_recording: () => {
        status = recording;
        return null;
      },
    });
    await user.click(await screen.findByRole("button", { name: "Continue" }));
    await screen.findByRole("button", { name: "Pause" });
    expect(called(tauri, "continue_recording")).toBe(1);
  });

  test("a Stop after the note already stopped shows no error", async () => {
    const { tauri, user } = open(() => recording, {
      stop_recording: () => {
        throw "No note is recording.";
      },
    });
    await user.click(await screen.findByRole("button", { name: "Stop" }));
    await waitFor(() => expect(called(tauri, "stop_recording")).toBe(1));
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText("No note is recording.")).toBeNull();
  });

  test("the × shows while the pointer is on the pill and hides the pill", async () => {
    const { tauri, user } = open(() => recording);
    await screen.findByRole("button", { name: "Pause" });
    await act(async () => {
      await tauri.emit("widget-pointer", { inside: true, x: 10, y: 10 });
    });
    const hide = screen.getByRole("button", { name: "Hide" });
    expect(hide.dataset.visible).toBe("true");
    await user.click(hide);
    expect(tauri.calls.filter((c) => c.cmd === "widget_hide").map((c) => c.args)).toEqual([{ noteId: "s1" }]);
    expect(called(tauri, "widget_open_note")).toBe(0);
  });

  // On macOS WebKit updates CSS hover late in the panel, so the pointer
  // events set the hover of the control under the pointer.
  test("a pointer event marks the control under the pointer, and leaving clears it", async () => {
    const { tauri } = open(() => recording);
    const stop = await screen.findByRole("button", { name: "Stop" });
    const elementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => stop.firstElementChild;
    try {
      await act(async () => {
        await tauri.emit("widget-pointer", { inside: true, x: 200, y: 30 });
      });
      expect(stop.dataset.hover).toBe("true");
      expect(screen.getByRole("button", { name: "Pause" }).dataset.hover).toBe("false");
      await act(async () => {
        await tauri.emit("widget-pointer", { inside: false, x: 300, y: 30 });
      });
      expect(stop.dataset.hover).toBe("false");
    } finally {
      document.elementFromPoint = elementFromPoint;
    }
  });

  test("a click on the pill outside its controls shows the note", async () => {
    const { tauri, user } = open(() => recording);
    await user.click(await screen.findByText("00:00:00"));
    expect(called(tauri, "widget_open_note")).toBe(1);
    await user.click(screen.getByRole("button", { name: "Stop" }));
    expect(called(tauri, "widget_open_note")).toBe(1);
  });

  test("a press that moves on the pill drags the window and does not show the note", async () => {
    const { tauri } = open(() => recording);
    const time = await screen.findByText("00:00:00");
    fireEvent.pointerDown(time, { button: 0, buttons: 1, clientX: 50, clientY: 20 });
    fireEvent.pointerMove(time, { buttons: 1, clientX: 52, clientY: 21 });
    expect(called(tauri, "plugin:window|start_dragging")).toBe(0);
    fireEvent.pointerMove(time, { buttons: 1, clientX: 70, clientY: 20 });
    await waitFor(() => expect(called(tauri, "plugin:window|start_dragging")).toBe(1));
    fireEvent.click(time);
    expect(called(tauri, "widget_open_note")).toBe(0);
  });

  test("a press on a control does not drag the window", async () => {
    const { tauri } = open(() => recording);
    const stop = await screen.findByRole("button", { name: "Stop" });
    fireEvent.pointerDown(stop, { button: 0, buttons: 1, clientX: 50, clientY: 20 });
    fireEvent.pointerMove(stop, { buttons: 1, clientX: 90, clientY: 20 });
    expect(called(tauri, "plugin:window|start_dragging")).toBe(0);
  });

  // R8.
  test("the pill says Continue, never Resume", async () => {
    open(() => paused);
    await screen.findByText("Paused");
    expect(document.body.textContent).not.toMatch(/resume/i);
    expect(document.body.innerHTML).not.toMatch(/resume/i);
  });

  test("no open note shows nothing", async () => {
    const { tauri } = open(() => idle);
    await waitFor(() => expect(called(tauri, "recording_status")).toBeGreaterThan(0));
    expect(screen.queryByRole("button")).toBeNull();
  });
});
