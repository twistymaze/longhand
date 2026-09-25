// The bar at the bottom of the session page while it records (R12, KD2).
// It keeps the two tiers of walking-skeleton U7, because a glance is all a
// participant at the table should spend:
// - always: the live dot, the elapsed time and Stop;
// - only while a condition exists: the level meter and the warnings.
// Nothing in the bar moves: the level bars on the Transcript tab show the
// input. The warnings appear at once, with no fade: motion would pull the
// eye.

import { useState } from "react";
import { commands, type Readiness, type SpeakerCountSetting } from "../bindings";
import { formatPosition, levelFraction } from "../lib/timeline";
import type { InputStatus } from "../recording/inputStatus";
import { type Capture, HISTORY } from "../recording/useCapture";
import { describeDestination } from "../settings/DestinationPicker";
import { Button, fieldClass, formatBytes } from "../settings/ui";

const statusText: Record<InputStatus, string> = {
  idle: "Recording stopped",
  starting: "Starting…",
  capturing: "Recording",
  quiet: "Recording",
  stalled: "No audio arriving",
  reconnecting: "Reconnecting…",
  lost: "Microphone lost",
};

/// Input conditions that show the level meter, so the user can see what
/// the microphone hears.
const showsLevel: InputStatus[] = ["quiet", "stalled", "reconnecting"];

/// A steady dot: red while audio arrives, orange when it does not.
function LiveDot({ input }: { input: InputStatus }) {
  const live = input === "capturing" || input === "quiet" || input === "starting";
  return (
    <span
      aria-hidden
      className={`size-2 shrink-0 rounded-full ${live ? "bg-negative" : "bg-caution"}`}
    />
  );
}

/// Stop, with the square that marks it on every recorder.
function StopButton({ disabled, onClick }: { disabled: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className="flex items-center gap-1.5 rounded-md bg-fill py-[3px] pr-2.5 pl-2 whitespace-nowrap text-label active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40"
    >
      <span aria-hidden className="size-[9px] rounded-[2px] bg-negative" />
      Stop
    </button>
  );
}

function LevelMeter({ levels }: { levels: number[] }) {
  const padded = Array.from({ length: HISTORY - levels.length }, () => -1).concat(levels);
  return (
    <div
      aria-label="Audio level, last 30 seconds"
      role="img"
      className="flex h-5 min-w-0 flex-1 items-center gap-px overflow-hidden"
    >
      {padded.map((peak, i) => (
        <span
          key={i}
          className="w-[2px] shrink-0 rounded-full bg-label-tertiary"
          // A hairline marks silence so a quiet room still shows as a trace.
          style={{ height: peak < 0 ? 0 : `${Math.max(2, levelFraction(peak) * 20)}px` }}
        />
      ))}
    </div>
  );
}

type Warning = { tone: "caution" | "negative"; text: string };

/// The exception tier: each warning names a condition that exists now.
function warnings(capture: Capture, readiness: Readiness | null): Warning[] {
  const items: Warning[] = [];
  if (capture.input === "quiet") {
    items.push({
      tone: "caution",
      text: "The microphone hears almost nothing. Check that it is on and near the people talking.",
    });
  }
  const status = capture.status;
  const free = status?.working_free_bytes;
  if (status && free != null) {
    // R7b: what processing will need by the time this session stops.
    const needed = (status.processing_bytes_per_second * capture.recordedMs) / 1000 + status.reserve_bytes;
    if (free < needed) {
      items.push({
        tone: "negative",
        text: `${formatBytes(free)} free: processing this note needs about ${formatBytes(needed)}.`,
      });
    }
  }
  const problem = capture.alert?.output_problem;
  if (problem) items.push({ tone: "caution", text: `If recording fails, ${problem}` });
  if (capture.alert && !capture.alert.tone_heard) {
    items.push({ tone: "caution", text: "The alert sound has not been tested. Test it in Settings." });
  }
  if (readiness && readiness.destination.kind !== "ready") {
    const d = describeDestination(readiness.destination);
    items.push({ tone: d.tone === "negative" ? "negative" : "caution", text: `Destination: ${d.text}` });
  }
  if (readiness && readiness.missing_models.length > 0) {
    items.push({ tone: "caution", text: "Processing will wait for model downloads." });
  }
  if (readiness && !readiness.endpoint_configured) {
    items.push({ tone: "caution", text: "No summarization endpoint is set, so the summary will wait." });
  }
  if (capture.lostReason) items.push({ tone: "negative", text: capture.lostReason });
  return items;
}

