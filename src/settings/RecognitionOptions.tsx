import { type ReactNode, useEffect, useRef, useState } from "react";
import {
  commands,
  type ExecutionSpec,
  type ModelStatus,
  type ProcessingMode,
  type RecognitionConfig,
  type Settings,
  type SpeakerSource,
} from "../bindings";
import { Group, fieldClass, formatBytes } from "./ui";
import { RecognitionModelPicker } from "./RecognitionModelPicker";

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
import { languageName } from "./languages";
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
  onChanged,
  modelAction,
  modelActivity,
}: {
  models: ModelStatus[];
  initial: RecognitionConfig;
  onResolved: (spec: ExecutionSpec | null) => void;
  onChanged?: (spec: ExecutionSpec) => void;
  modelAction?: (model: ModelStatus) => ReactNode;
  modelActivity?: (model: ModelStatus) => ReactNode;
}) {
  const [draft, setDraft] = useState(initial);
  const edited = useRef(false);
  const invalidate = () => {
    edited.current = true;
    onResolved(null);
  };
  const publish = (spec: ExecutionSpec) => {
    onResolved(spec);
    if (edited.current) {
      edited.current = false;
      onChanged?.(spec);
    }
  };
  const [engine, setEngine] = useState<SpeakerSource | null>(() => {
    const preferred = models.find((m) => m.id === initial.asr_model)
      ?.speaker_preferences?.[
      initial.speaker_processing_mode ?? "after_stop"
    ]?.[0];
    return initial.speaker_source === "off" ||
      initial.speaker_source === preferred
      ? null
      : initial.speaker_source;
  });
  const [advanced, setAdvanced] = useState(false);
  const [valid, setValid] = useState<ExecutionSpec[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
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
      const score = (s: ExecutionSpec) => choiceScore(s, draft);
      const best = choices
        .filter(
          (s) =>
            eligible(s) &&
            (s.config.speaker_source === "off") ===
              (draft.speaker_source === "off"),
        )
        .sort((a, b) => score(b) - score(a))[0];
      setLoading(false);
      setError(
        best
          ? null
          : "No compatible choice. Change the model or expected speaker count.",
      );
      if (best) {
        if (
          best.config.processing_mode !== draft.processing_mode ||
          (best.config.speaker_source === "off") !==
            (draft.speaker_source === "off") ||
          best.config.speaker_processing_mode !== draft.speaker_processing_mode
        ) {
          const rejected = results.find(
            (r) =>
              r.config.speaker_source === draft.speaker_source &&
              r.config.processing_mode === draft.processing_mode &&
              r.config.speaker_processing_mode ===
                draft.speaker_processing_mode,
          );
          setNotice(
            `The processing choices changed to match this model and speaker count. ${(rejected?.error ?? "").replace(draft.asr_model, selected?.name ?? "This model")}`,
          );
        }
        setDraft(best.config);
        publish(best);
      }
    });
    return () => {
      active = false;
    };
  }, [
    draft.asr_model,
    JSON.stringify(draft.speaker_count),
    draft.language,
    engine,
  ]);

  function choiceScore(spec: ExecutionSpec, wanted: RecognitionConfig) {
    const c = spec.config;
    const prefs = selected?.speaker_preferences?.[
      c.speaker_processing_mode ?? "after_stop"
    ] ?? [initial.speaker_source];
    const rank = prefs.indexOf(c.speaker_source);
    const preferred = engine ? c.speaker_source === engine : rank >= 0;
    return (
      Number(
        (c.speaker_source === "off") === (wanted.speaker_source === "off"),
      ) *
        1000 +
      Number(c.processing_mode === wanted.processing_mode) * 100 +
      Number(c.speaker_processing_mode === wanted.speaker_processing_mode) *
        50 +
      Number(preferred) * 20 -
      (rank >= 0 ? rank : 10)
    );
  }
  const eligible = (spec: ExecutionSpec) =>
    spec.config.speaker_source === "off" ||
    (engine
      ? spec.config.speaker_source === engine
      : (
          selected?.speaker_preferences?.[
            spec.config.speaker_processing_mode ?? "after_stop"
          ] ?? [initial.speaker_source]
        ).includes(spec.config.speaker_source));
  const chooseMode = (
    field: "processing_mode" | "speaker_processing_mode",
    value: string,
  ) => {
    const off =
      field === "speaker_processing_mode"
        ? value === "off"
        : draft.speaker_source === "off";
    const wanted = {
      ...draft,
      [field]: value === "off" ? "after_stop" : value,
      speaker_source: off
        ? "off"
        : draft.speaker_source === "off"
          ? "nemotron"
          : draft.speaker_source,
    } as RecognitionConfig;
    const candidates = valid.filter(
      (s) =>
        eligible(s) &&
        (s.config.speaker_source === "off") === off &&
        s.config[field] === wanted[field],
    );
    const next = candidates.sort(
      (a, b) => choiceScore(b, wanted) - choiceScore(a, wanted),
    )[0];
    if (next) {
      setDraft(next.config);
      edited.current = true;
      publish(next);
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
  const missingArtifacts =
    current?.artifacts.filter(
      (artifact) =>
        models.find((model) => model.id === artifact.id)?.state.state !==
        "present",
    ) ?? [];
  const downloadBytes = missingArtifacts.reduce((sum, artifact) => {
    const state = models.find((model) => model.id === artifact.id)?.state;
    const partial = state?.state === "absent" ? state.partial_bytes : 0;
    return sum + Math.max(0, artifact.size_bytes - partial);
  }, 0);
  const canMode = (
    field: "processing_mode" | "speaker_processing_mode",
    value: string,
  ) =>
    valid.some(
      (s) =>
        eligible(s) &&
        s.config[field] === value &&
        (field !== "processing_mode" ||
          (s.config.speaker_source === "off") ===
            (draft.speaker_source === "off")) &&
        (field !== "speaker_processing_mode" ||
          (s.config.speaker_source !== "off" &&
            s.config.processing_mode === draft.processing_mode)),
    );
  return (
    <div className="grid min-w-0 gap-6">
      <Group header="Transcription model">
        <div className="p-4">
          <RecognitionModelPicker
            models={models}
            modelAction={modelAction}
            modelActivity={modelActivity}
            value={draft.asr_model}
            onChange={(asr_model) => {
              if (asr_model === draft.asr_model) return;
              invalidate();
              setEngine(null);
              setDraft({ ...draft, asr_model });
            }}
          />
        </div>
      </Group>
      <Group header="Processing options">
        <div className="grid gap-3 p-4">
          <div className="grid gap-3 sm:grid-cols-2">
            <label className="grid gap-1">
              Transcription timing
              <select
                aria-label="Transcription timing"
                className={fieldClass}
                value={draft.processing_mode}
                disabled={loading}
                onChange={(e) => chooseMode("processing_mode", e.target.value)}
              >
                {timings.map(([id, text]) => (
                  <option
                    key={id}
                    value={id}
                    disabled={!canMode("processing_mode", id)}
                  >
                    {text}
                  </option>
                ))}
              </select>
            </label>
            <label className="grid gap-1">
              Speaker recognition
              <select
                aria-label="Speaker recognition"
                className={fieldClass}
                value={
                  draft.speaker_source === "off"
                    ? "off"
                    : draft.speaker_processing_mode
                }
                disabled={loading}
                onChange={(e) =>
                  chooseMode("speaker_processing_mode", e.target.value)
                }
              >
                <option value="off">Off</option>
                {timings.map(([id, text]) => (
                  <option
                    key={id}
                    value={id}
                    disabled={!canMode("speaker_processing_mode", id)}
                  >
                    {text}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <p className="text-[0.85rem] text-label-secondary">
            Work done during recording reduces the work left after Stop. Speaker
            labels distinguish voices; they do not identify people.
          </p>
          {current && (
            <p className="text-[0.85rem] text-label-secondary">
              {missingArtifacts.length === 0
                ? "Models ready"
                : downloadBytes === 0
                  ? "Downloaded models need verification"
                  : `Download needed: ${formatBytes(downloadBytes)}`}
              {current.capabilities.maximum_speakers != null &&
                ` · Up to ${current.capabilities.maximum_speakers} speakers`}
            </p>
          )}
          {draft.speaker_source === "integrated" && (
            <p className="text-[0.85rem] text-label-secondary">
              Speaker labels can change between processing sections.
            </p>
          )}
          <details onToggle={(e) => setAdvanced(e.currentTarget.open)}>
            <summary className="cursor-pointer text-[0.85rem]">
              Advanced options
            </summary>
            {advanced && (
              <div className="mt-3 grid gap-3">
                <label className="grid gap-1">
                  Speaker model override
                  <select
                    aria-label="Speaker model override"
                    className={fieldClass}
                    value={engine ?? "automatic"}
                    disabled={loading}
                    onChange={(e) => {
                      invalidate();
                      setEngine(
                        e.target.value === "automatic"
                          ? null
                          : (e.target.value as SpeakerSource),
                      );
                    }}
                  >
                    <option value="automatic">
                      Automatic (from model catalog)
                    </option>
                    {sources
                      .filter(([id]) => id !== "off")
                      .map(([id, label]) => (
                        <option
                          key={id}
                          value={id}
                          disabled={
                            !valid.some((s) => s.config.speaker_source === id)
                          }
                        >
                          {label}
                        </option>
                      ))}
                  </select>
                </label>
                <label className="grid gap-1">
                  Language
                  <select
                    aria-label="Language"
                    className={fieldClass}
                    value={draft.language ?? ""}
                    onChange={(e) => {
                      invalidate();
                      setDraft({ ...draft, language: e.target.value || null });
                    }}
                  >
                    <option value="">Automatic</option>
                    {draft.language &&
                      !selected?.languages.includes(draft.language) && (
                        <option value={draft.language} disabled>
                          {draft.language} (not supported by this model)
                        </option>
                      )}
                    {selected?.languages.map((code) => (
                      <option key={code} value={code}>
                        {languageName(code)}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="grid gap-1">
                  Expected speakers
                  <select
                    aria-label="Expected speakers"
                    className={fieldClass}
                    value={draft.speaker_count?.kind ?? "automatic"}
                    onChange={(e) => {
                      invalidate();
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
                      invalidate();
                      setDraft({
                        ...draft,
                        speaker_count: {
                          kind: "exact",
                          count: Number(e.target.value),
                        },
                      });
                    }}
                  />
                )}
                {draft.speaker_count?.kind === "range" && (
                  <div className="flex gap-2">
                    {(["min", "max"] as const).map((field) => (
                      <input
                        key={field}
                        aria-label={
                          field === "min" ? "Fewest speakers" : "Most speakers"
                        }
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
                            invalidate();
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
                        Speaker labels can change between processing sections.
                        Labels from different sections do not identify the same
                        voice.
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
              </div>
            )}
          </details>
          {loading && <p role="status">Checking compatible choices…</p>}
          {notice && <p role="status">{notice}</p>}
          {error && (
            <p role="alert" className="text-negative">
              {error}
            </p>
          )}
        </div>
      </Group>
    </div>
  );
}
