// The live transcript of one note (KTD10). The hook reads the catch-up when
// the view opens, then applies the live events for the note's id. Events
// that arrive before the catch-up returns wait, and apply after it.

import { useEffect, useState } from "react";
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
} from "../bindings";

export type LiveView = {
  /// The saved parts, in order of segment and part.
  parts: LivePartText[];
  /// The text of the open part, or null.
  current: LiveCurrent | null;
  /// What the live worker does now. Null when no worker reported.
  state: LiveState | null;
  /// The catch-up returned.
  loaded: boolean;
};

const none: LiveView = { parts: [], current: null, state: null, loaded: false };

type Change =
  | { kind: "catchup"; data: LiveTranscript }
  | { kind: "text"; data: LiveTranscriptEvent }
  | { kind: "saved"; data: LivePartSaved }
  | { kind: "status"; data: LiveTranscriptStatus };

const same = (a: { segment: number; part: number }, b: { segment: number; part: number }) =>
  a.segment === b.segment && a.part === b.part;

const order = (a: LivePartText, b: LivePartText) => a.segment - b.segment || a.part - b.part;

/// Add a saved part, or replace the part with the same number.
function withPart(parts: LivePartText[], part: LivePartText): LivePartText[] {
  return [...parts.filter((p) => !same(p, part)), part].sort(order);
}

export function apply(live: LiveView, change: Change): LiveView {
  switch (change.kind) {
    case "catchup": {
      let parts = live.parts;
      for (const p of change.data.parts) parts = withPart(parts, p);
      const current = change.data.current;
      return {
        parts,
        current: current && !parts.some((p) => same(p, current)) ? current : null,
        state: change.data.state,
        loaded: true,
      };
    }
    case "text": {
      const { segment, part, committed, tentative } = change.data;
      // A late event of a part that is already saved changes nothing.
      if (live.parts.some((p) => same(p, change.data))) return live;
      return { ...live, current: { segment, part, committed, tentative } };
    }
    case "saved": {
      const { segment, part, start_ms, end_ms, text } = change.data;
      const saved = { segment, part, start_ms, end_ms, text };
      return {
        ...live,
        parts: withPart(live.parts, saved),
        current: live.current && same(live.current, saved) ? null : live.current,
      };
    }
    case "status":
      return { ...live, state: change.data.state };
  }
}

export function useLiveTranscript(sessionId: string): LiveView {
  const [live, setLive] = useState<LiveView>(none);

  useEffect(() => {
    setLive(none);
    let open = true;
    // Null after the catch-up: events then apply at once.
    let waiting: Change[] | null = [];
    const take = (change: Change) => {
      if (!open) return;
      if (waiting) waiting.push(change);
      else setLive((l) => apply(l, change));
    };
    const unlisteners = [
      events.liveTranscriptEvent.listen(({ payload }) => {
        if (payload.session_id === sessionId) take({ kind: "text", data: payload });
      }),
      events.livePartSaved.listen(({ payload }) => {
        if (payload.session_id === sessionId) take({ kind: "saved", data: payload });
      }),
      events.liveTranscriptStatus.listen(({ payload }) => {
        if (payload.session_id === sessionId) take({ kind: "status", data: payload });
      }),
    ];
    // Listen first, then read: no event falls between the two.
    void Promise.all(unlisteners).then(async () => {
      const result = await commands.liveTranscript(sessionId);
      if (!open) return;
      const queued = waiting ?? [];
      waiting = null;
      setLive((l) => {
        let next = result.status === "ok" ? apply(l, { kind: "catchup", data: result.data }) : { ...l, loaded: true };
        for (const change of queued) next = apply(next, change);
        return next;
      });
    });
    return () => {
      open = false;
      for (const u of unlisteners) void u.then((f) => f());
    };
  }, [sessionId]);

  return live;
}
