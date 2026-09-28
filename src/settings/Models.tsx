import { useState } from "react";
import {
  commands,
  type DownloadUpdate,
  type ModelRole,
  type ModelStatus,
  type Settings,
} from "../bindings";
import { sentence } from "../processing/describe";
import { RecognitionOptions, recognitionDefaults } from "./RecognitionOptions";
import { Button, ErrorText, Pane, ProgressBar, formatBytes } from "./ui";
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

/// The Transcription tab. The user sees only transcription models. Each
/// row stands for the model bundle: the model and its supporting models
/// (R7). Longhand downloads and deletes the supporting models with it.
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
  /// Download the missing files of the bundle of `id` (R10).
  const download = async (id: string) => {
    setBusy(true);
    try {
      const result = await commands.downloadModelBundle(id);
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
  const updatesOf = (m: ModelStatus) =>
    m.bundle.map((id) => live[id]).filter((u): u is DownloadUpdate => Boolean(u));
  const pending = (m: ModelStatus) =>
    settings.pending_recognition?.asr_model === m.id || settings.pending_asr_model === m.id;
  const downloading = (m: ModelStatus) =>
    m.state.state === "downloading" ||
    updatesOf(m).some((u) => u.kind === "progress" || u.kind === "verifying");
  const modelActivity = (m: ModelStatus) => {
    const updates = updatesOf(m);
    const progress = updates.filter((u): u is Extract<DownloadUpdate, { kind: "progress" }> => u.kind === "progress");
    if (progress.length) {
      const downloaded = progress.reduce((n, u) => n + u.downloaded, 0);
      const total = progress.reduce((n, u) => n + u.total, 0);
      return (
        <div className="grid gap-2 text-[0.85rem] text-accent">
          <span className="flex items-center justify-between gap-3">
            <span role="status">
              Downloading… {formatBytes(downloaded)} of {formatBytes(total)}
            </span>
            <Button onClick={actions.cancel}>Cancel</Button>
          </span>
          <ProgressBar fraction={total ? downloaded / total : 0} />
        </div>
      );
    }
    const failed = updates.find((u) => u.kind === "failed");
    if (failed?.kind === "failed") return <p className="text-[0.85rem] text-negative">{failed.message}</p>;
    const message = updates.some((u) => u.kind === "verifying") ? "Checking download…"
      : saving && savingModel === m.id ? "Preparing model…"
      : downloading(m) || pending(m) ? "Downloading…" : null;
    return message && <p role="status" className="text-[0.85rem] text-accent">{message}</p>;
  };
  const modelAction = (m: ModelStatus) => {
    const selected = m.id === settings.asr_model;
    const present = m.state.state === "present";
    const repair =
      !m.bundle_complete && (present || selected) && !downloading(m) && !pending(m) ? (
        <Button disabled={busy} onClick={() => void download(m.id)}>
          Download
        </Button>
      ) : null;
    if (!present) return repair;
    if (deleting === m.id)
      return (
        <>
          <Button disabled={busy} onClick={() => setDeleting(null)}>
            Cancel
          </Button>
          <Button destructive disabled={busy} onClick={() => void remove(m.id)}>
            Delete
          </Button>
        </>
      );
    return (
      <>
        {repair}
        <Button disabled={busy || saving || active || selected} onClick={() => setDeleting(m.id)}>
          Delete…
        </Button>
      </>
    );
  };
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
      {saving && (
        <p role="status" className="px-4 text-label-secondary">
          Saving…
        </p>
      )}
      {(error || defaults.error) && <ErrorText>{error || defaults.error}</ErrorText>}
      <RecognitionOptions
        models={models}
        initial={recognitionDefaults(settings)}
        onResolved={() => {}}
        onChanged={(next) => {
          setError(null);
          defaults.save(next.config);
        }}
        modelAction={modelAction}
        modelActivity={modelActivity}
      />
    </Pane>
  );
}
