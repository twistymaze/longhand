// The open session at the top of Home (R3, AE6): the elapsed time while it
// records, and a caution when the recording stopped by itself. Selecting it
// opens the session page. The time updates in place and nothing animates:
// Home must not pull the eye while the room talks.

import { formatPosition } from "../lib/timeline";
import { ChevronIcon, WaveformIcon } from "../settings/icons";

export type RecordingNow = { sessionId: string; elapsedMs: number; interrupted: boolean };

export function RecordingRow({ recording, onOpen }: { recording: RecordingNow; onOpen: (id: string) => void }) {
  const tint = recording.interrupted ? "text-caution bg-caution/12" : "text-negative bg-negative/12";
  return (
    <button
      type="button"
      onClick={() => onOpen(recording.sessionId)}
      className="flex w-full items-center gap-3 rounded-[12px] bg-group px-3 py-2.5 text-left shadow-[0_0_0_0.5px_var(--separator),0_1px_3px_rgb(0_0_0/0.06)] hover:bg-row-hover active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent"
    >
      <span aria-hidden className={`flex size-8 shrink-0 items-center justify-center rounded-[8px] ${tint}`}>
        <WaveformIcon className="size-[17px]" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-medium">
          {recording.interrupted ? "Recording interrupted" : "Recording"}
        </span>
        <span className="block truncate text-[0.8rem] text-label-secondary">
          {recording.interrupted ? "Open to resume or stop" : "In progress"}
        </span>
      </span>
      <span className="shrink-0 font-medium tabular-nums tracking-tight">{formatPosition(recording.elapsedMs)}</span>
      <ChevronIcon className="size-3.5 text-label-tertiary" />
    </button>
  );
}
