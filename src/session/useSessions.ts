// The session switcher's state. The store is the truth: the list is read on
// mount, on focus and on every lifecycle event, and runner events only
// refine it between reads (AE4). A view asks for its session again when
// that session's `version` moves.

import { useCallback, useEffect, useRef, useState } from "react";
import { commands, events, type SessionEntry } from "../bindings";

export type Live = { stage: string; segment: number; done: number; total: number };

export type Sessions = {
  entries: SessionEntry[];
  /// The stage each session last started, from events.
  stages: Record<string, string>;
  /// The latest progress event per session.
  live: Record<string, Live>;
  version: Record<string, number>;
  refresh: () => Promise<void>;
};

/// Progress arrives per window; views re-read the store at most this often.
const REREAD_MS = 1000;

export function useSessions(): Sessions {
  const [entries, setEntries] = useState<SessionEntry[]>([]);
  const [stages, setStages] = useState<Record<string, string>>({});
  const [live, setLive] = useState<Record<string, Live>>({});
  const [version, setVersion] = useState<Record<string, number>>({});
  const lastBump = useRef<Record<string, number>>({});

  const refresh = useCallback(async () => {
    const result = await commands.listSessions();
    if (result.status === "ok") setEntries(result.data);
  }, []);

  const bump = useCallback((id: string) => {
    lastBump.current[id] = performance.now();
    setVersion((v) => ({ ...v, [id]: (v[id] ?? 0) + 1 }));
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
        if (update.kind === "stage_started") {
          setStages((s) => ({ ...s, [id]: update.stage }));
          setLive((l) => {
            const next = { ...l };
            delete next[id];
            return next;
          });
        }
        bump(id);
        void refresh();
      }),
      events.recordingChanged.listen(() => void refresh()),
    ];
    return () => {
      window.removeEventListener("focus", refresh);
      for (const u of unlisteners) void u.then((f) => f());
    };
  }, [refresh, bump]);

  return { entries, stages, live, version, refresh };
}
