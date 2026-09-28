import { useRef, useState } from "react";
import {
  commands,
  type DownloadUpdate,
  type ExecutionSpec,
  type ModelRole,
  type ModelStatus,
  type Settings,
  type SpeakerModelsStatus,
} from "../bindings";
import { sentence } from "../processing/describe";
import { RecognitionOptions, recognitionDefaults } from "./RecognitionOptions";
import { Button, ErrorText, Group, Pane, Row, formatBytes } from "./ui";

export const roleLabel: Record<ModelRole, string> = {
  recognition: "Speech recognition",
  recognition_artifact: "Transcription model",
  segmentation: "Speaker segmentation",
  embedding: "Speaker embedding",
};
export type Actions = {
  download: (ids: string[]) => void;
  cancel: () => void;
  select: (id: string) => Promise<string | null>;
  refresh: () => Promise<void>;
};
const languageNames = new Intl.DisplayNames(["en"], { type: "language" });
export function describeLanguages(codes: string[]): string {
  return codes.length > 2
    ? `${codes.length} languages`
    : codes.map((c) => languageNames.of(c) ?? c).join(" and ");
}

export function Models({
  models,
  live,
  actions,
  settings,
  onSettings,
}: {
  models: ModelStatus[];
  speakers: SpeakerModelsStatus | null;
  live: Record<string, DownloadUpdate>;
  actions: Actions;
  settings: Settings;
  onSettings: (s: Settings) => void;
}) {
  const [spec, setSpec] = useState<ExecutionSpec | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const saveQueue = useRef(Promise.resolve());
  const saveVersion = useRef(0);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const remove = async (id: string) => {
    setBusy(true);
    try {
      const result = await commands.deleteModel(id);
      if (result.status === "error") setError(sentence(result.error));
      else {
        setError(null);
        await actions.refresh();
      }
    } catch {
      setError("Could not delete the model.");
    } finally {
      setDeleting(null);
      setBusy(false);
    }
  };
  const apply = (next: ExecutionSpec) => {
    const version = ++saveVersion.current;
    setSaving(true);
    setError(null);
    saveQueue.current = saveQueue.current.then(async () => {
      if (version !== saveVersion.current) return;
      try {
        const result = await commands.setRecognitionDefaults(next.config);
        if (version !== saveVersion.current) return;
        if (result.status === "error") setError(sentence(result.error));
        else {
          onSettings(result.data);
          setMessage(
            result.data.pending_recognition
              ? "The required models are downloading. Your current defaults stay until they are ready."
              : null,
          );
        }
      } catch {
        if (version === saveVersion.current)
          setError(
            "Could not save the transcription defaults. Change the choice to try again.",
          );
      } finally {
        if (version === saveVersion.current) setSaving(false);
      }
    });
  };
  const download = async () => {
    if (!spec) return;
    setBusy(true);
    try {
      const result = await commands.downloadRecognitionArtifacts(spec);
      setError(result.status === "error" ? sentence(result.error) : null);
    } catch {
      setError("Could not start the model download.");
    } finally {
      setBusy(false);
    }
  };
  const active = Object.values(live).some(
    (v) => v.kind === "progress" || v.kind === "verifying",
  );
  return (
    <Pane title="Transcription">
      <Group
        header="Default recognition choices"
        footer="New notes use these choices. You can change them for one note before recording."
      >
        {settings.pending_recognition && (
          <p role="status" className="px-4 pt-3">
            Models are downloading for{" "}
            {models.find(
              (m) => m.id === settings.pending_recognition?.asr_model,
            )?.name ?? settings.pending_recognition.asr_model}
            . Current defaults stay until they are ready.
          </p>
        )}
        <RecognitionOptions
          models={models}
          initial={recognitionDefaults(settings)}
          onResolved={setSpec}
          onChanged={apply}
        />
        {saving && (
          <p role="status" className="px-4 pb-2 text-label-secondary">
            Saving…
          </p>
        )}
        <Row label="Required models">
          <Button
            disabled={!spec || busy || active}
            onClick={() => void download()}
          >
            Download required models
          </Button>
          {active && <Button onClick={actions.cancel}>Cancel download</Button>}
        </Row>
        {Object.entries(live).map(([id, update]) =>
          update.kind === "progress" ? (
            <p
              key={id}
              role="status"
              className="px-4 py-1 text-label-secondary"
            >
              {models.find((m) => m.id === id)?.name ?? id}:{" "}
              {formatBytes(update.downloaded)} of {formatBytes(update.total)}
            </p>
          ) : update.kind === "failed" ? (
            <ErrorText key={id}>{update.message}</ErrorText>
          ) : null,
        )}
        {message && (
          <p role="status" className="px-4 pb-3">
            {message}
          </p>
        )}
        {error && <ErrorText>{error}</ErrorText>}
      </Group>
      <Group
        header="Downloaded models"
        footer="You can download a deleted model again. Models in use cannot be deleted."
      >
        {models
          .filter((m) => m.state.state === "present")
          .map((m) => (
            <Row key={m.id} label={m.name} detail={formatBytes(m.size_bytes)}>
              {deleting === m.id ? (
                <>
                  <Button disabled={busy} onClick={() => setDeleting(null)}>
                    Cancel
                  </Button>
                  <Button
                    destructive
                    disabled={busy}
                    onClick={() => void remove(m.id)}
                  >
                    Delete
                  </Button>
                </>
              ) : (
                <Button
                  disabled={busy || m.required || m.id === settings.asr_model}
                  onClick={() => setDeleting(m.id)}
                >
                  Delete…
                </Button>
              )}
            </Row>
          ))}
      </Group>
    </Pane>
  );
}
