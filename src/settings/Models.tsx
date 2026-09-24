import { useEffect, useState } from "react";
import {
  commands,
  type DownloadUpdate,
  type ModelRole,
  type ModelStatus,
  type Settings,
  type SpeakerCountSetting,
} from "../bindings";
import { Button, ErrorText, Group, Pane, ProgressBar, Row, Status, fieldClass, formatBytes } from "./ui";

export const roleLabel: Record<ModelRole, string> = {
  recognition: "Speech recognition",
  segmentation: "Speaker segmentation",
  embedding: "Speaker embedding",
};

export type Actions = { download: (ids: string[]) => void; cancel: () => void };

function ModelRow({
  model,
  live,
  actions,
}: {
  model: ModelStatus;
  live: DownloadUpdate | undefined;
  actions: Actions;
}) {
  const downloading = model.state.state === "downloading";
  const partial = model.state.state === "absent" ? model.state.partial_bytes : 0;

  let trailing;
  if (model.state.state === "present") {
    trailing = <Status tone="positive">Downloaded</Status>;
  } else if (downloading) {
    const progress = live?.kind === "progress" ? live : null;
    trailing = (
      <>
        {live?.kind === "verifying" ? (
          <span className="text-label-secondary">Verifying…</span>
        ) : (
          <ProgressBar fraction={progress ? progress.downloaded / progress.total : partial / model.size_bytes} />
        )}
        <Button onClick={actions.cancel}>Cancel</Button>
      </>
    );
  } else {
    trailing = (
      <Button onClick={() => actions.download([model.id])}>
        {partial > 0 ? "Resume" : "Download"}
      </Button>
    );
  }

  const detail = [roleLabel[model.role], formatBytes(model.size_bytes)];
  if (!downloading && partial > 0) detail.push(`${formatBytes(partial)} so far`);

  return (
    <>
      <Row label={model.name} detail={detail.join(" · ")}>
        {trailing}
      </Row>
      {live?.kind === "failed" && !downloading && <ErrorText>{live.message}</ErrorText>}
    </>
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

  const apply = async (next: SpeakerCountSetting | null) => {
    const result = await commands.setDefaultSpeakerCount(next);
    if (result.status === "error") setError(result.error);
    else {
      setError(null);
      onSettings(result.data);
    }
  };

  const choose = (next: CountMode) => {
    setMode(next);
    if (next === "automatic") void apply(null);
    else if (next === "exact") void apply({ kind: "exact", count: exact });
    else void apply({ kind: "range", min, max });
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
      footer="A count is never an identity: sessions record how many voices to expect, not who they are. Each session can override this before it is processed."
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
          {number(exact, setExact, (n) => void apply({ kind: "exact", count: n }), "Exact speaker count")}
        </Row>
      )}
      {mode === "range" && (
        <Row label="Speakers">
          {number(min, setMin, (n) => void apply({ kind: "range", min: n, max }), "Fewest speakers")}
          <span className="text-label-secondary">to</span>
          {number(max, setMax, (n) => void apply({ kind: "range", min, max: n }), "Most speakers")}
        </Row>
      )}
      {error && <ErrorText>{error}</ErrorText>}
    </Group>
  );
}

export function Models({
  models,
  live,
  actions,
  settings,
  onSettings,
}: {
  models: ModelStatus[];
  live: Record<string, DownloadUpdate>;
  actions: Actions;
  settings: Settings;
  onSettings: (s: Settings) => void;
}) {
  const required = models.filter((m) => m.required);
  const missing = required.filter((m) => m.state.state === "absent");
  const remaining = missing.reduce((sum, m) => sum + m.size_bytes, 0);

  return (
    <Pane title="Recognition">
      <Group
        header="Models"
        footer={
          missing.length > 0
            ? `Recognition and speaker detection run on this machine, so these weights are downloaded once from their publishers (${formatBytes(remaining)}). Recording works without them; processing waits until they are here.`
            : "Recognition and speaker detection run entirely on this machine. Session audio never leaves it."
        }
      >
        {required.map((m) => (
          <ModelRow key={m.id} model={m} live={live[m.id]} actions={actions} />
        ))}
        {missing.length > 1 && (
          <Row label={`${missing.length} models to download`}>
            <Button prominent onClick={() => actions.download(missing.map((m) => m.id))}>
              Download All
            </Button>
          </Row>
        )}
      </Group>

      <SpeakerCount settings={settings} onSettings={onSettings} />
    </Pane>
  );
}
