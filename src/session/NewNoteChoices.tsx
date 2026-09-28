import { useEffect, useState } from "react";
import {
  commands,
  events,
  type ExecutionSpec,
  type ModelStatus,
  type RecognitionConfig,
  type Settings,
} from "../bindings";
import {
  recognitionDefaults,
  RecognitionOptions,
} from "../settings/RecognitionOptions";
import { Button, ErrorText, formatBytes } from "../settings/ui";
import { RecognitionDialog } from "./RecognitionDialog";

export function NewNoteChoices({
  disabled,
  onStart,
}: {
  disabled: boolean;
  onStart: (config: RecognitionConfig) => Promise<boolean>;
}) {
  const [choice, setChoice] = useState<{
    settings: Settings;
    models: ModelStatus[];
  } | null>(null);
  const [spec, setSpec] = useState<ExecutionSpec | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [downloadStatus, setDownloadStatus] = useState<string | null>(null);
  const missing =
    spec?.artifacts.filter(
      (a) =>
        choice?.models.find((m) => m.id === a.id)?.state.state !== "present",
    ) ?? [];
  useEffect(() => {
    if (!choice || !spec) return;
    let active = true;
    const unlisten = events.modelDownloadEvent.listen(({ payload }) => {
      if (!active || !spec.artifacts.some((a) => a.id === payload.model_id))
        return;
      const update = payload.update;
      if (update.kind === "progress")
        setDownloadStatus(
          `Downloading required models: ${formatBytes(update.downloaded)} of ${formatBytes(update.total)} for this file.`,
        );
      if (update.kind === "verifying")
        setDownloadStatus("Checking the downloaded model…");
      if (update.kind === "failed") {
        setError(update.message);
        setDownloadStatus(null);
      }
      if (update.kind === "cancelled")
        setDownloadStatus("Download cancelled. You can try again.");
      if (
        update.kind === "completed" ||
        update.kind === "cancelled" ||
        update.kind === "failed"
      ) {
        void commands
          .modelStatuses()
          .then((models) => {
            if (!active) return;
            setChoice((previous) => previous && { ...previous, models });
            if (
              spec.artifacts.every((a) =>
                models.some(
                  (m) => m.id === a.id && m.state.state === "present",
                ),
              )
            )
              setDownloadStatus("Required models are ready.");
          })
          .catch(() => {
            if (active)
              setError(
                "Could not check the downloaded models. Close these choices and open them again.",
              );
          });
      }
    });
    return () => {
      active = false;
      void unlisten.then((stop) => stop());
    };
  }, [Boolean(choice), spec]);
  const download = async () => {
    if (!spec) return;
    setBusy(true);
    setError(null);
    try {
      const result = await commands.downloadRecognitionArtifacts(spec);
      if (result.status === "error") setError(result.error);
      else {
        setDownloadStatus(
          "Download requested. Recording is available when all required models are ready.",
        );
        const models = await commands.modelStatuses();
        setChoice((previous) => previous && { ...previous, models });
      }
    } catch {
      setError("Could not start the model download.");
    } finally {
      setBusy(false);
    }
  };
  const open = async () => {
    setBusy(true);
    setError(null);
    setDownloadStatus(null);
    try {
      const [settings, models] = await Promise.all([
        commands.getSettings(),
        commands.modelStatuses(),
      ]);
      setChoice({ settings, models });
    } catch {
      setError("Could not read the recognition choices.");
    } finally {
      setBusy(false);
    }
  };
  const start = async () => {
    if (!spec) return;
    setBusy(true);
    setError(null);
    try {
      if (await onStart(spec.config)) setChoice(null);
      else
        setError("Could not start recording. Your app defaults are unchanged.");
    } catch {
      setError("Could not start recording.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <Button disabled={disabled || busy} onClick={() => void open()}>
        Choices for next note…
      </Button>
      {!choice && error && <ErrorText>{error}</ErrorText>}
      {choice && (
        <RecognitionDialog
          title="Recognition for this note"
          action="Record this note"
          busy={busy}
          disabled={!spec || missing.length > 0}
          onConfirm={() => void start()}
          onCancel={() => {
            setChoice(null);
            setSpec(null);
          }}
        >
          <p className="mt-2 text-label-secondary">
            These choices apply to this note only. App defaults stay the same.
          </p>
          <RecognitionOptions
            models={choice.models}
            initial={recognitionDefaults(choice.settings)}
            onResolved={setSpec}
          />
          {missing.length > 0 && (
            <p className="mb-2 text-label-secondary">
              {missing.length} required model{" "}
              {missing.length === 1 ? "file is" : "files are"} not downloaded.
            </p>
          )}
          <Button disabled={!spec || busy} onClick={() => void download()}>
            Download required models
          </Button>
          {downloadStatus && (
            <p role="status" className="mt-2 text-label-secondary">
              {downloadStatus}
            </p>
          )}
          {error && <ErrorText>{error}</ErrorText>}
        </RecognitionDialog>
      )}
    </>
  );
}
