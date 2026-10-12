import { afterEach, beforeEach, describe, expect, mock, spyOn, test } from "bun:test";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type {
  LiveTranscript,
  Readiness,
  RecordingStatus,
  SessionDetail,
  SessionDocument,
  SessionEntry,
} from "../bindings";
import { sessionTitle } from "../processing/describe";
import type { Capture } from "../recording/useCapture";
import { type CommandHandler, mockTauri } from "../test/tauri";
import { SessionPage } from "./SessionPage";
import { type Sessions, useSessions } from "./useSessions";
import { features } from "../lib/features";

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
  paused: false,
  recorded_ms: 0,
};

const started = new Date(2026, 8, 24, 14, 5).toISOString();

const entry = (id: string, over: Partial<SessionEntry> = {}): SessionEntry => ({
  id,
  started,
  duration_ms: 35 * 60_000 + 48_000,
  state: "saved",
  stage: null,
  title: null,
  found_speakers: 3,
  storage_unavailable: false,
  remote: null,
  note_type: null,
  tags: [],
  ...over,
});

const detail = (e: SessionEntry, over: Partial<SessionDetail> = {}): SessionDetail => ({
  entry: e,
  done: [],
  current: null,
  windows_done: null,
  windows_total: null,
  speakers: null,
  failed_stage: null,
  message: null,
  summary_problem: null,
  resumable: false,
  record: null,
  home_path: null,
  endpoint: null,
  audio_kept: true,
  transcript_kept: true,
  audio: "present",
  asr_model: null,
  live_transcript: false,
  transcribe_again: false,
  transcribe_again_error: null,
  retry_starts_over: false,
  summary_not_updated: null,
  waiting: [],
  conflict_files: [],
  not_saved_yet: null,
  ...over,
});

const doc: SessionDocument = {
  has_speakers: true,
  lines: [{ kind: "turn", position_ms: 0, speaker: 1, text: "Hello." }],
  transcript_markdown: "# Transcript\n\n- [00:00:00] Speaker 1: Hello.\n",
  summary: { body: "We agreed the budget.", stub: false, host: "api.example.com", model: "m", prompt: null, prompt_id: null },
};

function captureOf(over: Partial<Capture> = {}): Capture {
  return {
    status: idle,
    input: "idle",
    levels: [],
    recordedMs: 0,
    lostReason: null,
    refresh: async () => {},
    ...over,
  };
}

const recordingCapture = (id: string, over: Partial<Capture> = {}) =>
  captureOf({
    status: { ...idle, session_id: id, state: "capturing", recording: true },
    input: "capturing",
    recordedMs: 5 * 60_000 + 7_000,
    ...over,
  });

function sessionsOf(entries: SessionEntry[]): Sessions {
  return { entries, stages: {}, live: {}, speakers: {}, version: {}, synced: {}, removed: {}, refresh: mock(async () => {}) };
}

function mockPage(e: SessionEntry, over: Partial<SessionDetail> = {}, extra: Record<string, CommandHandler> = {}) {
  return mockTauri({
    load_session_notes: () => ({ markdown: "", fingerprint: "f0" }),
    save_session_notes: () => ({ kind: "saved", fingerprint: "f1" }),
    session_detail: () => detail(e, over),
    session_document: () => doc,
    note_types: () => ({
      types: [
        { name: "One on One", valid: true, shared: true, notes: 1, general: false },
        { name: "Pathfinder", valid: true, shared: true, notes: 0, general: false },
      ],
      default_type: null,
    }),
    list_tags: () => [],
    ...extra,
  });
}

function renderPage({
  id,
  capture = captureOf(),
  sessions,
  started = false,
  onBack = () => {},
}: {
  id: string;
  capture?: Capture;
  sessions: Sessions;
  started?: boolean;
  onBack?: () => void;
}) {
  const ui = (c: Capture, s: Sessions) => (
    <SessionPage
      id={id}
      capture={c}
      sessions={s}
      readiness={readiness}
      started={started}
      onBack={onBack}
    />
  );
  const view = render(ui(capture, sessions));
  return { ...view, rerenderWith: (c: Capture, s: Sessions) => view.rerender(ui(c, s)) };
}

const tab = (name: string) => screen.getByRole("tab", { name });
type Tab = "summary" | "transcript";