/// The number of speakers in this session, a number or a range (R36).
function SpeakerField({ onDone }: { onDone: () => void }) {
  const [value, setValue] = useState("");
  const [error, setError] = useState(false);
  const commit = async () => {
    const trimmed = value.trim();
    let count: SpeakerCountSetting | null = null;
    const range = trimmed.match(/^(\d+)\s*[-–]\s*(\d+)$/);
    if (range) count = { kind: "range", min: Number(range[1]), max: Number(range[2]) };
    else if (/^\d+$/.test(trimmed)) count = { kind: "exact", count: Number(trimmed) };
    else if (trimmed !== "") {
      setError(true);
      return;
    }
    const result = await commands.setSessionSpeakerCount(count);
    if (result.status === "error") setError(true);
    else onDone();
  };
  return (
    <label className="flex items-center gap-1.5 text-label-secondary">
      Speakers
      <input
        aria-label="Speakers in this note, a number or a range like 4-6"
        autoFocus
        className={`${fieldClass} w-16 text-center tabular-nums ${error ? "border-negative" : ""}`}
        placeholder="auto"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onBlur={() => void commit()}
        onKeyDown={(e) => {
          if (e.key === "Enter") void commit();
          if (e.key === "Escape") onDone();
        }}
      />
    </label>
  );
}

export function RecordingBar({
  capture,
  readiness,
  speakers,
  onSpeakersDone,
}: {
  capture: Capture;
  readiness: Readiness | null;
  /// Show the speaker count field, which the page's menu asks for.
  speakers: boolean;
  onSpeakersDone: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const status = capture.status;
  const interruption = status?.interruption ?? null;

  const act = async (action: () => Promise<{ status: "ok" } | { status: "error"; error: string }>) => {
    setBusy(true);
    const result = await action();
    setBusy(false);
    setError(result.status === "error" ? result.error : null);
    await capture.refresh();
  };

  const items = warnings(capture, readiness);
  const id = status?.session_id ?? null;

  return (
    <section
      aria-label="Recording"
      className="shrink-0 border-t border-separator bg-group select-none"
    >
      <div className="mx-auto flex max-w-[44rem] items-center gap-2.5 px-8 py-2.5">
        <LiveDot input={interruption ? "lost" : capture.input} />
        <span className="shrink-0 font-medium">
          {interruption ? "Recording interrupted" : statusText[capture.input]}
        </span>
        <span className="shrink-0 tabular-nums text-label-secondary">{formatPosition(capture.recordedMs)}</span>
        {showsLevel.includes(capture.input) ? <LevelMeter levels={capture.levels} /> : <span className="flex-1" />}
        {speakers && <SpeakerField onDone={onSpeakersDone} />}
        <StopButton disabled={busy} onClick={() => void act(() => commands.stopRecording())} />
        {interruption && id && (
          <Button prominent disabled={busy} onClick={() => void act(() => commands.resumeRecording(id))}>
            Resume
          </Button>
        )}
      </div>
      {(interruption || items.length > 0 || error) && (
        <div role="status" className="mx-auto grid max-w-[44rem] gap-1 px-8 pb-2.5 text-[0.85rem]">
          {interruption && (
            <>
              <p className="font-medium text-negative">{interruption.headline}</p>
              {interruption.disk && <p className="text-label-secondary">Free some space on this disk, then resume.</p>}
            </>
          )}
          {items.map((item) => (
            <p key={item.text} className={item.tone === "negative" ? "text-negative" : "text-caution"}>
              {item.text}
            </p>
          ))}
          {error && <p className="text-negative">{error}</p>}
        </div>
      )}
    </section>
  );
}
