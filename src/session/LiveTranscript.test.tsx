import { describe, expect, test } from "bun:test";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useRef } from "react";
import type { LiveTranscript as Catchup } from "../bindings";
import { mockTauri } from "../test/tauri";
import { LiveTranscript, type LiveMode } from "./LiveTranscript";

const empty: Catchup = { parts: [], current: null, state: "running" };

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
      parts: [
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
        parts: [{ segment: 0, part: 0, start_ms: 0, end_ms: 240_000, text: "Older part." }],
        current: { segment: 0, part: 1, committed: "Newer", tentative: "" },
        state: "running",
      }),
    );
    expect(screen.getAllByTestId("live-part").map((p) => p.textContent)).toEqual(["Older part.", "Newer text"]);
  });

  test("while the model loads, the view says the live transcript starts", async () => {
    const tauri = mockLive({ parts: [], current: null, state: "starting" });
    render(<Harness />);
    expect(await screen.findByText("Starting the live transcript…")).toBeTruthy();
    await act(() => tauri.emit("live-transcript-event", text("rec", "First words")));
    expect(screen.queryByText("Starting the live transcript…")).toBeNull();
  });

  test("a gap shows the paused line, and the text from before the gap stays", async () => {
    const tauri = mockLive({
      parts: [{ segment: 0, part: 0, start_ms: 0, end_ms: 240_000, text: "Before the gap." }],
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
    const tauri = mockLive({ parts: [], current: null, state: "starting" });
    render(<Harness />);
    await screen.findByText("Starting the live transcript…");
    await act(() => tauri.emit("live-transcript-status", { session_id: "rec", segment: 0, state: "unavailable" }));
    await act(() => tauri.emit("live-transcript-status", { session_id: "rec", segment: 0, state: "ended" }));
    expect(screen.getByText("The transcript is ready after you stop recording.")).toBeTruthy();
  });

  test("an interrupted recording keeps the text from before the break", async () => {
    mockLive({
      parts: [{ segment: 0, part: 0, start_ms: 0, end_ms: 240_000, text: "Before the break." }],
      current: null,
      state: "ended",
    });
    render(<Harness mode="interrupted" />);
    await screen.findByText("Before the break.");
    expect(screen.getByText(/Recording interrupted/)).toBeTruthy();
  });

  test("after Stop, the text stays with a line that says the final transcript comes", async () => {
    mockLive({
      parts: [{ segment: 0, part: 0, start_ms: 0, end_ms: 240_000, text: "All we said." }],
      current: null,
      state: "ended",
    });
    render(<Harness mode="processing" />);
    await screen.findByText("All we said.");
    expect(screen.getByText(/final transcript/)).toBeTruthy();
  });

  test("a scroll up stops the follow and shows Scroll to bottom; a click scrolls down and follows again", async () => {
    const tauri = mockLive({
      parts: [{ segment: 0, part: 0, start_ms: 0, end_ms: 240_000, text: "Long text." }],
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
