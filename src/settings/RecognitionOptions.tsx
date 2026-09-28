import { useEffect, useState } from "react";
import {
  commands,
  type ExecutionSpec,
  type ModelStatus,
  type ProcessingMode,
  type RecognitionConfig,
  type Settings,
  type SpeakerSource,
} from "../bindings";
import { fieldClass, formatBytes } from "./ui";

const sources: [SpeakerSource, string][] = [
  ["off", "Off"],
  ["integrated", "Transcription model labels"],
  ["pyannote", "pyannote / WeSpeaker"],
  ["sortformer", "Sortformer"],
  ["nemotron", "Nemotron 3"],
];
const timings: [ProcessingMode, string][] = [
  ["during_recording", "During recording"],
  ["after_stop", "After Stop"],
];
const languageNames = new Intl.DisplayNames(["en"], { type: "language" });
const languages = (codes: string[]) =>
  codes.length > 2
    ? `${codes.length} languages`
    : codes.map((code) => languageNames.of(code) ?? code).join(" and ");
export function recognitionDefaults(settings: Settings): RecognitionConfig {
  return {
    asr_model: settings.asr_model,
    speaker_source: settings.speaker_source,
    processing_mode: settings.live_transcript
      ? "during_recording"
      : "after_stop",
    speaker_processing_mode: settings.speaker_processing_mode,
    speaker_count: settings.default_speaker_count,
    language: settings.recognition_language,
  };
}

