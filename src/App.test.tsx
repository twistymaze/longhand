import { afterEach, describe, expect, test } from "bun:test";
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Readiness, RecordingStatus, SessionEntry, Settings } from "./bindings";
import App from "./App";
import { mockTauri, shortcutStatuses } from "./test/tauri";

const readiness: Readiness = {
  microphone_authorized: true,
  computer_audio: "allowed",
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
  microphone: null,
  microphone_fallback: false,
  computer_audio_silent: false,
  microphone_choice: null,
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
    get_settings: () => ({ shortcut: "Alt+Shift+L" }) as Partial<Settings>,
    readiness: () => readiness,
    shortcut_status: () => shortcutStatuses,
    model_statuses: () => [],
    speaker_models_status: () => ({ ids: [], size_bytes: 0, state: { state: "present" } }),
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
    expect(await screen.findByRole("tab", { name: "General" })).toBeTruthy();
    for (const name of ["Storage", "Summaries", "Transcription", "Licences"]) {
      expect(screen.getByRole("tab", { name: new RegExp(name) })).toBeTruthy();
    }
    expect(screen.queryByRole("heading", { name: "Notes" })).toBeNull();
  });

  // Covers AE6 and KTD2.
  test("a recording that starts opens its page, and Back shows Home with the recording row", async () => {
    const status = { current: idle };
    const entries = { current: [] as SessionEntry[] };
    const tauri = mockApp(status, entries);
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Notes" })).toBeTruthy();

    status.current = { ...idle, session_id: "rec", state: "capturing", recording: true };
    entries.current = [recordingEntry];
    await act(() => tauri.emit("recording-changed", { session_id: "rec" }));
    const back = await screen.findByRole("button", { name: "Back" });

    await userEvent.setup().click(back);
    expect(screen.getByRole("button", { name: /Recording/ })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Notes" })).toBeTruthy();
  });

  // Covers R3: the tray's "Open Longhand" during a recording.
  test("a request to show a session opens its page", async () => {
    const status = { current: { ...idle, session_id: "rec", state: "capturing", recording: true } };
    const entries = { current: [recordingEntry] };
    const tauri = mockApp(status, entries);
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Notes" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Back" })).toBeNull();
    await act(() => tauri.emit("show-session", { session_id: "rec" }));
    expect(await screen.findByRole("button", { name: "Back" })).toBeTruthy();
  });

  // A page that opens in place of another starts again, so an edit on the
  // first page never goes to the second session.
  test("a title typed on one page is not given to the session that opens in its place", async () => {
    const saved = (id: string, title: string): SessionEntry => ({ ...recordingEntry, id, title, state: "saved" });
    const entries = [saved("a", "Alpha"), saved("b", "Beta")];
    const tauri = mockTauri({
      get_settings: () => ({ shortcut: "Alt+Shift+L" }) as Partial<Settings>,
      readiness: () => readiness,
      shortcut_status: () => shortcutStatuses,
      model_statuses: () => [],
      recording_status: () => idle,
      list_sessions: () => entries,
      alert_check: () => ({ reason: null }),
      session_notes: () => "",
      session_detail: ({ sessionId }) => ({
        entry: entries.find((e) => e.id === sessionId),
        done: [],
        current: null,
        resumable: false,
      }),
      session_document: () => ({ has_speakers: false, lines: [], transcript_markdown: "", summary: null }),
      rename_session: () => ({ kind: "later" }),
    });
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Notes" })).toBeTruthy();
    await screen.findByRole("option", { name: /Alpha/ });

    await act(() => tauri.emit("show-session", { session_id: "a" }));
    const first = (await screen.findByRole("textbox", { name: "Title" })) as HTMLInputElement;
    expect(first.value).toBe("Alpha");
    const user = userEvent.setup();
    await user.click(first);
    await user.keyboard(" draft");

    await act(() => tauri.emit("show-session", { session_id: "b" }));
    const second = screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement;
    await act(async () => {
      fireEvent.blur(second);
    });
    expect(second.value).toBe("Beta");
    expect(tauri.calls.filter((c) => c.cmd === "rename_session").map((c) => c.args)).toEqual([]);
  });

  test("the Settings window shows an error that reads the settings", async () => {
    setURL("http://localhost/?window=settings");
    mockTauri({
      get_settings: () => {
        throw "the settings file is damaged";
      },
      readiness: () => readiness,
      shortcut_status: () => shortcutStatuses,
      model_statuses: () => [],
    });
    render(<App />);
    expect(await screen.findByText("The settings file is damaged.")).toBeTruthy();
  });

  test("a read that fails when the Settings window comes back keeps the panes and shows the error", async () => {
    setURL("http://localhost/?window=settings");
    let fail = false;
    mockTauri({
      get_settings: () => ({ shortcut: "Alt+Shift+L" }) as Partial<Settings>,
      readiness: () => readiness,
      shortcut_status: () => shortcutStatuses,
      model_statuses: () => {
        if (fail) throw "the models folder cannot be read";
        return [];
      },
      speaker_models_status: () => ({ ids: [], size_bytes: 0, state: { state: "present" } }),
    });
    render(<App />);
    expect(await screen.findByRole("tab", { name: "General" })).toBeTruthy();
    fail = true;
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
    });
    expect(await screen.findByText("The models folder cannot be read.")).toBeTruthy();
    expect(screen.getByRole("tab", { name: "General" })).toBeTruthy();
  });
});
