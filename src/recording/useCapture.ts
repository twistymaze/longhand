import { useCallback, useEffect, useRef, useState } from "react";
import { type AlertCheck, commands, events, type RecordingStatus } from "../bindings";
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
  /// Whether an alert would reach the user: checked as recording starts and
  /// kept current while it lasts (R5a).
  alert: AlertCheck | null;
  refresh: () => Promise<void>;
};

export function useCapture(): Capture {
  const [status, setStatus] = useState<RecordingStatus | null>(null);
  const [levels, setLevels] = useState<number[]>([]);
  const [recordedMs, setRecordedMs] = useState(0);
  const [lostReason, setLostReason] = useState<string | null>(null);
  const [alert, setAlert] = useState<AlertCheck | null>(null);
  const [now, setNow] = useState(() => performance.now());
  const lastTickAt = useRef<number | null>(null);
  const startedAt = useRef<number | null>(null);

  const checkAlert = useCallback(async (force: boolean) => {
    const result = await commands.alertCheck(force);
    if (result.status === "ok") setAlert(result.data);
  }, []);

  const refresh = useCallback(async () => {
    const result = await commands.recordingStatus();
    if (result.status === "ok") {
      setStatus(result.data);
      // Re-probed only when the default output has changed.
      if (result.data.recording) void checkAlert(false);
    }
  }, [checkAlert]);

  useEffect(() => {
    void refresh();
    // Disk headroom changes slowly; the status is re-read at this pace.
    const poll = window.setInterval(refresh, 5000);
    const unlisteners = [
      events.captureTick.listen(({ payload }) => {
        lastTickAt.current = performance.now();
        setRecordedMs(payload.recorded_ms);
        setLevels((l) => {
          const next = l.length >= HISTORY ? l.slice(l.length - HISTORY + 1) : l.slice();
          next.push(payload.peak);
          return next;
        });
      }),
      events.recordingChanged.listen(({ payload }) => {
        lastTickAt.current = null;
        startedAt.current = payload.session_id ? performance.now() : null;
        setLostReason(null);
        if (payload.session_id) {
          setLevels([]);
          void checkAlert(true);
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
  }, [refresh, checkAlert]);

  const recording = status?.recording ?? false;

  // The stall check needs time to pass even when no event arrives. A window
  // opened mid-session starts its stall clock when it first sees the
  // recording, so an input already dead is still caught.
  useEffect(() => {
    if (!recording) return;
    if (startedAt.current === null) startedAt.current = performance.now();
    const timer = window.setInterval(() => setNow(performance.now()), 500);
    return () => window.clearInterval(timer);
  }, [recording]);

  return {
    status,
    input: inputStatus({
      recording,
      reconnecting: status?.reconnecting ?? false,
      lostReason,
      lastTickAt: lastTickAt.current,
      startedAt: startedAt.current,
      now,
    }),
    levels,
    recordedMs,
    lostReason,
    alert,
    refresh,
  };
}
