import { type ReactNode, useId } from "react";
import type { ModelStatus } from "../bindings";
import { formatBytes } from "./ui";
import { CheckIcon } from "./icons";

import { describeLanguages } from "./languages";

const recommendationOrder = [
  "nemotron-speech-streaming-en-0.6b-q80",
  "parakeet-unified-en-0.6b-q80",
  "granite-speech-4.1-2b-plus-q5km",
];

function recommendationRank(model: ModelStatus): number {
  if (!model.recommended) return recommendationOrder.length + 1;
  const index = recommendationOrder.indexOf(model.id);
  return index < 0 ? recommendationOrder.length : index;
}

function Rating({
  label,
  score,
}: {
  label: string;
  score: number | null | undefined;
}) {
  return (
    <span
      className="grid gap-0.5"
      aria-label={`${label}: ${score == null ? "Not rated" : `${score}/100`}`}
    >
      <span className="min-h-[2.8em] text-[0.8rem] font-medium leading-[1.4] text-label-secondary">
        {label}
      </span>
      <span className="tabular-nums text-[1.25rem] font-semibold leading-tight text-label">
        {score == null ? (
          <span className="text-[0.85rem] font-normal">Not rated</span>
        ) : (
          <>
            {score}
            <span className="ml-0.5 text-[0.75rem] font-normal text-label-secondary">
              /100
            </span>
          </>
        )}
      </span>
    </span>
  );
}

export function RecognitionModelPicker({
  models,
  value,
  onChange,
  modelAction,
  modelActivity,
}: {
  models: ModelStatus[];
  value: string;
  onChange: (id: string) => void;
  modelAction?: (model: ModelStatus) => ReactNode;
  modelActivity?: (model: ModelStatus) => ReactNode;
}) {
  const groupId = useId();
  const choices = models
    .filter((m) => m.role === "recognition")
    .sort(
      (a, b) =>
        recommendationRank(a) - recommendationRank(b) ||
        a.name.localeCompare(b.name),
    );
  return (
    <fieldset className="min-w-0 @container">
      <legend className="sr-only">Transcription model</legend>
      <div
        className="divide-y divide-separator"
        role="radiogroup"
        aria-label="Transcription model"
      >
        {choices.map((m, index) => {
          const selected = m.id === value;
          const activity = modelActivity?.(m);
          return (
            <div
              key={m.id}
              className={`rounded-lg ${selected ? "bg-accent/10" : "hover:bg-row-hover"}`}
            >
              <button
                id={`${groupId}-${index}`}
                type="button"
                role="radio"
                aria-checked={selected}
                aria-label={m.name}
                aria-describedby={`${groupId}-${index}-details`}
                tabIndex={selected ? 0 : -1}
                onClick={() => onChange(m.id)}
                onKeyDown={(event) => {
                  const step =
                    event.key === "ArrowDown" || event.key === "ArrowRight"
                      ? 1
                      : event.key === "ArrowUp" || event.key === "ArrowLeft"
                        ? -1
                        : 0;
                  if (!step && event.key !== "Home" && event.key !== "End")
                    return;
                  event.preventDefault();
                  const next =
                    event.key === "Home"
                      ? 0
                      : event.key === "End"
                        ? choices.length - 1
                        : (index + step + choices.length) % choices.length;
                  onChange(choices[next].id);
                  document.getElementById(`${groupId}-${next}`)?.focus();
                }}
                className="flex w-full min-w-0 cursor-default items-start gap-2.5 rounded-lg px-2.5 py-4 text-left outline-offset-[-2px] focus-visible:outline-2 focus-visible:outline-accent active:bg-fill-pressed"
              >
                <span
                  aria-hidden
                  className="mt-0.5 flex w-4 shrink-0 justify-center text-accent"
                >
                  {selected && <CheckIcon className="size-4" />}
                </span>
                <span
                  id={`${groupId}-${index}-details`}
                  className="grid min-w-0 flex-1 gap-3 @min-[32rem]:grid-cols-[minmax(0,1fr)_12rem] @min-[32rem]:items-center @min-[32rem]:gap-5"
                >
                  <span className="grid min-w-0 gap-2">
                    <span className="flex flex-wrap items-center gap-x-2 gap-y-1.5">
                      <span className="text-[1rem] font-semibold leading-snug [overflow-wrap:anywhere]">
                        {m.name}
                      </span>
                      {m.recommended && (
                        <span className="rounded-md bg-accent px-1.5 py-0.5 text-[0.75rem] font-semibold leading-snug text-white">
                          Recommended
                        </span>
                      )}
                    </span>
                    <span className="flex flex-wrap gap-1.5">
                      <span className="rounded-md bg-fill px-2 py-0.5 text-[0.8rem] font-medium text-label">
                        {m.streaming
                          ? "Live transcription"
                          : "Transcription afterwards"}
                      </span>
                    </span>
                    <span className="text-[0.85rem] leading-relaxed text-label-secondary">
                      {describeLanguages(m.languages)}
                      <span aria-hidden> · </span>
                      {formatBytes(m.bundle_size_bytes)}
                      <span aria-hidden> · </span>
                      {m.bundle_complete
                        ? "Downloaded"
                        : m.state?.state === "present"
                          ? "Some files missing"
                          : "Download required"}
                    </span>
                  </span>
                  <span className="grid w-44 grid-cols-[1.6fr_1fr] gap-4 @min-[32rem]:w-auto">
                    <Rating
                      label="Transcription accuracy"
                      score={m.assessment?.asr_accuracy_score}
                    />
                    <Rating label="Speed" score={m.assessment?.speed_score} />
                  </span>
                </span>
              </button>
              {activity && <div className="px-3 pb-2">{activity}</div>}
              {modelAction && (
                <div className="flex justify-end gap-2 px-2.5 pb-3">
                  {modelAction(m)}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </fieldset>
  );
}
