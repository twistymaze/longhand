// A session's notes beside its processing or its documents, editable in
// every state (R11a). An anchored note's position is a link into the
// transcript once there is one (R12); an un-anchored note has none.

import { useCallback, useEffect, useState } from "react";
import { commands, type NoteView } from "../bindings";
import { formatPosition } from "../lib/timeline";

function Anchor({ note, onFollow }: { note: NoteView; onFollow?: (note: NoteView) => void }) {
  const base = "w-[5.25rem] shrink-0 text-left tabular-nums";
  if (note.position_ms === null) {
    return (
      <span className={`${base} text-label-tertiary`} title="Typed outside a recording: no position">
        —
      </span>
    );
  }
  // "~" marks a note typed between segments, placed at the earlier one's end.
  const text = (
    <>
      {note.gap_anchored && <span className="text-label-tertiary">~</span>}
      {formatPosition(note.position_ms)}
    </>
  );
  if (!onFollow) return <span className={`${base} text-label-secondary`}>{text}</span>;
  return (
    <button
      type="button"
      onClick={() => onFollow(note)}
      aria-label={`Show ${formatPosition(note.position_ms)} in the transcript`}
      title={note.gap_anchored ? "Typed between segments: shows the end of the one before" : "Show in the transcript"}
      className={`${base} rounded-sm text-accent active:opacity-60 outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent`}
    >
      {text}
    </button>
  );
}

function Line({
  note,
  onFollow,
  onError,
}: {
  note: NoteView;
  onFollow?: (note: NoteView) => void;
  onError: (message: string | null) => void;
}) {
  const [text, setText] = useState(note.text);
  return (
    <li className="flex items-baseline gap-1.5 px-3 py-1">
      <Anchor note={note} onFollow={onFollow} />
      <textarea
        aria-label="Note"
        rows={1}
        className="field-sizing-content min-w-0 flex-1 resize-none bg-transparent outline-none"
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          // Written through, so a landed folder's notes.md follows (R22f).
          void commands
            .editNote(note.id, e.target.value)
            .then((r) => onError(r.status === "error" ? r.error : null));
        }}
      />
    </li>
  );
}

export function NotesPane({
  sessionId,
  onFollow,
}: {
  sessionId: string;
  onFollow?: (note: NoteView) => void;
}) {
  const [notes, setNotes] = useState<NoteView[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    const result = await commands.sessionNotes(sessionId);
    if (result.status === "ok") setNotes(result.data);
    else setError(result.error);
  }, [sessionId]);

  // Read once per session: re-reading on progress would reset a line
  // being typed, and nothing else edits these notes.
  useEffect(() => {
    void load();
  }, [load]);

  return (
    <section aria-label="Notes" className="flex min-h-0 flex-col">
      <h2 className="px-3 pt-3 pb-1 text-[0.85rem] font-semibold text-label-secondary">Notes</h2>
      <ol className="min-h-0 flex-1 overflow-y-auto pb-2">
        {notes?.length === 0 && <li className="px-3 py-1 text-label-tertiary">No notes in this session.</li>}
        {notes?.map((note) => (
          <Line key={note.id} note={note} onFollow={onFollow} onError={setError} />
        ))}
      </ol>
      {error && <p className="px-3 pb-2 text-[0.85rem] text-negative">{error}</p>}
    </section>
  );
}
