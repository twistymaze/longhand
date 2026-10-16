import { useEffect, useRef, useState } from "react";
import {
  commands,
  type DownloadUpdate,
  type ModelRole,
  type ModelStatus,
  type RecognitionConfig,
  type Settings,
} from "../bindings";
import { sentence } from "../processing/describe";
import { OwnSaves } from "./ownSaves";
import { RecognitionOptions, recognitionDefaults } from "./RecognitionOptions";
import { readOnlyReason } from "./shared";
import { Button, ErrorText, Group, Pane, ProgressBar, Row, Status, formatBytes } from "./ui";
import type { RecognitionDefaultsSave } from "./useAppSettings";

export const roleLabel: Record<ModelRole, string> = {
  recognition: "Speech recognition",
  recognition_artifact: "Transcription model",
  segmentation: "Speaker segmentation",
  embedding: "Speaker embedding",
};
/// A text that identifies the six values of a recognition configuration.
function configKey(c: RecognitionConfig): string {
  return JSON.stringify([
    c.asr_model,
    c.speaker_source,
    c.processing_mode,
    c.speaker_processing_mode ?? "after_stop",
    c.speaker_count,
    c.language,
  ]);
}

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
  // The models that started a bundle download in this pane: by a click on
  // Download, or by a selection, which also downloads the bundle. Many
  // bundles share a supporting model, so only these rows, and the row of
  // the pending model, show the progress of a shared file.
  const [owners, setOwners] = useState<ReadonlySet<string>>(new Set());
  const own = (id: string) => setOwners((current) => new Set(current).add(id));
  /// Download the missing files of the bundle of `id` (R10).
  const download = async (id: string) => {
    own(id);
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
  const modelName = (id: string) => models.find((m) => m.id === id)?.name ?? id;
  const active = Object.values(live).some(
    (v) => v.kind === "progress" || v.kind === "verifying",
  );
  const chosen = settings.chosen_recognition;
  const waiting = settings.recognition.differs && !settings.recognition.chosen_not_known;
  const failure = waiting ? settings.recognition.failure : null;
  const pending = (m: ModelStatus) => waiting && !failure && chosen.asr_model === m.id;
  const readOnly = readOnlyReason(settings);
  // A chosen configuration that this pane did not send, for example from
  // another computer, shows in new choices (R20). The pane's own choice
  // keeps the choices, and the focus, where they are. A configuration that
  // this pane sent counts as its own only until the settings show it, so a
  // later change back to it from another computer shows too.
  const chosenKey = configKey(recognitionDefaults(settings));
  const ownSaves = useRef(new OwnSaves<string>());
  const [optionsKey, setOptionsKey] = useState(chosenKey);
  const shownKey = useRef(chosenKey);
  useEffect(() => {
    if (chosenKey === shownKey.current) return;
    shownKey.current = chosenKey;
    if (!ownSaves.current.confirm(chosenKey)) setOptionsKey(chosenKey);
  }, [chosenKey]);
  // The progress of the files that the chosen configuration waits for.
  const waitingUpdates = settings.recognition.waiting_for
    .map((id) => live[id])
    .filter((u): u is DownloadUpdate => Boolean(u));
  const waitingBytes = waitingUpdates.reduce(
    (sum, u) => (u.kind === "progress" ? { done: sum.done + u.downloaded, total: sum.total + u.total } : sum),
    { done: 0, total: 0 },
  );
  const waitingProgress = waitingBytes.total > 0 ? waitingBytes.done / waitingBytes.total : null;
  const waitingText =
    waitingProgress !== null
      ? `Downloading… ${formatBytes(waitingBytes.done)} of ${formatBytes(waitingBytes.total)}`
      : waitingUpdates.some((u) => u.kind === "verifying")
        ? "Checking download…"
        : "Waiting to download…";
  const retry = async () => {
    setBusy(true);
    try {
      await commands.retryRecognitionDownloads();
      setError(null);
      await actions.refresh();
    } catch {
      setError("Could not start the model download.");
    } finally {
      setBusy(false);
    }
  };
  const updatesOf = (m: ModelStatus) =>
    (owners.has(m.id) || pending(m) ? m.bundle : [m.id])
      .map((id) => live[id])
      .filter((u): u is DownloadUpdate => Boolean(u));
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
    const selected = m.id === settings.asr_model || m.id === chosen.asr_model;
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
      {readOnly && (
        <p role="status" className="text-[0.85rem] text-caution">
          {readOnly} Transcription choices can’t be changed now.
        </p>
      )}
      {settings.recognition.differs && (
        <Group header="Model change">
          {settings.recognition.chosen_not_known ? (
            <Row
              label="Chosen in a newer version of Longhand"
              detail="Update Longhand on this computer to use it."
            />
          ) : failure ? (
            <Row
              label={<Status tone="negative">{modelName(chosen.asr_model)} could not download</Status>}
              detail={sentence(failure.message)}
            >
              <Button prominent disabled={busy} onClick={() => void retry()}>
                Retry
              </Button>
            </Row>
          ) : (
            <Row label={modelName(chosen.asr_model)} detail={<span role="status">{waitingText}</span>}>
              {waitingProgress !== null && <ProgressBar fraction={waitingProgress} />}
            </Row>
          )}
          <Row
            label={modelName(settings.asr_model)}
            detail={
              failure || settings.recognition.chosen_not_known
                ? "New notes use this model."
                : "New notes use this model until the download is done."
            }
          >
            <span className="text-label-secondary">In use</span>
          </Row>
        </Group>
      )}
      {saving && (
        <p role="status" className="px-4 text-label-secondary">
          Saving…
        </p>
      )}
      {(error || defaults.error) && <ErrorText>{error || defaults.error}</ErrorText>}
      <RecognitionOptions
        key={optionsKey}
        models={models}
        initial={recognitionDefaults(settings)}
        onResolved={() => {}}
        onChanged={(next) => {
          setError(null);
          own(next.config.asr_model);
          const saves = ownSaves.current;
          saves.sent(configKey(next.config));
          void defaults.save(next.config).finally(() => saves.settled());
        }}
        disabled={readOnly !== null}
        modelAction={modelAction}
        modelActivity={modelActivity}
      />
    </Pane>
  );
}
