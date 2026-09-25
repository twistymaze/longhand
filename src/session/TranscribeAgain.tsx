// Transcribe Again on a saved note (R15, R16, R17). The question names the
// model selected now, says when that model is the one the note already
// used, and says what is replaced. Cancel changes nothing.

import { useState } from "react";
import { commands } from "../bindings";
import { sentence } from "../processing/describe";
import { ConfirmDialog } from "../lib/ConfirmDialog";

/// The model that Transcribe Again uses: the model selected now.
type Choice = {
  name: string;
  /// The note was transcribed with this model.
  current: boolean;
  /// The model is not downloaded yet: the note waits for it (R17).
  waits: boolean;
};

/// The Transcribe Again question for the note page. `ask` reads the
/// selected model and shows the question; `dialog` is the question to
/// render, or null.
export function useTranscribeAgain({
  sessionId,
  noteModel,
  onQueued,
  onFailed,
}: {
  sessionId: string;
  /// The model the note was transcribed with, if it recorded one.
  noteModel: string | null;
  onQueued: () => void;
  /// The note was not queued, with a sentence that says why.
  onFailed: (text: string) => void;
}) {
  const [choice, setChoice] = useState<Choice | null>(null);
  const [busy, setBusy] = useState(false);

  const ask = async () => {
    const [settings, models] = await Promise.all([commands.getSettings(), commands.modelStatuses()]);
    const selected = models.find((m) => m.id === settings.asr_model);
    setChoice({
      name: selected?.name ?? settings.asr_model,
      current: settings.asr_model === noteModel,
      waits: selected !== undefined && selected.state.state !== "present",
    });
  };

  const confirm = async () => {
    setBusy(true);
    const result = await commands.transcribeAgain(sessionId);
    setBusy(false);
    setChoice(null);
    if (result.status === "ok") onQueued();
    else onFailed(sentence(result.error));
  };

  const dialog =
    choice === null ? null : (
      <ConfirmDialog
        title="Transcribe this note again?"
        message={
          <>
            <p>
              Longhand makes a new transcript with <span className="font-medium text-label">{choice.name}</span>
              {choice.current ? ", the model this note already used" : ""}, and then a new summary. They
              replace the current transcript and summary.
            </p>
            <p>My Notes and the title stay the same.</p>
            {choice.waits && <p>{choice.name} isn’t downloaded yet. The note waits until it is.</p>}
          </>
        }
        action="Transcribe Again"
        destructive={false}
        busy={busy}
        onConfirm={() => void confirm()}
        onCancel={() => setChoice(null)}
      />
    );

  return { ask: () => void ask(), dialog };
}
