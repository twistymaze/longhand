// The scratchpad (R12, R12a, R13). A line's anchor is fixed at its first
// keystroke, by the backend on the capture clock; editing never moves it.
// Every change is written through, so a crash loses at most the keystroke in
// flight. Keyboard actions do not animate.

import { useCallback, useEffect, useRef, useState } from "react";
import { commands, events, type NoteView } from "../bindings";
import { formatPosition } from "../lib/timeline";

function Anchor({ note }: { note: NoteView }) {
  if (note.position_ms === null) {
    return <span className="w-[5.5rem] shrink-0 text-label-tertiary">—</span>;
  }
  return (
    <span
      className="w-[5.5rem] shrink-0 text-label-secondary tabular-nums"
      title={note.gap_anchored ? "Typed between segments: anchored to the end of the one before" : undefined}
    >
      {formatPosition(note.position_ms)}
      {note.gap_anchored && <span className="ml-1 text-label-tertiary">~gap</span>}
    </span>
  );
}

function Line({ note, onChange }: { note: NoteView; onChange: (text: string) => void }) {
  const [text, setText] = useState(note.text);
  return (
    <li className="flex items-baseline gap-2 px-3 py-1">
      <Anchor note={note} />
      <input
        aria-label={`Note at ${note.position_ms === null ? "no position" : formatPosition(note.position_ms)}`}
        className="min-w-0 flex-1 bg-transparent outline-none"
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          onChange(e.target.value);
        }}
      />
    </li>
  );
}

export function Scratchpad({ sessionId, recording }: { sessionId: string | null; recording: boolean }) {
  const [notes, setNotes] = useState<NoteView[]>([]);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  // The note being typed: created at its first keystroke. Keystrokes that
  // arrive before the backend answers wait on the same promise.
  const pending = useRef<Promise<number | null> | null>(null);
  const input = useRef<HTMLInputElement>(null);

  const load = useCallback(async () => {
    const result = await commands.scratchpadNotes();
    if (result.status === "ok") setNotes(result.data);
  }, []);

  useEffect(() => {
    void load();
  }, [load, sessionId]);

  useEffect(() => {
    const unlisten = events.recordingChanged.listen(() => void load());
    return () => void unlisten.then((f) => f());
  }, [load]);

  const write = (id: number, text: string) =>
    void commands.editNote(id, text).then((r) => setError(r.status === "error" ? r.error : null));

  const typeDraft = (text: string) => {
    setDraft(text);
    if (pending.current === null) {
      pending.current = commands.beginNote(text).then((r) => {
        if (r.status === "error") {
          setError(r.error);
          return null;
        }
        setError(null);
        return r.data.id;
      });
    } else {
      void pending.current.then((id) => id !== null && write(id, text));
    }
  };

  const commitDraft = async () => {
    if (pending.current === null) return;
    await pending.current;
    pending.current = null;
    setDraft("");
    await load();
  };

  return (
    <section aria-label="Scratchpad" className="flex min-h-0 flex-1 flex-col">
      <ol className="min-h-0 flex-1 overflow-y-auto py-1">
        {notes.length === 0 && (
          <li className="px-3 py-1 text-label-tertiary">
            {recording
              ? "Type a line and press Return. It is placed at the moment you started typing it."
              : "Lines typed now join the next session you record."}
          </li>
        )}
        {notes.map((note) => (
          <Line key={note.id} note={note} onChange={(text) => write(note.id, text)} />
        ))}
      </ol>
      <div className="flex items-center gap-2 border-t border-separator px-3 py-2">
        <input
          ref={input}
          autoFocus
          aria-label="New note"
          className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-label-tertiary"
          placeholder={recording ? "Note…" : "Note for the next session…"}
          value={draft}
          onChange={(e) => typeDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void commitDraft();
          }}
        />
      </div>
      {error && <p className="px-3 pb-2 text-[0.85rem] text-negative">{error}</p>}
    </section>
  );
}
