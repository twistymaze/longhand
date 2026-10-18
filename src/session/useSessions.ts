// The session list's state. The store is the truth: the list is read on
// mount, on focus, on every lifecycle event and on every library change,
// and runner events only refine it between reads (AE4). A view asks for its
// session again when that session's `version` moves.

import { useCallback, useEffect, useRef, useState } from "react";
import { commands, events, type SessionEntry, type SpeakerProgressView } from "../bindings";

/// `layer` is the summary layer of a summarize call: 0 for the parts of the
/// transcript, 1 or more for the calls that combine summaries.
export type Live = { stage: string; segment: number; layer: number; done: number; total: number };

export type Sessions = {
  entries: SessionEntry[];
  /// The stage each processing session last started, from events. It
  /// goes when the session waits or is not processing, so an old run's
  /// stage does not show for a later run.
  stages: Record<string, string>;
  /// The number of stage starts per session since the page opened. A view
  /// compares it with its value when a run started, to know that a stage
  /// of that run started (KTD10).
  starts: Record<string, number>;
  /// The latest progress event per session.
  live: Record<string, Live>;
  /// The latest speaker progress event per session.
  speakers: Record<string, SpeakerProgressView>;
  version: Record<string, number>;
  /// Moves when a library change names the session: a sync changed its
  /// folder (KTD10). The notes tab reads the notes again on it.
  synced: Record<string, number>;
  /// The sessions whose folder a sync removed while Longhand was open.
  removed: Record<string, true>;
  refresh: () => Promise<void>;
};

/// Progress arrives per window; views re-read the store at most this often.
const REREAD_MS = 1000;

const increment = (ids: string[]) => (v: Record<string, number>) => {
  const next = { ...v };
  for (const id of ids) next[id] = (next[id] ?? 0) + 1;
  return next;
};

export function useSessions(): Sessions & { loaded: boolean; loadError: string | null; listRevision: number; getListRevision: () => number } {
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [listRevision, setListRevision] = useState(0);
  const acceptedRevision = useRef(0);
  const getListRevision = useCallback(() => acceptedRevision.current, []);
  const request = useRef(0);
  const [entries, setEntries] = useState<SessionEntry[]>([]);
  const [stages, setStages] = useState<Record<string, string>>({});
  const [starts, setStarts] = useState<Record<string, number>>({});
  const [live, setLive] = useState<Record<string, Live>>({});
  const [speakers, setSpeakers] = useState<Record<string, SpeakerProgressView>>({});
  const [version, setVersion] = useState<Record<string, number>>({});
  const [synced, setSynced] = useState<Record<string, number>>({});
  const [removed, setRemoved] = useState<Record<string, true>>({});
  const lastBump = useRef<Record<string, number>>({});

  const refresh = useCallback(async () => {
    const current = ++request.current;
    try {
      const result = await commands.listSessions();
      if (current !== request.current) return;
      if (result.status === "ok") {
        acceptedRevision.current = current;
        setEntries(result.data);
        // A stage of a run that ended does not show for a later run.
        const notProcessing = new Set(result.data.filter((e) => e.state !== "processing").map((e) => e.id));
        setStages((s) => {
          const gone = Object.keys(s).filter((id) => notProcessing.has(id));
          if (gone.length === 0) return s;
          const next = { ...s };
          for (const id of gone) delete next[id];
          return next;
        });
        setListRevision(current);
        setLoaded(true);
        setLoadError(null);
      } else {
        setLoadError("Could not load notes.");
      }
    } catch {
      if (current === request.current) setLoadError("Could not load notes.");
    }
  }, []);

  const bump = useCallback((id: string) => {
    lastBump.current[id] = performance.now();
    setVersion(increment([id]));
  }, []);

  useEffect(() => {
    void refresh();
    window.addEventListener("focus", refresh);
    const unlisteners = [
      events.processingUpdate.listen(({ payload }) => {
        const { session_id: id, update } = payload;
        if (update.kind === "progress") {
          setLive((l) => ({ ...l, [id]: update }));
          if (performance.now() - (lastBump.current[id] ?? 0) >= REREAD_MS) bump(id);
          return;
        }
        if (update.kind === "speaker_progress") {
          const { kind: _, ...progress } = update;
          setSpeakers((s) => ({ ...s, [id]: progress }));
          if (performance.now() - (lastBump.current[id] ?? 0) >= REREAD_MS) bump(id);
          return;
        }
        const forget = <T,>(l: Record<string, T>) => {
          const next = { ...l };
          delete next[id];
          return next;
        };
        // Any other update ends the speaker attempt, so its events no longer count.
        setSpeakers(forget);
        if (update.kind === "stage_started") {
          setStages((s) => ({ ...s, [id]: update.stage }));
          setStarts(increment([id]));
          setLive(forget);
        }
        // A note that waits runs no stage until the next start (KTD10).
        if (update.kind === "waits" && update.wait) setStages(forget);
        bump(id);
        void refresh();
      }),
      events.recordingChanged.listen(() => void refresh()),
      events.libraryChanged.listen(({ payload }) => {
        const { changed, removed: gone } = payload;
        for (const id of changed) lastBump.current[id] = performance.now();
        setVersion(increment(changed));
        setSynced(increment(changed));
        setRemoved((r) => {
          const next = { ...r };
          // A note that comes back is listed again.
          for (const id of changed) delete next[id];
          for (const id of gone) next[id] = true;
          return next;
        });
        void refresh();
      }),
    ];
    return () => {
      window.removeEventListener("focus", refresh);
      for (const u of unlisteners) void u.then((f) => f());
    };
  }, [refresh, bump]);

  return { entries, loaded, loadError, listRevision, getListRevision, stages, starts, live, speakers, version, synced, removed, refresh };
}