describe("SessionPage", () => {
  test("a page opened for a recording shows My Notes with the cursor in the editor", async () => {
    const e = entry("rec", { state: "capturing", duration_ms: 0, found_speakers: null });
    mockPage(e);
    renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    expect(tab("My Notes").getAttribute("aria-selected")).toBe("true");
    const editor = await screen.findByRole("textbox", { name: "My Notes" });
    expect(document.activeElement === editor || editor.contains(document.activeElement)).toBe(true);
  });

  test("My Notes keeps the cursor where the user put it after another tab shows", async () => {
    const e = entry("rec", { state: "capturing", duration_ms: 0, found_speakers: null });
    mockPage(e, {}, { load_session_notes: () => ({ markdown: "first\n\nsecond", fingerprint: "f0" }) });
    renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    const editor = await screen.findByRole("textbox", { name: "My Notes" });
    await screen.findByText("first");
    const user = userEvent.setup();
    // Select all the notes: a selection that is not the cursor at the end.
    await user.keyboard(/Mac/.test(navigator.platform) ? "{Meta>}a{/Meta}" : "{Control>}a{/Control}");
    const selected = () => window.getSelection()!.isCollapsed === false;
    expect(selected()).toBe(true);
    await user.click(tab("Transcript"));
    await user.click(tab("My Notes"));
    // The editor puts its cursor back a short time after it gets the focus.
    await waitFor(() => expect(editor.contains(window.getSelection()!.anchorNode)).toBe(true));
    expect(selected()).toBe(true);
  });

  // WebKit puts typed text into the text selection, also when a tab button
  // has the focus. Text typed then went to the hidden notes, or to the
  // editor's earlier cursor, and Enter and Space went to the tab button.
  test("a tab change takes the selection out of My Notes, and a click on My Notes gives the editor the focus again", async () => {
    const e = entry("rec", { state: "capturing", duration_ms: 0, found_speakers: null });
    mockPage(e, {}, { load_session_notes: () => ({ markdown: "first\n\nsecond", fingerprint: "f0" }) });
    renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    const editor = await screen.findByRole("textbox", { name: "My Notes" });
    await screen.findByText("first");
    const user = userEvent.setup();
    await user.keyboard(/Mac/.test(navigator.platform) ? "{Meta>}a{/Meta}" : "{Control>}a{/Control}");
    expect(document.activeElement).toBe(editor);
    await user.click(tab("Transcript"));
    expect(editor.contains(window.getSelection()!.anchorNode)).toBe(false);
    await user.click(tab("My Notes"));
    expect(document.activeElement).toBe(editor);
    await waitFor(() => expect(window.getSelection()!.isCollapsed).toBe(false));
  });

  test("an arrow key to My Notes keeps the focus on the tab", async () => {
    const e = entry("rec", { state: "capturing", duration_ms: 0, found_speakers: null });
    mockPage(e, {}, { load_session_notes: () => ({ markdown: "first", fingerprint: "f0" }) });
    renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    await screen.findByText("first");
    const user = userEvent.setup();
    await user.click(screen.getByRole("textbox", { name: "My Notes" }));
    await user.click(tab("Transcript"));
    await user.keyboard("{ArrowLeft}");
    expect(tab("My Notes").getAttribute("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(tab("My Notes"));
  });

  test("a recording that the list knows before the capture status opens on My Notes and stays there", async () => {
    const e = entry("rec", { state: "capturing", duration_ms: 0, found_speakers: null });
    mockPage(e);
    const sessions = sessionsOf([e]);
    const page = renderPage({ id: "rec", sessions });
    const selected = () => tab("My Notes").getAttribute("aria-selected");
    expect(selected()).toBe("true");
    await act(async () => page.rerenderWith(recordingCapture("rec"), sessions));
    expect(selected()).toBe("true");
  });

  const tabNames = () => screen.getAllByRole("tab").map((t) => t.textContent);

  test("the tabs read Summary, My Notes, Transcript, because a returning reader wants the summary first", () => {
    const e = entry("s");
    mockPage(e);
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    expect(tabNames()).toEqual(["Summary", "My Notes", "Transcript"]);
    expect(tab("Summary").getAttribute("aria-selected")).toBe("true");
  });

  test("while the note records, Summary does not show", () => {
    const e = entry("rec", { state: "capturing" });
    mockPage(e);
    renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    expect(tabNames()).toEqual(["My Notes", "Transcript"]);
    expect(screen.queryByRole("tab", { name: "Summary" })).toBeNull();
    expect(tab("My Notes").getAttribute("aria-selected")).toBe("true");
  });

  test("Stop shows Summary and moves the page to it, where the processing shows", async () => {
    const e = entry("rec", { state: "capturing" });
    mockPage(e);
    const view = renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    expect(tab("My Notes").getAttribute("aria-selected")).toBe("true");
    view.rerenderWith(captureOf(), sessionsOf([{ ...e, state: "processing", stage: "transcribe" }]));
    expect(tabNames()).toEqual(["Summary", "My Notes", "Transcript"]);
    expect(tab("Summary").getAttribute("aria-selected")).toBe("true");
  });

  test("a processing note says it waits while processing waits, also after the other recording stops", async () => {
    const e = entry("prev", { state: "processing", stage: "assemble" });
    mockPage(e, { current: "assemble" });
    const notice = "Processing waits for the current recording to finish.";
    // The other note stopped, but its live transcript still saves its last part.
    const finishing = captureOf({ status: { ...idle, recording: false, processing_waits: true } });
    const view = renderPage({ id: "prev", capture: finishing, sessions: sessionsOf([e]) });
    expect(await screen.findByText(notice)).toBeTruthy();
    view.rerenderWith(captureOf(), sessionsOf([e]));
    expect(screen.queryByText(notice)).toBeNull();
  });

  test("Resume hides Summary and moves the page to My Notes, and Stop shows it again", async () => {
    const e = entry("s");
    mockPage(e);
    const view = renderPage({ id: "s", sessions: sessionsOf([e]) });
    expect(tab("Summary").getAttribute("aria-selected")).toBe("true");
    view.rerenderWith(recordingCapture("s"), sessionsOf([{ ...e, state: "capturing" }]));
    expect(tabNames()).toEqual(["My Notes", "Transcript"]);
    expect(tab("My Notes").getAttribute("aria-selected")).toBe("true");
    view.rerenderWith(captureOf(), sessionsOf([{ ...e, state: "processing", stage: "transcribe" }]));
    expect(tabNames()).toEqual(["Summary", "My Notes", "Transcript"]);
    expect(tab("Summary").getAttribute("aria-selected")).toBe("true");
  });

  test("the arrow keys move through the tabs that show and wrap at the ends", async () => {
    const e = entry("s");
    mockPage(e);
    const user = userEvent.setup();
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    const selected = () => screen.getAllByRole("tab").find((t) => t.getAttribute("aria-selected") === "true")?.textContent;
    tab("Summary").focus();
    await user.keyboard("{ArrowRight}");
    expect(selected()).toBe("My Notes");
    expect(document.activeElement).toBe(tab("My Notes"));
    await user.keyboard("{ArrowRight}");
    expect(selected()).toBe("Transcript");
    await user.keyboard("{ArrowRight}");
    expect(selected()).toBe("Summary");
    await user.keyboard("{ArrowLeft}");
    expect(selected()).toBe("Transcript");
  });

  test("while recording, the arrow keys cycle only My Notes and Transcript", async () => {
    const e = entry("rec", { state: "capturing" });
    mockPage(e);
    const user = userEvent.setup();
    renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    // The editor takes focus when it loads. Wait for it, then move focus to the tab.
    await screen.findByRole("textbox", { name: "My Notes" });
    tab("My Notes").focus();
    await user.keyboard("{ArrowRight}");
    expect(tab("Transcript").getAttribute("aria-selected")).toBe("true");
    await user.keyboard("{ArrowRight}");
    expect(tab("My Notes").getAttribute("aria-selected")).toBe("true");
    // Showing My Notes gives the editor focus again, so move focus back to the tab.
    tab("My Notes").focus();
    await user.keyboard("{ArrowLeft}");
    expect(tab("Transcript").getAttribute("aria-selected")).toBe("true");
  });

  test("while recording, the Transcript tab shows bars that follow the input level", () => {
    const e = entry("rec", { state: "capturing" });
    mockPage(e);
    const scales = () =>
      [...tab("Transcript").querySelectorAll<HTMLElement>("[data-bar]")].map((b) =>
        Number(b.style.transform.match(/scaleY\(([\d.]+)\)/)?.[1]),
      );
    const view = renderPage({
      id: "rec",
      capture: recordingCapture("rec", { levels: [0, 0.001, 0.1, 1] }),
      sessions: sessionsOf([e]),
    });
    const loud = scales();
    expect(loud).toHaveLength(4);
    expect(loud[3]).toBe(1);
    expect(loud[0]).toBeLessThan(loud[2]);
    expect(loud[2]).toBeLessThan(loud[3]);

    // Silence: the bars are flat, all at the smallest height.
    view.rerenderWith(recordingCapture("rec", { levels: [0, 0, 0, 0] }), sessionsOf([e]));
    const quiet = scales();
    expect(new Set(quiet).size).toBe(1);
    expect(quiet[0]).toBe(loud[0]);
  });

  test("with reduced motion, the bars hold still and show only whether sound arrives", () => {
    const e = entry("rec", { state: "capturing" });
    mockPage(e);
    const original = window.matchMedia;
    window.matchMedia = ((q: string) => ({ ...original(q), matches: q.includes("reduced-motion") })) as typeof window.matchMedia;
    try {
      const view = renderPage({
        id: "rec",
        capture: recordingCapture("rec", { levels: [0, 0.001, 0.1, 1] }),
        sessions: sessionsOf([e]),
      });
      const meter = () => tab("Transcript").querySelector<HTMLElement>("[data-meter]");
      const transforms = () => [...meter()!.querySelectorAll<HTMLElement>("[data-bar]")].map((b) => b.style.transform);
      const still = transforms();
      expect(meter()?.dataset.signal).toBe("true");
      view.rerenderWith(recordingCapture("rec", { levels: [0.5, 0.02, 0.9, 0.3] }), sessionsOf([e]));
      expect(transforms()).toEqual(still);
      view.rerenderWith(recordingCapture("rec", { levels: [0, 0, 0, 0] }), sessionsOf([e]));
      expect(meter()?.dataset.signal).toBe("false");
    } finally {
      window.matchMedia = original;
    }
  });

  test("no bars show once the recording stops", () => {
    const e = entry("s");
    mockPage(e);
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    expect(tab("Transcript").querySelector("[data-meter]")).toBeNull();
  });

  test("the page calls a session a note, never a session", async () => {
    const rec = entry("rec", { state: "capturing" });
    mockPage(rec);
    const view = renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([rec]) });
    expect(document.body.textContent).not.toMatch(/session/i);
    const failed = entry("rec", { state: "processing_failed", stage: "transcribe" });
    mockPage(failed, { failed_stage: "transcribe", message: "the model crashed" });
    view.rerenderWith(captureOf(), sessionsOf([failed]));
    await screen.findByRole("button", { name: "Retry" });
    expect(document.body.textContent).not.toMatch(/session/i);
    view.rerenderWith(captureOf(), sessionsOf([]));
    expect(document.body.textContent).toMatch(/note/);
    expect(document.body.textContent).not.toMatch(/session/i);
  });

  test("a completed session opens on Summary", async () => {
    const e = entry("s", { title: "Budget review" });
    mockPage(e);
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    expect(tab("Summary").getAttribute("aria-selected")).toBe("true");
    expect(await screen.findByText("We agreed the budget.")).toBeTruthy();
  });

  // Covers AE9.
  test("a title typed during the recording renames the session", async () => {
    const e = entry("rec", { state: "capturing" });
    const tauri = mockPage(e, {}, { rename_session: () => ({ kind: "later" }) });
    const sessions = sessionsOf([e]);
    renderPage({ id: "rec", capture: recordingCapture("rec"), sessions });
    const title = screen.getByRole("textbox", { name: "Title" });
    expect(title.getAttribute("placeholder")).toBe(sessionTitle(started));
    const user = userEvent.setup();
    await user.click(title);
    await user.keyboard("Budget review{Enter}");
    await act(async () => {});
    expect(tauri.calls.filter((c) => c.cmd === "rename_session").map((c) => c.args)).toEqual([
      { sessionId: "rec", title: "Budget review" },
    ]);
    expect(sessions.refresh).toHaveBeenCalled();
  });

  // Covers R35.
  test("an empty title clears the title, and the start time shows", async () => {
    const e = entry("s", { title: "Old title" });
    const tauri = mockPage(e, {}, { rename_session: () => ({ kind: "moved", path: "/x/y" }) });
    const view = renderPage({ id: "s", sessions: sessionsOf([e]) });
    const title = screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement;
    expect(title.value).toBe("Old title");
    const user = userEvent.setup();
    await user.clear(title);
    await user.keyboard("{Enter}");
    await act(async () => {});
    expect(tauri.calls.filter((c) => c.cmd === "rename_session").map((c) => c.args)).toEqual([
      { sessionId: "s", title: "" },
    ]);
    view.rerenderWith(captureOf(), sessionsOf([{ ...e, title: null }]));
    expect(title.value).toBe("");
    expect(title.getAttribute("placeholder")).toBe(sessionTitle(started));
  });

  test("Escape in the title field keeps the old title", async () => {
    const e = entry("s", { title: "Old title" });
    const tauri = mockPage(e);
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    const title = screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement;
    const user = userEvent.setup();
    await user.clear(title);
    await user.keyboard("Discarded{Escape}");
    expect(title.value).toBe("Old title");
    expect(tauri.calls.some((c) => c.cmd === "rename_session")).toBe(false);
  });

  test("the recording bar shows the elapsed time, and the quiet-input warning only while the input is quiet", async () => {
    const e = entry("rec", { state: "capturing" });
    mockPage(e);
    const view = renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    const bar = screen.getByRole("region", { name: "Recording" });
    // Stop is in the toolbar, in the place of Record and Resume.
    expect(within(bar).queryByRole("button", { name: "Stop" })).toBeNull();
    expect(bar.textContent).toContain("00:05:07");
    expect(within(bar).queryByText(/hears almost nothing/)).toBeNull();
    expect(within(bar).queryByRole("img", { name: /Audio level/ })).toBeNull();

    view.rerenderWith(recordingCapture("rec", { input: "quiet", levels: [0, 0, 0] }), sessionsOf([e]));
    expect(within(bar).getByText(/hears almost nothing/)).toBeTruthy();
    expect(within(bar).getByRole("img", { name: /Audio level/ })).toBeTruthy();
  });

  test("the recording bar shows a warning only while its condition exists", () => {
    const e = entry("rec", { state: "capturing" });
    mockPage(e);
    renderPage({
      id: "rec",
      capture: recordingCapture("rec", { input: "stalled" }),
      sessions: sessionsOf([e]),
    });
    const bar = screen.getByRole("region", { name: "Recording" });
    expect(within(bar).getByText("No audio arriving")).toBeTruthy();
  });

  test("the recording bar has no alert audibility warning", () => {
    const e = entry("rec", { state: "capturing" });
    mockPage(e);
    renderPage({
      id: "rec",
      capture: recordingCapture("rec"),
      sessions: sessionsOf([e]),
    });
    const bar = screen.getByRole("region", { name: "Recording" });
    expect(bar.textContent).not.toMatch(/alert sound/i);

    expect(bar.textContent).not.toMatch(/If recording fails|output is muted/i);
  });

  test("while Computer audio is hidden, the bar and the menu have no Computer audio control", async () => {
    const rec = entry("rec", { state: "capturing" });
    mockPage(rec, {}, { list_microphones: () => ({ default_id: null, inputs: [] }) });
    renderPage({
      id: "rec",
      capture: recordingCapture("rec", {
        status: { ...idle, session_id: "rec", state: "capturing", recording: true, microphone: "MacBook Pro Microphone" },
      }),
      sessions: sessionsOf([rec]),
    });
    const bar = screen.getByRole("region", { name: "Recording" });
    expect(within(bar).queryByRole("button", { name: "Computer audio" })).toBeNull();
    await userEvent.setup().click(screen.getByRole("button", { name: "Actions" }));
    expect(screen.getByRole("menuitem", { name: "Microphone" })).toBeTruthy();
    expect(screen.queryByRole("menuitemcheckbox", { name: "Record Computer Audio" })).toBeNull();
  });

  describe("microphone and Computer audio", () => {
    beforeEach(() => {
      features.systemAudio = true;
    });
    afterEach(() => {
      features.systemAudio = false;
    });
    const rec = entry("rec", { state: "capturing" });
    const withStatus = (over: Partial<RecordingStatus>) =>
      recordingCapture("rec", {
        status: { ...idle, session_id: "rec", state: "capturing", recording: true, microphone: "MacBook Pro Microphone", ...over },
      });
    const bar = () => screen.getByRole("region", { name: "Recording" });
    const microphones = {
      default_id: "builtin",
      inputs: [
        { id: "builtin", name: "MacBook Pro Microphone" },
        { id: "usb", name: "Jabra Speak 510" },
      ],
    };

    test("the rail shows the microphone name, and a Computer audio toggle that shows its state", async () => {
      const tauri = mockPage(rec, {}, {
        set_note_system_audio: () => ({ ...idle, session_id: "rec", recording: true }),
      });
      const view = renderPage({ id: "rec", capture: withStatus({}), sessions: sessionsOf([rec]) });
      expect(within(bar()).getByText("MacBook Pro Microphone")).toBeTruthy();
      const off = within(bar()).getByRole("button", { name: "Computer audio" });
      expect(off.getAttribute("aria-pressed")).toBe("false");
      const user = userEvent.setup();
      await user.click(off);
      expect(tauri.calls.find((c) => c.cmd === "set_note_system_audio")?.args).toEqual({
        sessionId: "rec",
        enabled: true,
      });

      view.rerenderWith(withStatus({ system_audio: true, system_audio_permission: "allowed" }), sessionsOf([rec]));
      expect(within(bar()).getByText("MacBook Pro Microphone")).toBeTruthy();
      const on = within(bar()).getByRole("button", { name: "Computer audio" });
      expect(on.getAttribute("aria-pressed")).toBe("true");
      await user.click(on);
      expect(tauri.calls.filter((c) => c.cmd === "set_note_system_audio").pop()?.args).toEqual({
        sessionId: "rec",
        enabled: false,
      });
    });

    // Covers R3: the inputs stay editable while paused.
    test("while paused, the Computer audio toggle and the Microphone menu stay available", async () => {
      const tauri = mockPage(rec, {}, {
        list_microphones: () => microphones,
        set_note_system_audio: () => null,
      });
      renderPage({
        id: "rec",
        capture: recordingCapture("rec", {
          status: { ...idle, session_id: "rec", state: "capturing", recording: true, paused: true },
          input: "paused",
        }),
        sessions: sessionsOf([rec]),
      });
      const toggle = within(bar()).getByRole("button", { name: "Computer audio" }) as HTMLButtonElement;
      expect(toggle.disabled).toBe(false);
      const user = userEvent.setup();
      await user.click(toggle);
      expect(tauri.calls.find((c) => c.cmd === "set_note_system_audio")?.args).toEqual({ sessionId: "rec", enabled: true });
      await user.click(screen.getByRole("button", { name: "Actions" }));
      expect(screen.getByRole("menuitem", { name: "Microphone" })).toBeTruthy();
      expect(screen.getByRole("menuitemcheckbox", { name: "Record Computer Audio" })).toBeTruthy();
    });

    test("with the microphone copy flag, the mixer warning shows", () => {
      mockPage(rec);
      renderPage({
        id: "rec",
        capture: withStatus({
          system_audio: true,
          system_audio_permission: "allowed",
          system_audio_recording: true,
          system_audio_mic_copy: true,
        }),
        sessions: sessionsOf([rec]),
      });
      expect(
        within(bar()).getByText(
          "Computer audio also has your microphone in it, so your voice is recorded twice. Check your audio mixer app.",
        ),
      ).toBeTruthy();
    });

    test("with the fallback flag, the fallback warning names both microphones", () => {
      mockPage(rec);
      renderPage({
        id: "rec",
        capture: withStatus({ microphone_fallback: true, microphone_choice: { id: "usb", label: "Jabra Speak 510" } }),
        sessions: sessionsOf([rec]),
      });
      expect(
        within(bar()).getByText("Using MacBook Pro Microphone: Jabra Speak 510 is not connected."),
      ).toBeTruthy();
    });

    test("with the permission denied and Computer audio on, the denied warning shows", () => {
      mockPage(rec);
      const view = renderPage({
        id: "rec",
        capture: withStatus({ system_audio: true, system_audio_permission: "denied", system_audio_failed: true }),
        sessions: sessionsOf([rec]),
      });
      expect(within(bar()).getByText("Computer audio is not recorded. Allow it in System Settings.")).toBeTruthy();
      // The denied permission explains the failure, so the failure text does not show too.
      expect(within(bar()).queryByText(/Computer audio stopped/)).toBeNull();

      view.rerenderWith(withStatus({ system_audio: false, system_audio_permission: "denied" }), sessionsOf([rec]));
      expect(within(bar()).queryByText(/Computer audio is not recorded/)).toBeNull();
    });

    test("with the permission not yet answered, the pending warning shows", () => {
      mockPage(rec);
      renderPage({
        id: "rec",
        capture: withStatus({ system_audio: true, system_audio_permission: "not_asked" }),
        sessions: sessionsOf([rec]),
      });
      expect(within(bar()).getByText("Computer audio is not recorded until you allow it.")).toBeTruthy();
    });

    test("with the failure flag, the failure warning shows", () => {
      mockPage(rec);
      renderPage({
        id: "rec",
        capture: withStatus({ system_audio: true, system_audio_permission: "allowed", system_audio_failed: true }),
        sessions: sessionsOf([rec]),
      });
      expect(within(bar()).getByText("Computer audio stopped. The microphone is still recording.")).toBeTruthy();
    });

    test("with the silent flag, the silent warning shows", () => {
      mockPage(rec);
      renderPage({
        id: "rec",
        capture: withStatus({ system_audio: true, system_audio_permission: "unknown", system_audio_silent: true }),
        sessions: sessionsOf([rec]),
      });
      expect(
        within(bar()).getByText("Computer audio is silent. Check the permission in System Settings."),
      ).toBeTruthy();
    });

    test("with the other-output flag, the other-output warning shows", () => {
      mockPage(rec);
      renderPage({
        id: "rec",
        capture: withStatus({ system_audio: true, system_audio_permission: "allowed", system_audio_other_output: true }),
        sessions: sessionsOf([rec]),
      });
      expect(
        within(bar()).getByText("Computer audio is silent. Sound is playing on another output."),
      ).toBeTruthy();
    });

    test("with no problem, no microphone or Computer audio warning shows", () => {
      mockPage(rec);
      renderPage({
        id: "rec",
        capture: withStatus({ system_audio: true, system_audio_permission: "allowed", system_audio_recording: true }),
        sessions: sessionsOf([rec]),
      });
      expect(within(bar()).queryByRole("status")).toBeNull();
    });

    test("choosing another microphone in the submenu calls the command with its ID", async () => {
      const tauri = mockPage(rec, {}, {
        list_microphones: () => microphones,
        set_note_microphone: () => ({ ...idle, session_id: "rec", recording: true }),
      });
      renderPage({ id: "rec", capture: withStatus({}), sessions: sessionsOf([rec]) });
      const user = userEvent.setup();
      await user.click(screen.getByRole("button", { name: "Actions" }));
      await user.click(screen.getByRole("menuitem", { name: "Microphone" }));
      const system = await screen.findByRole("menuitemradio", { name: /System default/ });
      expect(system.getAttribute("aria-checked")).toBe("true");
      expect(screen.getByRole("menuitemradio", { name: "Jabra Speak 510" }).getAttribute("aria-checked")).toBe("false");
      await user.click(screen.getByRole("menuitemradio", { name: "Jabra Speak 510" }));
      const call = tauri.calls.find((c) => c.cmd === "set_note_microphone");
      expect(call?.args).toEqual({ sessionId: "rec", microphone: { id: "usb", label: "Jabra Speak 510" } });
      // The menu closes after the choice.
      expect(screen.queryByRole("menu")).toBeNull();
    });

    test("the submenu checks the note's microphone and opens from the keyboard", async () => {
      mockPage(rec, {}, { list_microphones: () => microphones });
      renderPage({
        id: "rec",
        capture: withStatus({ microphone_choice: { id: "usb", label: "Jabra Speak 510" } }),
        sessions: sessionsOf([rec]),
      });
      const user = userEvent.setup();
      await user.click(screen.getByRole("button", { name: "Actions" }));
      const item = screen.getByRole("menuitem", { name: "Microphone" });
      act(() => item.focus());
      await user.keyboard("{ArrowRight}");
      const usb = await screen.findByRole("menuitemradio", { name: "Jabra Speak 510" });
      expect(usb.getAttribute("aria-checked")).toBe("true");
      expect(document.activeElement?.getAttribute("role")).toBe("menuitemradio");
      await user.keyboard("{ArrowLeft}");
      expect(screen.queryByRole("menuitemradio", { name: "Jabra Speak 510" })).toBeNull();
      expect(document.activeElement).toBe(item);
    });

    test("the Record Computer Audio item shows a check mark when on and calls the command when chosen", async () => {
      const tauri = mockPage(rec, {}, {
        list_microphones: () => microphones,
        set_note_system_audio: () => ({ ...idle, session_id: "rec", recording: true }),
      });
      const view = renderPage({ id: "rec", capture: withStatus({ system_audio: false }), sessions: sessionsOf([rec]) });
      const user = userEvent.setup();
      await user.click(screen.getByRole("button", { name: "Actions" }));
      const off = screen.getByRole("menuitemcheckbox", { name: "Record Computer Audio" });
      expect(off.getAttribute("aria-checked")).toBe("false");
      await user.click(off);
      expect(tauri.calls.find((c) => c.cmd === "set_note_system_audio")?.args).toEqual({
        sessionId: "rec",
        enabled: true,
      });

      view.rerenderWith(withStatus({ system_audio: true, system_audio_permission: "allowed" }), sessionsOf([rec]));
      await user.click(screen.getByRole("button", { name: "Actions" }));
      const on = screen.getByRole("menuitemcheckbox", { name: "Record Computer Audio" });
      expect(on.getAttribute("aria-checked")).toBe("true");
      await user.click(on);
      expect(tauri.calls.filter((c) => c.cmd === "set_note_system_audio").pop()?.args).toEqual({
        sessionId: "rec",
        enabled: false,
      });
    });

    test("a saved note's menu has no microphone or Computer audio item", async () => {
      const e = entry("s");
      mockPage(e);
      renderPage({ id: "s", sessions: sessionsOf([e]) });
      await userEvent.setup().click(screen.getByRole("button", { name: "Actions" }));
      expect(screen.queryByRole("menuitem", { name: "Microphone" })).toBeNull();
      expect(screen.queryByRole("menuitemcheckbox", { name: "Record Computer Audio" })).toBeNull();
    });
  });

  test("Stop is unavailable for the first second, then stops the recording", async () => {
    const e = entry("rec", { state: "capturing" });
    const tauri = mockPage(e, {}, { stop_recording: () => "rec" });
    renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    const stop = screen.getByRole("button", { name: "Stop" }) as HTMLButtonElement;
    expect(stop.disabled).toBe(true);
    await waitFor(() => expect(stop.disabled).toBe(false), { timeout: 2000 });
    await userEvent.setup().click(stop);
    expect(tauri.calls.some((c) => c.cmd === "stop_recording")).toBe(true);
  });

  describe("Pause and Continue", () => {
    const e = entry("rec", { state: "capturing" });
    const paused = (over: Partial<Capture> = {}) =>
      recordingCapture("rec", {
        status: { ...idle, session_id: "rec", state: "capturing", recording: true, paused: true, recorded_ms: 312_000 },
        input: "paused",
        recordedMs: 312_000,
        ...over,
      });
    const toolbarButtons = () =>
      within(screen.getByRole("banner")).getAllByRole("button").map((b) => b.textContent);

    // Covers R1.
    test("on a recording note, the toolbar shows Pause next to Stop", () => {
      mockPage(e);
      renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
      const names = toolbarButtons();
      expect(names.indexOf("Stop")).toBe(names.indexOf("Pause") + 1);
      expect(screen.queryByRole("button", { name: "Continue" })).toBeNull();
    });

    // Covers R1, R8.
    test("on a paused note, the toolbar shows Continue and Stop, and Continue continues the note", async () => {
      const tauri = mockPage(e, {}, { continue_recording: () => null });
      renderPage({ id: "rec", capture: paused(), sessions: sessionsOf([e]) });
      const names = toolbarButtons();
      expect(names.indexOf("Stop")).toBe(names.indexOf("Continue") + 1);
      expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
      await userEvent.setup().click(screen.getByRole("button", { name: "Continue" }));
      expect(tauri.calls.some((c) => c.cmd === "continue_recording")).toBe(true);
    });

    // Covers R5.
    test("on an interrupted note, the toolbar has no Pause or Continue", () => {
      mockPage(e);
      renderPage({
        id: "rec",
        capture: captureOf({
          status: { ...idle, session_id: "rec", state: "interrupted", interruption: { headline: "Microphone lost", disk: false } },
          input: "idle",
        }),
        sessions: sessionsOf([e]),
      });
      expect(screen.getByRole("button", { name: "Stop" })).toBeTruthy();
      expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
      expect(screen.queryByRole("button", { name: "Continue" })).toBeNull();
    });

    // Covers R8.
    test("a paused note's recording bar shows Paused, the frozen time, and no level meter", () => {
      mockPage(e);
      renderPage({ id: "rec", capture: paused({ levels: [0, 0, 0] }), sessions: sessionsOf([e]) });
      const bar = screen.getByRole("region", { name: "Recording" });
      expect(within(bar).getByText("Paused")).toBeTruthy();
      expect(bar.textContent).toContain("00:05:12");
      expect(within(bar).queryByRole("img", { name: /Audio level/ })).toBeNull();
      expect(bar.textContent).not.toMatch(/No audio arriving|Recording/);
    });

    // The WKWebView lesson: a click does not focus a button, so the click
    // handler focuses it.
    test("a click on Pause pauses the note and leaves the focus on the Continue control", async () => {
      const tauri = mockPage(e, {}, { pause_recording: () => null });
      const view = renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
      // The editor of a recording takes the focus when its notes load.
      const editor = await screen.findByRole("textbox", { name: "My Notes" });
      await waitFor(() => expect(editor.contains(document.activeElement)).toBe(true));
      const pause = screen.getByRole("button", { name: "Pause" });
      // A click event with no focus change, as WebKit sends it.
      fireEvent.click(pause);
      await act(async () => {});
      expect(tauri.calls.some((c) => c.cmd === "pause_recording")).toBe(true);
      view.rerenderWith(paused(), sessionsOf([e]));
      expect(document.activeElement).toBe(screen.getByRole("button", { name: "Continue" }));
    });
  });

  test("while recording, Transcript says what comes after Stop, and no text promises a summary", async () => {
    const e = entry("rec", { state: "capturing" });
    mockPage(e);
    renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    const user = userEvent.setup();
    await user.click(tab("Transcript"));
    expect(screen.getByText(/transcript is ready after you stop recording/)).toBeTruthy();
    expect(document.body.textContent).not.toMatch(/summary is written after you stop recording/);
  });

  test("during processing, Summary shows the stage rows and My Notes stays editable", async () => {
    const e = entry("p", { state: "processing", stage: "transcribe" });
    mockPage(e, { done: ["assemble", "diarize"], current: "transcribe", windows_done: 1, windows_total: 4 });
    renderPage({ id: "p", sessions: sessionsOf([e]) });
    expect(tab("Summary").getAttribute("aria-selected")).toBe("true");
    expect(await screen.findByText("Transcribing")).toBeTruthy();
    expect(screen.getByText("Finding speakers")).toBeTruthy();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("25");

    await userEvent.setup().click(tab("My Notes"));
    const editor = await screen.findByRole("textbox", { name: "My Notes" });
    expect(editor.getAttribute("contenteditable")).toBe("true");
  });

  describe("the speaker meter", () => {
    const running = { done: ["assemble"], current: "diarize" };
    const progress = (attempt: string, percent: number | null, measuring: boolean) => ({ attempt, percent, measuring });
    const meter = () => screen.getByRole("progressbar", { name: "Finding speakers" });

    test("Finding speakers comes before Transcribing, with the saved share", async () => {
      // Covers AE1.
      const e = entry("p", { state: "processing", stage: "diarize" });
      mockPage(e, { ...running, speakers: progress("a", 40, true) });
      renderPage({ id: "p", sessions: sessionsOf([e]) });
      const speakers = await screen.findByText("Finding speakers");
      const transcribing = screen.getByText("Transcribing");
      expect(speakers.compareDocumentPosition(transcribing) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      expect(screen.getByText("40%")).toBeTruthy();
      expect(meter().getAttribute("aria-valuenow")).toBe("40");
      // Until a live event arrives, the saved value shows as active work.
      expect(meter().getAttribute("aria-valuetext")).toBe("40%, still working");
    });

    test("live events advance the whole-note value without a reset", async () => {
      // Covers AE2.
      const e = entry("p", { state: "processing", stage: "diarize" });
      mockPage(e, { ...running, speakers: progress("a", 40, true) });
      const page = renderPage({ id: "p", sessions: sessionsOf([e]) });
      await screen.findByText("40%");
      await act(async () => page.rerenderWith(captureOf(), { ...sessionsOf([e]), speakers: { p: progress("a", 62, true) } }));
      expect(screen.getByText("62%")).toBeTruthy();
      expect(meter().getAttribute("aria-valuetext")).toBe("62%");
      expect(meter().className).not.toContain("progress-active");
      // A percent that does not change for a while shows activity too (R4).
      await act(async () => new Promise((resolve) => setTimeout(resolve, 3100)));
      expect(meter().getAttribute("aria-valuetext")).toBe("62%, still working");
      expect(meter().className).toContain("progress-active");
    });

    test("work with no fraction shows activity, not a new number", async () => {
      // Covers AE3.
      const e = entry("p", { state: "processing", stage: "diarize" });
      mockPage(e, { ...running, speakers: progress("a", null, false) });
      const page = renderPage({ id: "p", sessions: sessionsOf([e]) });
      await screen.findByText("Finding speakers");
      expect(meter().getAttribute("aria-valuenow")).toBeNull();
      expect(meter().getAttribute("aria-valuetext")).toBe("In progress");
      expect(meter().className).toContain("progress-active");
      // The reduced-motion text is in the row.
      expect(screen.getByText("Still working")).toBeTruthy();
      await act(async () => page.rerenderWith(captureOf(), { ...sessionsOf([e]), speakers: { p: progress("a", 99, false) } }));
      expect(screen.getByText("99%")).toBeTruthy();
      expect(meter().className).toContain("progress-active");
    });

    test("the stage never shows 100%, and a done stage removes the meter", async () => {
      // Covers AE4.
      const e = entry("p", { state: "processing", stage: "transcribe" });
      mockPage(e, { done: ["assemble", "diarize"], current: "transcribe", speakers: null });
      renderPage({ id: "p", sessions: { ...sessionsOf([e]), speakers: { p: progress("a", 99, false) } } });
      await screen.findByText("Transcribing");
      expect(screen.queryByRole("progressbar", { name: "Finding speakers" })).toBeNull();
      expect(screen.queryByText("100%")).toBeNull();
    });

    test("a failed stage clears the meter of its attempt", async () => {
      const e = entry("f", { state: "processing_failed", stage: "diarize" });
      mockPage(e, { ...running, failed_stage: "diarize", message: "boom", speakers: null });
      renderPage({ id: "f", sessions: { ...sessionsOf([e]), speakers: { f: progress("a", 50, true) } } });
      await screen.findByText("Retry");
      expect(screen.queryByRole("progressbar", { name: "Finding speakers" })).toBeNull();
    });
  });

  test("a failed session shows Retry on the page", async () => {
    const e = entry("f", { state: "processing_failed", stage: "transcribe" });
    const tauri = mockPage(
      e,
      { failed_stage: "transcribe", message: "the model crashed" },
      { retry_session: () => null },
    );
    renderPage({ id: "f", sessions: sessionsOf([e]) });
    const retry = await screen.findByRole("button", { name: "Retry" });
    await userEvent.setup().click(retry);
    expect(tauri.calls.find((c) => c.cmd === "retry_session")?.args).toEqual({ sessionId: "f", startOver: false });
  });

  test("a second failure at the same step asks before Retry starts over", async () => {
    const e = entry("f", { state: "processing_failed", stage: "diarize" });
    const tauri = mockPage(
      e,
      { failed_stage: "diarize", message: "the live text does not match", retry_starts_over: true },
      { retry_session: () => null },
    );
    renderPage({ id: "f", sessions: sessionsOf([e]) });
    const user = userEvent.setup();
    expect(await screen.findByText("Retry starts over from the recorded audio.")).toBeTruthy();
    await user.click(await screen.findByRole("button", { name: "Retry" }));
    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(tauri.calls.some((c) => c.cmd === "retry_session")).toBe(false);
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Start Over" }));
    expect(tauri.calls.find((c) => c.cmd === "retry_session")?.args).toEqual({ sessionId: "f", startOver: true });
  });

  test("Back returns to Home, and the recording continues", async () => {
    const e = entry("rec", { state: "capturing" });
    const tauri = mockPage(e);
    const onBack = mock(() => {});
    renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]), onBack });
    await userEvent.setup().click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalled();
    expect(tauri.calls.some((c) => c.cmd === "stop_recording")).toBe(false);
  });

  // Covers AE5.
  test("the menu hides Delete for a processing session", async () => {
    const e = entry("p", { state: "processing" });
    mockPage(e);
    renderPage({ id: "p", sessions: sessionsOf([e]) });
    await userEvent.setup().click(screen.getByRole("button", { name: "Actions" }));
    expect(screen.getByRole("menuitem", { name: "Rename" })).toBeTruthy();
    expect(screen.queryByRole("menuitem", { name: "Delete…" })).toBeNull();
  });

  test("a stopped session shows Resume, which records again, and its menu has Delete", async () => {
    const e = entry("s");
    const tauri = mockPage(e, { resumable: true, record: { kind: "resume", available: true, reason: null } }, {
      resume_recording: () => "s",
    });
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    await act(async () => {});
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Actions" }));
    expect(screen.queryByRole("menuitem", { name: "Resume Recording" })).toBeNull();
    expect(screen.getByRole("menuitem", { name: "Delete…" })).toBeTruthy();
    await user.keyboard("{Escape}");
    const resume = screen.getByRole("button", { name: "Resume" });
    expect(resume.hasAttribute("disabled")).toBe(false);
    await user.click(resume);
    expect(tauri.calls.filter((c) => c.cmd === "resume_recording").map((c) => c.args)).toEqual([
      { sessionId: "s" },
    ]);
  });

  test("Delete in the menu asks once, moves the session to the Trash, and goes back", async () => {
    const e = entry("s", { title: "Budget review" });
    const tauri = mockPage(e, {}, { delete_session: () => null });
    const onBack = mock(() => {});
    const sessions = sessionsOf([e]);
    renderPage({ id: "s", sessions, onBack });
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Delete…" }));
    const confirm = screen.getByRole("alertdialog", { name: "Delete note?" });
    await user.click(within(confirm).getByRole("button", { name: "Delete" }));
    expect(tauri.calls.filter((c) => c.cmd === "delete_session").map((c) => c.args)).toEqual([
      { sessionId: "s" },
    ]);
    expect(onBack).toHaveBeenCalled();
    expect(sessions.refresh).toHaveBeenCalled();
  });

  test("Rename in the menu puts the cursor in the title field", async () => {
    const e = entry("s", { title: "Budget review" });
    mockPage(e);
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "Title" }));
  });

  test("a session that completes while its page is open shows Summary", async () => {
    const p = entry("p", { state: "processing" });
    mockPage(p);
    const view = renderPage({ id: "p", sessions: sessionsOf([p]) });
    const user = userEvent.setup();
    await user.click(tab("Transcript"));
    view.rerenderWith(captureOf(), sessionsOf([{ ...p, state: "saved" }]));
    expect(tab("Summary").getAttribute("aria-selected")).toBe("true");
  });

  test("a session that completes while the user types in My Notes stays on My Notes", async () => {
    const p = entry("p", { state: "processing" });
    mockPage(p);
    const view = renderPage({ id: "p", sessions: sessionsOf([p]) });
    await userEvent.setup().click(tab("My Notes"));
    const editor = await screen.findByRole("textbox", { name: "My Notes" });
    editor.focus();
    expect(editor.contains(document.activeElement)).toBe(true);
    view.rerenderWith(captureOf(), sessionsOf([{ ...p, state: "saved" }]));
    expect(tab("My Notes").getAttribute("aria-selected")).toBe("true");
  });

  test("Copy writes the open tab's markdown: the transcript file, or the notes", async () => {
    const e = entry("s", { title: "Budget review" });
    mockTauri({
      load_session_notes: () => ({ markdown: "# Agenda\n\n- ask about headcount", fingerprint: "f0" }),
      save_session_notes: () => ({ kind: "saved", fingerprint: "f1" }),
      session_detail: () => detail(e),
      session_document: () => doc,
    });
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    const user = userEvent.setup();
    // After setup, which puts its own clipboard in place.
    const writeText = mock(async (_t: string) => {});
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    await screen.findByText("We agreed the budget.");
    await user.click(tab("Transcript"));
    await user.click(screen.getByRole("button", { name: "Copy" }));
    expect(writeText).toHaveBeenLastCalledWith("# Transcript\n\n- [00:00:00] Speaker 1: Hello.\n");
    expect(screen.getByRole("button", { name: "Copied" })).toBeTruthy();
    await user.click(tab("My Notes"));
    await screen.findByRole("textbox", { name: "My Notes" });
    await user.click(screen.getByRole("button", { name: /^Cop/ }));
    expect(writeText).toHaveBeenLastCalledWith("# Agenda\n\n- ask about headcount");
  });

  test("a stub summary shows its text and Regenerate, which summarizes again", async () => {
    const e = entry("s");
    const stub = {
      ...doc,
      summary: {
        body: "# Summary\n\nNo summary was produced: the endpoint did not answer",
        stub: true,
        host: null,
        model: null,
        prompt: null,
        prompt_id: null,
      },
    };
    const tauri = mockPage(e, {}, { session_document: () => stub, regenerate_summary: () => "/x/summary.md" });
    const sessions = sessionsOf([e]);
    renderPage({ id: "s", sessions });
    expect(await screen.findByText(/No summary was produced/)).toBeTruthy();
    await userEvent.setup().click(screen.getByRole("button", { name: "Regenerate Summary" }));
    await act(async () => {});
    expect(tauri.calls.filter((c) => c.cmd === "regenerate_summary").map((c) => c.args)).toEqual([
      { sessionId: "s" },
    ]);
    expect(sessions.refresh).toHaveBeenCalled();
  });

  describe("Regenerate Summary", () => {
    const endpoint = { host: "api.example.com", model: "m", encrypted: true };
    const newDoc = { ...doc, summary: { ...doc.summary!, body: "The launch moves to May 12." } };

    /// A regenerate_summary that completes only when the test says.
    function pending() {
      let finish: (fail?: string | Error) => void = () => {};
      const run = () =>
        new Promise((resolve, reject) => {
          finish = (fail) => (fail ? reject(fail) : resolve("/x/summary.md"));
        });
      return { run, finish: (fail?: string | Error) => finish(fail) };
    }

    async function start(first: Tab = "summary") {
      const e = entry("s");
      const regen = pending();
      let current = doc;
      mockPage(e, { endpoint }, { session_document: () => current, regenerate_summary: regen.run });
      const sessions = sessionsOf([e]);
      const page = renderPage({ id: "s", sessions });
      await screen.findByText("We agreed the budget.");
      const user = userEvent.setup();
      if (first === "transcript") await user.click(tab("Transcript"));
      await user.click(screen.getByRole("button", { name: "Actions" }));
      await user.click(screen.getByRole("menuitem", { name: "Regenerate Summary" }));
      return {
        e,
        page,
        user,
        sessions,
        succeed: async () => {
          current = newDoc;
          await act(async () => regen.finish());
        },
        fail: async () => act(async () => regen.finish("The endpoint did not answer.")),
        // The binding throws an Error (an IPC failure) instead of returning it.
        crash: async () => act(async () => regen.finish(new Error("IPC failed"))),
      };
    }

    test("the summarize stage shows in place of the summary, and Transcript still shows", async () => {
      const { user } = await start();
      expect(tab("Summary").getAttribute("aria-selected")).toBe("true");
      expect(screen.getByText("Summarizing")).toBeTruthy();
      expect(screen.getByText(/Sends the transcript and notes to api\.example\.com \(m\)/)).toBeTruthy();
      expect(screen.queryByText("We agreed the budget.")).toBeNull();
      expect(screen.queryByRole("button", { name: "Regenerate Summary" })).toBeNull();
      await user.click(tab("Transcript"));
      expect(await screen.findByText("Hello.")).toBeTruthy();
      await user.click(tab("Summary"));
      expect(screen.getByText("Summarizing")).toBeTruthy();
    });

    test("a summary in more than one part shows its progress", async () => {
      const { e, page } = await start();
      expect(screen.queryByRole("progressbar")).toBeNull();
      const live = { s: { stage: "summarize", segment: 0, done: 1, total: 3 } };
      await act(async () => page.rerenderWith(captureOf(), { ...sessionsOf([e]), live }));
      expect(screen.getByText("33%")).toBeTruthy();
      expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("33");
    });

    test("the progress of an earlier run does not show", async () => {
      const e = entry("s");
      const live = { s: { stage: "summarize", segment: 0, done: 3, total: 3 } };
      mockPage(e, { endpoint }, { regenerate_summary: pending().run });
      renderPage({ id: "s", sessions: { ...sessionsOf([e]), live } });
      await screen.findByText("We agreed the budget.");
      const user = userEvent.setup();
      await user.click(screen.getByRole("button", { name: "Regenerate Summary" }));
      expect(screen.getByText("Summarizing")).toBeTruthy();
      expect(screen.queryByRole("progressbar")).toBeNull();
    });

    test("Regenerate from the menu on Transcript moves the page to Summary", async () => {
      await start("transcript");
      expect(tab("Summary").getAttribute("aria-selected")).toBe("true");
      expect(screen.getByText("Summarizing")).toBeTruthy();
    });

    test("when it completes, the new summary shows", async () => {
      const { succeed } = await start();
      await succeed();
      expect(await screen.findByText("The launch moves to May 12.")).toBeTruthy();
      expect(screen.queryByText("Summarizing")).toBeNull();
      expect(screen.getByRole("button", { name: "Regenerate Summary" })).toBeTruthy();
    });

    test("when it fails, the previous summary shows again with the error", async () => {
      const { fail } = await start();
      await fail();
      expect(await screen.findByText("We agreed the budget.")).toBeTruthy();
      expect(screen.getByText("The endpoint did not answer.")).toBeTruthy();
      expect(screen.queryByText("Summarizing")).toBeNull();
    });

    test("when the call throws, the previous summary shows again with an error", async () => {
      const { crash } = await start();
      const quiet = spyOn(console, "error").mockImplementation(() => {});
      await crash();
      quiet.mockRestore();
      expect(await screen.findByText("We agreed the budget.")).toBeTruthy();
      expect(screen.getByText("The summary could not be updated.")).toBeTruthy();
      expect(screen.queryByText("Summarizing")).toBeNull();
    });
  });

  describe("Transcribe Again", () => {
    const whisper = { model_id: "whisper-large-v3-turbo-q5km", model_name: "Whisper Large v3 Turbo" };

    async function openMenu(extra: Record<string, CommandHandler>) {
      const e = entry("s", { title: "Budget review" });
      const tauri = mockPage(e, {}, extra);
      const sessions = sessionsOf([e]);
      const view = renderPage({ id: "s", sessions });
      await act(async () => {});
      const user = userEvent.setup();
      await user.click(screen.getByRole("button", { name: "Actions" }));
      await user.click(screen.getByRole("menuitem", { name: "Transcribe Again…" }));
      return { e, tauri, sessions, user, view };
    }

    // Covers AE1.
    test("one plain confirmation queues the note with the model of the check", async () => {
      const { tauri, sessions, user } = await openMenu({
        check_transcribe_again: () => whisper,
        transcribe_again: () => null,
      });
      const dialog = await screen.findByRole("alertdialog", { name: "Transcribe this note again?" });
      expect(dialog.textContent).toContain("The transcript and summary are not available until the new ones are ready.");
      expect(dialog.textContent).toContain("If it fails, the current ones come back.");
      expect(dialog.textContent).toContain("This uses Whisper Large v3 Turbo.");
      expect(within(dialog).queryByRole("checkbox")).toBeNull();
      expect(document.activeElement).toBe(within(dialog).getByRole("button", { name: "Cancel" }));
      await user.click(within(dialog).getByRole("button", { name: "Transcribe Again" }));
      expect(tauri.calls.filter((c) => c.cmd === "transcribe_again").map((c) => c.args)).toEqual([
        { sessionId: "s", modelId: whisper.model_id },
      ]);
      expect(screen.queryByRole("alertdialog")).toBeNull();
      expect(sessions.refresh).toHaveBeenCalled();
    });

    test("Cancel and Escape close the dialog and send nothing", async () => {
      const { tauri, user } = await openMenu({ check_transcribe_again: () => whisper });
      await user.click(await screen.findByRole("button", { name: "Cancel" }));
      expect(screen.queryByRole("alertdialog")).toBeNull();
      await user.click(screen.getByRole("button", { name: "Actions" }));
      await user.click(screen.getByRole("menuitem", { name: "Transcribe Again…" }));
      await screen.findByRole("alertdialog");
      await user.keyboard("{Escape}");
      expect(screen.queryByRole("alertdialog")).toBeNull();
      expect(tauri.calls.some((c) => c.cmd === "transcribe_again")).toBe(false);
    });

    test("two clicks on Transcribe Again send one command", async () => {
      let finish = () => {};
      const { tauri, user } = await openMenu({
        check_transcribe_again: () => whisper,
        transcribe_again: () => new Promise<null>((resolve) => (finish = () => resolve(null))),
      });
      const run = await screen.findByRole("button", { name: "Transcribe Again" });
      await user.dblClick(run);
      await act(async () => finish());
      expect(tauri.calls.filter((c) => c.cmd === "transcribe_again")).toHaveLength(1);
    });

    // Covers AE3.
    test("a setting that cannot run shows the reason and Open Settings, and no dialog", async () => {
      const { tauri, user } = await openMenu({
        check_transcribe_again: () => {
          throw { message: "Whisper Large v3 Turbo is not downloaded. Download it in Settings.", in_settings: true };
        },
        open_settings: () => null,
      });
      expect(await screen.findByText("Whisper Large v3 Turbo is not downloaded. Download it in Settings.")).toBeTruthy();
      expect(screen.queryByRole("alertdialog")).toBeNull();
      await user.click(screen.getByRole("button", { name: "Open Settings…" }));
      expect(tauri.calls.some((c) => c.cmd === "open_settings")).toBe(true);
      expect(tauri.calls.some((c) => c.cmd === "transcribe_again")).toBe(false);
    });

    test("a reason that Settings cannot fix has no Open Settings button", async () => {
      await openMenu({
        check_transcribe_again: () => {
          throw { message: "Whisper Large v3 Turbo is still downloading. Try again when it is ready.", in_settings: false };
        },
      });
      expect(await screen.findByText("Whisper Large v3 Turbo is still downloading. Try again when it is ready.")).toBeTruthy();
      expect(screen.queryByRole("button", { name: "Open Settings…" })).toBeNull();
    });

    test("a refused confirmation closes the dialog and shows the reason", async () => {
      const { user } = await openMenu({
        check_transcribe_again: () => whisper,
        transcribe_again: () => {
          throw { message: "The transcription settings changed. Choose Transcribe Again to use the new settings.", in_settings: false };
        },
      });
      await user.click(await screen.findByRole("button", { name: "Transcribe Again" }));
      expect(await screen.findByText("The transcription settings changed. Choose Transcribe Again to use the new settings.")).toBeTruthy();
      expect(screen.queryByRole("alertdialog")).toBeNull();
    });

    test("the dialog closes when the note starts to process", async () => {
      const { e, view } = await openMenu({ check_transcribe_again: () => whisper });
      await screen.findByRole("alertdialog");
      await act(async () => view.rerenderWith(captureOf(), sessionsOf([{ ...e, state: "processing" }])));
      expect(screen.queryByRole("alertdialog")).toBeNull();
    });

    test("a check that returns after the note was unavailable opens no dialog", async () => {
      let finish = () => {};
      const { e, view } = await openMenu({
        check_transcribe_again: () => new Promise((resolve) => (finish = () => resolve(whisper))),
      });
      await act(async () => view.rerenderWith(captureOf(), sessionsOf([{ ...e, state: "processing" }])));
      await act(async () => view.rerenderWith(captureOf(), sessionsOf([e])));
      await act(async () => finish());
      expect(screen.queryByRole("alertdialog")).toBeNull();
    });


    test("after a failed run, the page says the previous transcript is unchanged", async () => {
      const e = entry("s");
      mockPage(e, { transcribe_again_error: "decoder exploded" });
      renderPage({ id: "s", sessions: sessionsOf([e]) });
      expect(await screen.findByText("The new transcript failed. The previous transcript is unchanged.")).toBeTruthy();
      expect(screen.getByText("We agreed the budget.")).toBeTruthy();
    });

    test("a run that kept the previous summary says so on Summary", async () => {
      const e = entry("s");
      mockPage(e, { summary_not_updated: "the endpoint did not answer" });
      renderPage({ id: "s", sessions: sessionsOf([e]) });
      expect(await screen.findByText("The summary could not be updated. It is from the previous transcript.")).toBeTruthy();
      expect(screen.getByText("We agreed the budget.")).toBeTruthy();
    });

    test("the menu has no Transcribe Again while the note processes", async () => {
      const p = entry("p", { state: "processing" });
      mockPage(p, { transcribe_again: true });
      renderPage({ id: "p", sessions: sessionsOf([p]) });
      await act(async () => {});
      await userEvent.setup().click(screen.getByRole("button", { name: "Actions" }));
      expect(screen.queryByRole("menuitem", { name: "Transcribe Again…" })).toBeNull();
    });
  });

  describe("Remove Audio", () => {
    const removedReason = "The audio of this note was removed.";

    async function openMenu(e: SessionEntry, over: Partial<SessionDetail>, extra: Record<string, CommandHandler> = {}) {
      const tauri = mockPage(e, over, extra);
      const sessions = sessionsOf([e]);
      renderPage({ id: e.id, sessions });
      await act(async () => {});
      const user = userEvent.setup();
      await user.click(screen.getByRole("button", { name: "Actions" }));
      return { tauri, sessions, user };
    }

    const item = (name: string) => screen.getByRole("menuitem", { name });

    // Covers AE8. The page sends no setting: the command removes the audio
    // whether the setting is on or off.
    test("one destructive confirmation removes the audio, and then Transcribe Again is unavailable", async () => {
      const e = entry("s", { title: "Budget review" });
      let audio: SessionDetail["audio"] = "present";
      const tauri = mockTauri({
        load_session_notes: () => ({ markdown: "", fingerprint: "f0" }),
        session_detail: () =>
          detail(e, {
            audio,
            resumable: audio === "present",
            record: { kind: "resume", available: audio === "present", reason: audio === "present" ? null : removedReason },
          }),
        session_document: () => doc,
        remove_audio: () => {
          audio = "removed";
          return null;
        },
      });
      const sessions = sessionsOf([e]);
      renderPage({ id: "s", sessions });
      await act(async () => {});
      const user = userEvent.setup();
      await user.click(screen.getByRole("button", { name: "Actions" }));
      await user.click(item("Remove Audio…"));
      const dialog = screen.getByRole("alertdialog", { name: "Remove the audio of this note?" });
      expect(dialog.textContent).toContain("The transcript, summary and notes stay.");
      expect(dialog.textContent).toContain("You can’t undo this.");
      expect(document.activeElement).toBe(within(dialog).getByRole("button", { name: "Cancel" }));
      await user.click(within(dialog).getByRole("button", { name: "Remove Audio" }));
      expect(tauri.calls.filter((c) => c.cmd === "remove_audio").map((c) => c.args)).toEqual([{ sessionId: "s" }]);
      await act(async () => {});
      expect(screen.queryByRole("alertdialog")).toBeNull();
      expect(sessions.refresh).toHaveBeenCalled();

      await user.click(screen.getByRole("button", { name: "Actions" }));
      for (const name of ["Transcribe Again…", "Remove Audio…"]) {
        expect(item(name).hasAttribute("disabled")).toBe(true);
        expect(item(name).getAttribute("title")).toBe(removedReason);
      }
      const resume = screen.getByRole("button", { name: "Resume" });
      expect(resume.hasAttribute("disabled")).toBe(true);
      expect(resume.closest("[title]")?.getAttribute("title")).toBe(removedReason);
    });

    test("Cancel and Escape close the dialog and remove nothing", async () => {
      const { tauri, user } = await openMenu(entry("s"), {}, { remove_audio: () => null });
      await user.click(item("Remove Audio…"));
      await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Cancel" }));
      expect(screen.queryByRole("alertdialog")).toBeNull();
      await user.click(screen.getByRole("button", { name: "Actions" }));
      await user.click(item("Remove Audio…"));
      await user.keyboard("{Escape}");
      expect(screen.queryByRole("alertdialog")).toBeNull();
      expect(tauri.calls.some((c) => c.cmd === "remove_audio")).toBe(false);
    });

    test("a refused removal closes the dialog and shows the reason", async () => {
      const message = "This note is in use on another computer. Try again later.";
      const { user } = await openMenu(entry("s"), {}, {
        remove_audio: () => {
          throw { kind: "refused", message };
        },
      });
      await user.click(item("Remove Audio…"));
      await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Remove Audio" }));
      expect(await screen.findByText(message)).toBeTruthy();
      expect(screen.queryByRole("alertdialog")).toBeNull();
    });

    // Covers AE6.
    test("a note with removed audio shows no mark, and its menu gives the removed reason", async () => {
      const e = entry("s");
      mockPage(e, { audio: "removed", resumable: false });
      renderPage({ id: "s", sessions: sessionsOf([e]) });
      await act(async () => {});
      expect(screen.queryByText(/audio/i)).toBeNull();
      expect(screen.queryAllByRole("status")).toEqual([]);
      await userEvent.setup().click(screen.getByRole("button", { name: "Actions" }));
      expect(item("Transcribe Again…").hasAttribute("disabled")).toBe(true);
      expect(item("Transcribe Again…").getAttribute("title")).toBe(removedReason);
      expect(item("Regenerate Summary").hasAttribute("disabled")).toBe(false);
      expect(item("Delete…").hasAttribute("disabled")).toBe(false);
    });

    test("a note whose audio waits for the sync keeps the syncing reason and can remove its audio", async () => {
      await openMenu(entry("s"), { audio: "waiting", waiting: ["audio"], resumable: true });
      expect(item("Transcribe Again…").getAttribute("title")).toBe("Wait until the note finishes syncing.");
      expect(item("Remove Audio…").hasAttribute("disabled")).toBe(false);
    });

    test("a note in an unavailable storage folder can't remove its audio", async () => {
      await openMenu(entry("s", { storage_unavailable: true }), {});
      expect(item("Remove Audio…").hasAttribute("disabled")).toBe(true);
      expect(item("Remove Audio…").getAttribute("title")).toBe("The storage folder for this note is unavailable.");
    });

    test("a note that processes has no Remove Audio", async () => {
      await openMenu(entry("p", { state: "processing" }), {});
      expect(screen.queryByRole("menuitem", { name: "Remove Audio…" })).toBeNull();
    });

    test("Retry of a failed note whose audio was removed is unavailable and gives the removed reason", async () => {
      const e = entry("f", { state: "processing_failed", stage: "transcribe" });
      const tauri = mockPage(e, { failed_stage: "transcribe", audio: "removed" }, { retry_session: () => null });
      renderPage({ id: "f", sessions: sessionsOf([e]) });
      const retry = await screen.findByRole("button", { name: "Retry" });
      expect(retry.hasAttribute("disabled")).toBe(true);
      expect(screen.getByText(removedReason)).toBeTruthy();
      await userEvent.setup().click(retry);
      expect(tauri.calls.some((c) => c.cmd === "retry_session")).toBe(false);
    });
  });

  // Covers AE6.
  test("the summary footer shows the name of the prompt that made the summary", async () => {
    const e = entry("s");
    const named = { ...doc, summary: { ...doc.summary!, prompt: "Team sync" } };
    mockPage(e, {}, { session_document: () => named });
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    expect(await screen.findByText("Team sync · Summarized by m on api.example.com")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Regenerate Summary" })).toBeTruthy();
  });

  test("a summary with no prompt name shows the footer with no prompt name", async () => {
    const e = entry("s");
    mockPage(e);
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    expect(await screen.findByText("Summarized by m on api.example.com")).toBeTruthy();
    expect(document.body.textContent).not.toContain(" · Summarized by");
  });

  test("a summary with nested bullets renders the children as nested lists", async () => {
    const e = entry("s");
    const nested = {
      ...doc,
      summary: { ...doc.summary!, body: "*   **Budget:** costs\n    *   $40,000\n    *   two contractors\n*   **Decision:** launch on May 12" },
    };
    mockPage(e, {}, { session_document: () => nested });
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    const amount = await screen.findByText("$40,000");
    const inner = amount.closest("ul")!;
    const outerItem = inner.parentElement!.closest("li")!;
    expect(outerItem.textContent).toContain("Budget:");
    expect(within(inner).getAllByRole("listitem")).toHaveLength(2);
    expect(outerItem.closest("ul")!.querySelectorAll(":scope > li")).toHaveLength(2);
  });

  test("the Transcript tab of a saved session shows the header and the turns", async () => {
    const e = entry("s");
    mockPage(e);
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    await screen.findByText("We agreed the budget.");
    await userEvent.setup().click(tab("Transcript"));
    expect(screen.getByText("3 speakers · 35:48")).toBeTruthy();
    expect(screen.getByText("Hello.")).toBeTruthy();
  });

  describe("live transcript", () => {
    const saidSoFar: LiveTranscript = {
      recognition: [],
      parts: [{ segment: 0, part: 0, offset_ms: 0, words: [{ start_ms: 0, end_ms: 2_000, text: "What we said so far." }], start_ms: 0, end_ms: 240_000, text: "What we said so far." }],
      current: null,
      state: "running",
    };
    test("with live mode on, the page opens on My Notes, an event does not change the tab, and Transcript shows the text", async () => {
      const e = entry("rec", { state: "capturing" });
      const tauri = mockPage(
        e,
        { live_transcript: true, asr_model: "parakeet-unified-en-0.6b" },
        { live_transcript: () => saidSoFar },
      );
      renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
      await act(async () => {});
      await act(() =>
        tauri.emit("live-transcript-event", {
          session_id: "rec",
          segment: 0,
          part: 1,
          offset_ms: 0,
          word_start: 0,
          words: ["And", "then", "we", "agreed"].map((text, i) => ({ start_ms: 250_000 + i * 500, end_ms: 250_500 + i * 500, text })),
          committed: "And then",
          tentative: " we",
        }),
      );
      expect(tab("My Notes").getAttribute("aria-selected")).toBe("true");
      await userEvent.setup().click(tab("Transcript"));
      expect(await screen.findByText("What we said so far.")).toBeTruthy();
      expect(screen.queryByText(/ready after you stop/)).toBeNull();
      await act(() =>
        tauri.emit("live-transcript-event", {
          session_id: "rec",
          segment: 0,
          part: 1,
          offset_ms: 0,
          word_start: 0,
          words: ["And", "then", "we", "agreed"].map((text, i) => ({ start_ms: 250_000 + i * 500, end_ms: 250_500 + i * 500, text })),
          committed: "And then we",
          tentative: " agreed",
        }),
      );
      expect(screen.getByText("And then we")).toBeTruthy();
    });

    test("a paused note shows the paused line on Transcript", async () => {
      const e = entry("rec", { state: "capturing" });
      mockPage(e, { live_transcript: true, asr_model: "parakeet-unified-en-0.6b" }, { live_transcript: () => ({ ...saidSoFar, state: "behind" }) });
      renderPage({
        id: "rec",
        capture: recordingCapture("rec", { status: { ...idle, session_id: "rec", state: "capturing", recording: true, paused: true } }),
        sessions: sessionsOf([e]),
      });
      await act(async () => {});
      await userEvent.setup().click(tab("Transcript"));
      expect(await screen.findByText("What we said so far.")).toBeTruthy();
      expect(screen.getByText("Paused. Nothing is recorded until you continue.")).toBeTruthy();
      expect(screen.queryByText(/The missing part/)).toBeNull();
    });

    test("offline text still shows independent live speaker analysis", async () => {
      const e = entry("rec", { state: "capturing" });
      const marker = { session_id: "rec", generation: "g", epoch: 0, key: "speakers", revision: 1, rows: [] };
      const tauri = mockPage(e, { live_transcript: false }, { live_transcript: () => ({ parts: [], current: null, state: "running", recognition: [
        { ...marker, phase: "snapshot_begin" },
        { ...marker, phase: "snapshot_batch", rows: [{ start_sample: 0, end_sample: 16000, text: null, speaker: "g:0:0:1", text_final: false, speaker_final: false }] },
        { ...marker, phase: "snapshot_end" },
      ] }) });
      renderPage({
        id: "rec",
        capture: recordingCapture("rec"),
        sessions: sessionsOf([e]),
      });
      await act(async () => {});
      await userEvent.setup().click(tab("Transcript"));
      expect(await screen.findByText("Speaker analysis so far")).toBeTruthy();
      expect(screen.getByText("The transcript is ready after you stop recording.")).toBeTruthy();
      expect(tauri.calls.some((c) => c.cmd === "live_transcript")).toBe(true);
    });

    test("a note whose model changed during the recording still shows its live text", async () => {
      const e = entry("rec", { state: "capturing" });
      mockPage(
        e,
        { live_transcript: true, asr_model: "parakeet-unified-en-0.6b" },
        { live_transcript: () => saidSoFar },
      );
      const sessions = sessionsOf([e]);
      const view = renderPage({ id: "rec", capture: recordingCapture("rec"), sessions });
      await act(async () => {});
      await userEvent.setup().click(tab("Transcript"));
      expect(await screen.findByText("What we said so far.")).toBeTruthy();
      // The user picks a model that does not stream. The page reads the
      // detail again, which keeps the note's own model and live mode (R14).
      view.rerenderWith(recordingCapture("rec"), { ...sessions, version: { rec: 1 } });
      await act(async () => {});
      expect(screen.getByText("What we said so far.")).toBeTruthy();
    });

    test("after Stop, the live text stays on Transcript until the saved transcript loads", async () => {
      const p = entry("p", { state: "processing", stage: "diarize" });
      let release: (d: SessionDocument) => void = () => {};
      mockPage(
        p,
        { live_transcript: true },
        {
          live_transcript: () => ({ ...saidSoFar, state: "ended" }),
          session_document: () => new Promise<SessionDocument>((r) => (release = r)),
        },
      );
      const view = renderPage({ id: "p", sessions: sessionsOf([p]) });
      await act(async () => {});
      const user = userEvent.setup();
      await user.click(tab("Transcript"));
      expect(await screen.findByText("What we said so far.")).toBeTruthy();
      expect(screen.getByText(/final transcript/)).toBeTruthy();

      view.rerenderWith(captureOf(), sessionsOf([{ ...p, state: "saved" }]));
      await user.click(tab("Transcript"));
      expect(await screen.findByText("What we said so far.")).toBeTruthy();
      await act(async () => release(doc));
      expect(screen.getByText("Hello.")).toBeTruthy();
      expect(screen.queryByText("What we said so far.")).toBeNull();
    });

    test("a failed note shows the processing rows on Transcript, not the live text", async () => {
      const f = entry("f", { state: "processing_failed", stage: "diarize" });
      mockPage(f, { live_transcript: true, failed_stage: "diarize", message: "boom" }, { live_transcript: () => saidSoFar });
      renderPage({ id: "f", sessions: sessionsOf([f]) });
      await act(async () => {});
      await userEvent.setup().click(tab("Transcript"));
      expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
      expect(screen.queryByText("What we said so far.")).toBeNull();
    });
  });

  test("a late session detail never replaces a newer one", async () => {
    const e = entry("s");
    let release: (d: SessionDetail) => void = () => {};
    const late = new Promise<SessionDetail>((r) => {
      release = r;
    });
    let n = 0;
    const resumeOf = (available: boolean) => ({
      record: { kind: "resume" as const, available, reason: available ? null : "No." },
    });
    mockPage(e, {}, { session_detail: () => (n++ === 0 ? late : detail(e, resumeOf(true))) });
    const sessions = sessionsOf([e]);
    const view = renderPage({ id: "s", sessions });
    await act(async () => {});
    // A change to the session reads the detail again, and that read ends first.
    view.rerenderWith(captureOf(), { ...sessions, version: { s: 1 } });
    await act(async () => {});
    await act(async () => release(detail(e, resumeOf(false))));
    expect(screen.getByRole("button", { name: "Resume" }).hasAttribute("disabled")).toBe(false);
  });
});

