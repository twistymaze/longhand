import { describe, expect, test } from "bun:test";
import type { LiveRecognitionEvent, LiveRecognitionRow } from "../bindings";
import { applyRecognition, applyRecognitionEvents, emptyRecognition, recognitionRows, type RecognitionView } from "./liveRecognition";

const row = (start: number, speaker: string | null = "one"): LiveRecognitionRow => ({
  start_sample: start, end_sample: start + 10, text: `Word ${start}`,
  speaker, text_final: true, speaker_final: true,
});
const event = (rows: LiveRecognitionRow[], overrides: Partial<LiveRecognitionEvent> = {}): LiveRecognitionEvent => ({
  session_id: "test", generation: "g", epoch: 0, key: "text", phase: "patch", revision: 1, rows, ...overrides,
});
function snapshot(view: RecognitionView, rows: LiveRecognitionRow[], epoch = 0, revision = 1) {
  for (const phase of ["snapshot_begin", "snapshot_batch", "snapshot_end"] as const)
    view = applyRecognition(view, event(phase === "snapshot_batch" ? rows : [], { phase, epoch, revision, key: "speakers" }));
  return view;
}

describe("live recognition growth", () => {
  test("batched replacement histories match sequential results", () => {
    let seed = 12345;
    const random = (limit: number) => {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      return seed % limit;
    };
    const updates = Array.from({ length: 300 }, (_, revision) => event(
      Array.from({ length: random(4) }, () => ({
        ...row(random(15) * 5, [null, "one", "two"][random(3)]),
        speaker_final: random(3) > 0, text: random(5) ? "Word" : null,
      })),
      { key: `word${random(12)}`, epoch: random(3), revision },
    ));
    const withSpeakers = snapshot(snapshot(emptyRecognition, [
      { ...row(0, "label one"), end_sample: 25, text: null },
      { ...row(25, "label two"), end_sample: 50, text: null, speaker_final: false },
      { ...row(50, null), end_sample: 65, text: null },
    ]), [{ ...row(0, "other epoch"), end_sample: 75, text: null }], 1);
    for (const initial of [emptyRecognition, withSpeakers]) {
      let sequential = initial;
      let batched = initial;
      for (let start = 0; start < updates.length; start += 17) {
        const batch = updates.slice(start, start + 17);
        sequential = batch.reduce(applyRecognition, sequential);
        batched = applyRecognitionEvents(batched, batch);
        expect(batched).toEqual(sequential);
        expect(recognitionRows(batched)).toEqual(recognitionRows(sequential));
      }
    }
  });
  test("a burst of word patches reads stored text once instead of once per event", () => {
    let reads = 0;
    const blocks: RecognitionView["blocks"] = {};
    for (let i = 0; i < 1000; i++) {
      const word = row(i * 10);
      Object.defineProperty(word, "text", { enumerable: true, get: () => { reads++; return "Word"; } });
      blocks[`word${i}`] = { kind: "text", event: event([word], { key: `word${i}` }) };
    }
    const updates = Array.from({ length: 1000 }, (_, i) => event([row((i + 1000) * 10)], { key: `new${i}` }));
    const next = applyRecognitionEvents({ ...emptyRecognition, blocks }, updates);
    expect(reads).toBeLessThan(10000);
    expect(recognitionRows(next)).toHaveLength(2000);
    expect(next.announcementId).toBe(0);
  });

  test("speaker snapshots do not make a patch burst reread every stored word per event", () => {
    let reads = 0;
    const blocks: RecognitionView["blocks"] = {};
    for (let i = 0; i < 1000; i++) {
      const word = row(i * 10);
      Object.defineProperty(word, "text", { enumerable: true, get: () => { reads++; return "Word"; } });
      blocks[`word${i}`] = { kind: "text", event: event([word], { key: `word${i}` }) };
    }
    blocks.speakers = { kind: "speakers", event: event([
      { ...row(0, "snapshot label"), end_sample: 20000, text: null },
    ]) };
    const updates = Array.from({ length: 1000 }, (_, i) => event([row((i + 1000) * 10)], { key: `new${i}` }));
    const next = applyRecognitionEvents({ ...emptyRecognition, blocks }, updates);
    expect(reads).toBeLessThan(10000);
    const rows = recognitionRows(next);
    expect(rows).toHaveLength(2000);
    expect(rows.every((row) => row.speaker === "snapshot label")).toBe(true);
    expect(next.announcementId).toBe(0);
  });

  test("batched patches preserve each revision, interim removal, scope, and announcement", () => {
    const updates: LiveRecognitionEvent[] = [
      event([], { phase: "interim_begin", key: "interim" }),
      event([row(0), row(10), row(20)], { phase: "interim_batch", key: "interim" }),
      event([], { phase: "interim_end", key: "interim" }),
      event([row(0, "two")]),
      event([row(0, "stale")]),
      event([row(0, "three")], { epoch: 1 }),
      event([row(40, null)], { key: "later" }),
      event([{ ...row(0, "four"), speaker_final: false }], { revision: 2 }),
      event([row(10, "two")], { key: "second" }),
      event([row(0, "three")], { revision: 3 }),
      event([], { key: "text", revision: 4 }),
      event([], { phase: "snapshot_begin", key: "speakers" }),
      event([{ ...row(10, "label"), text: null }], { phase: "snapshot_batch", key: "speakers" }),
      event([], { phase: "snapshot_end", key: "speakers" }),
      event([row(30)], { key: "last" }),
    ];
    for (let end = 1; end <= updates.length; end++) {
      const prefix = updates.slice(0, end);
      const expected = prefix.reduce(applyRecognition, emptyRecognition);
      const actual = applyRecognitionEvents(emptyRecognition, prefix);
      expect(actual).toEqual(expected);
      expect(recognitionRows(actual)).toEqual(recognitionRows(expected));
    }
  });

  test("one patch does not compare every pair of stored rows", () => {
    const rows = Array.from({ length: 1000 }, (_, i) => row(i * 10));
    const view: RecognitionView = { ...emptyRecognition, blocks: { text: { kind: "text", event: event(rows) } } };
    // Count predicate visits instead of elapsed time so this check is independent of host load.
    const original = Array.prototype.some;
    let visits = 0;
    Array.prototype.some = function (predicate, thisArg) {
      return original.call(this, (value, index, array) => {
        visits++;
        return predicate.call(thisArg, value, index, array);
      });
    };
    let result: RecognitionView;
    try {
      result = applyRecognition(view, event([row(10000)], { key: "new" }));
    } finally {
      Array.prototype.some = original;
    }
    expect(recognitionRows(result)).toHaveLength(1001);
    expect(result.announcementId).toBe(0);
    expect(visits).toBeLessThan(10000);
  });

  test("speaker scope reads grow with blocks instead of text rows times blocks", () => {
    let scopeReads = 0;
    const blocks: RecognitionView["blocks"] = {};
    for (let epoch = 0; epoch < 100; epoch++) {
      blocks[`text${epoch}`] = { kind: "text", event: event(Array.from({ length: 10 }, (_, i) => row(i * 10, null)), { epoch }) };
      const labels = event([{ ...row(0), end_sample: 100, text: null }], { epoch });
      Object.defineProperty(labels, "generation", { get: () => { scopeReads++; return "g"; } });
      blocks[`speakers${epoch}`] = { kind: "speakers", event: labels };
    }
    const rows = recognitionRows({ ...emptyRecognition, blocks });
    expect(rows).toHaveLength(1000);
    expect(rows.every((r) => r.speaker === "one")).toBe(true);
    expect(scopeReads).toBeLessThan(1000);
  });

  test("corrections preserve timestamp matching with duplicate spans and scopes", () => {
    let view = applyRecognition(emptyRecognition, event([row(0, "one")]));
    view = applyRecognition(view, event([row(0, "two")], { epoch: 1 }));
    expect(view.announcementId).toBe(1);
    view = applyRecognition(view, event([row(20)], { key: "new" }));
    // The existing contract compares all rows with the same bounds, including other scopes.
    expect(view.announcementId).toBe(2);
    expect(recognitionRows(view).map((r) => [r.start_sample, r.speaker, r.scope])).toEqual([
      [0, "one", '["g",0]'], [0, "two", '["g",1]'], [20, "one", '["g",0]'],
    ]);
    expect(applyRecognition(view, event([row(20)], { key: "new" }))).toBe(view);
  });

  test("snapshot joins preserve strict overlap, weighting, ties, null labels, and finality", () => {
    const words = [row(0, "original"), row(10, "original"), row(20, "original"), row(30, "original")];
    let view = applyRecognition(emptyRecognition, event(words));
    view = snapshot(view, [
      { ...row(0, "a"), end_sample: 5, text: null },
      { ...row(5, "b"), end_sample: 10, text: null },
      { ...row(10, "a"), end_sample: 16, text: null, speaker_final: false },
      { ...row(16, "b"), end_sample: 20, text: null },
      { ...row(20, null), text: null, speaker_final: false },
    ]);
    view = snapshot(view, [{ ...row(30, "other scope"), text: null }], 1);
    expect(recognitionRows(view).map((r) => [r.speaker, r.speaker_final])).toEqual([
      [null, true], ["a", false], [null, false], ["original", true],
    ]);
    view = snapshot(view, [], 0, 2);
    expect(recognitionRows(view).map((r) => r.speaker)).toEqual(words.map((r) => r.speaker));
  });

  test("committed text removes overlapping interim rows but preserves adjacent rows and other epochs", () => {
    let view = emptyRecognition;
    for (const phase of ["interim_begin", "interim_batch", "interim_end"] as const)
      view = applyRecognition(view, event(phase === "interim_batch" ? [row(0), row(10)] : [], { phase, key: "interim" }));
    view = applyRecognition(view, event([row(0, "two")]));
    for (const phase of ["interim_begin", "interim_batch", "interim_end"] as const)
      view = applyRecognition(view, event(phase === "interim_batch" ? [row(0), row(10)] : [], { phase, key: "interim", revision: 2 }));
    expect(recognitionRows(view).map((r) => [r.start_sample, r.speaker])).toEqual([[0, "two"], [10, "one"]]);
    view = applyRecognition(view, event([row(0, "three")], { epoch: 1 }));
    expect(recognitionRows(view)).toHaveLength(3);
  });
});
