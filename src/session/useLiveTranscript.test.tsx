import { describe, expect, test } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { LiveTranscript as Catchup, LiveWord } from "../bindings";
import { mockTauri } from "../test/tauri";
import { useLiveTranscript } from "./useLiveTranscript";

const word = (i: number, text = `w${i}`): LiveWord => ({ start_ms: i * 100, end_ms: i * 100 + 80, text });
const words = (from: number, to: number) => Array.from({ length: to - from }, (_, i) => word(from + i));

const text = (segment: number, part: number, word_start: number, sent: LiveWord[], note_id = "rec") => ({
  note_id,
  segment,
  part,
  offset_ms: segment * 60_000,
  committed: sent.map((w) => w.text).join(" "),
  tentative: "",
  word_start,
  words: sent,
});

const saved = (segment: number, part: number, sent: LiveWord[], note_id = "rec") => ({
  note_id,
  segment,
  part,
  offset_ms: segment * 60_000,
  start_ms: 0,
  end_ms: 1_000,
  text: sent.map((w) => w.text).join(" "),
  words: sent,
});

async function mounted(catchup: Catchup = { recognition: [], parts: [], current: null, state: "running" }, id = "rec") {
  const tauri = mockTauri({ live_transcript: () => catchup });
  const hook = renderHook(({ noteId }) => useLiveTranscript(noteId), { initialProps: { noteId: id } });
  await waitFor(() => expect(hook.result.current.loaded).toBe(true));
  return { tauri, hook };
}

const texts = (list: LiveWord[]) => list.map((w) => w.text);

describe("useLiveTranscript", () => {
  test("a catch-up then 2 text events give the full word list of the open part", async () => {
    const { tauri, hook } = await mounted({
      recognition: [],
      parts: [],
      current: { segment: 0, part: 0, offset_ms: 0, committed: "w0 w1", tentative: "", words: words(0, 2) },
      state: "running",
    });
    await act(() => tauri.emit("live-transcript-event", text(0, 0, 2, words(2, 4))));
    await act(() => tauri.emit("live-transcript-event", text(0, 0, 4, words(4, 5))));
    const live = hook.result.current;
    expect(live.open).toHaveLength(1);
    expect(texts(live.open[0].words)).toEqual(["w0", "w1", "w2", "w3", "w4"]);
    expect(live.current?.words).toEqual(live.open[0].words);
  });

  test("a text event with word_start 3 replaces words 3 and later and keeps words 0 to 2", async () => {
    const { tauri, hook } = await mounted();
    await act(() => tauri.emit("live-transcript-event", text(0, 0, 0, words(0, 5))));
    await act(() => tauri.emit("live-transcript-event", text(0, 0, 3, [word(3, "new3")])));
    expect(texts(hook.result.current.open[0].words)).toEqual(["w0", "w1", "w2", "new3"]);
  });

  test("a text event for segment 2 before the saved event of segment 1 keeps segment 1's open words until it saves", async () => {
    const { tauri, hook } = await mounted();
    await act(() => tauri.emit("live-transcript-event", text(1, 0, 0, words(0, 3))));
    await act(() => tauri.emit("live-transcript-event", text(2, 0, 0, words(10, 12))));
    let live = hook.result.current;
    expect(live.open.map((p) => [p.segment, p.part])).toEqual([[1, 0], [2, 0]]);
    expect(texts(live.open[0].words)).toEqual(["w0", "w1", "w2"]);
    expect(live.open[0].offset_ms).toBe(60_000);
    expect(live.current?.segment).toBe(2);

    await act(() => tauri.emit("live-part-saved", saved(1, 0, words(0, 3))));
    live = hook.result.current;
    expect(live.open.map((p) => [p.segment, p.part])).toEqual([[2, 0]]);
    expect(live.parts).toHaveLength(1);
    expect(texts(live.parts[0].words)).toEqual(["w0", "w1", "w2"]);
    expect(live.parts[0].offset_ms).toBe(60_000);
  });

  test("a text event for the next part of the same segment ends the earlier open part", async () => {
    const { tauri, hook } = await mounted();
    await act(() => tauri.emit("live-transcript-event", text(0, 0, 0, words(0, 2))));
    // The worker did not save part 0 (a lost part). Part 1 replaces it.
    await act(() => tauri.emit("live-transcript-event", text(0, 1, 0, words(5, 6))));
    expect(hook.result.current.open.map((p) => [p.segment, p.part])).toEqual([[0, 1]]);
  });

  test("a late text event for a part that is already saved changes nothing", async () => {
    const { tauri, hook } = await mounted();
    await act(() => tauri.emit("live-transcript-event", text(0, 0, 0, words(0, 2))));
    await act(() => tauri.emit("live-part-saved", saved(0, 0, words(0, 2))));
    const before = hook.result.current;
    await act(() => tauri.emit("live-transcript-event", text(0, 0, 2, words(2, 3))));
    expect(hook.result.current).toBe(before);
    expect(hook.result.current.open).toEqual([]);
    expect(hook.result.current.current).toBeNull();
  });

  test("a remount for the same session keeps the number map", async () => {
    const first = await mounted(undefined, "keep");
    first.hook.result.current.numbers.set("g:0:0:1", 1);
    first.hook.unmount();
    const second = await mounted(undefined, "keep");
    expect(second.hook.result.current.numbers.get("g:0:0:1")).toBe(1);
  });

  test("a different session starts with an empty number map", async () => {
    const { hook } = await mounted(undefined, "one");
    hook.result.current.numbers.set("g:0:0:1", 1);
    hook.rerender({ noteId: "two" });
    await waitFor(() => expect(hook.result.current.loaded).toBe(true));
    expect(hook.result.current.numbers.size).toBe(0);
    // The view of the new session shows no state of the old one.
    expect(hook.result.current.open).toEqual([]);
  });

  test("a view of a new session does not show the old session's words before its catch-up", async () => {
    const { tauri, hook } = await mounted(undefined, "a");
    await act(() => tauri.emit("live-transcript-event", text(0, 0, 0, words(0, 2), "a")));
    expect(hook.result.current.open).toHaveLength(1);
    hook.rerender({ noteId: "b" });
    expect(hook.result.current.open).toEqual([]);
    expect(hook.result.current.current).toBeNull();
  });
});
