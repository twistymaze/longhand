// The Transcription pane: the recognition model picker (R4, R5, R6), the
// Live transcript switch (R18), one row for the speaker-detection weights
// (R7) and the default number of speakers.

import { type MouseEvent, useEffect, useRef, useState } from "react";
import {
  commands,
  type DownloadUpdate,
  type ModelRole,
  type ModelStatus,
  type Settings,
  type SpeakerCountSetting,
  type SpeakerModelsStatus,
} from "../bindings";
import { sentence } from "../processing/describe";
import { CheckIcon } from "./icons";
import {
  Button,
  ErrorText,
  Group,
  Pane,
  ProgressBar,
  Row,
  Status,
  Switch,
  Tag,
  fieldClass,
  formatBytes,
} from "./ui";

export const roleLabel: Record<ModelRole, string> = {
  recognition: "Speech recognition",
  segmentation: "Speaker segmentation",
  embedding: "Speaker embedding",
};

export type Actions = {
  download: (ids: string[]) => void;
  cancel: () => void;
  /// Pick a recognition model. Resolves to why the pick failed, or null.
  select: (id: string) => Promise<string | null>;
  /// Read the model statuses again.
  refresh: () => Promise<void>;
};

const FAILED = "Couldn't download. Try again.";

const languageNames = new Intl.DisplayNames(["en"], { type: "language" });

/// "English", "English and Chinese", or "28 languages".
export function describeLanguages(codes: string[]): string {
  if (codes.length === 0) return "";
  if (codes.length > 2) return `${codes.length} languages`;
  return codes.map((c) => languageNames.of(c) ?? c).join(" and ");
}

/// A download in progress for one model, from its live events or its state.
function progressOf(model: ModelStatus, live: DownloadUpdate | undefined) {
  const active = live?.kind === "progress" || live?.kind === "verifying";
  const downloading = active || model.state.state === "downloading";
  const downloaded =
    live?.kind === "progress"
      ? live.downloaded
      : model.state.state === "present"
        ? model.size_bytes
        : model.state.state === "absent"
          ? model.state.partial_bytes
          : 0;
  const total = live?.kind === "progress" ? live.total : model.size_bytes;
  const partial = model.state.state === "absent" && model.state.partial_bytes > 0;
  return { downloading, checking: live?.kind === "verifying", downloaded, total, partial };
}

function Checking() {
  return <span className="text-label-secondary">Checking…</span>;
}

function ModelOption({
  model,
  live,
  selected,
  pending,
  actions,
}: {
  model: ModelStatus;
  live: DownloadUpdate | undefined;
  selected: boolean;
  pending: boolean;
  actions: Actions;
}) {
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const cancelDelete = useRef<HTMLButtonElement>(null);
  const { downloading, checking, downloaded, total, partial } = progressOf(model, live);
  const present = model.state.state === "present";
  const failed = live?.kind === "failed" && !downloading;

  useEffect(() => {
    if (confirming) cancelDelete.current?.focus();
  }, [confirming]);

  const pick = async () => {
    setError(null);
    setConfirming(false);
    setError(await actions.select(model.id));
  };

  const remove = async () => {
    const result = await commands.deleteModel(model.id);
    setConfirming(false);
    if (result.status === "error") setError(sentence(result.error));
    else {
      setError(null);
      await actions.refresh();
    }
  };

  // A click on a control in the row does not also pick the model.
  const only = (f: () => void) => (e: MouseEvent) => {
    e.stopPropagation();
    f();
  };

  let trailing = null;
  if (downloading) {
    trailing = (
      <>
        {checking ? <Checking /> : <ProgressBar fraction={downloaded / total} />}
        <Button onClick={only(actions.cancel)}>Cancel</Button>
      </>
    );
  } else if (confirming) {
    trailing = (
      <>
        <Button ref={cancelDelete} onClick={only(() => setConfirming(false))}>
          Cancel
        </Button>
        <Button destructive onClick={only(() => void remove())}>
          Delete
        </Button>
      </>
    );
  } else if (present && !selected) {
    trailing = <Button onClick={only(() => setConfirming(true))}>Delete…</Button>;
  } else if (!present && selected) {
    trailing = (
      <Button prominent onClick={only(() => void pick())}>
        {partial ? "Resume" : "Download"}
      </Button>
    );
  }

  const meta = [describeLanguages(model.languages), formatBytes(model.size_bytes)].filter(Boolean);
  if (present) meta.push("Downloaded");
  let note: string | null = null;
  if (confirming) note = `Delete this model to free ${formatBytes(model.size_bytes)}? You can download it again later.`;
  else if (downloading && pending) note = "Longhand switches to this model when the download is done.";

  return (
    <div
      data-model={model.id}
      // While Delete asks, a click on the row only closes the question.
      onClick={() => (confirming ? setConfirming(false) : void pick())}
      onKeyDown={(e) => {
        if (e.key === "Escape" && confirming) setConfirming(false);
      }}
      className="settings-row flex cursor-default items-stretch pl-2 hover:bg-row-hover active:bg-fill"
    >
      {/* The check mark is on the line of the model name. */}
      <span aria-hidden className="flex w-6 shrink-0 justify-center pt-[13px] text-accent">
        {selected && <CheckIcon className="size-[14px]" />}
      </span>
      <div className="flex min-h-[3.25rem] min-w-0 flex-1 items-center gap-4 py-2.5 pr-4 pl-1">
        <button
          type="button"
          role="radio"
          aria-checked={selected}
          className="min-w-0 flex-1 text-left outline-offset-2 focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-accent"
        >
          <span className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
            <span className="font-medium">{model.name}</span>
            {model.streaming && <Tag accent>Live</Tag>}
            {model.recommended && <Tag>Recommended</Tag>}
          </span>
          <span className="mt-0.5 block text-[0.85rem] leading-[1.4] text-label-secondary">
            {meta.join(" · ")}
          </span>
          {note && <span className="mt-0.5 block text-[0.85rem] leading-[1.4] text-label-secondary">{note}</span>}
          {(failed || error) && (
            <span className="mt-0.5 block text-[0.85rem] leading-[1.4] text-negative">{error ?? FAILED}</span>
          )}
        </button>
        {trailing && <div className="flex shrink-0 items-center gap-2">{trailing}</div>}
      </div>
    </div>
  );
}