/// The page with the real session list, so library events reach it.
function Synced({ id }: { id: string }) {
  const sessions = useSessions();
  return <SessionPage id={id} capture={captureOf()} sessions={sessions} readiness={readiness} onBack={() => {}} />;
}

describe("SessionPage type and tags row", () => {
  const typeRow = () => screen.getByRole("group", { name: "Type and tags" });

  test("the row under the title shows the type and the tags on a recording, a processing and a saved note", async () => {
    const tagged = { note_type: "One on One", tags: ["Person::Nathan"] };
    const rec = entry("s", { ...tagged, state: "capturing", duration_ms: 0 });
    mockPage(rec);
    const view = renderPage({ id: "s", capture: recordingCapture("s"), sessions: sessionsOf([rec]) });
    for (const [capture, e] of [
      [recordingCapture("s"), rec],
      [captureOf(), { ...rec, state: "processing", stage: "transcribe" }],
      [captureOf(), { ...rec, state: "saved" }],
    ] as const) {
      view.rerenderWith(capture, sessionsOf([e]));
      expect(within(typeRow()).getByRole("button", { name: "Type: One on One" })).toBeTruthy();
      expect(within(typeRow()).getByRole("button", { name: "Remove Person: Nathan" })).toBeTruthy();
      expect(within(typeRow()).getByRole("button", { name: "Add Tag" })).toBeTruthy();
    }
  });

  test("during a recording, a capture tick does not change the row", async () => {
    const e = entry("rec", { state: "capturing", duration_ms: 0, note_type: "One on One", tags: ["urgent"] });
    mockPage(e);
    const view = renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    await screen.findByRole("textbox", { name: "My Notes" });
    const before = typeRow().outerHTML;
    view.rerenderWith(recordingCapture("rec", { recordedMs: 9 * 60_000, levels: [0.4, 0.9, 0.1] }), sessionsOf([e]));
    view.rerenderWith(recordingCapture("rec", { input: "quiet", levels: [0, 0, 0] }), sessionsOf([{ ...e }]));
    expect(typeRow().outerHTML).toBe(before);
  });

  test("during a recording, a tag edit from another computer shows only after the recording stops", async () => {
    const e = entry("rec", { state: "capturing", duration_ms: 0, tags: ["urgent"] });
    mockPage(e);
    const view = renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    const before = typeRow().outerHTML;
    const synced = { ...e, tags: ["urgent", "Person::Ana"], note_type: "Pathfinder" };
    view.rerenderWith(recordingCapture("rec"), sessionsOf([synced]));
    expect(typeRow().outerHTML).toBe(before);
    view.rerenderWith(captureOf(), sessionsOf([{ ...synced, state: "processing", stage: "transcribe" }]));
    expect(within(typeRow()).getByRole("button", { name: "Remove Person: Ana" })).toBeTruthy();
    expect(within(typeRow()).getByRole("button", { name: "Type: Pathfinder" })).toBeTruthy();
  });

  test("a type change on a busy note shows the new type and no folder notice", async () => {
    const e = entry("p", { state: "processing", stage: "transcribe" });
    const tauri = mockPage(e, {}, { set_note_type: () => ({ kind: "later" }) });
    const sessions = sessionsOf([e]);
    renderPage({ id: "p", sessions });
    const user = userEvent.setup();
    await user.click(within(typeRow()).getByRole("button", { name: "Type: General" }));
    await user.click(await screen.findByRole("menuitemradio", { name: "Pathfinder" }));
    expect(await within(typeRow()).findByRole("button", { name: "Type: Pathfinder" })).toBeTruthy();
    expect(tauri.calls.find((c) => c.cmd === "set_note_type")?.args).toEqual({ sessionId: "p", noteType: "Pathfinder" });
    await waitFor(() => expect(sessions.refresh).toHaveBeenCalled());
    expect(screen.queryByText(/The type changed/)).toBeNull();
    expect(screen.queryByText(/The folder moves/)).toBeNull();
  });

  test("text typed in + Tag never reaches My Notes", async () => {
    const e = entry("rec", { state: "capturing", duration_ms: 0 });
    const tauri = mockPage(e, {}, {
      load_session_notes: () => ({ markdown: "first", fingerprint: "f0" }),
      set_note_tags: (args) => args.add,
    });
    renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    const editor = await screen.findByRole("textbox", { name: "My Notes" });
    await screen.findByText("first");
    const user = userEvent.setup();
    expect(editor.contains(document.activeElement)).toBe(true);
    await user.click(within(typeRow()).getByRole("button", { name: "Add Tag" }));
    expect(editor.contains(window.getSelection()?.anchorNode ?? null)).toBe(false);
    expect(document.activeElement).toBe(screen.getByRole("combobox", { name: "Tag" }));
    await user.keyboard("Follow up{Enter}");
    expect(await within(typeRow()).findByRole("button", { name: "Remove Follow up" })).toBeTruthy();
    expect(editor.textContent).toBe("first");
    expect(tauri.calls.some((c) => c.cmd === "save_session_notes")).toBe(false);
  });
});

