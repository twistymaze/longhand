import { afterEach, describe, expect, test } from "bun:test";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Readiness, RecordingStatus, SessionEntry, Settings } from "./bindings";
import App from "./App";
import { mockTauri, shortcutStatuses } from "./test/tauri";

const available: Settings["shared"] = { availability: { kind: "available" }, newer_version: false, unknown_values: [] };

const readiness: Readiness = {
  microphone_authorized: true,
  system_audio: "allowed",
  destination: { kind: "ready", path: "/x", free_bytes: null },
  armed: true,
  shortcut: { chord: "Alt+Shift+L", registered: true, error: null },
  missing_models: [],
  endpoint_configured: true, can_start_note: true, note_start_refusal: null,
};

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

const recordingEntry: SessionEntry = {
  id: "rec",
  started: new Date().toISOString(),
  duration_ms: 0,
  state: "capturing",
  stage: null,
  title: null,
  found_speakers: null,
  storage_unavailable: false,
  remote: null,
  note_type: null,
  type_move_waits: false,
};

function mockApp(
  status: { current: RecordingStatus },
  entries: { current: SessionEntry[] },
  leave: () => { kind: "removed" | "kept" } = () => ({ kind: "kept" }),
) {
  return mockTauri({
    set_shown_note: () => null,
    leave_note: () => leave(),
    get_settings: () => ({ shortcut: "Alt+Shift+L", shared: available }) as Partial<Settings>,
    readiness: () => readiness,
    shortcut_status: () => shortcutStatuses,
    login_item_status: () => "disabled",
    model_statuses: () => [],
    speaker_models_status: () => ({ ids: [], size_bytes: 0, state: { state: "present" } }),
    recording_status: () => status.current,
    list_sessions: () => entries.current,
    load_session_notes: () => ({ markdown: "", fingerprint: "f" }),
    session_detail: ({ sessionId }) => ({ entry: entries.current.find((e) => e.id === sessionId) ?? recordingEntry, done: [], current: null, resumable: false }),
    session_document: () => ({ has_speakers: false, lines: [], transcript_markdown: "", summary: null }),
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
    for (const name of ["Storage", "Summaries", "Transcription", "About"]) {
      expect(screen.getByRole("tab", { name: new RegExp(name) })).toBeTruthy();
    }
    expect(screen.queryByRole("tab", { name: /Licences/ })).toBeNull();
    expect(screen.queryByRole("heading", { name: "Notes" })).toBeNull();
  });

  test("the licences query shows the full licence list and not the main window", async () => {
    setURL("http://localhost/?window=licences");
    mockTauri({ licence_text: () => "Rust crates\n\nexample 1.0.0\nMIT text" });
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Rust crates" })).toBeTruthy();
    expect(screen.queryByRole("heading", { name: "Notes" })).toBeNull();
    expect(screen.queryByRole("tab", { name: "General" })).toBeNull();
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
    expect(screen.getByRole("option", { name: /Recording/ })).toBeTruthy();
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

  // The shortcut opens a new note that does not record with `showSession`.
  test("a new note that the shortcut opens has the cursor in its editor", async () => {
    const note: SessionEntry = { ...recordingEntry, id: "n", state: "saved" };
    const tauri = mockApp({ current: idle }, { current: [note] });
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Notes" })).toBeTruthy();
    await act(() => tauri.emit("show-session", { session_id: "n" }));
    const editor = await screen.findByRole("textbox", { name: "My Notes" });
    await waitFor(() => expect(editor === document.activeElement || editor.contains(document.activeElement)).toBe(true));
  });

  // A page that opens in place of another starts again, so an edit on the
  // first page never goes to the second session.
  test("a title typed on one page is not given to the session that opens in its place", async () => {
    const saved = (id: string, title: string): SessionEntry => ({ ...recordingEntry, id, title, state: "saved" });
    const entries = [saved("a", "Alpha"), saved("b", "Beta")];
    const tauri = mockTauri({
      set_shown_note: () => null,
      leave_note: () => ({ kind: "kept" }),
      get_settings: () => ({ shortcut: "Alt+Shift+L", shared: available }) as Partial<Settings>,
      readiness: () => readiness,
      shortcut_status: () => shortcutStatuses,
      login_item_status: () => "disabled",
      model_statuses: () => [],
      recording_status: () => idle,
      list_sessions: () => entries,
        load_session_notes: () => ({ markdown: "", fingerprint: "f" }),
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
      login_item_status: () => "disabled",
      model_statuses: () => [],
    });
    render(<App />);
    expect(await screen.findByText("The settings file is damaged.")).toBeTruthy();
  });

  test("a read that fails when the Settings window comes back keeps the panes and shows the error", async () => {
    setURL("http://localhost/?window=settings");
    let fail = false;
    mockTauri({
      get_settings: () => ({ shortcut: "Alt+Shift+L", shared: available }) as Partial<Settings>,
      readiness: () => readiness,
      shortcut_status: () => shortcutStatuses,
      login_item_status: () => "disabled",
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


test("an older selection survives opening, returning, and a newly synced note", async () => {
  const entries = { current: [
    { ...recordingEntry, id: "a", title: "Older", state: "saved", started: "2026-09-23T12:00:00Z" },
    { ...recordingEntry, id: "b", title: "Recent", state: "saved", started: "2026-09-24T12:00:00Z" },
  ] };
  const tauri = mockApp({ current: idle }, entries);
  render(<App />);
  const older = await screen.findByRole("option", { name: /Older/ });
  const user = userEvent.setup();
  await user.dblClick(older);
  expect(await screen.findByRole("button", { name: "Back" })).toBeTruthy();
  entries.current = [...entries.current, { ...recordingEntry, id: "c", title: "Newest", state: "saved", started: "2026-09-25T12:00:00Z" }];
  await act(() => tauri.emit("library-changed", { changed: ["c"], removed: [] }));
  await user.click(screen.getByRole("button", { name: "Back" }));
  expect(screen.getByRole("option", { name: /Older/ }).getAttribute("aria-selected")).toBe("true");
  expect(screen.getByRole("option", { name: /Newest/ })).toBeTruthy();
  await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("option", { name: /Older/ })));
});

describe("the shown note (KTD7, R18)", () => {
  const saved = (id: string, title: string): SessionEntry => ({ ...recordingEntry, id, title, state: "saved" });
  const args = (tauri: { calls: { cmd: string; args: Record<string, unknown> }[] }, cmd: string) =>
    tauri.calls.filter((c) => c.cmd === cmd).map((c) => c.args);
  /** The last shown note that the page reported. */
  const shownLast = (tauri: { calls: { cmd: string; args: Record<string, unknown> }[] }) => {
    const reports = args(tauri, "set_shown_note");
    return reports[reports.length - 1];
  };

  test("each view change reports the shown note, and the note that the window stops showing is left", async () => {
    const tauri = mockApp({ current: idle }, { current: [saved("a", "Alpha")] });
    render(<App />);
    const user = userEvent.setup();
    await user.dblClick(await screen.findByRole("option", { name: /Alpha/ }));
    await waitFor(() => expect(shownLast(tauri)).toEqual({ sessionId: "a" }));
    expect(args(tauri, "leave_note")).toEqual([]);

    await user.click(await screen.findByRole("button", { name: "Back" }));
    await waitFor(() => expect(shownLast(tauri)).toEqual({ sessionId: null }));
    expect(args(tauri, "leave_note")).toEqual([{ sessionId: "a" }]);
  });

  test("a note that opens in place of another leaves the first note", async () => {
    const tauri = mockApp({ current: idle }, { current: [saved("a", "Alpha"), saved("b", "Beta")] });
    render(<App />);
    await screen.findByRole("option", { name: /Alpha/ });
    await act(() => tauri.emit("show-session", { session_id: "a" }));
    await screen.findByRole("button", { name: "Back" });
    await act(() => tauri.emit("show-session", { session_id: "b" }));
    await waitFor(() => expect(shownLast(tauri)).toEqual({ sessionId: "b" }));
    expect(args(tauri, "leave_note")).toEqual([{ sessionId: "a" }]);
  });

  test("when the main window hides, the shown note is left and is reported again when the window comes back", async () => {
    const tauri = mockApp({ current: idle }, { current: [saved("a", "Alpha")] });
    render(<App />);
    await screen.findByRole("option", { name: /Alpha/ });
    await act(() => tauri.emit("show-session", { session_id: "a" }));
    await screen.findByRole("button", { name: "Back" });

    await act(() => tauri.emit("main-window-hidden", null));
    await waitFor(() => expect(args(tauri, "leave_note")).toEqual([{ sessionId: "a" }]));
    expect(shownLast(tauri)).toEqual({ sessionId: null });
    // The note stays, so its page stays.
    expect(screen.getByRole("button", { name: "Back" })).toBeTruthy();

    await act(async () => {
      window.dispatchEvent(new Event("focus"));
    });
    await waitFor(() => expect(shownLast(tauri)).toEqual({ sessionId: "a" }));
  });

  // Covers AE4 when the window closes.
  test("an empty note that is removed when the main window hides gives Home in its place", async () => {
    const tauri = mockApp({ current: idle }, { current: [saved("a", "Alpha")] }, () => ({ kind: "removed" }));
    render(<App />);
    await screen.findByRole("option", { name: /Alpha/ });
    await act(() => tauri.emit("show-session", { session_id: "a" }));
    await screen.findByRole("button", { name: "Back" });

    await act(() => tauri.emit("main-window-hidden", null));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Back" })).toBeNull());
    expect(screen.getByRole("heading", { name: "Notes" })).toBeTruthy();
    // The note is left one time only.
    expect(args(tauri, "leave_note")).toEqual([{ sessionId: "a" }]);
    expect(shownLast(tauri)).toEqual({ sessionId: null });
  });
});

describe("a leave that waits (R18)", () => {
  const saved = (id: string, title: string): SessionEntry => ({ ...recordingEntry, id, title, state: "saved" });
  const args = (tauri: { calls: { cmd: string; args: Record<string, unknown> }[] }, cmd: string) =>
    tauri.calls.filter((c) => c.cmd === cmd).map((c) => c.args);

  /// The app with a `leave_note` that waits until `finish` is called. Like
  /// the backend, it keeps a note that the last report names as shown.
  function waitingLeave(extra: Record<string, (args: Record<string, unknown>) => unknown> = {}) {
    let shown: unknown = null;
    const waiting: (() => void)[] = [];
    const tauri = mockTauri({
      set_shown_note: ({ sessionId }) => {
        shown = sessionId;
        return null;
      },
      leave_note: ({ sessionId }) =>
        new Promise((resolve) => {
          waiting.push(() => resolve({ kind: shown === sessionId ? "kept" : "removed" }));
        }),
      get_settings: () => ({ shortcut: "Alt+Shift+L", shared: available }) as Partial<Settings>,
      readiness: () => readiness,
      shortcut_status: () => shortcutStatuses,
      login_item_status: () => "disabled",
      model_statuses: () => [],
      speaker_models_status: () => ({ ids: [], size_bytes: 0, state: { state: "present" } }),
      recording_status: () => idle,
      list_sessions: () => [saved("a", "Alpha")],
      load_session_notes: () => ({ markdown: "", fingerprint: "f" }),
      session_detail: () => ({ entry: saved("a", "Alpha"), done: [], current: null, resumable: false }),
      session_document: () => ({ has_speakers: false, lines: [], transcript_markdown: "", summary: null }),
      ...extra,
    });
    const finish = async () => {
      await act(async () => {
        for (const f of waiting.splice(0)) f();
      });
    };
    return { tauri, finish };
  }

  async function openAlpha(tauri: ReturnType<typeof mockTauri>) {
    render(<App />);
    await screen.findByRole("option", { name: /Alpha/ });
    await act(() => tauri.emit("show-session", { session_id: "a" }));
    await screen.findByRole("button", { name: "Back" });
  }

  test("a note that opens again before its leave ends stays on the page", async () => {
    const { tauri, finish } = waitingLeave();
    await openAlpha(tauri);
    const user = userEvent.setup();

    await user.click(screen.getByRole("button", { name: "Back" }));
    await waitFor(() => expect(args(tauri, "leave_note")).toEqual([{ sessionId: "a" }]));
    await user.dblClick(screen.getByRole("option", { name: /Alpha/ }));
    await screen.findByRole("button", { name: "Back" });
    await waitFor(() => expect(args(tauri, "set_shown_note").slice(-1)[0]).toEqual({ sessionId: "a" }));

    await finish();
    expect(screen.getByRole("button", { name: "Back" })).toBeTruthy();
    expect(args(tauri, "leave_note")).toEqual([{ sessionId: "a" }]);
  });

  test("a window that shows the note again before its leave ends keeps the page", async () => {
    const { tauri, finish } = waitingLeave();
    await openAlpha(tauri);

    await act(() => tauri.emit("main-window-hidden", null));
    await waitFor(() => expect(args(tauri, "leave_note")).toEqual([{ sessionId: "a" }]));
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
    });
    await waitFor(() => expect(args(tauri, "set_shown_note").slice(-1)[0]).toEqual({ sessionId: "a" }));

    await finish();
    expect(screen.getByRole("button", { name: "Back" })).toBeTruthy();
  });

  test("a leave waits for the notes that the user typed just before", async () => {
    let finishSave = () => {};
    const { tauri, finish } = waitingLeave({
      save_session_notes: () =>
        new Promise((resolve) => {
          finishSave = () => resolve({ kind: "saved", fingerprint: "f1" });
        }),
    });
    await openAlpha(tauri);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await userEvent.setup().type(box, "x");

    // The editor has not sent the change yet: the leave sends it first.
    await act(() => tauri.emit("main-window-hidden", null));
    await waitFor(() =>
      expect(args(tauri, "save_session_notes")).toEqual([{ sessionId: "a", markdown: "x", loadedFingerprint: "f" }]),
    );
    await act(async () => {});
    expect(args(tauri, "leave_note")).toEqual([]);

    await act(async () => finishSave());
    await waitFor(() => expect(args(tauri, "leave_note")).toEqual([{ sessionId: "a" }]));
    await finish();
  });
});
