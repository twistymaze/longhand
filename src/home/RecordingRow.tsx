// A recording, paused or interrupted note in the library. The elapsed time
// updates without animation to keep the recording surface quiet.

import { formatPosition } from "../lib/timeline";
import { ChevronIcon, WaveformIcon } from "../settings/icons";
import { NoteLabels } from "./NoteLabels";

export type RecordingNow = {
  noteId: string;
  elapsedMs: number;
  interrupted: boolean;
  /// The user paused the note (R9). Its time stops.
  paused?: boolean;
  noteType?: string | null;
  tags?: string[];
};

/// The icon tint, title and subtitle of each state: red while recording,
/// the caution colour while interrupted, and neutral grey while paused.
function look(recording: RecordingNow): { tint: string; title: string; detail: string } {
  if (recording.interrupted) {
    return { tint: "text-caution bg-caution/12", title: "Recording interrupted", detail: "Open to resume or stop" };
  }
  if (recording.paused) {
    return { tint: "text-label-secondary bg-fill", title: "Paused", detail: "Open to continue or stop" };
  }
  return { tint: "text-negative bg-negative/12", title: "Recording", detail: "In progress" };
}

export function RecordingRow({ recording, selected, onSelect, onOpen }: { recording: RecordingNow; selected: boolean; onSelect: () => void; onOpen: (id: string) => void }) {
  const { tint, title, detail } = look(recording);
  return (
    <button
      type="button"
      role="option"
      aria-selected={selected}
      tabIndex={selected ? 0 : -1}
      data-session={recording.noteId}
      onClick={(e) => { e.currentTarget.focus(); onSelect(); }}
      onDoubleClick={() => onOpen(recording.noteId)}
      onKeyDown={(e) => {
        if (e.key === "Enter") { e.preventDefault(); onOpen(recording.noteId); }
      }}
      className={`${selected ? "bg-accent/15 ring-1 ring-accent" : "bg-group"} flex w-full items-center gap-3 rounded-[12px] px-3 py-2.5 text-left shadow-[0_0_0_0.5px_var(--separator),0_1px_3px_rgb(0_0_0/0.06)] hover:bg-row-hover active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent`}
    >
      <span aria-hidden className={`flex size-8 shrink-0 items-center justify-center rounded-[8px] ${tint}`}>
        <WaveformIcon className="size-[17px]" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-medium">{title}</span>
        {/* The type and tag labels follow the details (R22). */}
        <span className="flex min-w-0 items-center gap-2">
          <span className="min-w-0 shrink-0 truncate text-[0.8rem] text-label-secondary">{detail}</span>
          <NoteLabels noteType={recording.noteType ?? null} tags={recording.tags ?? []} />
        </span>
      </span>
      <span className="shrink-0 font-medium tabular-nums tracking-tight">{formatPosition(recording.elapsedMs)}</span>
      <ChevronIcon className="size-3.5 text-label-tertiary" />
    </button>
  );
}
