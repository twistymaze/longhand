import type { LiveRecognitionEvent, LiveRecognitionRow } from "../bindings";
import type { SpeakerBlock } from "./liveRows";

type Block = {
  event: LiveRecognitionEvent;
  kind: "text" | "speakers" | "interim";
};
type Batch = { revision: number; events: LiveRecognitionEvent[] };
export type RecognitionView = {
  blocks: Record<string, Block>;
  batches: Record<string, Batch>;
  revisions: Record<string, number>;
  error: string | null;
};
export const emptyRecognition: RecognitionView = {
  blocks: {},
  batches: {},
  revisions: {},
  error: null,
};
const scope = (e: LiveRecognitionEvent) =>
  JSON.stringify([e.generation, e.epoch]);
const identity = (e: LiveRecognitionEvent) =>
  JSON.stringify([e.generation, e.epoch, e.key]);
const overlaps = (a: LiveRecognitionRow, b: LiveRecognitionRow) =>
  a.start_sample < b.end_sample && b.start_sample < a.end_sample;

export function applyRecognition(
  view: RecognitionView,
  event: LiveRecognitionEvent,
): RecognitionView {
  if (event.phase === "failed")
    return {
      ...view,
      error:
        event.rows
          .map((r) => r.text)
          .filter(Boolean)
          .join(" ") || "Speaker recognition stopped.",
    };
  const family = event.phase.startsWith("snapshot") ? "speakers" : "interim";
  const eventScope = scope(event);
  const batchKey = `${eventScope}/${family}`;
  if (event.phase.endsWith("_begin")) {
    if (
      event.revision <= (view.revisions[batchKey] ?? -1) ||
      event.revision <= (view.batches[batchKey]?.revision ?? -1)
    )
      return view;
    return {
      ...view,
      batches: {
        ...view.batches,
        [batchKey]: { revision: event.revision, events: [] },
      },
    };
  }
  if (event.phase.endsWith("_batch")) {
    const batch = view.batches[batchKey];
    if (!batch || batch.revision !== event.revision) return view;
    return {
      ...view,
      batches: {
        ...view.batches,
        [batchKey]: { ...batch, events: [...batch.events, event] },
      },
    };
  }
  let incoming = [event];
  const blocks = { ...view.blocks };
  const batches = { ...view.batches };
  const revisions = { ...view.revisions };
  if (event.phase.endsWith("_end")) {
    const batch = batches[batchKey];
    if (
      !batch ||
      batch.revision !== event.revision ||
      event.revision <= (revisions[batchKey] ?? -1)
    )
      return view;
    incoming = batch.events;
    for (const [key, block] of Object.entries(blocks)) {
      if (scope(block.event) === eventScope && block.kind === family)
        delete blocks[key];
    }
    delete batches[batchKey];
    revisions[batchKey] = event.revision;
  } else {
    const key = identity(event);
    if (event.revision <= (revisions[key] ?? -1)) return view;
    revisions[key] = event.revision;
    for (const [key, block] of Object.entries(blocks)) {
      if (block.kind !== "interim" || scope(block.event) !== eventScope)
        continue;
      const rows = block.event.rows.filter(
        (row) => !event.rows.some((next) => overlaps(row, next)),
      );
      if (rows.length)
        blocks[key] = { ...block, event: { ...block.event, rows } };
      else delete blocks[key];
    }
  }
  const committedByScope = new Map<string, LiveRecognitionRow[]>();
  if (family === "interim" && event.phase !== "patch") {
    for (const block of Object.values(blocks)) {
      if (block.kind !== "text") continue;
      const key = scope(block.event);
      const rows = committedByScope.get(key) ?? [];
      for (const row of block.event.rows) if (row.text_final) rows.push(row);
      committedByScope.set(key, rows);
    }
  }
  for (const next of incoming) {
    const committed = committedByScope.get(scope(next)) ?? [];
    const rows =
      family === "interim" && event.phase !== "patch"
        ? next.rows.filter(
            (row) =>
              !committed.some((final) => overlaps(row, final)),
          )
        : next.rows;
    const kind = event.phase === "patch" ? "text" : family;
    blocks[`${identity(next)}/${kind}`] = { event: { ...next, rows }, kind };
  }
  return { ...view, blocks, batches, revisions };
}

/** A text row with its speaker from the snapshot, and its speaker analysis window. */
export type ScopedRow = LiveRecognitionRow & { scope: string; generation: string; epoch: number };

