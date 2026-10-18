import { describe, expect, test } from "bun:test";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Readiness } from "../bindings";
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
