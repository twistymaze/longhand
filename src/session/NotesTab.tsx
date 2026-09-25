// My notes (R20–R23): the session's notes document in the markdown editor,
// editable in every state. The editor sends the whole document as the user
// types; each document goes to the store, and on a saved session to the
// folder's notes.md.

import { useEffect, useRef, useState } from "react";
import { commands } from "../bindings";
import { NoteEditor } from "../editor/NoteEditor";

export function NotesTab({
  sessionId,
  autoFocus,
  onMarkdown,
}: {
  sessionId: string;
  autoFocus: boolean;
  /// The document as read, and after each change the editor sends.
  onMarkdown?: (markdown: string) => void;
}) {
  const [markdown, setMarkdown] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Read once per session: the editor holds the document from then on.
  useEffect(() => {
    let current = true;
    void commands.sessionNotes(sessionId).then((r) => {
      if (!current) return;
      if (r.status === "ok") {
        setMarkdown(r.data);
        onMarkdown?.(r.data);
      } else setError(r.error);
    });
    return () => {
      current = false;
    };
    // `onMarkdown` is read when the notes arrive, not watched.
  }, [sessionId]);

  // One save at a time, so that an older document never replaces a newer
  // one. While a save runs, only the latest document waits. Each document
  // keeps the session id it was typed for, and a document that waits when
  // the tab closes is still sent.
  const saving = useRef<{ busy: boolean; next: { id: string; markdown: string } | null }>({
    busy: false,
    next: null,
  });
  const save = (id: string, markdown: string) => {
    const s = saving.current;
    if (s.busy) {
      s.next = { id, markdown };
      return;
    }
    s.busy = true;
    void commands.setSessionNotes(id, markdown).then((r) => {
      setError(r.status === "error" ? r.error : null);
      s.busy = false;
      const next = s.next;
      s.next = null;
      if (next) save(next.id, next.markdown);
    });
  };

  const send = (next: string) => {
    onMarkdown?.(next);
    save(sessionId, next);
  };

  return (
    <div className="mx-auto max-w-[44rem] px-8 pt-2 pb-10 [&_.prosemirror-editor]:min-h-[50vh]">
      {error && <p className="mb-2 text-[0.85rem] text-negative">{error}</p>}
      {markdown !== null && <NoteEditor key={sessionId} markdown={markdown} onChange={send} autoFocus={autoFocus} />}
    </div>
  );
}
