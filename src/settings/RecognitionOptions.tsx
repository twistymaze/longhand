import { type ReactNode, useEffect, useRef, useState } from "react";
import {
  commands,
  type ExecutionSpec,
  type ModelStatus,
  type ProcessingMode,
  type RecognitionChoice,
  type RecognitionConfig,
  type Settings,
  type SpeakerSource,
} from "../bindings";
import { Group, fieldClass, formatBytes } from "./ui";
import { RecognitionModelPicker } from "./RecognitionModelPicker";

const sources: [SpeakerSource, string][] = [
  ["off", "Off"],
  ["pyannote", "pyannote / WeSpeaker"],
  ["sortformer", "Sortformer"],
  ["nemotron", "Nemotron 3"],
];
const timings: [ProcessingMode, string][] = [
  ["during_recording", "During recording"],
  ["after_stop", "After Stop"],
];
import { languageName } from "./languages";
/// The chosen configuration, which the Settings window shows and changes.
/// The configuration in use can differ while the chosen one downloads.
export function recognitionDefaults(settings: Settings): RecognitionConfig {
  return choiceConfig(settings.chosen_recognition);
}

/// A recognition choice as a configuration, with the live and language
/// preferences as they are stored, also when the model cannot use them.
export function choiceConfig(chosen: RecognitionChoice): RecognitionConfig {
  return {
    asr_model: chosen.asr_model,
    speaker_source: chosen.speaker_source,
    processing_mode: chosen.live_transcript
      ? "during_recording"
      : "after_stop",
    speaker_processing_mode: chosen.speaker_processing_mode,
    speaker_count: chosen.default_speaker_count,
    language: chosen.recognition_language,
  };
}