export function RecognitionOptions({
  models,
  initial,
  onResolved,
}: {
  models: ModelStatus[];
  initial: RecognitionConfig;
  onResolved: (spec: ExecutionSpec | null) => void;
}) {
  const [draft, setDraft] = useState(initial);
  const [valid, setValid] = useState<ExecutionSpec[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [unavailable, setUnavailable] = useState<Record<string, string>>({});
  const selected = models.find((m) => m.id === draft.asr_model);
  useEffect(() => {
    let active = true;
    setLoading(true);
    setNotice(null);
    onResolved(null);
    // Resolve each dependent choice against the exact backend catalog.
    void Promise.all(
      sources.flatMap(([speaker_source]) =>
        timings.flatMap(([processing_mode]) =>
          timings.map(async ([speaker_processing_mode]) => {
            const config = {
              ...draft,
              speaker_source,
              processing_mode,
              speaker_processing_mode,
            };
            try {
              const result = await commands.resolveRecognition(config);
              return {
                config,
                spec: result.status === "ok" ? result.data : null,
                error: result.status === "error" ? result.error : null,
              };
            } catch {
              return {
                config,
                spec: null,
                error: "The model capabilities could not be read.",
              };
            }
          }),
        ),
      ),
    ).then((results) => {
      if (!active) return;
      const choices = results
        .map((r) => r.spec)
        .filter((s): s is ExecutionSpec => s !== null);
      setValid(choices);
      setUnavailable(
        Object.fromEntries(
          sources
            .filter(
              ([source]) =>
                !choices.some((s) => s.config.speaker_source === source),
            )
            .map(([source]) => [
              source,
              results.find(
                (r) =>
                  r.config.speaker_source === source &&
                  r.config.processing_mode === "after_stop" &&
                  r.config.speaker_processing_mode === "after_stop",
              )?.error ?? "This choice is not supported.",
            ]),
        ),
      );
      const score = (s: ExecutionSpec) =>
        Number(s.config.speaker_source === draft.speaker_source) * 4 +
        Number(s.config.processing_mode === draft.processing_mode) * 2 +
        Number(
          s.config.speaker_processing_mode === draft.speaker_processing_mode,
        );
      const best = [...choices].sort((a, b) => score(b) - score(a))[0];
      setLoading(false);
      setError(
        best
          ? null
          : "No compatible choice. Change the model or expected speaker count.",
      );
      if (best) {
        if (score(best) !== 7) {
          const rejected = results.find(
            (r) =>
              r.config.speaker_source === draft.speaker_source &&
              r.config.processing_mode === draft.processing_mode &&
              r.config.speaker_processing_mode ===
                draft.speaker_processing_mode,
          );
          setNotice(
            `The processing choices changed to match this model and speaker count. ${rejected?.error ?? ""} Review them before applying.`,
          );
        }
        setDraft(best.config);
        onResolved(best);
      }
    });
    return () => {
      active = false;
    };
  }, [draft.asr_model, JSON.stringify(draft.speaker_count), draft.language]);

  const choose = (
    field: "speaker_source" | "processing_mode" | "speaker_processing_mode",
    value: string,
  ) => {
    const candidates = valid.filter((s) => s.config[field] === value);
    const score = (s: ExecutionSpec) =>
      Number(s.config.speaker_source === draft.speaker_source) * 4 +
      Number(s.config.processing_mode === draft.processing_mode) * 2 +
      Number(
        s.config.speaker_processing_mode === draft.speaker_processing_mode,
      );
    const next = candidates.sort((a, b) => score(b) - score(a))[0];
    if (next) {
      setDraft(next.config);
      onResolved(next);
      setNotice(null);
    }
  };
  const current = loading
    ? undefined
    : valid.find(
        (s) =>
          s.config.asr_model === draft.asr_model &&
          s.config.speaker_source === draft.speaker_source &&
          s.config.processing_mode === draft.processing_mode &&
          s.config.speaker_processing_mode === draft.speaker_processing_mode,
      );
  const can = (
    field: "speaker_source" | "processing_mode" | "speaker_processing_mode",
    value: string,
  ) =>
    valid.some(
      (s) =>
        s.config[field] === value &&
        (field === "speaker_source" ||
          s.config.speaker_source === draft.speaker_source) &&
        (field !== "speaker_processing_mode" ||
          s.config.processing_mode === draft.processing_mode),
    );
  const assessment = selected?.assessment;
  return (
    <div className="grid min-w-0 gap-3 p-4">
      <label className="grid min-w-0 gap-1">
        Transcription model
        <select
          aria-label="Transcription model"
          className={`${fieldClass} min-w-0 w-full max-w-full`}
          value={draft.asr_model}
          onChange={(e) => {
            onResolved(null);
            setDraft({ ...draft, asr_model: e.target.value });
          }}
        >
          {models
            .filter((m) => m.role === "recognition")
            .map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
        </select>
      </label>
      <p className="text-[0.85rem] text-label-secondary">
        {languages(selected?.languages ?? [])} ·{" "}
        {formatBytes(
          current?.artifacts.reduce((sum, a) => sum + a.size_bytes, 0) ??
            selected?.size_bytes ??
            0,
        )}
      </p>
      <label className="grid gap-1">
        Language
        <select
          aria-label="Language"
          className={fieldClass}
          value={draft.language ?? ""}
          onChange={(e) => {
            onResolved(null);
            setDraft({ ...draft, language: e.target.value || null });
          }}
        >
          <option value="">Automatic</option>
          {draft.language && !selected?.languages.includes(draft.language) && (
            <option value={draft.language} disabled>
              {draft.language} (not supported by this model)
            </option>
          )}
          {selected?.languages.map((code) => (
            <option key={code} value={code}>
              {languageNames.of(code) ?? code}
            </option>
          ))}
        </select>
      </label>
      <label className="grid gap-1">
        Speaker recognition
        <select
          aria-label="Speaker recognition"
          className={fieldClass}
          value={draft.speaker_source}
          disabled={loading}
          onChange={(e) => choose("speaker_source", e.target.value)}
        >
          {sources.map(([id, label]) => (
            <option
              key={id}
              value={id}
              title={unavailable[id]}
              disabled={!can("speaker_source", id)}
            >
              {label}
            </option>
          ))}
        </select>
      </label>
      {Object.keys(unavailable).length > 0 && (
        <details className="text-[0.85rem] text-label-secondary">
          <summary>Unavailable speaker choices</summary>
          {sources
            .filter(([id]) => unavailable[id])
            .map(([id, label]) => (
              <p key={id}>
                {label}: {unavailable[id]}
              </p>
            ))}
        </details>
      )}
      <p className="text-[0.85rem] text-label-secondary">
        Anonymous labels distinguish voices. They do not identify people.
      </p>
      {(
        [
          ["processing_mode", "Transcription timing"],
          ["speaker_processing_mode", "Speaker timing"],
        ] as const
      ).map(([field, label]) => (
        <label key={field} className="grid gap-1">
          {label}
          <select
            aria-label={label}
            className={fieldClass}
            value={draft[field]}
            disabled={
              loading ||
              (field === "speaker_processing_mode" &&
                draft.speaker_source === "off")
            }
            onChange={(e) => choose(field, e.target.value)}
          >
            {timings.map(([id, text]) => (
              <option key={id} value={id} disabled={!can(field, id)}>
                {text}
              </option>
            ))}
          </select>
        </label>
      ))}
      <label className="grid gap-1">
        Expected speakers
        <select
          aria-label="Expected speakers"
          className={fieldClass}
          value={draft.speaker_count?.kind ?? "automatic"}
          onChange={(e) => {
            onResolved(null);
            setDraft({
              ...draft,
              speaker_count:
                e.target.value === "automatic"
                  ? null
                  : e.target.value === "exact"
                    ? { kind: "exact", count: 4 }
                    : { kind: "range", min: 2, max: 6 },
            });
          }}
        >
          <option value="automatic">Automatic</option>
          <option value="exact">Exactly</option>
          <option value="range">Between</option>
        </select>
      </label>
      {draft.speaker_count?.kind === "exact" && (
        <input
          aria-label="Exact speaker count"
          className={fieldClass}
          type="number"
          min={1}
          max={32}
          value={draft.speaker_count.count}
          onChange={(e) => {
            onResolved(null);
            setDraft({
              ...draft,
              speaker_count: { kind: "exact", count: Number(e.target.value) },
            });
          }}
        />
      )}
      {draft.speaker_count?.kind === "range" && (
        <div className="flex gap-2">
          {(["min", "max"] as const).map((field) => (
            <input
              key={field}
              aria-label={field === "min" ? "Fewest speakers" : "Most speakers"}
              className={`${fieldClass} min-w-0 w-24`}
              type="number"
              min={1}
              max={32}
              value={
                draft.speaker_count?.kind === "range"
                  ? draft.speaker_count[field]
                  : 0
              }
              onChange={(e) => {
                if (draft.speaker_count?.kind === "range") {
                  onResolved(null);
                  setDraft({
                    ...draft,
                    speaker_count: {
                      ...draft.speaker_count,
                      [field]: Number(e.target.value),
                    },
                  });
                }
              }}
            />
          ))}
        </div>
      )}
      {current && (
        <div className="text-[0.85rem] text-label-secondary">
          <p>
            Live transcription:{" "}
            {current.capabilities.live_transcription
              ? "Supported"
              : "Not supported"}
          </p>
          <p>
            Live speaker analysis:{" "}
            {current.capabilities.live_speaker_analysis
              ? "Supported"
              : "Not supported"}
          </p>
          <p>
            Live text with speaker labels:{" "}
            {current.capabilities.live_attributed_text
              ? "Supported"
              : "Not supported"}
          </p>
          <p>
            Speaker limit:{" "}
            {current.capabilities.maximum_speakers ?? "Not specified"}
          </p>
          {draft.speaker_source === "integrated" && (
            <p>
              Speaker labels can change between processing sections. Labels from
              different sections do not identify the same voice.
            </p>
          )}
          <details>
            <summary>Downloaded artifact details</summary>
            {current.artifacts.map((a) => (
              <p key={a.id} className="[overflow-wrap:anywhere]">
                {a.filename} · {a.format} · {a.revision}
              </p>
            ))}
          </details>
        </div>
      )}
      <div className="text-[0.85rem] text-label-secondary">
        <p>
          Speed rating:{" "}
          {assessment?.speed_score == null
            ? "Unknown"
            : `${assessment.speed_score}/100`}
        </p>
        <p>
          Transcription accuracy rating:{" "}
          {assessment?.asr_accuracy_score == null
            ? "Unknown"
            : `${assessment.asr_accuracy_score}/100`}
        </p>
        <p>
          Speaker accuracy: {assessment?.speaker_accuracy_score ?? "Unknown"}
        </p>
        {assessment && (
          <details>
            <summary>Handy assessments and source</summary>
            <p className="[overflow-wrap:anywhere]">
              {assessment.source} · {assessment.source_revision}.{" "}
              {assessment.benchmark_context ??
                "Benchmark context not published."}{" "}
              Ratings do not measure speaker accuracy.
            </p>
          </details>
        )}
      </div>
      {loading && <p role="status">Checking compatible choices…</p>}
      {notice && <p role="status">{notice}</p>}
      {error && (
        <p role="alert" className="text-negative">
          {error}
        </p>
      )}
    </div>
  );
}
