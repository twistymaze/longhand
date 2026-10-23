// The live transcript of one note (KTD10). The hook reads the catch-up when
// the view opens, then applies the live events for the note's id. Events
// that arrive before the catch-up returns wait, and apply after it.

import { useEffect, useMemo, useState } from "react";
import {
  commands,
  events,
  type LiveCurrent,
  type LivePartSaved,
  type LivePartText,
  type LiveState,
  type LiveTranscript,
  type LiveTranscriptEvent,
  type LiveTranscriptStatus,
  type LiveRecognitionEvent,
} from "../bindings";
import { applyRecognition, applyRecognitionEvents, emptyRecognition, type RecognitionView } from "./liveRecognition";
import type { NumberMap } from "./liveRows";

export type LiveView = {
  /// The saved parts, with their words and segment offset, in order of
  /// segment and part.
  parts: LivePartText[];
  /// The open parts, in order of segment and part. A part stays open until
  /// its saved event arrives, so the last part of a segment can stay open
  /// while the first part of the next segment grows.
  open: LiveCurrent[];
  /// The newest open part, or null.
  current: LiveCurrent | null;
  /// What the live worker does now. Null when no worker reported.
  state: LiveState | null;
  /// The catch-up returned.
  loaded: boolean;
  recognition: RecognitionView;
  /// The shown speaker numbers of the note (KTD12). The row builder adds a
  /// number when a heading first shows. The map stays while the app runs.
  numbers: NumberMap;
};

type State = Omit<LiveView, "numbers">;

const none: State = { parts: [], open: [], current: null, state: null, loaded: false, recognition: emptyRecognition };

// The number map of each note (KTD12). It is module state, so a remount of
// the view (a tab switch or a reopened note) keeps the numbers that the
// reader saw. Each note has its own map.
const numberMaps = new Map<string, NumberMap>();

/// The shown speaker numbers of a note. The same note always gets the same map.
export function liveNumbers(noteId: string): NumberMap {
  let numbers = numberMaps.get(noteId);
  if (!numbers) {
    numbers = new Map();
    numberMaps.set(noteId, numbers);
  }
  return numbers;
}

type Change =
  | { kind: "recognition"; data: LiveRecognitionEvent }
  | { kind: "catchup"; data: LiveTranscript }
  | { kind: "text"; data: LiveTranscriptEvent }
  | { kind: "saved"; data: LivePartSaved }
  | { kind: "status"; data: LiveTranscriptStatus };

const same = (a: { segment: number; part: number }, b: { segment: number; part: number }) =>
  a.segment === b.segment && a.part === b.part;

const order = (a: { segment: number; part: number }, b: { segment: number; part: number }) => a.segment - b.segment || a.part - b.part;

/// Add a saved part, or replace the part with the same number.
function withPart(parts: LivePartText[], part: LivePartText): LivePartText[] {
  return [...parts.filter((p) => !same(p, part)), part].sort(order);
}

/// Set the open parts, and the newest open part.
function withOpen(live: State, open: LiveCurrent[]): State {
  return { ...live, open, current: open[open.length - 1] ?? null };
}

export function apply(live: State, change: Change): State {
  switch (change.kind) {
    case "recognition": return { ...live, recognition: applyRecognition(live.recognition, change.data) };
    case "catchup": {
      // One merge and one sort: a long note has many saved parts.
      const byKey = new Map(live.parts.map((p) => [`${p.segment}-${p.part}`, p]));
      for (const p of change.data.parts) byKey.set(`${p.segment}-${p.part}`, p);
      const parts = [...byKey.values()].sort(order);
      // The catch-up holds the full word list of its open part (KTD4).
      const current = change.data.current;
      const open = current && !parts.some((p) => same(p, current)) ? [current] : [];
      return withOpen({
        recognition: change.data.recognition ? applyRecognitionEvents(emptyRecognition, change.data.recognition) : live.recognition,
        parts,
        open,
        current: null,
        state: change.data.state,
        loaded: true,
      }, open);
    }
    case "text": {
      const { segment, part, offset_ms, committed, tentative, word_start, words } = change.data;
      // A late event of a part that is already saved changes nothing.
      if (live.parts.some((p) => same(p, change.data))) return live;
      const earlier = live.open.find((p) => same(p, change.data));
      // Keep the words before word_start, then add the changed words (KTD3).
      const before = earlier ? earlier.words.slice(0, word_start) : [];
      const next: LiveCurrent = { segment, part, offset_ms, committed, tentative, words: [...before, ...words] };
      // One worker sends the parts of a segment in order, and it sends the
      // saved event of a part before the text of the next part. So an
      // earlier open part of the same segment was not saved, and it ends
      // here. The open part of an earlier segment stays until its saved event.
      const others = live.open.filter((p) => !same(p, next) && !(p.segment === segment && p.part < part));
      return withOpen(live, [...others, next].sort(order));
    }
    case "saved": {
      const { segment, part, offset_ms, start_ms, end_ms, text, words } = change.data;
      const saved = { segment, part, offset_ms, start_ms, end_ms, text, words };
      return withOpen({ ...live, parts: withPart(live.parts, saved) }, live.open.filter((p) => !same(p, saved)));
    }
    case "status":
      return { ...live, state: change.data.state };
  }
}

