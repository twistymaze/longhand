import { useCallback, useEffect, useRef, useState } from "react";
import {
  commands,
  type ExecutionSpec,
  type ModelStatus,
  type ReprocessPreview,
  type Settings,
} from "../bindings";
import { sentence } from "../processing/describe";
import {
  RecognitionOptions,
  recognitionDefaults,
} from "../settings/RecognitionOptions";
import { Button, ErrorText } from "../settings/ui";
import { RecognitionDialog } from "./RecognitionDialog";

export function useTranscribeAgain({
  sessionId,
  onQueued,
  onFailed,
}: {
  sessionId: string;
  noteModel: string | null;
  onQueued: () => void;
  onFailed: (text: string) => void;
}) {
  const [choice, setChoice] = useState<{
    settings: Settings;
    models: ModelStatus[];
  } | null>(null);
  const [spec, setSpec] = useState<ExecutionSpec | null>(null);
  const [preview, setPreview] = useState<ReprocessPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [consent, setConsent] = useState(false);
  const revision = useRef(0);
  const operation = useRef(0);
  const resolve = useCallback((next: ExecutionSpec | null) => {
    revision.current += 1;
    setSpec(next);
    setPreview(null);
    setConsent(false);
    setError(null);
  }, []);
  useEffect(
    () => () => {
      revision.current += 1;
      operation.current += 1;
    },
    [sessionId],
  );
  const ask = async () => {
    try {
      const [settings, models] = await Promise.all([
        commands.getSettings(),
        commands.modelStatuses(),
      ]);
      setChoice({ settings, models });
      setSpec(null);
      setPreview(null);
      setError(null);
      setConsent(false);
    } catch {
      onFailed("Could not read the recognition choices.");
    }
  };
  const confirm = async () => {
    if (!spec || busy) return;
    const requestedRevision = revision.current;
    const requestedOperation = ++operation.current;
    setBusy(true);
    setError(null);
    try {
      if (!preview) {
        const result = await commands.previewReprocess(sessionId, spec.config);
        if (revision.current !== requestedRevision) return;
        if (result.status === "error") setError(sentence(result.error));
        else {
          setPreview(result.data);
          setConsent(false);
        }
      } else {
        const result = await commands.reprocessSession(
          sessionId,
          preview,
          consent,
        );
        if (result.status === "ok") {
          setChoice(null);
          onQueued();
        } else setError(sentence(result.error));
      }
    } catch {
      if (revision.current !== requestedRevision) return;
      setError(
        "Could not prepare the replacement. The previous result stays unchanged.",
      );
    } finally {
      if (operation.current === requestedOperation) setBusy(false);
    }
  };
  const dialog = choice && (
    <RecognitionDialog
      title="Reprocess this note?"
      action={preview ? "Reprocess note" : "Preview changes"}
      busy={busy}
      disabled={!spec || Boolean(preview?.requires_transcription && !consent)}
      onConfirm={() => void confirm()}
      onCancel={() => setChoice(null)}
    >
      <p className="mt-2 text-label-secondary">
        The previous result stays until the replacement transcript and summary
        are ready. My Notes and the title stay the same.
      </p>
      {!preview ? (
        <RecognitionOptions
          models={choice.models}
          initial={spec?.config ?? recognitionDefaults(choice.settings)}
          onResolved={resolve}
        />
      ) : (
        <div className="mt-4 grid gap-3">
          <p>{preview.explanation}</p>
          <p>
            {choice.models.find(
              (m) => m.id === preview.execution.config.asr_model,
            )?.name ?? preview.execution.config.asr_model}
          </p>
          {preview.requires_transcription && (
            <label className="flex items-start gap-2">
              <input
                type="checkbox"
                checked={consent}
                onChange={(e) => setConsent(e.target.checked)}
              />
              Transcribe the audio again and replace the transcript and summary.
            </label>
          )}
          <Button disabled={busy} onClick={() => setPreview(null)}>
            Change choices
          </Button>
        </div>
      )}
      {error && <ErrorText>{error}</ErrorText>}
    </RecognitionDialog>
  );
  return { ask: () => void ask(), dialog };
}
