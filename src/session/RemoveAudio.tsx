// Remove Audio on a saved note (R13). One confirmation, then the audio goes
// at once, whether the setting of audio removal is on or off. The
// transcript, summary and notes stay.

import { useRef, useState } from "react";
import { commands } from "../bindings";
import { ConfirmDialog } from "../lib/ConfirmDialog";
import { sentence } from "../processing/describe";

/// Why an action that needs the audio is unavailable (R14). The note shows
/// the fact nowhere else.
export const REMOVED_REASON = "The audio of this note was removed.";

export function useRemoveAudio({
  sessionId,
  onRemoved,
  onFailed,
}: {
  sessionId: string;
  /// The audio of the note is gone.
  onRemoved: () => void;
  /// The removal was refused or did not complete, with a sentence that says why.
  onFailed: (text: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  // A second click before `busy` renders sends no second command.
  const sending = useRef(false);

  const remove = async () => {
    if (sending.current) return;
    sending.current = true;
    setBusy(true);
    try {
      const result = await commands.removeAudio(sessionId);
      setOpen(false);
      if (result.status === "ok") onRemoved();
      else onFailed(sentence(result.error.message));
    } catch {
      setOpen(false);
      onFailed("The audio of this note could not be removed.");
    } finally {
      sending.current = false;
      setBusy(false);
    }
  };

  const dialog = open && (
    <ConfirmDialog
      title="Remove the audio of this note?"
      message={
        <>
          <p>The transcript, summary and notes stay.</p>
          <p>You can’t undo this. You can’t transcribe the note again or resume its recording.</p>
        </>
      }
      action="Remove Audio"
      busy={busy}
      onConfirm={() => void remove()}
      onCancel={() => setOpen(false)}
    />
  );
  return { ask: () => setOpen(true), dialog };
}