function applyChanges(live: State, changes: Change[]): State {
  let next = live;
  for (let i = 0; i < changes.length;) {
    const recognition: LiveRecognitionEvent[] = [];
    while (i < changes.length && changes[i].kind === "recognition") {
      recognition.push((changes[i] as Extract<Change, { kind: "recognition" }>).data);
      i++;
    }
    if (recognition.length) {
      const updated = applyRecognitionEvents(next.recognition, recognition);
      if (updated !== next.recognition) next = { ...next, recognition: updated };
    }
    if (i < changes.length) next = apply(next, changes[i++]);
  }
  return next;
}

export function useLiveTranscript(noteId: string): LiveView {
  // The state records its note. Before the effect of a new note resets it,
  // a render shows no state of the earlier note.
  const [held, setHeld] = useState<{ id: string; live: State }>({ id: noteId, live: none });
  const live = held.id === noteId ? held.live : none;

  useEffect(() => {
    const setLive = (change: (l: State) => State) =>
      setHeld((h) => ({ id: noteId, live: change(h.id === noteId ? h.live : none) }));
    setHeld({ id: noteId, live: none });
    let open = true;
    // Null after the catch-up: recognition events then apply once per frame.
    let waiting: Change[] | null = [];
    let pending: Change[] = [];
    let frame: number | null = null;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const flush = () => {
      if (frame !== null) cancelAnimationFrame(frame);
      if (timer !== null) clearTimeout(timer);
      frame = null;
      timer = null;
      if (!open || !pending.length) return;
      const queued = pending;
      pending = [];
      setLive((l) => applyChanges(l, queued));
    };
    const take = (change: Change) => {
      if (!open) return;
      if (waiting) waiting.push(change);
      else {
        pending.push(change);
        if (change.kind !== "recognition") flush();
        else if (frame === null) {
          frame = requestAnimationFrame(flush);
          // A hidden window can stop animation frames. Keep its queue bounded in time.
          timer = setTimeout(flush, 100);
        }
      }
    };
    const unlisteners = [
      events.liveRecognitionEvent.listen(({ payload }) => {
        if (payload.note_id === noteId) take({ kind: "recognition", data: payload });
      }),
      events.liveTranscriptEvent.listen(({ payload }) => {
        if (payload.note_id === noteId) take({ kind: "text", data: payload });
      }),
      events.livePartSaved.listen(({ payload }) => {
        if (payload.note_id === noteId) take({ kind: "saved", data: payload });
      }),
      events.liveTranscriptStatus.listen(({ payload }) => {
        if (payload.note_id === noteId) take({ kind: "status", data: payload });
      }),
    ];
    // Listen first, then read: no event falls between the two.
    void Promise.all(unlisteners).then(async () => {
      try {
        const result = await commands.liveTranscript(noteId);
        if (!open) return;
        const queued = waiting ?? [];
        waiting = null;
        setLive((l) => {
          const next = result.status === "ok" ? apply(l, { kind: "catchup", data: result.data }) : { ...l, loaded: true };
          return applyChanges(next, queued);
        });
      } catch {
        // The IPC call itself rejected (not a Result error). Leave the
        // starting line, but stop waiting: later events reach the view.
        if (!open) return;
        const queued = waiting ?? [];
        waiting = null;
        setLive((l) => {
          return applyChanges({ ...l, loaded: true }, queued);
        });
      }
    });
    return () => {
      open = false;
      if (frame !== null) cancelAnimationFrame(frame);
      if (timer !== null) clearTimeout(timer);
      pending = [];
      for (const u of unlisteners) void u.then((f) => f());
    };
  }, [noteId]);

  return useMemo(() => ({ ...live, numbers: liveNumbers(noteId) }), [live, noteId]);
}
