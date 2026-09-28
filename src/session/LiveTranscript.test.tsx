import { describe, expect, test } from "bun:test";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useRef } from "react";
import type { LiveTranscript as Catchup } from "../bindings";
import { mockTauri } from "../test/tauri";
import { LiveTranscript, type LiveMode } from "./LiveTranscript";

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
  committed,
  tentative,
});

// happy-dom does no layout: give the scroller a size, so the view can tell
// whether it is at the bottom.
function sized(el: HTMLElement, scrollHeight: () => number, clientHeight: number) {
  Object.defineProperty(el, "scrollHeight", { configurable: true, get: scrollHeight });
  Object.defineProperty(el, "clientHeight", { configurable: true, get: () => clientHeight });
}

describe("LiveTranscript", () => {
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
        { segment: 0, part: 1, start_ms: 240_000, end_ms: 480_000, text: "Second part." },
        { segment: 0, part: 0, start_ms: 0, end_ms: 240_000, text: "First part." },
        { segment: 0, part: 2, start_ms: 480_000, end_ms: 600_000, text: "Third part." },
      ],
      current: { segment: 0, part: 3, committed: "Now", tentative: " speaking" },
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
        end_ms: 240_000,
        text: "We agreed the budget.",
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
    await act(() => tauri.emit("live-transcript-event", text("rec", "Newer text", "", 1)));
    await act(async () =>
      answer({
        recognition: [], parts: [{ segment: 0, part: 0, start_ms: 0, end_ms: 240_000, text: "Older part." }],
        current: { segment: 0, part: 1, committed: "Newer", tentative: "" },
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
      recognition: [], parts: [{ segment: 0, part: 0, start_ms: 0, end_ms: 240_000, text: "Before the gap." }],
      current: null,
      state: "running",
    });
    render(<Harness />);
    await screen.findByText("Before the gap.");
    await act(() => tauri.emit("live-transcript-status", { session_id: "rec", segment: 0, state: "paused" }));
    expect(screen.getByText("Live transcript paused. The missing part is transcribed after you stop.")).toBeTruthy();
    expect(screen.getByText("Before the gap.")).toBeTruthy();
    // The worker reports running again with the first text after the gap.
    await act(() => tauri.emit("live-transcript-status", { session_id: "rec", segment: 0, state: "running" }));
    await act(() => tauri.emit("live-transcript-event", text("rec", "After the gap", "", 1)));
    expect(screen.queryByText(/Live transcript paused/)).toBeNull();
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
      recognition: [], parts: [{ segment: 0, part: 0, start_ms: 0, end_ms: 240_000, text: "Before the break." }],
      current: null,
      state: "ended",
    });
    render(<Harness mode="interrupted" />);
    await screen.findByText("Before the break.");
    expect(screen.getByText(/Recording interrupted/)).toBeTruthy();
  });

  test("after Stop, the text stays with a line that says the final transcript comes", async () => {
    mockLive({
      recognition: [], parts: [{ segment: 0, part: 0, start_ms: 0, end_ms: 240_000, text: "All we said." }],
      current: null,
      state: "ended",
    });
    render(<Harness mode="processing" />);
    await screen.findByText("All we said.");
    expect(screen.getByText(/final transcript/)).toBeTruthy();
  });

  test("a scroll up stops the follow and shows Scroll to bottom; a click scrolls down and follows again", async () => {
    const tauri = mockLive({
      recognition: [], parts: [{ segment: 0, part: 0, start_ms: 0, end_ms: 240_000, text: "Long text." }],
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
