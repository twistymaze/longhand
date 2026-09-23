// The recording rail (R9, R1c, R10, R22c, R36). Two tiers, because equal-weight
// readouts become something to read rather than glance at, and a glance is
// all a participant at the table should spend:
// - persistent: input status, 30 s of level history, elapsed time;
// - exceptions: silent until something is degraded.
// Nothing here animates except an exception or the one-time model sentence
// arriving, with a short fade. The trace updates in place: it is data, and easing it
// would only make it lag the room.

import { useEffect, useState } from "react";
import {
  commands,
  type ModelStatus,
  type Readiness,
  type SpeakerCountSetting,
} from "../bindings";
import { describeDestination } from "../settings/DestinationPicker";
import { Button, fieldClass, formatBytes } from "../settings/ui";
import { formatPosition, levelFraction } from "../lib/timeline";
import type { InputStatus } from "./inputStatus";
import { type Capture, HISTORY } from "./useCapture";

/// Below this, the working location is named in the exception tier. A
/// four-hour session needs about 2.3 GB of working space (U15 projects it).
const LOW_HEADROOM_BYTES = 3e9;
const MODEL_SENTENCE_MS = 8000;

const statusText: Record<InputStatus, string> = {
  idle: "Not recording",
  starting: "Starting…",
  capturing: "Recording",
  stalled: "No audio arriving",
  reconnecting: "Reconnecting…",
  lost: "Microphone lost",
};

function StatusDot({ input }: { input: InputStatus }) {
  const tone =
    input === "capturing" || input === "starting"
      ? "bg-negative"
      : input === "idle"
        ? "bg-label-tertiary"
        : "bg-caution";
  return <span aria-hidden className={`size-2.5 shrink-0 rounded-full ${tone}`} />;
}

function Trace({ levels, input }: { levels: number[]; input: InputStatus }) {
  const dim =
    input === "stalled" || input === "reconnecting" || input === "lost" || input === "idle";
  const padded = Array.from({ length: HISTORY - levels.length }, () => -1).concat(levels);
  return (
    <div
      aria-label="Audio level, last 30 seconds"
      role="img"
      className={`flex h-7 min-w-0 flex-1 items-center gap-px ${dim ? "opacity-35" : ""}`}
    >
      {padded.map((peak, i) => (
        <span
          key={i}
          className="w-[2px] shrink-0 rounded-full bg-label-secondary"
          style={{
            // A hairline marks silence so a quiet room still reads as a trace.
            height: peak < 0 ? 0 : `${Math.max(2, levelFraction(peak) * 28)}px`,
          }}
        />
      ))}
    </div>
  );
}

function SpeakerField() {
  const [value, setValue] = useState("");
  const [error, setError] = useState(false);
  const commit = async (text: string) => {
    const trimmed = text.trim();
    let count: SpeakerCountSetting | null = null;
    const range = trimmed.match(/^(\d+)\s*[-–]\s*(\d+)$/);
    if (range) count = { kind: "range", min: Number(range[1]), max: Number(range[2]) };
    else if (/^\d+$/.test(trimmed)) count = { kind: "exact", count: Number(trimmed) };
    else if (trimmed !== "") {
      setError(true);
      return;
    }
    const result = await commands.setSessionSpeakerCount(count);
    setError(result.status === "error");
  };
  return (
    <label className="flex items-center gap-1.5 text-label-secondary">
      Speakers
      <input
        aria-label="Speakers in this session, a number or a range like 4-6"
        className={`${fieldClass} w-14 text-center tabular-nums ${error ? "border-negative" : ""}`}
        placeholder="auto"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onBlur={() => void commit(value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void commit(value);
        }}
      />
    </label>
  );
}

function Exceptions({
  capture,
  readiness,
}: {
  capture: Capture;
  readiness: Readiness;
}) {
  const items: { tone: "caution" | "negative"; text: string }[] = [];
  const free = capture.status?.working_free_bytes;
  if (free != null && free < LOW_HEADROOM_BYTES) {
    items.push({ tone: "caution", text: `${formatBytes(free)} free for audio on this disk` });
  }
  if (readiness.destination.kind !== "ready") {
    const d = describeDestination(readiness.destination);
    items.push({ tone: d.tone === "negative" ? "negative" : "caution", text: `Destination: ${d.text}` });
  }
  if (readiness.missing_models.length > 0) {
    items.push({ tone: "caution", text: "Processing will wait for model downloads" });
  }
  if (!readiness.endpoint_configured) {
    items.push({ tone: "caution", text: "No summarization endpoint — summaries will wait" });
  }
  if (capture.lostReason) {
    items.push({ tone: "negative", text: capture.lostReason });
  }
  if (items.length === 0) return null;
  return (
    <ul className="grid gap-1 border-t border-separator px-3 py-2 text-[0.85rem]">
      {items.map((item) => (
        <li
          key={item.text}
          className={`fade-in ${item.tone === "negative" ? "text-negative" : "text-caution"}`}
        >
          {item.text}
        </li>
      ))}
    </ul>
  );
}

