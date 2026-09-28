import { useState } from "react";
import {
  commands,
  type DownloadUpdate,
  type ExecutionSpec,
  type ModelRole,
  type ModelStatus,
  type Settings,
} from "../bindings";
import { sentence } from "../processing/describe";
import { RecognitionOptions, recognitionDefaults } from "./RecognitionOptions";
import { Button, ErrorText, Group, Pane, Row, ProgressBar, formatBytes } from "./ui";
import type { RecognitionDefaultsSave } from "./useAppSettings";

export const roleLabel: Record<ModelRole, string> = {
  recognition: "Speech recognition",
  recognition_artifact: "Transcription model",
  segmentation: "Speaker segmentation",
  embedding: "Speaker embedding",
};
export type Actions = {
  cancel: () => void;
  refresh: () => Promise<void>;
};

export function Models({
  models,
  live,
  actions,
  settings,
  defaults,
}: {
  models: ModelStatus[];
  live: Record<string, DownloadUpdate>;
  actions: Actions;
  settings: Settings;
  defaults: RecognitionDefaultsSave;
}) {
  const [spec, setSpec] = useState<ExecutionSpec | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { savingModel } = defaults;
  const saving = savingModel !== null;
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
    setError(null);
    defaults.save(next.config);
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
  const modelActivity = (m: ModelStatus) => {
    const ids = new Set([m.id]);
    if (spec?.config.asr_model === m.id) for (const artifact of spec.artifacts) ids.add(artifact.id);
    const updates = [...ids].map((id) => live[id]).filter(Boolean);
    const progress = updates.filter((u): u is Extract<DownloadUpdate, { kind: "progress" }> => u.kind === "progress");
    if (progress.length) {
      const downloaded = progress.reduce((n, u) => n + u.downloaded, 0);
      const total = progress.reduce((n, u) => n + u.total, 0);
      return <div className="grid gap-2 text-[0.85rem] text-accent"><span>Downloading… {formatBytes(downloaded)} of {formatBytes(total)}</span><ProgressBar fraction={total ? downloaded / total : 0} /></div>;
    }
    const failed = updates.find((u) => u.kind === "failed");
    if (failed?.kind === "failed") return <p className="text-[0.85rem] text-negative">{failed.message}</p>;
    const message = updates.some((u) => u.kind === "verifying") ? "Checking download…"
      : saving && savingModel === m.id ? "Preparing model…"
      : m.state.state === "downloading" || settings.pending_recognition?.asr_model === m.id ? "Downloading required models…" : null;
    return message && <p role="status" className="text-[0.85rem] text-accent">{message}</p>;
  };
  const modelAction = (m: ModelStatus) =>
    m.state.state !== "present" ? null : deleting === m.id ? (
      <>
        <Button disabled={busy} onClick={() => setDeleting(null)}>
          Cancel
        </Button>
        <Button destructive disabled={busy} onClick={() => void remove(m.id)}>
          Delete
        </Button>
      </>
    ) : (
      <Button
        disabled={busy || saving || active || m.id === settings.asr_model}
        onClick={() => setDeleting(m.id)}
      >
        Delete…
      </Button>
    );
  return (
    <Pane title="Transcription">
      <p className="text-[0.85rem] text-label-secondary">
        New notes use these choices.
      </p>
      {settings.pending_recognition && (
        <p role="status" className="px-4 pt-3">
          Models are downloading for{" "}
          {models.find((m) => m.id === settings.pending_recognition?.asr_model)
            ?.name ?? settings.pending_recognition.asr_model}
          . Current defaults stay until they are ready.
        </p>
      )}
      <RecognitionOptions
        models={models}
        initial={recognitionDefaults(settings)}
        onResolved={setSpec}
        onChanged={apply}
        modelAction={modelAction}
        modelActivity={modelActivity}
      />
      <Group header="Downloads">
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
        {(error || defaults.error) && <ErrorText>{error || defaults.error}</ErrorText>}
      </Group>
      <Group
        header="Supporting models"
        footer="You can download a deleted model again. Models in use cannot be deleted."
      >
        {models
          .filter(
            (m) => m.role !== "recognition" && m.state.state === "present",
          )
          .map((m) => (
            <Row key={m.id} label={m.name} detail={formatBytes(m.size_bytes)}>
              {modelAction(m)}
            </Row>
          ))}
      </Group>
    </Pane>
  );
}
