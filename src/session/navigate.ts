// Following a note's anchor into the transcript (R12). An anchored note
// goes to the turn spanning its position: the last turn starting at or
// before it. A gap-anchored note goes to the gap marker at the end of its
// segment, or to the end when its segment was the last. An un-anchored note
// has nowhere to go.

import type { DocLine, NoteView } from "../bindings";

export function targetLine(lines: DocLine[], note: NoteView): number | null {
  const at = note.position_ms;
  if (at === null || lines.length === 0) return null;

  if (note.gap_anchored) {
    let best: number | null = null;
    let distance = Infinity;
    lines.forEach((line, i) => {
      if (line.kind === "gap" && Math.abs(line.at_ms - at) < distance) {
        best = i;
        distance = Math.abs(line.at_ms - at);
      }
    });
    // The gap after a segment ends at that segment's end; a note after the
    // last segment has no gap marker below it.
    if (best !== null && distance <= 1_000) return best;
    return lastTurn(lines, Infinity) ?? lines.length - 1;
  }

  return lastTurn(lines, at) ?? firstTurn(lines);
}

function lastTurn(lines: DocLine[], at: number): number | null {
  let found: number | null = null;
  lines.forEach((line, i) => {
    if (line.kind === "turn" && line.position_ms <= at) found = i;
  });
  return found;
}

function firstTurn(lines: DocLine[]): number | null {
  const i = lines.findIndex((l) => l.kind === "turn");
  return i === -1 ? null : i;
}