/// The speaker-detection weights as one row (R7). The row never names the
/// individual weights.
function OtherModels({
  speakers,
  models,
  live,
  actions,
}: {
  speakers: SpeakerModelsStatus;
  models: ModelStatus[];
  live: Record<string, DownloadUpdate>;
  actions: Actions;
}) {
  const weights = models.filter((m) => speakers.ids.includes(m.id));
  const parts = weights.map((m) => progressOf(m, live[m.id]));
  const downloading = speakers.state.state === "downloading" || parts.some((p) => p.downloading);
  // One progress for both weights: a present weight counts in full.
  const downloaded = parts.reduce((sum, p) => sum + p.downloaded, 0);
  const total = parts.reduce((sum, p) => sum + p.total, 0) || speakers.size_bytes;
  const checking = parts.some((p) => p.checking) && !weights.some((m) => live[m.id]?.kind === "progress");
  const failed = !downloading && weights.some((m) => live[m.id]?.kind === "failed");
  const present = speakers.state.state === "present";
  const partial = parts.some((p) => p.partial);

  let trailing;
  if (present) trailing = <Status tone="positive">Downloaded</Status>;
  else if (downloading)
    trailing = (
      <>
        {checking ? <Checking /> : <ProgressBar fraction={downloaded / total} />}
        <Button onClick={actions.cancel}>Cancel</Button>
      </>
    );
  else
    trailing = (
      <Button prominent onClick={() => actions.download(speakers.ids)}>
        {partial ? "Resume" : "Download"}
      </Button>
    );

  const detail = ["Tells the speakers apart", formatBytes(speakers.size_bytes)];
  if (!present && !downloading && partial) detail.push(`${formatBytes(downloaded)} so far`);

  return (
    <Group
      footer={
        present
          ? "Transcription and speaker detection run on this computer. Your audio never leaves it."
          : "Recording works without these models. Notes wait to be processed until they are downloaded."
      }
    >
      <Row label="Other required models" detail={detail.join(" · ")}>
        {trailing}
      </Row>
      {failed && <ErrorText>{FAILED}</ErrorText>}
    </Group>
  );
}

function LiveTranscript({ settings, onSettings }: { settings: Settings; onSettings: (s: Settings) => void }) {
  const [error, setError] = useState<string | null>(null);
  const change = async (enabled: boolean) => {
    const result = await commands.setLiveTranscript(enabled);
    if (result.status === "error") setError(sentence(result.error));
    else {
      setError(null);
      onSettings(result.data);
    }
  };
  return (
    <Group footer="When this is off, the transcript is made after you stop recording. This uses less power.">
      <Row label="Live transcript" detail="Show the transcript while you record">
        <Switch label="Live transcript" checked={settings.live_transcript} onChange={(v) => void change(v)} />
      </Row>
      {error && <ErrorText>{error}</ErrorText>}
    </Group>
  );
}

