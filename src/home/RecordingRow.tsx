// The open session at the top of Home (R3, AE6): the elapsed time while it
// records, and a caution when the recording stopped by itself. Selecting it
// opens the session page. The time updates in place and nothing animates:
// Home must not pull the eye while the room talks.

import { formatPosition } from "../lib/timeline";

export type RecordingNow = { sessionId: string; elapsedMs: number; interrupted: boolean };

export function RecordingRow({ recording, onOpen }: { recording: RecordingNow; onOpen: (id: string) => void }) {
  return (
    <button
      type="button"
      onClick={() => onOpen(recording.sessionId)}
      className="flex w-full items-center gap-2.5 rounded-[10px] bg-group px-3 py-2 text-left shadow-[0_0_0_0.5px_var(--separator)] active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent"
    >
      <span
        aria-hidden
        className={`size-2.5 shrink-0 rounded-full ${recording.interrupted ? "bg-caution" : "bg-negative"}`}
      />
      <span className="min-w-0 flex-1 truncate font-medium">
        {recording.interrupted ? "Recording interrupted" : "Recording"}
      </span>
      <span className="shrink-0 font-medium tabular-nums tracking-tight">{formatPosition(recording.elapsedMs)}</span>
      <span aria-hidden className="shrink-0 text-label-tertiary">
        ›
      </span>
    </button>
  );
}