export function RecognitionOptions({
  models,
  initial,
  onResolved,
  onChanged,
  modelAction,
  modelActivity,
  disabled = false,
}: {
  models: ModelStatus[];
  initial: RecognitionConfig;
  onResolved: (spec: ExecutionSpec | null) => void;
  /// `choice` is what the user chose: the resolved model, speaker source
  /// and count, with the live and language preferences.
  onChanged?: (spec: ExecutionSpec, choice: RecognitionChoice) => void;
  modelAction?: (model: ModelStatus) => ReactNode;
  modelActivity?: (model: ModelStatus) => ReactNode;
  /// The choices are read-only, for example while the storage folder is not
  /// available (R16).
  disabled?: boolean;
}) {
  // The language that `asr_model` uses for the preferred `language`.
  const usableLanguage = (asr_model: string, language: string | null) =>
    language &&
    models.find((m) => m.id === asr_model)?.languages.includes(language)
      ? language
      : null;
  // The live and language preferences, which their controls show. Only a
  // change of their own control changes them. A model that cannot use one
  // uses After Stop or automatic language, and a line tells the user.
  const preferences = useRef({
    processing_mode: initial.processing_mode,
    speaker_processing_mode: initial.speaker_processing_mode ?? "after_stop",
    language: initial.language,
  });
  const [shown, setShown] = useState(preferences.current);
  const prefer = (change: Partial<typeof preferences.current>) => {
    preferences.current = { ...preferences.current, ...change };
    setShown(preferences.current);
  };
  // The configuration that the selected model uses, which the controls show.
  const [draft, setDraft] = useState(() => ({
    ...initial,
    language: usableLanguage(initial.asr_model, initial.language),
  }));
  // `base` with the timing preferences, for the choice that fits them best.
  const wanted = (base: RecognitionConfig): RecognitionConfig => ({
    ...base,
    processing_mode: preferences.current.processing_mode,
    speaker_processing_mode: preferences.current.speaker_processing_mode,
  });
  const edited = useRef(false);
  const invalidate = () => {
    edited.current = true;
    onResolved(null);
  };
  const publish = (spec: ExecutionSpec) => {
    onResolved(spec);
    if (edited.current) {
      edited.current = false;
      const { processing_mode, speaker_processing_mode, language } =
        preferences.current;
      onChanged?.(spec, {
        asr_model: spec.config.asr_model,
        live_transcript: processing_mode === "during_recording",
        speaker_source: spec.config.speaker_source,
        speaker_processing_mode,
        recognition_language: language,
        default_speaker_count: spec.config.speaker_count,
      });
    }
  };
  const [engine, setEngine] = useState<SpeakerSource | null>(() => {
    const preferred = models.find((m) => m.id === initial.asr_model)
      ?.speaker_preferences;
    return initial.speaker_source === "off" ||
      initial.speaker_source === preferred?.during_recording[0] ||
      initial.speaker_source === preferred?.after_stop[0]
      ? null
      : initial.speaker_source;
  });
  // The first resolve can show that the stored speaker source is the
  // automatic one for the speaker count, and not an override.
  const checkInitialEngine = useRef(engine !== null);
  const [advanced, setAdvanced] = useState(false);
  const [valid, setValid] = useState<ExecutionSpec[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const selected = models.find((m) => m.id === draft.asr_model);
  useEffect(() => {
    let active = true;
    setLoading(true);
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
      if (checkInitialEngine.current) {
        checkInitialEngine.current = false;
        const automatic = selected?.speaker_preferences?.after_stop.find(
          (source) =>
            choices.some(
              (s) =>
                s.config.speaker_source === source &&
                s.config.speaker_processing_mode === "after_stop",
            ),
        );
        if (engine === automatic) {
          setEngine(null);
          return;
        }
      }
      setValid(choices);
      const target = wanted(draft);
      const score = (s: ExecutionSpec) => choiceScore(s, target);
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
    const target = {
      ...wanted(draft),
      [field]: value === "off" ? "after_stop" : value,
      speaker_source: off
        ? "off"
        : draft.speaker_source === "off"
          ? "nemotron"
          : draft.speaker_source,
    } as RecognitionConfig;
    const candidates = valid.filter(
      (s) => eligible(s) && (s.config.speaker_source === "off") === off,
    );
    // The best choice with the chosen timing. When none has it, the timing
    // stays a preference and the best choice without it applies.
    const next = candidates.sort(
      (a, b) =>
        Number(b.config[field] === target[field]) -
          Number(a.config[field] === target[field]) ||
        choiceScore(b, target) - choiceScore(a, target),
    )[0];
    if (next) {
      // Off is a speaker source, not a timing: the timing preference stays.
      if (value !== "off") prefer({ [field]: value as ProcessingMode });
      setDraft(next.config);
      edited.current = true;
      publish(next);
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
  // The live preferences that the current choice cannot use.
  const transcriptAfterStop =
    !!current &&
    shown.processing_mode === "during_recording" &&
    draft.processing_mode !== "during_recording";
  const speakersAfterStop =
    !!current &&
    draft.speaker_source !== "off" &&
    shown.speaker_processing_mode === "during_recording" &&
    draft.speaker_processing_mode !== "during_recording";
  // What makes speaker recognition run after Stop: the speaker model
  // override, the transcription model, or the expected speaker count.
  const model = selected?.name ?? "This model";
  const count = draft.speaker_count;
  const speakerCause = !speakersAfterStop
    ? null
    : engine && engine !== "sortformer" && engine !== "nemotron"
      ? sources.find(([id]) => id === engine)?.[1]
      : count && selected?.speaker_preferences?.during_recording.length
        ? count
        : model;
  const both = transcriptAfterStop && speakerCause === model;
  const speakers = speakerCause === null || both ? null : speakerCause;
  const afterStop = [
    both
      ? `Transcription and speaker recognition will run after Stop when using ${model} because it does not support live transcription or live speaker recognition.`
      : transcriptAfterStop
        ? `Transcription will run after Stop when using ${model} because it does not support live transcription.`
        : null,
    speakers === null
      ? null
      : typeof speakers === "object"
        ? `Speaker recognition will run after Stop with ${speakers.kind === "exact" ? speakers.count : `up to ${speakers.max}`} expected speakers because live speaker recognition does not support that many speakers.`
        : `Speaker recognition will run after Stop when using ${speakers} because it does not support live speaker recognition.`,
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <div className="grid min-w-0 gap-6">
      <Group header="Transcription model">
        <div className="p-4">
          <RecognitionModelPicker
            models={models}
            modelAction={modelAction}
            modelActivity={modelActivity}
            disabled={disabled}
            value={draft.asr_model}
            onChange={(asr_model) => {
              if (asr_model === draft.asr_model) return;
              invalidate();
              setEngine(null);
              setDraft({
                ...draft,
                asr_model,
                language: usableLanguage(
                  asr_model,
                  preferences.current.language,
                ),
              });
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
                value={shown.processing_mode}
                disabled={loading || disabled}
                onChange={(e) => chooseMode("processing_mode", e.target.value)}
              >
                {timings.map(([id, text]) => (
                  <option key={id} value={id}>
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
                    : shown.speaker_processing_mode
                }
                disabled={loading || disabled}
                onChange={(e) =>
                  chooseMode("speaker_processing_mode", e.target.value)
                }
              >
                <option value="off">Off</option>
                {timings.map(([id, text]) => (
                  <option key={id} value={id}>
                    {text}
                  </option>
                ))}
              </select>
            </label>
          </div>
          {afterStop && (
            <p className="text-[0.85rem] text-caution">
              {afterStop}
            </p>
          )}
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
                    disabled={loading || disabled}
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
                    disabled={disabled}
                    value={shown.language ?? ""}
                    onChange={(e) => {
                      invalidate();
                      const language = e.target.value || null;
                      prefer({ language });
                      setDraft({
                        ...draft,
                        language: usableLanguage(draft.asr_model, language),
                      });
                    }}
                  >
                    <option value="">Automatic</option>
                    {shown.language &&
                      !selected?.languages.includes(shown.language) && (
                        <option value={shown.language}>
                          {languageName(shown.language)}
                        </option>
                      )}
                    {selected?.languages.map((code) => (
                      <option key={code} value={code}>
                        {languageName(code)}
                      </option>
                    ))}
                  </select>
                  {shown.language && shown.language !== draft.language && (
                    <span className="text-[0.85rem] text-caution">
                      The language will be detected automatically when using{" "}
                      {model} because it does not support{" "}
                      {languageName(shown.language)}.
                    </span>
                  )}
                </label>
                <label className="grid gap-1">
                  Expected speakers
                  <select
                    aria-label="Expected speakers"
                    className={fieldClass}
                    disabled={disabled}
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
                    disabled={disabled}
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
                        disabled={disabled}
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
