import { beforeEach, describe, expect, test, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useRef } from "react";
import type { LiveTranscript as Catchup } from "../bindings";
import { mockTauri } from "../test/tauri";
import { LiveTranscript, type LiveMode } from "./LiveTranscript";
import { liveNumbers } from "./useLiveTranscript";

const empty: Catchup = { recognition: [], parts: [], current: null, state: "running" };

function mockLive(catchup: Catchup = empty) {
  return mockTauri({ live_transcript: () => catchup });
}

function Harness({ id = "rec", mode = "recording" }: { id?: string; mode?: LiveMode }) {
  const scroller = useRef<HTMLElement>(null);
  return (
    <main ref={scroller} data-testid="scroller">
      <LiveTranscript sessionId={id} mode={mode} scroller={scroller} />
    </main>
  );
}

const text = (session_id: string, committed: string, tentative = "", part = 0) => ({
  session_id,
  segment: 0,
  part,
  offset_ms: 0,
  committed,
  tentative,
  word_start: 0,
  words: [],
});

/// The words of a text, one each `step` ms from `start`.
const timed = (words: string, start: number, step = 500) =>
  words.split(" ").map((word, i) => ({ start_ms: start + i * step, end_ms: start + (i + 1) * step, text: word }));

/// A saved part of segment 0 with timed words.
const saved = (part: number, words: string, start: number) => ({
  segment: 0, part, offset_ms: 0, words: timed(words, start), start_ms: start, end_ms: start + 240_000, text: words,
});

/// A speaker turn from `start` to `end` ms.
const turn = (start: number, end: number, speaker: string, speaker_final = true) => ({
  start_sample: start * 16, end_sample: end * 16, text: null, speaker, text_final: false, speaker_final,
});

/// A complete speaker snapshot of one epoch.
const snapshot = (revision: number, rows: ReturnType<typeof turn>[], epoch = 0) => {
  const base = { session_id: "rec", generation: "g", epoch, key: "speakers", revision };
  return [
    { ...base, phase: "snapshot_begin" as const, rows: [] },
    { ...base, phase: "snapshot_batch" as const, rows },
    { ...base, phase: "snapshot_end" as const, rows: [] },
  ];
};

async function send(tauri: ReturnType<typeof mockTauri>, events: unknown[]) {
  for (const event of events) await act(() => tauri.emit("live-recognition-event", event));
  await act(() => new Promise((done) => setTimeout(done, 120)));
}

const headings = () => screen.queryAllByRole("heading").map((h) => h.textContent);
const paragraphs = () => screen.queryAllByTestId("live-part").map((p) => p.textContent);

// The numbers of a note stay while the app runs (KTD12). Each test starts
// with no numbers.
beforeEach(() => liveNumbers("rec").clear());

// happy-dom does no layout: give the scroller a size, so the view can tell
// whether it is at the bottom.
function sized(el: HTMLElement, scrollHeight: () => number, clientHeight: number) {
  Object.defineProperty(el, "scrollHeight", { configurable: true, get: scrollHeight });
  Object.defineProperty(el, "clientHeight", { configurable: true, get: () => clientHeight });
}

function controlledRecognitionSchedule() {
  const timers: {
    setTimeout: (callback: (...args: unknown[]) => void, delay?: number, ...args: unknown[]) => ReturnType<typeof setTimeout>;
    clearTimeout: (id: ReturnType<typeof setTimeout> | number | undefined) => void;
  } = globalThis;
  let frame: FrameRequestCallback | undefined;
  let timer: (() => void) | undefined;
  const timerId = 456 as unknown as ReturnType<typeof setTimeout>;
  const originalSetTimeout = timers.setTimeout;
  const originalClearTimeout = timers.clearTimeout;
  const request = vi.spyOn(globalThis, "requestAnimationFrame").mockImplementation((next) => {
    frame = next;
    return 123;
  });
  const cancel = vi.spyOn(globalThis, "cancelAnimationFrame").mockImplementation(() => {});
  const setTimer = vi.spyOn(timers, "setTimeout").mockImplementation((callback, delay, ...args) => {
    if (delay !== 100) return originalSetTimeout(callback, delay, ...args);
    timer = () => callback(...args);
    return timerId;
  });
  const clearTimer = vi.spyOn(timers, "clearTimeout").mockImplementation((id) => {
    if (id !== timerId) originalClearTimeout(id);
  });
  return {
    request, cancel, setTimer, clearTimer, timerId,
    frame: () => frame?.(0),
    timer: () => timer?.(),
    restore: () => {
      request.mockRestore();
      cancel.mockRestore();
      setTimer.mockRestore();
      clearTimer.mockRestore();
    },
  };
}

