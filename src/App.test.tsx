import { afterEach, describe, expect, test } from "bun:test";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Readiness, RecordingStatus, SessionEntry, Settings } from "./bindings";
import App from "./App";
import { mockTauri } from "./test/tauri";

const readiness: Readiness = {
  microphone_authorized: true,
  destination: { kind: "ready", path: "/x", free_bytes: null },
  armed: true,
  shortcut: { chord: "Alt+Shift+L", registered: true, error: null },
  missing_models: [],
  endpoint_configured: true,
};

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
};

const recordingEntry: SessionEntry = {
  id: "rec",
  started: new Date().toISOString(),
  duration_ms: 0,
  state: "capturing",
  stage: null,
  title: null,
  found_speakers: null,
};

function mockApp(status: { current: RecordingStatus }, entries: { current: SessionEntry[] }) {
  return mockTauri({
    get_settings: () => ({ shortcut: "Alt+Shift+L", alert_tone_heard: true }) as Partial<Settings>,
    readiness: () => readiness,
    model_statuses: () => [],
    recording_status: () => status.current,
    list_sessions: () => entries.current,
    alert_check: () => ({ reason: null }),
    session_notes: () => "",
    session_detail: () => ({ entry: recordingEntry, done: [], current: null, resumable: false }),
  });
}

// happy-dom starts at about:blank, where history cannot set a query.
const setURL = (url: string) =>
  (window as unknown as { happyDOM: { setURL: (u: string) => void } }).happyDOM.setURL(url);

afterEach(() => setURL("about:blank"));

describe("App", () => {
  test("the settings query shows the Settings window with its panes and no sessions", async () => {
    setURL("http://localhost/?window=settings");
    mockApp({ current: idle }, { current: [] });
    render(<App />);
    expect(await screen.findByRole("tab", { name: "Status" })).toBeTruthy();
    for (const name of ["Destination", "Summarization", "Recognition", "Licences"]) {
      expect(screen.getByRole("tab", { name: new RegExp(name) })).toBeTruthy();
    }
    expect(screen.queryByText("Sessions")).toBeNull();
  });

  // Covers AE6 and KTD2.
  test("a recording that starts opens its page, and Back shows Home with the recording row", async () => {
    const status = { current: idle };
    const entries = { current: [] as SessionEntry[] };
    const tauri = mockApp(status, entries);
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Sessions" })).toBeTruthy();

    status.current = { ...idle, session_id: "rec", state: "capturing", recording: true };
    entries.current = [recordingEntry];
    await act(() => tauri.emit("recording-changed", { session_id: "rec" }));
    const back = await screen.findByRole("button", { name: "← Back" });

    await userEvent.setup().click(back);
    expect(screen.getByRole("button", { name: /Recording/ })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Sessions" })).toBeTruthy();
  });

  // Covers R3: the tray's "Open Longhand" during a recording.
  test("a request to show a session opens its page", async () => {
    const status = { current: { ...idle, session_id: "rec", state: "capturing", recording: true } };
    const entries = { current: [recordingEntry] };
    const tauri = mockApp(status, entries);
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Sessions" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "← Back" })).toBeNull();
    await act(() => tauri.emit("show-session", { session_id: "rec" }));
    expect(await screen.findByRole("button", { name: "← Back" })).toBeTruthy();
  });
});
