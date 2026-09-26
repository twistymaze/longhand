// My Notes (R20–R23): the session's notes document in the markdown editor,
// editable in every state. The editor sends the whole document as the user
// types; each document goes to the store, and on a saved session to the
// folder's notes.md.
//
// Each save sends the fingerprint of the notes.md that the editor loaded
// (KTD10). When another computer changed the file since then, the edit goes
// into a draft file beside it, and the editor loads the file again as a
// sync does. A sync
// that changes the notes reloads them only when no save waits and the user
// has not typed for 2 seconds, so a sync never replaces a local edit.

import { useCallback, useEffect, useRef, useState } from "react";
import { commands } from "../bindings";
import { NoteEditor } from "../editor/NoteEditor";
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
}) {
  // `generation` gives the editor a new key for each document that is read
  // again, since the editor reads its document at mount only.
  const [loaded, setLoaded] = useState<{ markdown: string; generation: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const fingerprint = useRef<string | null>(null);
  const lastInput = useRef(0);
  const onMarkdownRef = useRef(onMarkdown);
  onMarkdownRef.current = onMarkdown;

  // One save at a time, so that an older document never replaces a newer
  // one. While a save runs, only the latest document waits. Each document
  // keeps the session id it was typed for, and a document that waits when
  // the tab closes is still sent.
  const saving = useRef<{ busy: boolean; next: { id: string; markdown: string } | null }>({
    busy: false,
    next: null,
  });
  // A sync changed the notes while a save waited or the user typed.
  const stale = useRef(false);
  const retry = useRef<ReturnType<typeof setTimeout>>(undefined);
  const current = useRef(sessionId);
  current.current = sessionId;
  const reload = useRef<() => void>(() => {});

  const read = useCallback(
    async (id: string, force: boolean) => {
      const r = await commands.loadSessionNotes(id);
      if (current.current !== id) return;
      if (r.status === "error") {
        setError(r.error);
        return;
      }
      const s = saving.current;
      const idle = !s.busy && !s.next && performance.now() - lastInput.current >= RELOAD_AFTER_INPUT_MS;
      if (!force && !idle) {
        // The user typed while the notes were read: try again later.
        reload.current();
        return;
      }
      if (!force && r.data.fingerprint === fingerprint.current) return;
      fingerprint.current = r.data.fingerprint;
      setLoaded((l) => ({ markdown: r.data.markdown, generation: (l?.generation ?? 0) + 1 }));
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
    stale.current = false;
    void read(current.current, false);
  }, [read]);
  reload.current = reloadWhenIdle;

  useEffect(() => {
    fingerprint.current = null;
    setLoaded(null);
    void read(sessionId, true);
    return () => clearTimeout(retry.current);
  }, [sessionId, read]);

  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    reloadWhenIdle();
  }, [synced, reloadWhenIdle]);

  const save = (id: string, markdown: string) => {
    const s = saving.current;
    if (s.busy) {
      s.next = { id, markdown };
      return;
    }
    s.busy = true;
    const loadedFingerprint = id === current.current ? fingerprint.current : null;
    void commands.saveSessionNotes(id, markdown, loadedFingerprint).then((r) => {
      s.busy = false;
      const next = s.next;
      s.next = null;
      if (id === current.current) {
        setError(r.status === "error" ? r.error : null);
        if (r.status === "ok" && r.data.kind === "saved") fingerprint.current = r.data.fingerprint;
        if (r.status === "ok" && r.data.kind === "conflict") {
          // The file keeps the other computer's text: the editor shows it
          // when the edit stops. An edit sent before then goes into a
          // draft too.
          onConflict?.(folderName(r.data.draft));
          stale.current = true;
        }
      }
      if (next) save(next.id, next.markdown);
      else if (stale.current) reloadWhenIdle();
    });
  };

  const send = (next: string) => {
    onMarkdown?.(next);
    save(sessionId, next);
  };

  return (
    <div
      className="mx-auto max-w-[44rem] px-8 pt-5 pb-12 [&_.prosemirror-editor]:min-h-[50vh]"
      onKeyDownCapture={() => (lastInput.current = performance.now())}
      onInputCapture={() => (lastInput.current = performance.now())}
    >
      {error && <p className="mb-2 text-[0.85rem] text-negative">{error}</p>}
      {loaded !== null && (
        <NoteEditor
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