/// Capture stopped and could not go on by itself (R5c): what happened, and
/// the two ways out. Stop acknowledges it; Resume opens the next segment.
function FailedCapture({ capture, onError }: { capture: Capture; onError: (e: string | null) => void }) {
  const status = capture.status;
  const [busy, setBusy] = useState(false);
  if (!status?.interruption || !status.session_id) return null;
  const id = status.session_id;
  const act = async (action: () => Promise<{ status: "ok" | "error"; error?: string }>) => {
    setBusy(true);
    const result = await action();
    setBusy(false);
    onError(result.status === "error" ? (result.error ?? null) : null);
    await capture.refresh();
  };
  return (
    <div role="alert" className="grid gap-1.5 border-t border-separator px-3 py-2">
      <p className="font-medium text-negative">{status.interruption.headline}</p>
      {capture.lostReason && <p className="text-[0.85rem] text-label-secondary">{capture.lostReason}</p>}
      {status.interruption.disk && (
        <p className="text-[0.85rem] text-label-secondary">Free some space on this disk, then resume.</p>
      )}
      <div className="flex justify-end gap-2">
        <Button disabled={busy} onClick={() => void act(() => commands.stopRecording())}>
          Stop
        </Button>
        <Button prominent disabled={busy} onClick={() => void act(() => commands.resumeRecording(id))}>
          Resume
        </Button>
      </div>
    </div>
  );
}

/// What stands between the shortcut and a recording (R1a).
export function missingPreconditions(readiness: Readiness): string {
  const missing: string[] = [];
  if (!readiness.microphone_authorized) missing.push("microphone access");
  if (readiness.destination.kind === "unset") missing.push("a destination folder");
  if (!readiness.shortcut.registered) missing.push("the shortcut could not be registered");
  return missing.join(", ");
}

export function Rail({
  capture,
  readiness,
  models,
  chord,
}: {
  capture: Capture;
  readiness: Readiness;
  models: ModelStatus[];
  chord: string;
}) {
  const recording = capture.status?.recording ?? false;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showModelSentence, setShowModelSentence] = useState(false);

  // R10: said once as the session starts, not held on screen.
  useEffect(() => {
    if (!recording) return;
    setShowModelSentence(true);
    const timer = window.setTimeout(() => setShowModelSentence(false), MODEL_SENTENCE_MS);
    return () => window.clearTimeout(timer);
  }, [recording, capture.status?.session_id]);

  const asr = models.find((m) => m.role === "recognition" && m.required);
  const modelSentence = asr?.streaming
    ? "A live transcript is available for this model, though this version does not show one."
    : "The transcript arrives when processing finishes.";

  const toggle = async () => {
    setBusy(true);
    const result = recording ? await commands.stopRecording() : await commands.startRecording();
    setBusy(false);
    setError(result.status === "error" ? result.error : null);
    await capture.refresh();
  };

  return (
    <section
      aria-label="Recording"
      className="overflow-hidden rounded-[10px] bg-group shadow-[0_0_0_0.5px_var(--separator)]"
    >
      <div className="flex items-center gap-3 px-3 py-2">
        <StatusDot input={capture.input} />
        <span className="w-32 shrink-0 font-medium">{statusText[capture.input]}</span>
        <Trace levels={capture.levels} input={capture.input} />
        <span className="w-[4.5rem] shrink-0 text-right text-[1.05rem] font-medium tabular-nums tracking-tight">
          {recording || capture.input === "lost" ? formatPosition(capture.recordedMs) : "--:--:--"}
        </span>
      </div>
      <div className="flex items-center justify-between gap-3 border-t border-separator px-3 py-1.5">
        {recording ? (
          <SpeakerField />
        ) : readiness.armed ? (
          <span className="text-label-secondary">Press {chord} to record</span>
        ) : (
          <span className="text-caution">Not armed: {missingPreconditions(readiness)}</span>
        )}
        <Button
          prominent={!recording && !capture.status?.interruption}
          disabled={busy || (!recording && !readiness.armed)}
          onClick={toggle}
        >
          {recording ? "Stop" : "Record"}
        </Button>
      </div>
      {recording && showModelSentence && (
        <p className="fade-in border-t border-separator px-3 py-1.5 text-[0.85rem] text-label-secondary">
          {modelSentence}
        </p>
      )}
      {recording && <Exceptions capture={capture} readiness={readiness} />}
      {!recording && capture.status?.interruption ? (
        <FailedCapture capture={capture} onError={setError} />
      ) : (
        !recording && capture.lostReason && <Exceptions capture={capture} readiness={readiness} />
      )}
      {error && <p className="border-t border-separator px-3 py-1.5 text-[0.85rem] text-negative">{error}</p>}
    </section>
  );
}