export function applyRecognitionEvents(view: RecognitionView, events: LiveRecognitionEvent[]): RecognitionView {
  let next = view;
  for (let i = 0; i < events.length;) {
    let end = i;
    while (end < events.length && events[end].phase === "patch") end++;
    if (end > i) {
      next = applyPatches(next, events.slice(i, end));
      i = end;
    } else {
      next = applyRecognition(next, events[i]);
      i++;
    }
  }
  return next;
}

/** Apply patches without copying all stored words for each event. */
function applyPatches(view: RecognitionView, events: LiveRecognitionEvent[]): RecognitionView {
  const blocks = { ...view.blocks };
  const revisions = { ...view.revisions };
  const interim = new Map<string, Map<string, Block>>();
  for (const [key, block] of Object.entries(blocks)) {
    if (block.kind !== "interim") continue;
    const eventScope = scope(block.event);
    const scoped = interim.get(eventScope) ?? new Map<string, Block>();
    scoped.set(key, block);
    interim.set(eventScope, scoped);
  }
  let changed = false;
  for (const event of events) {
    const key = identity(event);
    if (event.revision <= (revisions[key] ?? -1)) continue;
    changed = true;
    revisions[key] = event.revision;
    const eventScope = scope(event);
    const scoped = interim.get(eventScope);
    if (scoped) for (const [blockKey, block] of scoped) {
      const rows = block.event.rows.filter(
        (row) => !event.rows.some((next) => overlaps(row, next)),
      );
      if (rows.length) {
        const next = { ...block, event: { ...block.event, rows } };
        blocks[blockKey] = next;
        scoped.set(blockKey, next);
      } else {
        delete blocks[blockKey];
        scoped.delete(blockKey);
      }
    }
    blocks[`${key}/text`] = { kind: "text", event: { ...event, rows: event.rows } };
  }
  if (!changed) return view;
  return { ...view, blocks, revisions };
}

function speakersByScope(blocks: Block[]): Map<string, LiveRecognitionRow[]> {
  const speakers = new Map<string, LiveRecognitionRow[]>();
  for (const block of blocks) {
    if (block.kind !== "speakers") continue;
    const key = scope(block.event);
    const rows = speakers.get(key) ?? [];
    for (const row of block.event.rows) rows.push(row);
    speakers.set(key, rows);
  }
  return speakers;
}

function projectSpeaker(row: LiveRecognitionRow, speakers: LiveRecognitionRow[] = []): LiveRecognitionRow {
  const candidates = speakers.filter((r) => overlaps(row, r));
  if (!candidates.length) return row;
  const bySpeaker = new Map<string, { samples: number; final: boolean }>();
  for (const candidate of candidates) {
    if (!candidate.speaker) continue;
    const previous = bySpeaker.get(candidate.speaker) ?? { samples: 0, final: true };
    previous.samples +=
      Math.min(row.end_sample, candidate.end_sample) -
      Math.max(row.start_sample, candidate.start_sample);
    previous.final &&= candidate.speaker_final;
    bySpeaker.set(candidate.speaker, previous);
  }
  const ranked = [...bySpeaker.entries()].sort((a, b) => b[1].samples - a[1].samples);
  const first = ranked[0];
  if (!first || first[1].samples === ranked[1]?.[1].samples)
    return { ...row, speaker: null, speaker_final: candidates.every((r) => r.speaker_final) };
  return { ...row, speaker: first[0], speaker_final: first[1].final };
}

export function recognitionRows(view: RecognitionView): ScopedRow[] {
  const blocks = Object.values(view.blocks);
  const speakers = speakersByScope(blocks);
  return blocks
    .filter((b) => b.kind !== "speakers")
    .flatMap(({ event }) => {
      const eventScope = scope(event);
      return event.rows
        .filter((r) => r.text)
        .map((row) => ({
          ...projectSpeaker(row, speakers.get(eventScope)),
          scope: eventScope,
          generation: event.generation,
          epoch: event.epoch,
        }));
    })
    .sort(
      (a, b) => a.start_sample - b.start_sample || a.end_sample - b.end_sample,
    );
}

/** The speaker turns of each speaker analysis window, for the row builder. */
export function speakerBlocks(view: RecognitionView): SpeakerBlock[] {
  const byScope = new Map<string, SpeakerBlock>();
  for (const block of Object.values(view.blocks)) {
    if (block.kind !== "speakers") continue;
    const { generation, epoch } = block.event;
    const key = scope(block.event);
    const entry = byScope.get(key) ?? { generation, epoch, turns: [] };
    for (const row of block.event.rows)
      entry.turns.push({
        start_sample: row.start_sample,
        end_sample: row.end_sample,
        speaker: row.speaker,
        speaker_final: row.speaker_final,
      });
    byScope.set(key, entry);
  }
  return [...byScope.values()];
}
