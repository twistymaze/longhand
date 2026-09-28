import { useId } from "react";
import type { ModelStatus } from "../bindings";
import { formatBytes } from "./ui";
import { CheckIcon } from "./icons";

const languageNames = new Intl.DisplayNames(["en"], { type: "language" });

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
}: {
  models: ModelStatus[];
  value: string;
  onChange: (id: string) => void;
}) {
  const groupId = useId();
  const choices = models.filter((m) => m.role === "recognition");
  return (
    <fieldset className="min-w-0 @container">
      <legend className="mb-2 font-semibold">Transcription model</legend>
      <div
        className="divide-y divide-separator"
        role="radiogroup"
        aria-label="Transcription model"
      >
        {choices.map((m, index) => {
          const nativeSpeakers =
            m.speaker_preferences?.after_stop.includes("integrated");
          const selected = m.id === value;
          return (
            <button
              key={m.id}
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
              className={`flex w-full min-w-0 cursor-default items-start gap-2.5 rounded-lg px-2.5 py-4 text-left outline-offset-[-2px] focus-visible:outline-2 focus-visible:outline-accent active:bg-fill-pressed ${selected ? "bg-accent/10" : "hover:bg-row-hover"}`}
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
                    {nativeSpeakers && (
                      <span className="rounded-md bg-fill px-2 py-0.5 text-[0.8rem] font-medium text-label">
                        Built-in speaker recognition
                      </span>
                    )}
                  </span>
                  <span className="text-[0.85rem] leading-relaxed text-label-secondary">
                    {m.languages.length > 2
                      ? `${m.languages.length} languages`
                      : m.languages
                          .map((c) => languageNames.of(c) ?? c)
                          .join(" and ")}
                    <span aria-hidden> · </span>
                    {formatBytes(m.size_bytes)}
                    <span aria-hidden> · </span>
                    {m.state?.state === "present"
                      ? "Downloaded"
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
          );
        })}
      </div>
      <p className="mt-3 text-[0.8rem] leading-relaxed text-label-secondary">
        Ratings are out of 100. Higher is better.
      </p>
    </fieldset>
  );
}
