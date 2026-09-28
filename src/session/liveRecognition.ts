import type { LiveRecognitionEvent, LiveRecognitionRow } from "../bindings";

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
  announcement: string;
  announcementId: number;
};
export const emptyRecognition: RecognitionView = {
  blocks: {},
  batches: {},
  revisions: {},
  error: null,
  announcement: "",
  announcementId: 0,
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
  const previous = new Map<number, Map<number, Set<string | null>>>();
  for (const row of recognitionRows(view)) {
    const ends = previous.get(row.start_sample) ?? new Map<number, Set<string | null>>();
    const speakers = ends.get(row.end_sample) ?? new Set<string | null>();
    speakers.add(row.speaker);
    ends.set(row.end_sample, speakers);
    previous.set(row.start_sample, ends);
  }
  const next = recognitionRows({ ...view, blocks });
  const corrected = next.some((row) => {
    if (!row.speaker_final) return false;
    const speakers = previous.get(row.start_sample)?.get(row.end_sample);
    return speakers !== undefined && (speakers.size > 1 || !speakers.has(row.speaker));
  });
  return {
    ...view,
    blocks,
    batches,
    revisions,
    announcement: corrected ? "Speaker labels updated." : view.announcement,
    announcementId: view.announcementId + Number(corrected),
  };
}

type ScopedRow = LiveRecognitionRow & { scope: string };

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

type SpanSpeakers = Map<string | null, { count: number; final: number }>;
const spanKey = (row: LiveRecognitionRow) => `${row.start_sample}/${row.end_sample}`;
const ambiguous = (speakers: SpanSpeakers | undefined) =>
  speakers !== undefined && speakers.size > 1 && [...speakers.values()].some((s) => s.final > 0);

/** Apply patches without copying all stored words for each event. */
function applyPatches(view: RecognitionView, events: LiveRecognitionEvent[]): RecognitionView {
  const blocks = { ...view.blocks };
  const speakerRows = speakersByScope(Object.values(blocks));
  const revisions = { ...view.revisions };
  const spans = new Map<string, SpanSpeakers>();
  const interim = new Map<string, Map<string, Block>>();
  let ambiguousCount = 0;
  const changeRow = (row: LiveRecognitionRow, delta: number) => {
    if (!row.text) return;
    const key = spanKey(row);
    const speakers = spans.get(key) ?? new Map();
    ambiguousCount -= Number(ambiguous(speakers));
    const counts = speakers.get(row.speaker) ?? { count: 0, final: 0 };
    counts.count += delta;
    counts.final += delta * Number(row.speaker_final);
    if (counts.count) speakers.set(row.speaker, counts);
    else speakers.delete(row.speaker);
    if (speakers.size) spans.set(key, speakers);
    else spans.delete(key);
    ambiguousCount += Number(ambiguous(speakers));
  };
  for (const [key, block] of Object.entries(blocks)) {
    if (block.kind === "speakers") continue;
    const eventScope = scope(block.event);
    for (const row of block.event.rows) {
      if (row.text) changeRow(projectSpeaker(row, speakerRows.get(eventScope)), 1);
    }
    if (block.kind === "interim") {
      const scoped = interim.get(eventScope) ?? new Map<string, Block>();
      scoped.set(key, block);
      interim.set(eventScope, scoped);
    }
  }
  let announcementId = view.announcementId;
  let changed = false;
  for (const event of events) {
    const key = identity(event);
    if (event.revision <= (revisions[key] ?? -1)) continue;
    changed = true;
    revisions[key] = event.revision;
    const before = new Map<string, Set<string | null>>();
    const eventScope = scope(event);
    let unchangedAmbiguous = ambiguousCount;
    const update = (row: LiveRecognitionRow, delta: number) => {
      if (!row.text) return;
      const key = spanKey(row);
      if (!before.has(key)) {
        const speakers = spans.get(key);
        before.set(key, new Set(speakers?.keys()));
        unchangedAmbiguous -= Number(ambiguous(speakers));
      }
      changeRow(projectSpeaker(row, speakerRows.get(eventScope)), delta);
    };
    const scoped = interim.get(eventScope);
    if (scoped) for (const [blockKey, block] of scoped) {
      const rows = block.event.rows.filter((row) => {
        if (!event.rows.some((next) => overlaps(row, next))) return true;
        update(row, -1);
        return false;
      });
      if (rows.length) {
        const next = { ...block, event: { ...block.event, rows } };
        blocks[blockKey] = next;
        scoped.set(blockKey, next);
      } else {
        delete blocks[blockKey];
        scoped.delete(blockKey);
      }
    }
    const blockKey = `${key}/text`;
    const old = blocks[blockKey];
    if (old) for (const row of old.event.rows) update(row, -1);
    for (const row of event.rows) update(row, 1);
    blocks[blockKey] = { kind: "text", event: { ...event, rows: event.rows } };
    let corrected = unchangedAmbiguous > 0;
    if (!corrected) for (const [key, previous] of before) {
      if (!previous.size) continue;
      const speakers = spans.get(key);
      if (speakers && [...speakers].some(([speaker, counts]) =>
        counts.final > 0 && (previous.size > 1 || !previous.has(speaker)))) {
        corrected = true;
        break;
      }
    }
    announcementId += Number(corrected);
  }
  if (!changed) return view;
  return {
    ...view, blocks, revisions, announcementId,
    announcement: announcementId > view.announcementId ? "Speaker labels updated." : view.announcement,
  };
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
        .map((row) => ({ ...projectSpeaker(row, speakers.get(eventScope)), scope: eventScope }));
    })
    .sort(
      (a, b) => a.start_sample - b.start_sample || a.end_sample - b.end_sample,
    );
}


// Match the unspaced scripts in asr::stream::join_words.
const unspaced = /[\u3000-\u30ff\u31f0-\u31ff\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff\uff00-\uffef\u{20000}-\u{2ffff}]/u;
function joinText(left: string, right: string): string {
  const characters = Array.from(left);
  const end = characters[characters.length - 1] ?? "";
  const start = Array.from(right)[0] ?? "";
  const space = left && right && !unspaced.test(end) && !unspaced.test(start)
    && !/^[,.;:!?%)\]}]/u.test(right) && !/[([{]$/u.test(left);
  return left + (space ? " " : "") + right;
}

/** Group timed words for display without changing the stored recognition rows. */
export function recognitionParagraphs(rows: ScopedRow[]): ScopedRow[] {
  const paragraphs: ScopedRow[] = [];
  for (const row of rows) {
    const text = row.text?.trim();
    if (!text) continue;
    const last = paragraphs[paragraphs.length - 1];
    if (last && last.scope === row.scope && last.speaker === row.speaker
      && row.start_sample - last.end_sample <= 2 * 16000
      && row.end_sample - last.start_sample <= 30 * 16000
      && (last.text?.length ?? 0) + text.length <= 600) {
      last.text = joinText(last.text ?? "", text);
      last.end_sample = Math.max(last.end_sample, row.end_sample);
      last.text_final &&= row.text_final;
      last.speaker_final &&= row.speaker_final;
    } else {
      paragraphs.push({ ...row, text });
    }
  }
  return paragraphs;
}
