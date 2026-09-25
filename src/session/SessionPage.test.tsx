import { describe, expect, mock, test } from "bun:test";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type {
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
import type { Sessions } from "./useSessions";

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

const started = new Date(2026, 8, 24, 14, 5).toISOString();

const entry = (id: string, over: Partial<SessionEntry> = {}): SessionEntry => ({
  id,
  started,
  duration_ms: 35 * 60_000 + 48_000,
  state: "saved",
  stage: null,
  title: null,
  found_speakers: 3,
  ...over,
});

const detail = (e: SessionEntry, over: Partial<SessionDetail> = {}): SessionDetail => ({
  entry: e,
  done: [],
  current: null,
  windows_done: null,
  windows_total: null,
  failed_stage: null,
  message: null,
  summary_problem: null,
  resumable: false,
  home_path: null,
  endpoint: null,
  audio_kept: true,
  transcript_kept: true,
  asr_model: null,
  live_transcript: false,
  ...over,
});

const doc: SessionDocument = {
  has_speakers: true,
  lines: [{ kind: "turn", position_ms: 0, speaker: 1, text: "Hello." }],
  transcript_markdown: "# Transcript\n\n- [00:00:00] Speaker 1: Hello.\n",
  summary: { body: "We agreed the budget.", stub: false, host: "api.example.com", model: "m" },
};

function captureOf(over: Partial<Capture> = {}): Capture {
  return {
    status: idle,
    input: "idle",
    levels: [],
    recordedMs: 0,
    lostReason: null,
    alert: { output_problem: null, tone_heard: true },
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
  return { entries, stages: {}, live: {}, version: {}, refresh: mock(async () => {}) };
}

function mockPage(e: SessionEntry, over: Partial<SessionDetail> = {}, extra: Record<string, CommandHandler> = {}) {
  return mockTauri({
    session_notes: () => "",
    set_session_notes: () => null,
    session_detail: () => detail(e, over),
    session_document: () => doc,
    ...extra,
  });
}

function renderPage({
  id,
  capture = captureOf(),
  sessions,
  onBack = () => {},
}: {
  id: string;
  capture?: Capture;
  sessions: Sessions;
  onBack?: () => void;
}) {
  const ui = (c: Capture, s: Sessions) => (
    <SessionPage
      id={id}
      capture={c}
      sessions={s}
      readiness={readiness}
      models={[]}
      onBack={onBack}
    />
  );
  const view = render(ui(capture, sessions));
  return { ...view, rerenderWith: (c: Capture, s: Sessions) => view.rerender(ui(c, s)) };
}

const tab = (name: string) => screen.getByRole("tab", { name });

describe("SessionPage", () => {
  test("a page opened for a recording shows My Notes with the cursor in the editor", async () => {
    const e = entry("rec", { state: "capturing", duration_ms: 0, found_speakers: null });
    mockPage(e);
    renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    expect(tab("My Notes").getAttribute("aria-selected")).toBe("true");
    const editor = await screen.findByRole("textbox", { name: "My Notes" });
    expect(document.activeElement === editor || editor.contains(document.activeElement)).toBe(true);
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

  test("the tabs follow the order things happen: My Notes, Transcript, Summary", () => {
    const e = entry("s");
    mockPage(e);
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual(["My Notes", "Transcript", "Summary"]);
  });

  test("Stop moves the page to Summary, which shows the processing", async () => {
    const e = entry("rec", { state: "capturing" });
    mockPage(e);
    const view = renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    expect(tab("My Notes").getAttribute("aria-selected")).toBe("true");
    view.rerenderWith(captureOf(), sessionsOf([{ ...e, state: "processing", stage: "transcribe" }]));
    expect(tab("Summary").getAttribute("aria-selected")).toBe("true");
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

  test("the recording bar shows Stop and the elapsed time, and the quiet-input warning only while the input is quiet", async () => {
    const e = entry("rec", { state: "capturing" });
    mockPage(e);
    const view = renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    const bar = screen.getByRole("region", { name: "Recording" });
    expect(within(bar).getByRole("button", { name: "Stop" })).toBeTruthy();
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

  test("Stop stops the recording", async () => {
    const e = entry("rec", { state: "capturing" });
    const tauri = mockPage(e, {}, { stop_recording: () => "rec" });
    renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    await userEvent.setup().click(screen.getByRole("button", { name: "Stop" }));
    expect(tauri.calls.some((c) => c.cmd === "stop_recording")).toBe(true);
  });

  test("while recording, Transcript and Summary say what comes after Stop", async () => {
    const e = entry("rec", { state: "capturing" });
    mockPage(e);
    renderPage({ id: "rec", capture: recordingCapture("rec"), sessions: sessionsOf([e]) });
    const user = userEvent.setup();
    await user.click(tab("Transcript"));
    expect(screen.getByText(/transcript is ready after you stop recording/)).toBeTruthy();
    await user.click(tab("Summary"));
    expect(screen.getByText(/summary is written after you stop recording/)).toBeTruthy();
  });

  test("during processing, Summary shows the stage rows and My Notes stays editable", async () => {
    const e = entry("p", { state: "processing", stage: "transcribe" });
    mockPage(e, { done: ["assemble", "diarize"], current: "transcribe", windows_done: 1, windows_total: 4 });
    renderPage({ id: "p", sessions: sessionsOf([e]) });
    expect(tab("Summary").getAttribute("aria-selected")).toBe("true");
    expect(await screen.findByText("Transcribing")).toBeTruthy();
    expect(screen.getByText("Telling speakers apart")).toBeTruthy();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("25");

    await userEvent.setup().click(tab("My Notes"));
    const editor = await screen.findByRole("textbox", { name: "My Notes" });
    expect(editor.getAttribute("contenteditable")).toBe("true");
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
    expect(tauri.calls.some((c) => c.cmd === "retry_session")).toBe(true);
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

  test("the menu shows Resume for a stopped session, and Resume records again", async () => {
    const e = entry("s");
    const tauri = mockPage(e, { resumable: true }, { resume_recording: () => "s" });
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    await act(async () => {});
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Actions" }));
    const resume = screen.getByRole("menuitem", { name: "Resume Recording" });
    expect(resume.hasAttribute("disabled")).toBe(false);
    expect(screen.getByRole("menuitem", { name: "Delete…" })).toBeTruthy();
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

  test("Copy writes the open tab's markdown: the transcript file, or the notes", async () => {
    const e = entry("s", { title: "Budget review" });
    mockTauri({
      session_notes: () => "# Agenda\n\n- ask about headcount",
      set_session_notes: () => null,
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

  test("the Transcript tab of a saved session shows the header and the turns", async () => {
    const e = entry("s");
    mockPage(e);
    renderPage({ id: "s", sessions: sessionsOf([e]) });
    await screen.findByText("We agreed the budget.");
    await userEvent.setup().click(tab("Transcript"));
    expect(screen.getByText("3 speakers · 35:48")).toBeTruthy();
    expect(screen.getByText("Hello.")).toBeTruthy();
  });

  test("a late session detail never replaces a newer one", async () => {
    const e = entry("s");
    let release: (d: SessionDetail) => void = () => {};
    const late = new Promise<SessionDetail>((r) => {
      release = r;
    });
    let n = 0;
    mockPage(e, {}, { session_detail: () => (n++ === 0 ? late : detail(e, { resumable: true })) });
    const sessions = sessionsOf([e]);
    const view = renderPage({ id: "s", sessions });
    await act(async () => {});
    // A change to the session reads the detail again, and that read ends first.
    view.rerenderWith(captureOf(), { ...sessions, version: { s: 1 } });
    await act(async () => {});
    await act(async () => release(detail(e, { resumable: false })));
    await userEvent.setup().click(screen.getByRole("button", { name: "Actions" }));
    expect(screen.getByRole("menuitem", { name: "Resume Recording" }).hasAttribute("disabled")).toBe(false);
  });
});