describe("SessionPage and the synced library", () => {
  test("a loaded note uses its saved speaker mode and updates the notice when its transcript changes", async () => {
    const e = entry("s", { found_speakers: null });
    let transcriptMarkdown = "# Transcript\n\n> Speaker recognition was off for this transcript.\n\n- [00:00:00] Hello.\n";
    const tauri = mockPage(e, {}, {
      list_sessions: () => [e],
      session_document: () => ({
        ...doc,
        has_speakers: false,
        lines: [{ kind: "turn", position_ms: 0, speaker: null, text: "Hello." }],
        transcript_markdown: transcriptMarkdown,
      }),
    });
    render(<Synced id="s" />);
    await screen.findByText("We agreed the budget.");
    await userEvent.setup().click(tab("Transcript"));
    expect(await screen.findByText("Speaker recognition was off for this note.")).toBeTruthy();
    expect(screen.queryByText(/Speaker separation failed/)).toBeNull();

    transcriptMarkdown = "# Transcript\n\n> This transcript carries no speaker marks: speaker separation failed for this session.\n\n- [00:00:00] Hello.\n";
    await act(() => tauri.emit("library-changed", { changed: ["s"], removed: [] }));
    expect(await screen.findByText("Speaker separation failed for this note, so the transcript has no speaker marks.")).toBeTruthy();
    expect(screen.queryByText("Speaker recognition was off for this note.")).toBeNull();
  });

  test("a library change for the open note reloads its detail and document", async () => {
    const e = entry("s");
    let body = "We agreed the budget.";
    const tauri = mockPage(e, {}, {
      list_sessions: () => [e],
      session_document: () => ({ ...doc, summary: { ...doc.summary!, body } }),
    });
    render(<Synced id="s" />);
    await screen.findByText("We agreed the budget.");
    const details = () => tauri.calls.filter((c) => c.cmd === "session_detail").length;
    const before = details();

    body = "We moved the budget to Friday.";
    await act(() => tauri.emit("library-changed", { changed: ["s"], removed: [] }));
    expect(await screen.findByText("We moved the budget to Friday.")).toBeTruthy();
    expect(details()).toBeGreaterThan(before);
  });

  test("a removed note says that it was deleted and keeps its title", async () => {
    const e = entry("s", { title: "Budget review" });
    let listed = [e];
    const tauri = mockPage(e, {}, { list_sessions: () => listed });
    render(<Synced id="s" />);
    await screen.findByText("We agreed the budget.");

    listed = [];
    await act(() => tauri.emit("library-changed", { changed: [], removed: ["s"] }));
    expect(await screen.findByText("This note was deleted.")).toBeTruthy();
    expect(screen.getByRole<HTMLInputElement>("textbox", { name: "Title" }).value).toBe("Budget review");
    expect(screen.queryByText("This note is no longer in the list.")).toBeNull();
  });

  // Covers AE7.
  test("a note whose notes wait for the sync says so, and the editor takes no input", async () => {
    const e = entry("s");
    mockPage(e, { waiting: ["notes", "audio"], resumable: true });
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    expect(await screen.findByText("Waiting for the notes and audio to sync.")).toBeTruthy();
    const box = await screen.findByRole("textbox", { name: "My Notes", hidden: true });
    expect(box.getAttribute("contenteditable")).toBe("false");

    await userEvent.setup().click(screen.getByRole("button", { name: "Actions" }));
    for (const name of ["Regenerate Summary", "Transcribe Again…"]) {
      expect(screen.getByRole("menuitem", { name }).hasAttribute("disabled")).toBe(true);
    }
  });

  // R12: audio that waits holds only Resume and Transcribe Again.
  test("a note whose audio waits for the sync keeps its notes and summary actions", async () => {
    const e = entry("s");
    mockPage(e, { waiting: ["audio"], resumable: true });
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    expect(await screen.findByText("Waiting for the audio to sync.")).toBeTruthy();
    const box = await screen.findByRole("textbox", { name: "My Notes", hidden: true });
    expect(box.getAttribute("contenteditable")).toBe("true");

    await userEvent.setup().click(screen.getByRole("button", { name: "Actions" }));
    for (const name of ["Transcribe Again…"]) {
      expect(screen.getByRole("menuitem", { name }).hasAttribute("disabled")).toBe(true);
    }
    expect(screen.getByRole("menuitem", { name: "Regenerate Summary" }).hasAttribute("disabled")).toBe(false);
  });

  // Covers AE8.
  test("a conflict notice names the other version", async () => {
    const e = entry("s");
    mockPage(e, { conflict_files: ["notes (1).md"] });
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    expect(
      await screen.findByText("Another version of this note exists: notes (1).md. Compare them in Finder."),
    ).toBeTruthy();
  });

  // Covers AE8, KTD10.
  test("a notes save that conflicts names the draft on the page", async () => {
    const e = entry("s");
    const draft = "notes (Longhand conflict 2026-09-26 1412).md";
    mockPage(e, { conflict_files: [draft] }, {
      save_session_notes: () => ({ kind: "conflict", draft: `/n/Planning/${draft}` }),
    });
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    const user = userEvent.setup();
    await user.click(tab("My Notes"));
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await user.type(box, "!");
    act(() => {
      box.blur();
    });
    expect(
      await screen.findByText(`The notes changed on another computer. Your edit is in ${draft}.`),
    ).toBeTruthy();
    // The draft is not named a second time as another version.
    expect(screen.queryByText(/Another version of this note exists/)).toBeNull();
  });

  // Covers AE6.
  test("a note in an unavailable storage folder says so, and its actions wait", async () => {
    const e = entry("s", { storage_unavailable: true });
    mockPage(e, { resumable: true }, {
      session_document: () => {
        throw "could not read session.md in /Volumes/Drive/Planning";
      },
    });
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    expect(await screen.findByText("The storage folder for this note is unavailable.")).toBeTruthy();
    expect(screen.queryByText(/could not read/)).toBeNull();

    await userEvent.setup().click(screen.getByRole("button", { name: "Actions" }));
    for (const name of ["Regenerate Summary", "Transcribe Again…", "Rename", "Delete…"]) {
      expect(screen.getByRole("menuitem", { name }).hasAttribute("disabled")).toBe(true);
    }
    const box = await screen.findByRole("textbox", { name: "My Notes", hidden: true });
    expect(box.getAttribute("contenteditable")).toBe("false");
  });

  test("the recording bar says Storage when the folder is not ready", async () => {
    const e = entry("rec", { state: "capturing" });
    mockPage(e);
    render(
      <SessionPage
        id="rec"
        capture={recordingCapture("rec")}
        sessions={sessionsOf([e])}
        readiness={{ ...readiness, destination: { kind: "missing", path: "/Volumes/Drive" } }}
        onBack={() => {}}
      />,
    );
    expect(await screen.findByText(/^Storage: Folder not found/)).toBeTruthy();
    expect(document.body.textContent).not.toMatch(/Destination/);
  });
});

