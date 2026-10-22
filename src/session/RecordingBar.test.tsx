import { describe, expect, test } from "vitest";
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Readiness, RecordingStatus } from "../bindings";
import type { Capture } from "../recording/useCapture";
import { mockTauri } from "../test/tauri";
import { RecordingBar } from "./RecordingBar";

const ready: Readiness = {
  microphone_authorized: true,
  system_audio: "allowed",
  destination: { kind: "ready", path: "/x", free_bytes: null },
  armed: true,
  shortcut: { chord: "Alt+Shift+L", registered: true, error: null },
  missing_models: [],
  endpoint_configured: true,
  can_start_note: true,
  note_start_refusal: null,
};

const capture: Capture = {
  status: null,
  input: "capturing",
  levels: [],
  recordedMs: 0,
  lostReason: null,
  refresh: async () => {},
};

function show(readiness: Readiness) {
  const tauri = mockTauri({ open_settings: () => null });
  render(<RecordingBar capture={capture} readiness={readiness} speakers={false} onSpeakersDone={() => {}} />);
  return { tauri, bar: screen.getByRole("region", { name: "Recording" }) };
}

const NO_SUMMARY = "The AI provider is not set up, so this note is saved without a summary.";

describe("RecordingBar", () => {
  test("with no provider, it says the note is saved without a summary and links to the AI Provider pane", async () => {
    const { tauri, bar } = show({ ...ready, endpoint_configured: false });
    expect(within(bar).getByText(NO_SUMMARY)).toBeTruthy();
    await userEvent.setup().click(within(bar).getByRole("button", { name: "Set Up AI Provider…" }));
    expect(tauri.calls.filter((c) => c.cmd === "open_settings").map((c) => c.args)).toEqual([{ pane: "provider" }]);
  });

  test("with a provider, it shows no provider warning", () => {
    const { bar } = show(ready);
    expect(within(bar).queryByText(NO_SUMMARY)).toBeNull();
    expect(within(bar).queryByRole("button", { name: "Set Up AI Provider…" })).toBeNull();
  });
});

describe("the app audio warnings", () => {
  const status: RecordingStatus = {
    session_id: "rec",
    state: "capturing",
    recording: true,
    paused: false,
    recorded_ms: 0,
    reconnecting: false,
    interruption: null,
    working_free_bytes: null,
    session_demand_bytes: null,
    processing_bytes_per_second: 0,
    reserve_bytes: 0,
    microphone: "MacBook Pro Microphone",
    microphone_fallback: false,
    microphone_choice: null,
    note_apps: [{ id: "us.zoom.xos", name: "zoom.us", path: "/Applications/zoom.us.app" }],
    app_audio: "recording",
  };
  function showStatus(over: Partial<RecordingStatus>) {
    const tauri = mockTauri({ open_system_audio_settings: () => null });
    render(
      <RecordingBar capture={{ ...capture, status: { ...status, ...over } }} readiness={ready} speakers={false} onSpeakersDone={() => {}} />,
    );
    return { tauri, bar: screen.getByRole("region", { name: "Recording" }) };
  }

  test("Covers AE5. Not allowed: says app audio is not recorded and how to allow it", async () => {
    const { tauri, bar } = showStatus({ app_audio: "not_allowed" });
    expect(
      within(bar).getByText(
        "App audio is not recorded. Allow Longhand under Screen & System Audio Recording in System Settings.",
      ),
    ).toBeTruthy();
    await userEvent.setup().click(within(bar).getByRole("button", { name: "Open System Settings" }));
    expect(tauri.calls.filter((c) => c.cmd === "open_system_audio_settings")).toHaveLength(1);
  });

  test("failed: says app audio is not recorded", () => {
    const { bar } = showStatus({ app_audio: "failed" });
    expect(within(bar).getByText("App audio is not recorded: it stopped and could not start again.")).toBeTruthy();
    expect(within(bar).queryByRole("button", { name: "Open System Settings" })).toBeNull();
  });

  test("Covers AE6. Recording, or no app set, shows no message", () => {
    const { bar } = showStatus({ app_audio: "recording" });
    expect(within(bar).queryByRole("status")).toBeNull();
    cleanup();
    const empty = showStatus({ note_apps: [], app_audio: null });
    expect(within(empty.bar).queryByRole("status")).toBeNull();
  });
});