type CountMode = "automatic" | "exact" | "range";

function SpeakerCount({ settings, onSettings }: { settings: Settings; onSettings: (s: Settings) => void }) {
  const stored = settings.default_speaker_count;
  const [mode, setMode] = useState<CountMode>(stored ? stored.kind : "automatic");
  const [exact, setExact] = useState(stored?.kind === "exact" ? stored.count : 4);
  const [min, setMin] = useState(stored?.kind === "range" ? stored.min : 2);
  const [max, setMax] = useState(stored?.kind === "range" ? stored.max : 6);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setMode(stored ? stored.kind : "automatic");
  }, [stored]);

  const save = async (next: SpeakerCountSetting | null) => {
    const result = await commands.setDefaultSpeakerCount(next);
    if (result.status === "error") setError(result.error);
    else {
      setError(null);
      onSettings(result.data);
    }
  };

  const choose = (next: CountMode) => {
    setMode(next);
    if (next === "automatic") void save(null);
    else if (next === "exact") void save({ kind: "exact", count: exact });
    else void save({ kind: "range", min, max });
  };

  const number = (value: number, set: (n: number) => void, commit: (n: number) => void, label: string) => (
    <input
      aria-label={label}
      type="number"
      min={1}
      max={32}
      className={`${fieldClass} w-14 text-right tabular-nums`}
      value={value}
      onChange={(e) => {
        const n = Number(e.target.value);
        set(n);
        if (Number.isInteger(n) && n >= 1) commit(n);
      }}
    />
  );

  const modes: { id: CountMode; label: string }[] = [
    { id: "automatic", label: "Automatic" },
    { id: "exact", label: "Exactly" },
    { id: "range", label: "Between" },
  ];

  return (
    <Group
      header="Speakers"
      footer="Longhand counts voices; it does not identify people. Each note can change this before it is processed."
    >
      <Row label="Default number of speakers">
        <div role="radiogroup" aria-label="Speaker count" className="flex rounded-md bg-fill p-0.5">
          {modes.map((m) => (
            <button
              key={m.id}
              type="button"
              role="radio"
              aria-checked={mode === m.id}
              onClick={() => choose(m.id)}
              className={`rounded-[5px] px-2.5 py-[1px] select-none ${
                mode === m.id ? "bg-control text-label shadow-[0_0.5px_1.5px_rgb(0_0_0/0.2)]" : "text-label-secondary active:bg-fill-pressed"
              }`}
            >
              {m.label}
            </button>
          ))}
        </div>
      </Row>
      {mode === "exact" && (
        <Row label="Speakers">
          {number(exact, setExact, (n) => void save({ kind: "exact", count: n }), "Exact speaker count")}
        </Row>
      )}
      {mode === "range" && (
        <Row label="Speakers">
          {number(min, setMin, (n) => void save({ kind: "range", min: n, max }), "Fewest speakers")}
          <span className="text-label-secondary">to</span>
          {number(max, setMax, (n) => void save({ kind: "range", min, max: n }), "Most speakers")}
        </Row>
      )}
      {error && <ErrorText>{error}</ErrorText>}
    </Group>
  );
}

export function Models({
  models,
  speakers,
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
  const recognition = models.filter((m) => m.role === "recognition");
  const selected = recognition.find((m) => m.id === settings.asr_model);

  return (
    <Pane title="Transcription">
      <Group
        header="Model"
        footer="When you pick a model that isn't downloaded, it downloads first. Longhand keeps the current model until the new one is ready. Models marked Live show the transcript while you record."
      >
        <div role="radiogroup" aria-label="Recognition model">
          {recognition.map((m) => (
            <ModelOption
              key={m.id}
              model={m}
              live={live[m.id]}
              selected={m.id === settings.asr_model}
              pending={m.id === settings.pending_asr_model}
              actions={actions}
            />
          ))}
        </div>
      </Group>

      {selected?.streaming && <LiveTranscript settings={settings} onSettings={onSettings} />}

      {speakers && <OtherModels speakers={speakers} models={models} live={live} actions={actions} />}

      <SpeakerCount settings={settings} onSettings={onSettings} />
    </Pane>
  );
}
