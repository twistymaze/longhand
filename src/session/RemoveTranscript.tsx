// Remove Transcript on a saved note (R17, R18 of the transcript removal
// plan). One confirmation, then the transcript and any audio that is left
// go at once, whether the transcript removal setting is on or off. The
// summary, notes, title and tags stay.

import { useRef, useState } from "react";
import { commands } from "../bindings";
import { ConfirmDialog } from "../lib/ConfirmDialog";
import { sentence } from "../processing/describe";

/// Why an action that needs the transcript is unavailable (R14, R15). The
/// note shows the fact nowhere else.
export const TRANSCRIPT_REMOVED_REASON = "The transcript of this note was removed.";

/// Why Regenerate Summary is unavailable. It is the text of the refusal of
/// the command.
export const SUMMARY_REMOVED_REASON =
  "The transcript of this note was removed, so its summary can't be made again.";

export function useRemoveTranscript({
  noteId,
  onRemoved,
  onFailed,
}: {
  noteId: string;
  /// The transcript of the note is gone.
  onRemoved: () => void;
  /// The removal was refused or did not complete, with a sentence that says why.
  onFailed: (text: string) => void;
}) {
  // The facts that the confirmation names. They are read when it opens.
  const [open, setOpen] = useState<{ audio: boolean; summary: boolean } | null>(null);
  const [busy, setBusy] = useState(false);
  // A second click before `busy` renders sends no second command.
  const sending = useRef(false);

  const remove = async () => {
    if (sending.current) return;
    sending.current = true;
    setBusy(true);
    try {
      const result = await commands.removeTranscript(noteId);
      setOpen(null);
      if (result.status === "ok") onRemoved();
      else onFailed(sentence(result.error.message));
    } catch {
      setOpen(null);
      onFailed("The transcript of this note could not be removed.");
    } finally {
      sending.current = false;
      setBusy(false);
    }
  };

  const dialog = open && (
    <ConfirmDialog
      title="Remove the transcript of this note?"
      message={
        <>
          {open.audio && <p>The audio of this note is removed too.</p>}
          <p>
            {open.summary
              ? "The summary and notes stay."
              : "This note has no summary, and none can be made after the removal. The notes stay."}
          </p>
          <p>You can’t undo this. You can’t make the summary again, transcribe the note again or resume its recording.</p>
        </>
      }
      action="Remove Transcript"
      busy={busy}
      onConfirm={() => void remove()}
      onCancel={() => setOpen(null)}
    />
  );
  return {
    /// `audio`: the note still has audio. `summary`: the note has a summary
    /// that is not the stub of a failed summary.
    ask: (facts: { audio: boolean; summary: boolean }) => setOpen(facts),
    dialog,
  };
}
