// Transcribe Again on a saved note (R1-R9). The recognition settings decide
// the models, so the only question is one confirmation. A check runs before
// the dialog opens: when the settings cannot run on this note, the note
// shows the reason in place of the dialog (R7).

import { useEffect, useRef, useState } from "react";
import { commands, type TranscribeAgainCheck, type TranscribeAgainRefusal } from "../bindings";
import { ConfirmDialog } from "../lib/ConfirmDialog";

export function useTranscribeAgain({
  sessionId,
  available,
  onQueued,
  onFailed,
}: {
  sessionId: string;
  /// The note is saved, its folder can be read, no part of it waits for
  /// sync, and no summary is being made. When this becomes false, the dialog closes (KTD7).
  available: boolean;
  onQueued: () => void;
  onFailed: (refusal: TranscribeAgainRefusal) => void;
}) {
  const [check, setCheck] = useState<TranscribeAgainCheck | null>(null);
  const [busy, setBusy] = useState(false);
  // A second click before `busy` renders sends no second command (KTD6).
  const sending = useRef(false);
  // Each check gets a number. When the note becomes unavailable, the number
  // changes, so a check that returns later does not open the dialog.
  const asked = useRef(0);

  useEffect(() => {
    if (available) return;
    asked.current += 1;
    setCheck(null);
  }, [available]);
  useEffect(() => () => setCheck(null), [sessionId]);

  const ask = async () => {
    const id = (asked.current += 1);
    try {
      const result = await commands.checkTranscribeAgain(sessionId);
      if (id !== asked.current) return;
      if (result.status === "ok") setCheck(result.data);
      else onFailed(result.error);
    } catch {
      onFailed({ message: "Could not check the transcription settings.", in_settings: false });
    }
  };

  const confirm = async () => {
    if (!check || sending.current) return;
    sending.current = true;
    setBusy(true);
    try {
      const result = await commands.transcribeAgain(sessionId, check.model_id);
      setCheck(null);
      if (result.status === "ok") onQueued();
      else onFailed(result.error);
    } catch {
      setCheck(null);
      onFailed({ message: "Could not start the new transcript. The current one is unchanged.", in_settings: false });
    } finally {
      sending.current = false;
      setBusy(false);
    }
  };

  const dialog = check && available && (
    <ConfirmDialog
      title="Transcribe this note again?"
      message={
        <>
          <p>
            The transcript and summary are not available until the new ones are ready. If it fails, the
            current ones come back.
          </p>
          <p>This uses {check.model_name}.</p>
        </>
      }
      action="Transcribe Again"
      destructive={false}
      busy={busy}
      onConfirm={() => void confirm()}
      onCancel={() => setCheck(null)}
    />
  );
  return { ask: () => void ask(), dialog };
}
