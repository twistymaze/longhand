import { useEffect, useState } from "react";
import {
  commands,
  events,
  type DeleteAllBlock,
  type DeleteScope,
  type DownloadUpdate,
  type ExecutionSpec,
  type ModelDeleteFailure,
  type ModelDeletionSummary,
  type ModelRole,
  type ModelStatus,
  type Settings,
} from "../bindings";
import { ConfirmDialog } from "../lib/ConfirmDialog";
import { sentence } from "../processing/describe";
import { ChevronIcon } from "./icons";
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

/// Why "Delete all" is off, as the dialog shows it.
const blockReason: Record<DeleteAllBlock, string> = {
  recording: "Delete all is off while a recording is in progress.",
  download: "Delete all is off while a download is in progress.",
  transcription: "Delete all is off while a note is transcribing.",
};

const totalBytes = (models: ModelDeletionSummary["models"]) =>
  models.reduce((n, m) => n + m.size_bytes, 0);

/// A list of names in plain language: "A", "A and B", "A, B and C".
function nameList(names: string[]): string {
  return names.length < 2 ? names.join("") : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

/// The error text for the models that the bulk delete could not delete.
function failureText(failures: ModelDeleteFailure[]): string {
  return `Could not delete ${nameList(failures.map((f) => f.name))}. Try again, or delete each model from its row.`;
}

/// True while a recording is in progress. The value follows the
/// `recording-changed` event. A failed status read counts as no recording:
/// the backend still refuses "Delete all" during a recording.
function useRecording(): boolean {
  const [recording, setRecording] = useState(false);
  useEffect(() => {
    let mounted = true;
    const read = async () => {
      try {
        const result = await commands.recordingStatus();
        if (mounted && result.status === "ok") setRecording(result.data.recording);
      } catch {
        // The status is not available. Keep the last value.
      }
    };
    void read();
    const unlisten = events.recordingChanged.listen(() => void read());
    return () => {
      mounted = false;
      void unlisten.then((f) => f());
    };
  }, []);
  return recording;
}

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
  const recording = useRecording();
  const [loadingSummary, setLoadingSummary] = useState(false);
  const [summary, setSummary] = useState<ModelDeletionSummary | null>(null);
  const [bulkBusy, setBulkBusy] = useState(false);
  const [showSupporting, setShowSupporting] = useState(false);
  const downloading = active || models.some((m) => m.state.state === "downloading");
  const anyDownloaded = models.some((m) => m.state.state === "present");
  const supporting = models.filter((m) => m.role !== "recognition" && m.state.state === "present");
  const openDeleteDialog = async () => {
    setLoadingSummary(true);
    try {
      const result = await commands.modelDeletionSummary();
      if (result.status === "error") setError(sentence(result.error));
      else {
        setError(null);
        setSummary(result.data);
      }
    } catch {
      setError("Could not read the downloaded models.");
    } finally {
      setLoadingSummary(false);
    }
  };
  const deleteByScope = async (scope: DeleteScope) => {
    setBulkBusy(true);
    try {
      const result = await commands.deleteModels(scope);
      if (result.status === "error") setError(sentence(result.error));
      else setError(result.data.length ? failureText(result.data) : null);
    } catch {
      setError("Could not delete the models.");
    } finally {
      await actions.refresh();
      setBulkBusy(false);
      setSummary(null);
    }
  };
  const deleteDialog = (s: ModelDeletionSummary) => {
    const unused = s.models.filter((m) => m.unused);
    const notes = s.unfinished_notes;
    const waiting =
      notes === 0 ? null : `${notes} unfinished ${notes === 1 ? "note" : "notes"} will wait for the models.`;
    return (
      <ConfirmDialog
        title="Delete models?"
        message={
          <>
            {unused.length > 0 && (
              <p>Delete unused keeps the models that your current choices need.</p>
            )}
            <p>
              {unused.length > 0 && "If you delete all, "}
              Longhand cannot transcribe new notes until you download the models again.
            </p>
            {waiting && <p>{waiting}</p>}
          </>
        }
        action={`Delete all (${formatBytes(totalBytes(s.models))})`}
        actionDisabled={s.delete_all_block !== null}
        actionReason={s.delete_all_block ? blockReason[s.delete_all_block] : undefined}
        secondary={
          unused.length > 0
            ? {
                label: `Delete unused (${formatBytes(totalBytes(unused))})`,
                onClick: () => void deleteByScope("unused"),
              }
            : undefined
        }
        busy={bulkBusy}
        onConfirm={() => void deleteByScope("all")}
        onCancel={() => {
          if (!bulkBusy) setSummary(null);
        }}
      />
    );
  };
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
          <Button
            disabled={
              recording || downloading || !anyDownloaded || busy || loadingSummary || bulkBusy || summary !== null
            }
            onClick={() => void openDeleteDialog()}
          >
            Delete models…
          </Button>
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
        <div className="settings-row px-4">
          <div>
            <button
              type="button"
              aria-expanded={showSupporting}
              aria-controls="supporting-models"
              onClick={() => setShowSupporting(!showSupporting)}
              className="flex w-full items-center gap-3 py-2.5 text-left outline-offset-2 focus-visible:outline-2 focus-visible:outline-accent"
            >
              <span className="min-w-0 flex-1 font-medium">Show models</span>
              <ChevronIcon
                className={`size-3 shrink-0 text-label-tertiary transition-transform duration-150 motion-reduce:transition-none ${showSupporting ? "rotate-90" : ""}`}
              />
            </button>
          </div>
        </div>
        {showSupporting && (
          // The first row inside the list gets the separator that the
          // adjacent-row rule cannot give it.
          <div id="supporting-models" className="[&>.settings-row:first-child>div]:border-t [&>.settings-row:first-child>div]:border-[var(--separator)]">
            {supporting.length === 0 && (
              <Row label={<span className="font-normal text-label-secondary">No supporting models are downloaded.</span>} />
            )}
            {supporting.map((m) => (
                <Row
                  key={m.id}
                  label={<span className="block min-w-0 [overflow-wrap:anywhere]">{m.name}</span>}
                  detail={formatBytes(m.size_bytes)}
                >
                  {modelAction(m)}
                </Row>
              ))}
          </div>
        )}
      </Group>
      {summary && deleteDialog(summary)}
    </Pane>
  );
}
