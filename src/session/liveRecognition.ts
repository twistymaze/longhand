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
  const batchKey = `${scope(event)}/${family}`;
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
      if (scope(block.event) === scope(event) && block.kind === family)
        delete blocks[key];
    }
    delete batches[batchKey];
    revisions[batchKey] = event.revision;
  } else {
    const key = identity(event);
    if (event.revision <= (revisions[key] ?? -1)) return view;
    revisions[key] = event.revision;
    for (const [key, block] of Object.entries(blocks)) {
      if (scope(block.event) !== scope(event) || block.kind !== "interim")
        continue;
      const rows = block.event.rows.filter(
        (row) => !event.rows.some((next) => overlaps(row, next)),
      );
      if (rows.length)
        blocks[key] = { ...block, event: { ...block.event, rows } };
      else delete blocks[key];
    }
  }
  for (const next of incoming) {
    const committed = Object.values(blocks)
      .filter((b) => b.kind === "text" && scope(b.event) === scope(next))
      .flatMap((b) => b.event.rows);
    const rows =
      family === "interim" && event.phase !== "patch"
        ? next.rows.filter(
            (row) =>
              !committed.some(
                (final) => final.text_final && overlaps(row, final),
              ),
          )
        : next.rows;
    const kind = event.phase === "patch" ? "text" : family;
    blocks[`${identity(next)}/${kind}`] = { event: { ...next, rows }, kind };
  }
  const previous = recognitionRows(view);
  const next = recognitionRows({ ...view, blocks });
  const corrected = next.some(
    (row) =>
      row.speaker_final &&
      previous.some(
        (old) =>
          old.start_sample === row.start_sample &&
          old.end_sample === row.end_sample &&
          old.speaker !== row.speaker,
      ),
  );
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

export function recognitionRows(view: RecognitionView): ScopedRow[] {
  const blocks = Object.values(view.blocks);
  const speakers = blocks.filter((b) => b.kind === "speakers");
  return blocks
    .filter((b) => b.kind !== "speakers")
    .flatMap(({ event }) =>
      event.rows
        .filter((r) => r.text)
        .map((row) => {
          const candidates = speakers
            .filter((b) => scope(b.event) === scope(event))
            .flatMap((b) => b.event.rows)
            .filter((r) => overlaps(row, r));
          if (!candidates.length) return row;
          const bySpeaker = new Map<
            string,
            { samples: number; final: boolean }
          >();
          for (const candidate of candidates) {
            if (!candidate.speaker) continue;
            const previous = bySpeaker.get(candidate.speaker) ?? {
              samples: 0,
              final: true,
            };
            previous.samples +=
              Math.min(row.end_sample, candidate.end_sample) -
              Math.max(row.start_sample, candidate.start_sample);
            previous.final &&= candidate.speaker_final;
            bySpeaker.set(candidate.speaker, previous);
          }
          const ranked = [...bySpeaker.entries()].sort(
            (a, b) => b[1].samples - a[1].samples,
          );
          const first = ranked[0];
          if (!first || first[1].samples === ranked[1]?.[1].samples)
            return {
              ...row,
              speaker: null,
              speaker_final: candidates.every((r) => r.speaker_final),
            };
          return { ...row, speaker: first[0], speaker_final: first[1].final };
        })
        .map((row) => ({ ...row, scope: scope(event) })),
    )
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
