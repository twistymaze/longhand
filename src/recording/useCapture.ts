import { useCallback, useEffect, useRef, useState } from "react";
import { commands, events, type RecordingStatus } from "../bindings";
import { type InputStatus, inputStatus } from "./inputStatus";

/// 30 s of history at one peak per 250 ms.
export const HISTORY = 120;

export type Capture = {
  status: RecordingStatus | null;
  input: InputStatus;
  levels: number[];
  /// Recorded so far, gaps between segments excluded: the elapsed time shown.
  recordedMs: number;
  lostReason: string | null;
  refresh: () => Promise<void>;
};

export function useCapture(): Capture {
  const [status, setStatus] = useState<RecordingStatus | null>(null);
  const [levels, setLevels] = useState<number[]>([]);
  const [recordedMs, setRecordedMs] = useState(0);
  const [lostReason, setLostReason] = useState<string | null>(null);
  const [now, setNow] = useState(() => performance.now());
  const lastTickAt = useRef<number | null>(null);
  /// The note of the last tick. A recording change inside the open segment
  /// keeps it, so the ticked time stays.
  const tickedSession = useRef<string | null>(null);
  const startedAt = useRef<number | null>(null);

  const refresh = useCallback(async () => {
    const result = await commands.recordingStatus();
    if (result.status === "ok") {
      setStatus(result.data);
      // While paused, interrupted or not recording, no tick arrives, and
      // the status holds the whole recorded time. While a segment records,
      // the status does not hold that segment, so the ticks give the time.
      // Before the first tick of a note, the status holds the best known
      // time, not the time of the note before.
      const d = result.data;
      if (d.paused || d.interruption || !d.recording || tickedSession.current !== d.note_id) {
        setRecordedMs(d.recorded_ms);
      }
    }
  }, []);

  useEffect(() => {
    void refresh();
    // Disk headroom changes slowly; the status is re-read at this pace.
    const poll = window.setInterval(refresh, 5000);
    const unlisteners = [
      events.captureTick.listen(({ payload }) => {
        lastTickAt.current = performance.now();
        tickedSession.current = payload.note_id;
        setRecordedMs(payload.recorded_ms);
        setLevels((l) => {
          const next = l.length >= HISTORY ? l.slice(l.length - HISTORY + 1) : l.slice();
          next.push(payload.peak);
          return next;
        });
      }),
      events.recordingChanged.listen(({ payload }) => {
        lastTickAt.current = null;
        startedAt.current = payload.note_id ? performance.now() : null;
        setLostReason(null);
        if (payload.note_id) {
          setLevels([]);
        }
        void refresh();
      }),
      events.captureEnded.listen(({ payload }) => {
        setLostReason(payload.reason);
        void refresh();
      }),
    ];
    return () => {
      window.clearInterval(poll);
      for (const u of unlisteners) void u.then((f) => f());
    };
  }, [refresh]);

  const recording = status?.recording ?? false;
  const paused = status?.paused ?? false;

  // The stall check needs time to pass even when no event arrives. A window
  // opened mid-session starts its stall clock when it first sees the
  // recording, so an input already dead is still caught. A paused note has
  // no input to check.
  useEffect(() => {
    if (!recording || paused) return;
    if (startedAt.current === null) startedAt.current = performance.now();
    const timer = window.setInterval(() => setNow(performance.now()), 500);
    return () => window.clearInterval(timer);
  }, [recording, paused]);

  return {
    status,
    input: inputStatus({
      recording,
      paused,
      reconnecting: status?.reconnecting ?? false,
      lostReason,
      lastTickAt: lastTickAt.current,
      startedAt: startedAt.current,
      now,
      levels,
    }),
    levels,
    recordedMs,
    lostReason,
    refresh,
  };
}