describe("LiveTranscript", () => {
  test("recognition still updates when a hidden window does not run animation frames", async () => {
    const tauri = mockLive();
    render(<Harness />);
    await screen.findByText("The transcript shows here as people speak.");
    const schedule = controlledRecognitionSchedule();
    try {
      await act(() => tauri.emit("live-recognition-event", {
        session_id: "rec", generation: "g", epoch: 0, key: "word", phase: "patch", revision: 1,
        rows: [{ start_sample: 0, end_sample: 16000, text: "Hidden window words", speaker: null, text_final: true, speaker_final: true }],
      }));
      expect(screen.queryByText("Hidden window words")).toBeNull();
      await act(async () => schedule.timer());
      expect(screen.getByText("Hidden window words")).toBeTruthy();
      expect(schedule.cancel).toHaveBeenCalledWith(123);
      expect(schedule.clearTimer).toHaveBeenCalledWith(schedule.timerId);
    } finally {
      schedule.restore();
    }
  });

  test("a burst of recognition events waits for one frame and retains every word", async () => {
    const tauri = mockLive();
    render(<Harness />);
    await screen.findByText("The transcript shows here as people speak.");
    const schedule = controlledRecognitionSchedule();
    try {
      await act(async () => {
        for (let i = 0; i < 1000; i++) await tauri.emit("live-recognition-event", {
          session_id: "rec", generation: "g", epoch: 0, key: `word${i}`, phase: "patch", revision: 1,
          rows: [{ start_sample: i * 16000, end_sample: (i + 1) * 16000, text: `word${i}`, speaker: "g:0:0:1", text_final: true, speaker_final: true }],
        });
      });
      expect(schedule.request).toHaveBeenCalledTimes(1);
      expect(schedule.setTimer).toHaveBeenCalledWith(expect.any(Function), 100);
      expect(screen.queryByText(/word999/)).toBeNull();
      await act(async () => schedule.frame());
      const words = screen.getAllByTestId("live-part").flatMap((part) => part.lastElementChild?.textContent?.split(" ") ?? []);
      expect(words).toEqual(Array.from({ length: 1000 }, (_, i) => `word${i}`));
      expect(schedule.cancel).toHaveBeenCalledWith(123);
      expect(schedule.clearTimer).toHaveBeenCalledWith(schedule.timerId);
    } finally {
      schedule.restore();
    }
  });

  test("switching notes cancels queued recognition and does not show it in the next note", async () => {
    const tauri = mockLive();
    const { rerender } = render(<Harness />);
    await screen.findByText("The transcript shows here as people speak.");
    const schedule = controlledRecognitionSchedule();
    try {
      await act(() => tauri.emit("live-recognition-event", {
        session_id: "rec", generation: "g", epoch: 0, key: "word", phase: "patch", revision: 1,
        rows: [{ start_sample: 0, end_sample: 16000, text: "Previous note", speaker: null, text_final: true, speaker_final: true }],
      }));
      rerender(<Harness id="next" />);
      await act(async () => { schedule.frame(); schedule.timer(); });
      expect(schedule.cancel).toHaveBeenCalledWith(123);
      expect(schedule.clearTimer).toHaveBeenCalledWith(schedule.timerId);
      expect(screen.queryByText("Previous note")).toBeNull();
    } finally {
      schedule.restore();
    }
  });

  test("a status event flushes an earlier recognition correction and cancels both callbacks", async () => {
    const initial = {
      session_id: "rec", generation: "g", epoch: 0, key: "word", phase: "patch" as const, revision: 1,
      rows: [{ start_sample: 0, end_sample: 16000, text: "Earlier words", speaker: "g:0:0:1", text_final: true, speaker_final: true }],
    };
    const tauri = mockLive({ ...empty, recognition: [initial] });
    render(<Harness />);
    await screen.findByText("Earlier words");
    const schedule = controlledRecognitionSchedule();
    try {
      await act(() => tauri.emit("live-recognition-event", {
        ...initial, revision: 2, rows: [{ ...initial.rows[0], text: "Corrected words", speaker: "g:0:0:2" }],
      }));
      expect(screen.queryByText("Corrected words")).toBeNull();
      await act(() => tauri.emit("live-transcript-status", { session_id: "rec", segment: 0, state: "behind" }));
      expect(screen.getByText("Corrected words")).toBeTruthy();
      expect(screen.getByText("Speaker labels updated.")).toBeTruthy();
      expect(screen.getByText("Live transcript paused. The missing part is transcribed after you stop.")).toBeTruthy();
      expect(schedule.cancel).toHaveBeenCalledWith(123);
      expect(schedule.clearTimer).toHaveBeenCalledWith(schedule.timerId);
      await act(async () => { schedule.frame(); schedule.timer(); });
      expect(screen.getAllByText("Corrected words")).toHaveLength(1);
    } finally {
      schedule.restore();
    }
  });

  test("native interim batches publish at End and final corrections announce once", async () => {
    const tauri = mockLive();
    render(<Harness />);
    await screen.findByText("The transcript shows here as people speak.");
    const send = async (phase: string, revision: number, speaker = "g:0:0:1") => act(() => tauri.emit("live-recognition-event", {
      session_id: "rec", generation: "g", epoch: 0, key: "interim", phase, revision,
      rows: phase.endsWith("batch") || phase === "patch" ? [{ start_sample: 0, end_sample: 16000, text: "Native words", speaker, text_final: phase === "patch", speaker_final: phase === "patch" }] : [],
    }));
    await send("interim_begin", 1);
    await send("interim_batch", 1);
    expect(screen.queryByText("Native words")).toBeNull();
    await send("interim_end", 1);
    expect(screen.getByText("Native words")).toBeTruthy();
    expect(screen.getByText(/Provisional text/)).toBeTruthy();
    expect(screen.getByText("Native words").closest('[aria-live]')).toBeNull();
    await send("patch", 2, "g:0:0:2");
    expect(screen.getAllByText("Native words")).toHaveLength(1);
    expect(screen.getByRole("status").textContent).toContain("Speaker labels updated");
    await send("patch", 1);
    expect(screen.getAllByText("Native words")).toHaveLength(1);
  });

  test("native recognition failures are errors and preserve transcript text", async () => {
    const tauri = mockLive();
    render(<Harness />);
    await screen.findByText("The transcript shows here as people speak.");
    await act(() => tauri.emit("live-recognition-event", {
      session_id: "rec", generation: "", epoch: 0, key: "failure", phase: "failed", revision: 0,
      rows: [{ start_sample: 0, end_sample: 0, text: "Recognition unavailable", speaker: null, text_final: false, speaker_final: false }],
    }));
    expect(screen.getByRole("alert").textContent).toContain("Recognition unavailable");
    expect(screen.queryAllByTestId("live-part")).toHaveLength(0);
  });

  test("a complete speaker snapshot replaces earlier labels and keeps epoch namespaces separate", async () => {
    const tauri = mockLive(); render(<Harness />);
    await screen.findByText("The transcript shows here as people speak.");
    const send = async (phase: string, revision: number, rows: unknown[], epoch = 0, key = "speech") => act(() => tauri.emit("live-recognition-event", { session_id: "rec", generation: "g", epoch, key, phase, revision, rows }));
    const words = (start: number, text: string) => ({ start_sample: start, end_sample: start + 16000, text, speaker: null, text_final: true, speaker_final: false });
    const label = (speaker: string) => ({ start_sample: 0, end_sample: 16000, text: null, speaker, text_final: false, speaker_final: true });
    await send("patch", 1, [words(0, "First epoch")]);
    await send("patch", 1, [words(16000, "Second epoch")], 1);
    await send("snapshot_begin", 2, []);
    await send("snapshot_batch", 2, [label("g:0:0:1")]);
    expect(screen.queryByText("Speaker 1")).toBeNull();
    await send("snapshot_end", 2, []);
    expect(screen.getAllByText("Speaker 1")).toHaveLength(1);
    await send("snapshot_begin", 3, []);
    await send("snapshot_batch", 3, [label("g:0:0:2")]);
    await send("snapshot_end", 3, []);
    expect(screen.queryByText("Speaker 1")).toBeNull();
    expect(screen.getAllByTestId("live-part")).toHaveLength(2);
    await send("snapshot_begin", 2, []);
    await send("snapshot_batch", 2, [label("g:0:0:1")]);
    await send("snapshot_end", 2, []);
    expect(screen.queryByText("Speaker 1")).toBeNull();
  });

  test("durable catch-up applies queued newer patches without restoring interim duplicates", async () => {
    let answer: (value: Catchup) => void = () => {};
    const tauri = mockTauri({ live_transcript: () => new Promise<Catchup>((resolve) => { answer = resolve; }) });
    render(<Harness />);
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "live_transcript")).toBe(true));
    const row = { start_sample: 0, end_sample: 16000, text: "Old words", speaker: null, text_final: true, speaker_final: true };
    const patch = { session_id: "rec", phase: "patch" as const, generation: "g", epoch: 0, key: "text", revision: 1, rows: [row] };
    await act(() => tauri.emit("live-recognition-event", { ...patch, revision: 2, rows: [{ ...row, text: "New words" }] }));
    await act(async () => answer({ recognition: [patch], parts: [], current: null, state: "running" }));
    expect(screen.getByText("New words")).toBeTruthy();
    expect(screen.queryByText("Old words")).toBeNull();
    const interim = { ...patch, key: "interim", revision: 1 };
    await act(() => tauri.emit("live-recognition-event", { ...interim, phase: "interim_begin", rows: [] }));
    await act(() => tauri.emit("live-recognition-event", { ...interim, phase: "interim_batch", rows: [{ ...row, text: "Outdated interim", text_final: false }] }));
    await act(() => tauri.emit("live-recognition-event", { ...interim, phase: "interim_end", rows: [] }));
    expect(screen.getAllByTestId("live-part")).toHaveLength(1);
    expect(screen.queryByText("Outdated interim")).toBeNull();
  });

  test("equal speaker overlaps stay unassigned instead of choosing the first label", async () => {
    const tauri = mockLive(); render(<Harness />);
    await screen.findByText("The transcript shows here as people speak.");
    const event = { session_id: "rec", generation: "g", epoch: 0, key: "text", revision: 1 };
    await act(() => tauri.emit("live-recognition-event", { ...event, phase: "patch", rows: [{ start_sample: 0, end_sample: 16000, text: "Overlapping speech", speaker: null, text_final: true, speaker_final: false }] }));
    await act(() => tauri.emit("live-recognition-event", { ...event, key: "speakers", phase: "snapshot_begin", rows: [] }));
    await act(() => tauri.emit("live-recognition-event", { ...event, key: "speakers", phase: "snapshot_batch", rows: [
      { start_sample: 0, end_sample: 8000, text: null, speaker: "g:0:0:1", text_final: false, speaker_final: true },
      { start_sample: 8000, end_sample: 16000, text: null, speaker: "g:0:0:2", text_final: false, speaker_final: true },
    ] }));
    await act(() => tauri.emit("live-recognition-event", { ...event, key: "speakers", phase: "snapshot_end", rows: [] }));
    expect(screen.getByText("Overlapping speech")).toBeTruthy();
    expect(screen.queryByText(/^Speaker 1/)).toBeNull();
    await act(() => tauri.emit("live-recognition-event", { ...event, key: "speakers", revision: 2, phase: "snapshot_begin", rows: [] }));
    await act(() => tauri.emit("live-recognition-event", { ...event, key: "speakers", revision: 2, phase: "snapshot_batch", rows: [
      { start_sample: 0, end_sample: 4000, text: null, speaker: "g:0:0:1", text_final: false, speaker_final: true },
      { start_sample: 4000, end_sample: 10000, text: null, speaker: "g:0:0:1", text_final: false, speaker_final: true },
      { start_sample: 10000, end_sample: 16000, text: null, speaker: "g:0:0:2", text_final: false, speaker_final: true },
    ] }));
    await act(() => tauri.emit("live-recognition-event", { ...event, key: "speakers", revision: 2, phase: "snapshot_end", rows: [] }));
    expect(screen.getByText("Speaker 1")).toBeTruthy();
  });
  test("text events for the note show committed and tentative text; events for another note show nothing", async () => {
    const tauri = mockLive();
    render(<Harness />);
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "live_transcript")).toBe(true));
    await act(() => tauri.emit("live-transcript-event", text("other", "Not this note.")));
    await act(() => tauri.emit("live-transcript-event", text("rec", "Hello there", " and wel")));
    expect(screen.getByText("Hello there")).toBeTruthy();
    const tentative = screen.getByText("and wel", { exact: false });
    expect(tentative.dataset.tentative).toBe("true");
    expect(screen.queryByText("Not this note.")).toBeNull();
  });

  test("a view opened after three saved parts shows them from the catch-up, in order", async () => {
    mockLive({
      recognition: [], parts: [
        saved(1, "Second part.", 240_000),
        saved(0, "First part.", 0),
        saved(2, "Third part.", 480_000),
      ],
      current: { segment: 0, part: 3, offset_ms: 0, words: timed("Now speaking", 600_000), committed: "Now", tentative: " speaking" },
      state: "running",
    });
    render(<Harness />);
    await screen.findByText("First part.");
    const paragraphs = screen.getAllByTestId("live-part").map((p) => p.textContent);
    expect(paragraphs).toEqual(["First part.", "Second part.", "Third part.", "Now speaking"]);
  });

  test("a saved part replaces the open text of that part, and the next part starts empty", async () => {
    const tauri = mockLive();
    render(<Harness />);
    await waitFor(() => expect(tauri.calls.length).toBeGreaterThan(0));
    await act(() => tauri.emit("live-transcript-event", text("rec", "We agreed", " the bud")));
    await act(() =>
      tauri.emit("live-part-saved", {
        session_id: "rec",
        segment: 0,
        part: 0,
        start_ms: 0,
        offset_ms: 0,
        end_ms: 240_000,
        text: "We agreed the budget.",
        words: [],
      }),
    );
    expect(screen.getAllByTestId("live-part").map((p) => p.textContent)).toEqual(["We agreed the budget."]);
    // A late text event of the saved part does not show it twice.
    await act(() => tauri.emit("live-transcript-event", text("rec", "We agreed", " the bud")));
    expect(screen.getAllByTestId("live-part")).toHaveLength(1);
  });

  test("events that arrive before the catch-up returns are kept", async () => {
    let answer: (c: Catchup) => void = () => {};
    const tauri = mockTauri({ live_transcript: () => new Promise<Catchup>((r) => (answer = r)) });
    render(<Harness />);
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "live_transcript")).toBe(true));
    await act(() => tauri.emit("live-transcript-event", { ...text("rec", "Newer text", "", 1), words: timed("Newer text", 250_000) }));
    await act(async () =>
      answer({
        recognition: [], parts: [saved(0, "Older part.", 0)],
        current: { segment: 0, part: 1, offset_ms: 0, words: timed("Newer", 250_000), committed: "Newer", tentative: "" },
        state: "running",
      }),
    );
    expect(screen.getAllByTestId("live-part").map((p) => p.textContent)).toEqual(["Older part.", "Newer text"]);
  });

  test("while the model loads, the view says the live transcript starts", async () => {
    const tauri = mockLive({ recognition: [], parts: [], current: null, state: "starting" });
    render(<Harness />);
    expect(await screen.findByText("Starting the live transcript…")).toBeTruthy();
    await act(() => tauri.emit("live-transcript-event", text("rec", "First words")));
    expect(screen.queryByText("Starting the live transcript…")).toBeNull();
  });

  test("a gap shows the paused line, and the text from before the gap stays", async () => {
    const tauri = mockLive({
      recognition: [], parts: [{ segment: 0, part: 0, offset_ms: 0, words: [], start_ms: 0, end_ms: 240_000, text: "Before the gap." }],
      current: null,
      state: "running",
    });
    render(<Harness />);
    await screen.findByText("Before the gap.");
    await act(() => tauri.emit("live-transcript-status", { session_id: "rec", segment: 0, state: "behind" }));
    expect(screen.getByText("Live transcript paused. The missing part is transcribed after you stop.")).toBeTruthy();
    expect(screen.getByText("Before the gap.")).toBeTruthy();
    // The worker reports running again with the first text after the gap.
    await act(() => tauri.emit("live-transcript-status", { session_id: "rec", segment: 0, state: "running" }));
    await act(() => tauri.emit("live-transcript-event", text("rec", "After the gap", "", 1)));
    expect(screen.queryByText(/Live transcript paused/)).toBeNull();
  });

  test("a paused note shows the paused line, not the line for a live transcript that fell behind", async () => {
    const tauri = mockLive({
      recognition: [], parts: [{ segment: 0, part: 0, offset_ms: 0, words: [], start_ms: 0, end_ms: 240_000, text: "Before the pause." }],
      current: null,
      state: "running",
    });
    render(<Harness mode="paused" />);
    await screen.findByText("Before the pause.");
    expect(screen.getByText("Paused. Nothing is recorded until you continue.")).toBeTruthy();
    // The worker can report that it fell behind when the segment ends.
    await act(() => tauri.emit("live-transcript-status", { session_id: "rec", segment: 0, state: "behind" }));
    expect(screen.getByText("Paused. Nothing is recorded until you continue.")).toBeTruthy();
    expect(screen.queryByText(/Live transcript paused/)).toBeNull();
    expect(screen.queryByText(/The missing part/)).toBeNull();
  });

  test("a paused note with no text shows the paused line", async () => {
    mockLive();
    render(<Harness mode="paused" />);
    expect(await screen.findByText("Paused. Nothing is recorded until you continue.")).toBeTruthy();
  });

  test("after Continue, new text shows below the text from before the pause", async () => {
    const tauri = mockLive({
      recognition: [], parts: [{ segment: 0, part: 0, offset_ms: 0, words: [], start_ms: 0, end_ms: 240_000, text: "Before the pause." }],
      current: null,
      state: "ended",
    });
    const { rerender } = render(<Harness mode="paused" />);
    await screen.findByText("Before the pause.");
    rerender(<Harness mode="recording" />);
    await act(() => tauri.emit("live-transcript-status", { session_id: "rec", segment: 1, state: "running" }));
    await act(() => tauri.emit("live-transcript-event", { ...text("rec", "After the pause"), segment: 1 }));
    expect(paragraphs()).toEqual(["Before the pause.", "After the pause"]);
    expect(screen.queryByText("Paused. Nothing is recorded until you continue.")).toBeNull();
  });

  test("a rejected live_transcript call leaves the starting line and later events still show", async () => {
    const tauri = mockTauri({
      live_transcript: () => {
        throw new Error("boom");
      },
    });
    render(<Harness />);
    await screen.findByText("Starting the live transcript…");
    await act(() => tauri.emit("live-transcript-event", text("rec", "First words")));
    await waitFor(() => expect(screen.queryByText("Starting the live transcript…")).toBeNull());
    expect(screen.getByText("First words")).toBeTruthy();
  });

  test("a live transcript that cannot run says the transcript is ready after Stop", async () => {
    const tauri = mockLive({ recognition: [], parts: [], current: null, state: "starting" });
    render(<Harness />);
    await screen.findByText("Starting the live transcript…");
    await act(() => tauri.emit("live-transcript-status", { session_id: "rec", segment: 0, state: "unavailable" }));
    await act(() => tauri.emit("live-transcript-status", { session_id: "rec", segment: 0, state: "ended" }));
    expect(screen.getByText("The transcript is ready after you stop recording.")).toBeTruthy();
  });

  test("an interrupted recording keeps the text from before the break", async () => {
    mockLive({
      recognition: [], parts: [{ segment: 0, part: 0, offset_ms: 0, words: [], start_ms: 0, end_ms: 240_000, text: "Before the break." }],
      current: null,
      state: "ended",
    });
    render(<Harness mode="interrupted" />);
    await screen.findByText("Before the break.");
    expect(screen.getByText(/Recording interrupted/)).toBeTruthy();
  });

  test("after Stop, the text stays with a line that says the final transcript comes", async () => {
    mockLive({
      recognition: [], parts: [{ segment: 0, part: 0, offset_ms: 0, words: [], start_ms: 0, end_ms: 240_000, text: "All we said." }],
      current: null,
      state: "ended",
    });
    render(<Harness mode="processing" />);
    await screen.findByText("All we said.");
    expect(screen.getByText(/final transcript/)).toBeTruthy();
  });

  test("a scroll up stops the follow and shows Scroll to bottom; a click scrolls down and follows again", async () => {
    const tauri = mockLive({
      recognition: [], parts: [{ segment: 0, part: 0, offset_ms: 0, words: [], start_ms: 0, end_ms: 240_000, text: "Long text." }],
      current: null,
      state: "running",
    });
    render(<Harness />);
    await screen.findByText("Long text.");
    const scroller = screen.getByTestId("scroller");
    let height = 2000;
    sized(scroller, () => height, 500);

    // New text while at the bottom: the view follows it.
    await act(() => tauri.emit("live-transcript-event", text("rec", "More", "", 1)));
    expect(scroller.scrollTop).toBe(2000);
    expect(screen.queryByRole("button", { name: "Scroll to bottom" })).toBeNull();

    // The user scrolls up: new text does not move the view.
    scroller.scrollTop = 400;
    act(() => {
      scroller.dispatchEvent(new Event("scroll"));
    });
    height = 2400;
    await act(() => tauri.emit("live-transcript-event", text("rec", "More text", "", 1)));
    expect(scroller.scrollTop).toBe(400);

    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Scroll to bottom" }));
    expect(scroller.scrollTop).toBe(2400);
    act(() => {
      scroller.dispatchEvent(new Event("scroll"));
    });
    expect(screen.queryByRole("button", { name: "Scroll to bottom" })).toBeNull();
    height = 2800;
    await act(() => tauri.emit("live-transcript-event", text("rec", "More text again", "", 1)));
    expect(scroller.scrollTop).toBe(2800);
  });
});


