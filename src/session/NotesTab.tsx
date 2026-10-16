// My Notes (R20–R23): the session's notes document in the markdown editor,
// editable in every state. The editor sends the whole document as the user
// types; each document goes to the store, and on a saved session to the
// folder's notes.md.
//
// Each save sends the fingerprint of the notes.md that the editor loaded
// (KTD10). When another computer changed the file since then, the edit goes
// into a draft file beside it, and the editor loads the file again as a
// sync does. Edits after the conflict wait, and the latest one goes into one
// more draft before that load, so a user who types on makes no more drafts.
// A sync
// that changes the notes reloads them only when no save waits and the user
// has not typed for 2 seconds, so a sync never replaces a local edit.

import { useCallback, useEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { commands } from "../bindings";
import { NoteEditor, type NoteEditorHandle } from "../editor/NoteEditor";
import { addNotesSend, trackNotesSave } from "../lib/notesSaves";
import { folderName } from "../settings/ui";

/// A sync reloads the notes only after this long with no input.
export const RELOAD_AFTER_INPUT_MS = 2000;
export function NotesTab({
  sessionId,
  autoFocus,
  editable = true,
  synced = 0,
  onMarkdown,
  onConflict,
  onNewNote,
}: {
  sessionId: string;
  autoFocus: boolean;
  /// False while the notes wait for the sync or the storage folder is
  /// unavailable: the editor then takes no input.
  editable?: boolean;
  /// Moves when a sync changed the note's folder.
  synced?: number;
  /// The document as read, and after each change the editor sends.
  onMarkdown?: (markdown: string) => void;
  /// A save found that another computer changed notes.md. `draft` is the
  /// name of the file that holds the edit.
  onConflict?: (draft: string) => void;
  /// Another computer removed this note, and the save put the text into
  /// the new note `id` (R24). Later saves go to that note.
  onNewNote?: (id: string) => void;
}) {
  // `generation` gives the editor a new key for each document that is read
  // again, since the editor reads its document at mount only.
  const [loaded, setLoaded] = useState<{ markdown: string; generation: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const fingerprint = useRef<string | null>(null);
  const lastInput = useRef(0);
  const onMarkdownRef = useRef(onMarkdown);
  onMarkdownRef.current = onMarkdown;
  const onConflictRef = useRef(onConflict);
  onConflictRef.current = onConflict;
  const onNewNoteRef = useRef(onNewNote);
  onNewNoteRef.current = onNewNote;
  // The note that holds the text of each note that another computer
  // removed (R24).
  const moved = useRef(new Map<string, string>());

  // One save at a time, so that an older document never replaces a newer
  // one. While a save runs, only the latest document waits. Each document
  // keeps the session id it was typed for, and a document that waits when
  // the tab closes is still sent.
  const saving = useRef<{
    busy: boolean;
    next: { id: string; markdown: string; loaded?: string | null } | null;
  }>({
    busy: false,
    next: null,
  });
  // A sync changed the notes while a save waited or the user typed.
  const stale = useRef(false);
  const retry = useRef<ReturnType<typeof setTimeout>>(undefined);
  const current = useRef(sessionId);
  current.current = sessionId;
  const reload = useRef<() => void>(() => {});

  // After a save conflicts, later saves of the same session wait here, since
  // each one would fail the same check and write another draft. The latest
  // one goes into one more draft before the editor reads the file again,
  // or when the tab closes.
  const held = useRef<{ id: string; fingerprint: string | null; markdown: string | null } | null>(null);
  const open = useRef(true);

  // Each read has a number. The tab ignores a response from an older read,
  // so a late response never replaces a newer one. A forced read that a
  // newer read replaces gives its force to that read.
  const readSeq = useRef(0);
  const forceRead = useRef(false);
  const editor = useRef<NoteEditorHandle>(null);
  const read = useCallback(
    async (id: string, forceThis: boolean) => {
      const seq = ++readSeq.current;
      if (forceThis) forceRead.current = true;
      const r = await commands.loadSessionNotes(id);
      if (current.current !== id || seq !== readSeq.current) return;
      const force = forceRead.current;
      forceRead.current = false;
      if (r.status === "error") {
        setError(r.error);
        return;
      }
      // A change that the editor still holds was made on the document that
      // it shows: send it now, with that document's fingerprint, before the
      // read can replace the document. The save then waits, and so does
      // the read.
      if (!force) editor.current?.flush();
      const s = saving.current;
      // An edit that waits after a conflict goes into a draft before the
      // read can replace the document.
      const waits = held.current?.id === id && held.current.markdown !== null;
      const idle =
        !s.busy && !s.next && !waits && performance.now() - lastInput.current >= RELOAD_AFTER_INPUT_MS;
      if (!force && !idle) {
        // The user typed while the notes were read: try again later.
        reload.current();
        return;
      }
      if (!force && r.data.fingerprint === fingerprint.current) {
        // The notes did not change, so the editor shows them. No edit waits
        // (see `waits`), and later edits go out again on the normal path.
        if (held.current?.id === id) held.current = null;
        return;
      }
      fingerprint.current = r.data.fingerprint;
      if (held.current?.id === id) held.current = null;
      // The new editor replaces the old one in this same task, right after
      // the flush above, so no change can reach the old editor between the
      // two. Its unmount then has no change to send with the new
      // fingerprint.
      flushSync(() => {
        setLoaded((l) => ({ markdown: r.data.markdown, generation: (l?.generation ?? 0) + 1 }));
      });
      onMarkdownRef.current?.(r.data.markdown);
    },
    [],
  );

  // Read the notes again after a sync, when it cannot replace an edit.
  // Otherwise try again when the edit is sent.
  const reloadWhenIdle = useCallback(() => {
    clearTimeout(retry.current);
    const s = saving.current;
    if (s.busy || s.next) {
      stale.current = true;
      return;
    }
    const wait = lastInput.current + RELOAD_AFTER_INPUT_MS - performance.now();
    if (wait > 0) {
      stale.current = true;
      retry.current = setTimeout(reloadWhenIdle, wait);
      return;
    }
    // An edit that waits after a conflict goes into a draft first.
    if (sendHeld()) return;
    stale.current = false;
    void read(current.current, false);
  }, [read]);
  reload.current = reloadWhenIdle;

  useEffect(() => {
    fingerprint.current = null;
    setLoaded(null);
    void read(sessionId, true);
    return () => {
      clearTimeout(retry.current);
      sendHeld();
      held.current = null;
    };
  }, [sessionId, read]);

  useEffect(() => {
    open.current = true;
    return () => {
      open.current = false;
      sendHeld();
    };
  }, []);

  // A leave of the note sends the editor's pending change, then the edit
  // that waits after a conflict, so that the leave waits for its save.
  const sendHeldRef = useRef(sendHeld);
  sendHeldRef.current = sendHeld;
  useEffect(
    () =>
      addNotesSend(() => {
        editor.current?.flush();
        sendHeldRef.current();
      }),
    [],
  );

  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    reloadWhenIdle();
  }, [synced, reloadWhenIdle]);

  // Send the edit that waits after a conflict, with the fingerprint that the
  // conflicting save sent. Return true when an edit was sent.
  function sendHeld() {
    const h = held.current;
    if (!h || h.markdown === null) return false;
    held.current = null;
    save(h.id, h.markdown, h.fingerprint);
    return true;
  }

  // `loaded` is the fingerprint to send; without it, the save sends the
  // fingerprint of the notes that the editor holds.
  function save(id: string, text: string, loaded?: string | null) {
    const h = held.current;
    if (loaded === undefined && h?.id === id && id === current.current && open.current) {
      h.markdown = text;
      return;
    }
    const s = saving.current;
    if (s.busy) {
      s.next = { id, markdown: text, loaded };
      return;
    }
    const target = moved.current.get(id) ?? id;
    s.busy = true;
    const loadedFingerprint = loaded !== undefined ? loaded : id === current.current ? fingerprint.current : null;
    // A leave of the note waits for this save and the saves after it.
    const saved = commands.saveSessionNotes(target, text, loadedFingerprint).then((r) => {
      s.busy = false;
      let next = s.next;
      s.next = null;
      if (r.status === "ok" && r.data.kind === "new_note") moved.current.set(id, r.data.session_id);
      if (id === current.current) {
        setError(r.status === "error" ? r.error : null);
        if (r.status === "ok" && r.data.kind === "saved") fingerprint.current = r.data.fingerprint;
        if (r.status === "ok" && r.data.kind === "new_note") {
          fingerprint.current = r.data.fingerprint;
          onNewNoteRef.current?.(r.data.session_id);
        }
        if (r.status === "ok" && r.data.kind === "conflict") {
          // The file keeps the other computer's text: the editor shows it
          // when the edit stops. Until then, later edits wait, and the
          // latest one goes into one more draft.
          onConflictRef.current?.(folderName(r.data.draft));
          stale.current = true;
          held.current = { id, fingerprint: loadedFingerprint, markdown: null };
          if (next?.id === id && open.current) {
            held.current.markdown = next.markdown;
            next = null;
          }
        }
      }
      if (next) save(next.id, next.markdown, next.loaded);
      else if (stale.current) reloadWhenIdle();
    });
    trackNotesSave(id, saved);
  }

  const send = (next: string) => {
    onMarkdown?.(next);
    save(sessionId, next);
  };

  return (
    <div
      className="note-column pt-5 pb-12 [&_.prosemirror-editor]:min-h-[50vh]"
      onKeyDownCapture={() => (lastInput.current = performance.now())}
      onInputCapture={() => (lastInput.current = performance.now())}
    >
      {error && <p className="mb-2 text-[0.85rem] text-negative">{error}</p>}
      {loaded !== null && (
        <NoteEditor
          ref={editor}
          key={`${sessionId}:${loaded.generation}`}
          markdown={loaded.markdown}
          onChange={send}
          autoFocus={autoFocus && loaded.generation === 1}
          editable={editable}
        />
      )}
    </div>
  );
}
