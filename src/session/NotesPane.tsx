// A session's notes document, editable in every state (R20). A plain text
// area until the markdown editor replaces it (U13). The whole document is
// sent 500 ms after the last keystroke, and at once when the pane closes,
// so a crash loses at most the last second of typing (R23).

import { useCallback, useEffect, useRef, useState } from "react";
import { commands } from "../bindings";

const SEND_DELAY_MS = 500;

export function NotesPane({ sessionId, autoFocus = false }: { sessionId: string; autoFocus?: boolean }) {
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const unsent = useRef<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Read once per session: re-reading on progress would reset the text
  // being typed, and nothing else edits these notes.
  useEffect(() => {
    void commands.sessionNotes(sessionId).then((r) => {
      if (r.status === "ok") setText(r.data);
      else setError(r.error);
    });
  }, [sessionId]);

  const send = useCallback(() => {
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = null;
    const markdown = unsent.current;
    if (markdown === null) return;
    unsent.current = null;
    void commands.setSessionNotes(sessionId, markdown).then((r) => setError(r.status === "error" ? r.error : null));
  }, [sessionId]);

  useEffect(() => send, [send]);

  return (
    <section aria-label="My notes" className="flex min-h-0 flex-1 flex-col">
      <h2 className="px-3 pt-3 pb-1 text-[0.85rem] font-semibold text-label-secondary">My notes</h2>
      {text !== null && (
        <textarea
          aria-label="My notes"
          autoFocus={autoFocus}
          className="min-h-0 flex-1 resize-none bg-transparent px-3 py-1 outline-none placeholder:text-label-tertiary"
          placeholder="Type your notes…"
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            unsent.current = e.target.value;
            if (timer.current !== null) clearTimeout(timer.current);
            timer.current = setTimeout(send, SEND_DELAY_MS);
          }}
          onBlur={send}
        />
      )}
      {error && <p className="px-3 pb-2 text-[0.85rem] text-negative">{error}</p>}
    </section>
  );
}