test("word-level live updates read as one paragraph with one provisional notice", async () => {
  const tauri = mockLive();
  render(<Harness />);
  await screen.findByText("The transcript shows here as people speak.");
  const event = { session_id: "rec", generation: "g", epoch: 0, key: "interim", revision: 1 };
  await act(() => tauri.emit("live-recognition-event", { ...event, phase: "interim_begin", rows: [] }));
  await act(() => tauri.emit("live-recognition-event", { ...event, phase: "interim_batch", rows: ["New", "test", "with", "the", "live", "transcript."].map((text, index) => ({
    start_sample: index * 4000, end_sample: (index + 1) * 4000,
    text, speaker: null, text_final: false, speaker_final: false,
  })) }));
  await act(() => tauri.emit("live-recognition-event", { ...event, phase: "interim_end", rows: [] }));
  expect(screen.getAllByTestId("live-part")).toHaveLength(1);
  expect(screen.getByTestId("live-part").textContent).toBe("New test with the live transcript.");
  expect(screen.getAllByText(/Provisional text/)).toHaveLength(1);
});



test("partial post-recording results do not duplicate the complete live transcript", async () => {
  mockLive({
    parts: [{ segment: 0, part: 0, offset_ms: 0, words: [], start_ms: 0, end_ms: 10000, text: "The first passage." }],
    current: { segment: 1, part: 0, offset_ms: 0, words: [], committed: "The resumed passage.", tentative: "" },
    state: "ended",
    recognition: [{ session_id: "rec", generation: "g", epoch: 0, key: "native-window/0/0", phase: "patch", revision: 1,
      rows: [{ start_sample: 0, end_sample: 160000, text: "The first passage.", speaker: "g:0:0:1", text_final: true, speaker_final: true }],
    }],
  });
  render(<Harness mode="processing" />);
  await screen.findByText("The resumed passage.");
  expect(screen.getAllByText("The first passage.")).toHaveLength(1);
  expect(screen.getAllByTestId("live-part")).toHaveLength(2);
});