describe("SessionPage: Record, Resume and other computers", () => {
  const noAudio = (id: string, over: Partial<SessionEntry> = {}) =>
    entry(id, { duration_ms: 0, found_speakers: null, ...over });
  const recordButton = (name: "Record" | "Resume") => screen.getByRole("button", { name });
  const reasonOf = (el: HTMLElement) => el.closest("[title]")?.getAttribute("title") ?? null;
  const openMenu = async () => {
    await userEvent.setup().click(screen.getByRole("button", { name: "Actions" }));
  };

  // R11, R14.
  test("a note that never recorded shows only My Notes and Record, and Record records into it", async () => {
    const e = noAudio("n");
    const tauri = mockPage(e, { record: { kind: "record", available: true, reason: null } }, {
      record_note: () => null,
    });
    renderPage({ id: "n", sessions: sessionsOf([e]) });
    await act(async () => {});
    expect(screen.queryByRole("tab", { name: "Summary" })).toBeNull();
    expect(screen.queryByRole("tab", { name: "Transcript" })).toBeNull();
    expect(await screen.findByRole("textbox", { name: "My Notes" })).toBeTruthy();
    await userEvent.setup().click(recordButton("Record"));
    expect(tauri.calls.filter((c) => c.cmd === "record_note").map((c) => c.args)).toEqual([{ sessionId: "n" }]);
  });

  test("a note that the user just started without recording has the cursor in the editor", async () => {
    const e = noAudio("n");
    mockPage(e, { record: { kind: "record", available: true, reason: null } });
    renderPage({ id: "n", sessions: sessionsOf([e]), started: true });
    const editor = await screen.findByRole("textbox", { name: "My Notes" });
    await act(async () => {});
    expect(document.activeElement === editor || editor.contains(document.activeElement)).toBe(true);
  });

  test("a note with no audio that opens from Home does not take the focus", async () => {
    const e = noAudio("n");
    mockPage(e, { record: { kind: "record", available: true, reason: null } });
    renderPage({ id: "n", sessions: sessionsOf([e]) });
    const editor = await screen.findByRole("textbox", { name: "My Notes" });
    await act(async () => {});
    expect(document.activeElement === editor || editor.contains(document.activeElement)).toBe(false);
  });

  // The page does not move when Record adds the tabs.
  test("a note with no audio keeps a placeholder of the tabs' height in the header", async () => {
    const e = noAudio("n");
    mockPage(e, { record: { kind: "record", available: true, reason: null } });
    const view = renderPage({ id: "n", sessions: sessionsOf([e]) });
    await act(async () => {});
    const placeholder = view.container.querySelector('[data-testid="tabs-placeholder"]');
    expect(placeholder?.className).toContain("p-[2px]");
    expect(placeholder?.querySelector(".py-\\[3px\\]")).toBeTruthy();
  });

  test("a note with no audio opens on My Notes before its detail loads", () => {
    const e = noAudio("n");
    mockPage(e, { record: { kind: "record", available: true, reason: null } });
    renderPage({ id: "n", sessions: sessionsOf([e]) });
    expect(screen.queryByRole("tab", { name: "Summary" })).toBeNull();
  });

  // R11.
  test("a note with audio shows Resume, which resumes it, and the menu has no Resume Recording", async () => {
    const e = entry("s");
    const tauri = mockPage(e, { resumable: true, record: { kind: "resume", available: true, reason: null } }, {
      resume_recording: () => "s",
    });
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    await act(async () => {});
    expect(screen.queryByRole("button", { name: "Record" })).toBeNull();
    await userEvent.setup().click(recordButton("Resume"));
    expect(tauri.calls.filter((c) => c.cmd === "resume_recording").map((c) => c.args)).toEqual([{ sessionId: "s" }]);
    await openMenu();
    expect(screen.queryByRole("menuitem", { name: "Resume Recording" })).toBeNull();
  });

  // Covers AE3.
  test("a note whose audio was removed shows Resume, unavailable, with the reason", async () => {
    const reason = "The audio of this note was removed.";
    const e = entry("s");
    const tauri = mockPage(e, { audio: "removed", record: { kind: "resume", available: false, reason } }, {
      resume_recording: () => "s",
    });
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    await act(async () => {});
    const button = recordButton("Resume");
    expect(button.hasAttribute("disabled")).toBe(true);
    expect(reasonOf(button)).toBe(reason);
    await userEvent.setup().click(button);
    expect(tauri.calls.some((c) => c.cmd === "resume_recording")).toBe(false);
  });

  test("while the note records, the page has no Record or Resume button", async () => {
    const e = entry("rec", { state: "capturing" });
    mockPage(e);
    renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    await act(async () => {});
    expect(screen.queryByRole("button", { name: "Record" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();
  });

  // R20, R21, F3.
  test("a note in use on another computer says so, keeps its notes editable, and holds its other actions", async () => {
    const e = entry("s", { remote: "in_use" });
    mockPage(e, {
      record: { kind: "resume", available: false, reason: "Another computer is using this note." },
    });
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    expect(await screen.findByText("Recording or processing on another computer.")).toBeTruthy();
    const box = await screen.findByRole("textbox", { name: "My Notes", hidden: true });
    expect(box.getAttribute("contenteditable")).toBe("true");
    expect(recordButton("Resume").hasAttribute("disabled")).toBe(true);
    await openMenu();
    for (const name of ["Regenerate Summary", "Transcribe Again…", "Remove Audio…", "Delete…"]) {
      expect(screen.getByRole("menuitem", { name }).hasAttribute("disabled")).toBe(true);
      expect(screen.getByRole("menuitem", { name }).getAttribute("title")).toBe(
        "Wait until the other computer is done with this note.",
      );
    }
    expect(screen.getByRole("menuitem", { name: "Rename" }).hasAttribute("disabled")).toBe(false);
  });

  test("on a note in use on another computer, the Summary tab's Regenerate Summary waits with the reason", async () => {
    const e = entry("s", { remote: "in_use" });
    const stub = { ...doc, summary: { body: "No summary was produced.", stub: true, host: null, model: null, prompt: null, prompt_id: null } };
    mockPage(e, {}, { session_document: () => stub });
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    const button = await screen.findByRole("button", { name: "Regenerate Summary" });
    expect(button.hasAttribute("disabled")).toBe(true);
    expect(reasonOf(button)).toBe("Wait until the other computer is done with this note.");
  });

  // R23, AE13.
  test("a note that another computer did not finish allows notes and Delete after a confirmation that names the recording", async () => {
    const e = noAudio("s", { remote: "not_finished", title: "Planning" });
    const tauri = mockPage(e, {
      record: { kind: "record", available: false, reason: "Another computer did not finish this note." },
    }, { delete_session: () => null });
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    expect(await screen.findByText("Not finished on another computer.")).toBeTruthy();
    const box = await screen.findByRole("textbox", { name: "My Notes", hidden: true });
    expect(box.getAttribute("contenteditable")).toBe("true");
    expect(recordButton("Record").hasAttribute("disabled")).toBe(true);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Delete…" }));
    const dialog = screen.getByRole("alertdialog");
    expect(dialog.textContent).toContain("“Planning” moves to the Trash.");
    expect(dialog.textContent).toContain("The recording that another computer did not save is lost.");
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));
    expect(tauri.calls.filter((c) => c.cmd === "delete_session").map((c) => c.args)).toEqual([{ sessionId: "s" }]);
  });

  // KTD4.
  test("a note whose folder is not written yet says Not saved yet, with the reason", async () => {
    const e = noAudio("n");
    mockPage(e, {
      record: { kind: "record", available: true, reason: null },
      not_saved_yet: "the storage folder is not available",
    });
    renderPage({ id: "n", sessions: sessionsOf([e]) });
    const line = await screen.findByText("Not saved yet.");
    expect(line.getAttribute("title")).toBe("The storage folder is not available.");
  });

  // R24.
  test("a notes save that another computer's removal turned into a new note says so and opens it", async () => {
    const e = noAudio("old");
    const opened: string[] = [];
    let saved = 0;
    const tauri = mockPage(e, { record: { kind: "record", available: true, reason: null } }, {
      save_session_notes: () =>
        saved++ === 0
          ? { kind: "new_note", session_id: "new", fingerprint: "f9" }
          : { kind: "saved", fingerprint: "f10" },
    });
    render(
      <SessionPage
        id="old"
        capture={captureOf()}
        sessions={sessionsOf([e])}
        readiness={readiness}
        onBack={() => {}}
        onOpenNote={(id) => opened.push(id)}
      />,
    );
    const user = userEvent.setup();
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await user.type(box, "a");
    expect(
      await screen.findByText("Another computer removed this note. Your notes are saved in a new note."),
    ).toBeTruthy();
    await user.type(box, "b");
    const saveIds = () => tauri.calls.filter((c) => c.cmd === "save_session_notes").map((c) => c.args.sessionId);
    await waitFor(() => expect(saveIds().length).toBeGreaterThan(1));
    const saves = saveIds();
    expect(saves[0]).toBe("old");
    expect(saves.slice(1).every((id) => id === "new")).toBe(true);
    await user.click(screen.getByRole("button", { name: "Open New Note" }));
    expect(opened).toEqual(["new"]);
  });
});

describe("SessionPage regenerate offer", () => {
  const prompt = (id: string, name: string) => ({ id, name, text: "", title_text: "", built_in: false });
  const prompts = {
    prompts: [prompt("topic-summary", "Topic summary"), prompt("play", "Play-by-play summary"), prompt("decisions", "Decisions")],
    active: "topic-summary",
    text_limit: 10_000,
    title_text_limit: 1_000,
  };
  const typeEntry = (name: string, id: string, general = false) => ({
    name,
    valid: true,
    shared: !general,
    notes: 1,
    general,
    prompt_choice: id === "topic-summary" ? null : id,
    prompt_name: null,
    summary_prompt_id: id,
  });
  const types = [
    typeEntry("General", "topic-summary", true),
    typeEntry("Debrief", "decisions"),
    typeEntry("One on One", "topic-summary"),
    typeEntry("Pathfinder", "play"),
    typeEntry("Work", "topic-summary"),
    // R7: the file of Recap is missing; summaries use the last copy read.
    { ...typeEntry("Board", "recap"), prompt_name: "Recap" },
  ];
  const typeRow = () => screen.getByRole("group", { name: "Type and tags" });
  const dismiss = () => screen.queryByRole("button", { name: "Dismiss" });
  /// The offer: its sentence, its Regenerate Summary and its Dismiss.
  const offer = () => dismiss()?.parentElement?.parentElement ?? null;

  /// A saved note of type One on One whose summary Topic summary made, on
  /// a page that reads the note list as the app does.
  function open(over: { note?: Partial<SessionEntry>; promptId?: string | null } = {}) {
    const note = { current: entry("s", { note_type: "One on One", ...over.note }) };
    const summary = { ...doc.summary!, prompt: "Topic summary", prompt_id: over.promptId === undefined ? "topic-summary" : over.promptId };
    const tauri = mockPage(note.current, {}, {
      list_sessions: () => [note.current],
      session_detail: () => detail(note.current),
      session_document: () => ({ ...doc, summary }),
      note_types: () => ({ types, default_type: null }),
      summary_prompts: () => prompts,
      set_note_type: (args) => {
        note.current = { ...note.current, note_type: args.noteType as string | null };
        return { kind: "done" };
      },
      regenerate_summary: () => "/x/summary.md",
    });
    const view = render(<Synced id="s" />);
    return { tauri, note, view, user: userEvent.setup() };
  }

  async function chooseType(user: ReturnType<typeof userEvent.setup>, from: string, to: string) {
    await user.click(await screen.findByRole("button", { name: `Type: ${from}` }));
    await user.click(await screen.findByRole("menuitemradio", { name: to }));
    await within(typeRow()).findByRole("button", { name: `Type: ${to}` });
  }

  /// Waits until the page read the prompts after the type change, so that
  /// an offer would show by now.
  async function settle(tauri: ReturnType<typeof mockTauri>) {
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "summary_prompts")).toBe(true));
    await act(async () => {});
  }

  // AE2.
  test("a type change to a type with another prompt offers a regeneration, and its action regenerates", async () => {
    const { tauri, user } = open();
    await screen.findByText("We agreed the budget.");
    expect(dismiss()).toBeNull();
    await chooseType(user, "One on One", "Pathfinder");
    await waitFor(() => expect(offer()).not.toBeNull());
    expect(within(offer()!).getByRole("status").textContent).toBe("Pathfinder uses Play-by-play summary.");
    await user.click(within(offer()!).getByRole("button", { name: "Regenerate Summary" }));
    await waitFor(() => expect(tauri.calls.filter((c) => c.cmd === "regenerate_summary").length).toBe(1));
    expect(dismiss()).toBeNull();
  });

  test("the offer names a chosen prompt whose file is missing by its last copy read (R7)", async () => {
    const { user } = open();
    await screen.findByText("We agreed the budget.");
    await chooseType(user, "One on One", "Board");
    await waitFor(() => expect(offer()).not.toBeNull());
    expect(within(offer()!).getByRole("status").textContent).toBe("Board uses Recap.");
  });

  // AE3.
  test("a type change between two types with the same prompt offers nothing", async () => {
    const { tauri, user } = open();
    await screen.findByText("We agreed the budget.");
    await chooseType(user, "One on One", "Work");
    await settle(tauri);
    expect(dismiss()).toBeNull();
  });

  // AE4.
  test("a type change during processing offers nothing until processing ends, then offers a regeneration", async () => {
    const { tauri, note, user } = open({ note: { state: "processing", stage: "summarize" } });
    await chooseType(user, "One on One", "Pathfinder");
    await settle(tauri);
    expect(dismiss()).toBeNull();
    note.current = { ...note.current, state: "saved", stage: null };
    await act(() => tauri.emit("library-changed", { changed: ["s"], removed: [] }));
    await screen.findByText("We agreed the budget.");
    await waitFor(() => expect(offer()).not.toBeNull());
  });

  test("a change back to the type of the summary removes the offer", async () => {
    const { user } = open();
    await screen.findByText("We agreed the budget.");
    await chooseType(user, "One on One", "Pathfinder");
    await waitFor(() => expect(offer()).not.toBeNull());
    await chooseType(user, "Pathfinder", "One on One");
    await waitFor(() => expect(dismiss()).toBeNull());
  });

  test("Dismiss removes the offer, and only a new type change can give it again", async () => {
    const { tauri, user } = open();
    await screen.findByText("We agreed the budget.");
    await chooseType(user, "One on One", "Pathfinder");
    await waitFor(() => expect(offer()).not.toBeNull());
    await user.click(dismiss()!);
    expect(dismiss()).toBeNull();
    await act(() => tauri.emit("settings-changed", { settings: {} }));
    await act(() => tauri.emit("library-changed", { changed: ["s"], removed: [] }));
    await act(async () => {});
    expect(dismiss()).toBeNull();
    await chooseType(user, "Pathfinder", "Debrief");
    await waitFor(() => expect(offer()).not.toBeNull());
    expect(within(offer()!).getByRole("status").textContent).toBe("Debrief uses Decisions.");
  });

  // R11.
  test("a type change from another computer offers nothing", async () => {
    const { tauri, note } = open();
    await screen.findByText("We agreed the budget.");
    note.current = { ...note.current, note_type: "Pathfinder" };
    await act(() => tauri.emit("library-changed", { changed: ["s"], removed: [] }));
    await screen.findByRole("button", { name: "Type: Pathfinder" });
    await act(async () => {});
    expect(dismiss()).toBeNull();
  });

  test("a summary with no recorded prompt id offers nothing", async () => {
    const { tauri, user } = open({ promptId: null });
    await screen.findByText("We agreed the budget.");
    await chooseType(user, "One on One", "Pathfinder");
    await settle(tauri);
    expect(dismiss()).toBeNull();
  });

  test("the offer shows on the Transcript tab and the My Notes tab", async () => {
    const { user } = open();
    await screen.findByText("We agreed the budget.");
    await chooseType(user, "One on One", "Pathfinder");
    await waitFor(() => expect(offer()).not.toBeNull());
    await user.click(tab("Transcript"));
    expect(offer()).not.toBeNull();
    await user.click(tab("My Notes"));
    expect(offer()).not.toBeNull();
  });

  test("the offer's Regenerate Summary waits while another computer works on the note", async () => {
    const { user } = open({ note: { remote: "in_use" } });
    await screen.findByText("We agreed the budget.");
    await chooseType(user, "One on One", "Pathfinder");
    await waitFor(() => expect(offer()).not.toBeNull());
    const button = within(offer()!).getByRole("button", { name: "Regenerate Summary" });
    expect(button.hasAttribute("disabled")).toBe(true);
    expect(button.closest("[title]")?.getAttribute("title")).toBe("Wait until the other computer is done with this note.");
    expect(within(offer()!).getByRole("button", { name: "Dismiss" }).hasAttribute("disabled")).toBe(false);
  });

  test("leaving the note and opening it again offers nothing", async () => {
    const { view, user } = open();
    await screen.findByText("We agreed the budget.");
    await chooseType(user, "One on One", "Pathfinder");
    await waitFor(() => expect(offer()).not.toBeNull());
    view.unmount();
    render(<Synced id="s" />);
    await screen.findByText("We agreed the budget.");
    await screen.findByRole("button", { name: "Type: Pathfinder" });
    await act(async () => {});
    expect(dismiss()).toBeNull();
  });
});