describe("speaker rows (U6)", () => {
  const words = "Hello there how are you";
  const openText = (committed: string, tentative = "") => ({
    ...text("rec", committed, tentative), words: timed(`${committed}${tentative}`.trim(), 0),
  });

  // Covers AE1.
  test("new text continues the last speaker, and a snapshot puts the Speaker 2 heading at the first word of its turn", async () => {
    const tauri = mockLive();
    render(<Harness />);
    await screen.findByText("The transcript shows here as people speak.");
    await send(tauri, snapshot(1, [turn(0, 1000, "g:0:0:1")]));
    await act(() => tauri.emit("live-transcript-event", openText(words)));
    expect(headings()).toEqual(["Speaker 1"]);
    expect(paragraphs()).toEqual([words]);
    await send(tauri, snapshot(2, [turn(0, 1000, "g:0:0:1"), turn(1000, 2500, "g:0:0:2")]));
    expect(headings()).toEqual(["Speaker 1", "Speaker 2"]);
    expect(paragraphs()).toEqual(["Hello there", "how are you"]);
  });

  test("with no speaker turns, the text shows as paragraphs with no heading and no notice", async () => {
    mockLive({ ...empty, parts: [saved(0, "One person talks.", 0), saved(1, "Later text.", 240_000)] });
    render(<Harness />);
    await screen.findByText("One person talks.");
    expect(headings()).toEqual([]);
    expect(paragraphs()).toEqual(["One person talks.", "Later text."]);
    expect(screen.queryByText(/Speaker labels may change/)).toBeNull();
  });

  test("a new speaker analysis window shows a divider, and its numbers start at 1", async () => {
    const tauri = mockLive({ ...empty, parts: [saved(0, "First window.", 0), saved(1, "Second window.", 240_000)] });
    render(<Harness />);
    await screen.findByText("First window.");
    await send(tauri, snapshot(1, [turn(0, 1000, "g:0:0:2")]));
    await send(tauri, snapshot(1, [turn(240_000, 241_000, "g:1:0:5")], 1));
    expect(headings()).toEqual(["Speaker 1", "Speaker 1"]);
    expect(screen.getAllByText("Speaker labels restart")).toHaveLength(1);
  });

  // Covers AE5.
  test("while the view follows the bottom, a revised turn leaves it at the bottom", async () => {
    const tauri = mockLive({ ...empty, parts: [saved(0, words, 0)] });
    render(<Harness />);
    await screen.findByText(words);
    const scroller = screen.getByTestId("scroller");
    sized(scroller, () => 3000, 500);
    await send(tauri, snapshot(1, [turn(0, 1000, "g:0:0:1"), turn(1000, 2500, "g:0:0:2")]));
    expect(scroller.scrollTop).toBe(3000);
    await send(tauri, snapshot(2, [turn(0, 1500, "g:0:0:1"), turn(1500, 2500, "g:0:0:2")]));
    expect(headings()).toEqual(["Speaker 1", "Speaker 2"]);
    expect(scroller.scrollTop).toBe(3000);
  });

  // Covers AE5.
  test("while the reader is scrolled up, a heading added above keeps the first visible paragraph in place", async () => {
    // Four paragraphs, 5 seconds apart.
    const part = { ...saved(0, "A. B. C. D.", 0), words: ["A.", "B.", "C.", "D."].map((w, i) => ({ start_ms: i * 5000, end_ms: i * 5000 + 500, text: w })) };
    const tauri = mockLive({ ...empty, parts: [part] });
    render(<Harness />);
    await screen.findByText("A.");
    const scroller = screen.getByTestId("scroller");
    sized(scroller, () => 3000, 500);
    await send(tauri, snapshot(1, [turn(0, 20_000, "g:0:0:1")]));
    // happy-dom does no layout: a heading is 20 px high and a paragraph 100 px.
    const layout = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      let y = 0;
      if (this !== scroller) {
        for (const el of scroller.querySelectorAll<HTMLElement>("h3, [data-key]")) {
          if (el === this) break;
          y += el.tagName === "H3" ? 20 : 100;
        }
        y -= scroller.scrollTop;
      }
      const height = this === scroller ? 500 : this.tagName === "H3" ? 20 : 100;
      return { top: y, bottom: y + height, left: 0, right: 0, width: 0, height, x: 0, y, toJSON: () => ({}) } as DOMRect;
    });
    try {
      // The reader scrolls up. "C." is the first visible paragraph, 30 px above the top.
      scroller.scrollTop = 250;
      act(() => { scroller.dispatchEvent(new Event("scroll")); });
      await send(tauri, snapshot(2, [turn(0, 5000, "g:0:0:1"), turn(5000, 20_000, "g:0:0:2")]));
      expect(headings()).toEqual(["Speaker 1", "Speaker 2"]);
      // The new heading above "C." adds 20 px, and the view scrolls by the same amount.
      expect(scroller.scrollTop).toBe(270);
    } finally {
      layout.mockRestore();
    }
  });

  test("the collapsed speaker list shows with turns and no text, and goes away when rows show", async () => {
    const tauri = mockLive();
    render(<Harness />);
    await screen.findByText("The transcript shows here as people speak.");
    await send(tauri, snapshot(1, [turn(0, 1000, "g:0:0:1"), turn(1000, 2500, "g:0:0:2")]));
    expect(screen.getByText("Speaker analysis so far")).toBeTruthy();
    expect(screen.getByText(/^Speaker 2 · 1–3 seconds/)).toBeTruthy();
    await act(() => tauri.emit("live-transcript-event", openText(words)));
    expect(screen.queryByText("Speaker analysis so far")).toBeNull();
    expect(headings()).toEqual(["Speaker 1", "Speaker 2"]);
  });

  test("the notice shows while a used turn is provisional and goes away when all are final", async () => {
    const tauri = mockLive({ ...empty, parts: [saved(0, words, 0)] });
    render(<Harness />);
    await screen.findByText(words);
    await send(tauri, snapshot(1, [turn(0, 2500, "g:0:0:1", false)]));
    expect(screen.getByText("Speaker labels may change")).toBeTruthy();
    await send(tauri, snapshot(2, [turn(0, 2500, "g:0:0:1")]));
    expect(screen.queryByText("Speaker labels may change")).toBeNull();
  });

  test("the notice does not show while the final transcript loads", async () => {
    const tauri = mockLive({ ...empty, parts: [saved(0, words, 0)] });
    render(<Harness mode="loading" />);
    await screen.findByText(words);
    await send(tauri, snapshot(1, [turn(0, 2500, "g:0:0:1", false)]));
    expect(headings()).toEqual(["Speaker 1"]);
    expect(screen.queryByText("Speaker labels may change")).toBeNull();
  });

  test("after Stop, a speaker event of the post-Stop pass adds no divider and changes no row", async () => {
    const tauri = mockLive({ ...empty, parts: [saved(0, words, 0)], state: "ended" });
    render(<Harness mode="processing" />);
    await screen.findByText(words);
    await send(tauri, snapshot(1, [turn(0, 2500, "g:0:0:1")]));
    expect(headings()).toEqual(["Speaker 1"]);
    await send(tauri, snapshot(1, [turn(0, 1000, "g:1:0:1"), turn(1000, 2500, "g:1:0:2")], 1));
    expect(headings()).toEqual(["Speaker 1"]);
    expect(paragraphs()).toEqual([words]);
    expect(screen.queryByText("Speaker labels restart")).toBeNull();
  });

  test("a failed speaker phase keeps the rows and shows the error line", async () => {
    const tauri = mockLive({ ...empty, parts: [saved(0, words, 0)] });
    render(<Harness />);
    await screen.findByText(words);
    await send(tauri, snapshot(1, [turn(0, 2500, "g:0:0:1")]));
    await send(tauri, [{
      session_id: "rec", generation: "g", epoch: 0, key: "failure", phase: "failed", revision: 0,
      rows: [{ start_sample: 0, end_sample: 0, text: "Speaker recognition stopped.", speaker: null, text_final: false, speaker_final: false }],
    }]);
    expect(screen.getByRole("alert").textContent).toBe("Speaker recognition stopped.");
    expect(headings()).toEqual(["Speaker 1"]);
    expect(paragraphs()).toEqual([words]);
  });

  test("tentative words keep the secondary style inside a speaker row", async () => {
    const tauri = mockLive();
    render(<Harness />);
    await screen.findByText("The transcript shows here as people speak.");
    await send(tauri, snapshot(1, [turn(0, 2500, "g:0:0:1")]));
    await act(() => tauri.emit("live-transcript-event", openText("Hello there", " how are")));
    const tentative = screen.getByText("how are", { exact: false });
    expect(tentative.dataset.tentative).toBe("true");
    expect(tentative.className).toContain("text-label-secondary");
    expect(tentative.closest("section")?.querySelector("h3")?.textContent).toBe("Speaker 1");
  });

  test("a revision that moves a heading plays the screen-reader message once; a revision that changes no row plays nothing", async () => {
    const tauri = mockLive({ ...empty, parts: [saved(0, words, 0)] });
    render(<Harness />);
    await screen.findByText(words);
    await send(tauri, snapshot(1, [turn(0, 1000, "g:0:0:1"), turn(1000, 2500, "g:0:0:2")]));
    expect(headings()).toEqual(["Speaker 1", "Speaker 2"]);
    // The first headings are not a change of labels that the reader saw.
    expect(screen.queryByText("Speaker labels updated.")).toBeNull();
    // The same turns again: nothing changes.
    await send(tauri, snapshot(2, [turn(0, 1000, "g:0:0:1"), turn(1000, 2500, "g:0:0:2")]));
    expect(screen.queryByText("Speaker labels updated.")).toBeNull();
    // The heading moves one word later.
    await send(tauri, snapshot(3, [turn(0, 1500, "g:0:0:1"), turn(1500, 2500, "g:0:0:2")]));
    expect(paragraphs()).toEqual(["Hello there how", "are you"]);
    const message = screen.getByText("Speaker labels updated.");
    expect(message.closest("[role=status]")).toBeTruthy();
    // A revision with no visible change does not play the message again.
    await send(tauri, snapshot(4, [turn(0, 1500, "g:0:0:1"), turn(1500, 2500, "g:0:0:2")]));
    expect(screen.getByText("Speaker labels updated.")).toBe(message);
  });

  test("a speaker revision and new tentative words in one update play no screen-reader message", async () => {
    const tauri = mockLive();
    render(<Harness />);
    await screen.findByText("The transcript shows here as people speak.");
    await send(tauri, snapshot(1, [turn(0, 2500, "g:0:0:1")]));
    await act(() => tauri.emit("live-transcript-event", openText("Hello there", " how are")));
    expect(headings()).toEqual(["Speaker 1"]);
    // The text event applies the queued speaker events in the same update.
    await act(() => {
      for (const event of snapshot(2, [turn(0, 2500, "g:0:0:1")])) tauri.emit("live-recognition-event", event);
      tauri.emit("live-transcript-event", openText("Hello there how", " is it"));
    });
    await act(() => new Promise((done) => setTimeout(done, 120)));
    expect(paragraphs()).toEqual(["Hello there how is it"]);
    expect(screen.queryByText("Speaker labels updated.")).toBeNull();
  });
});
