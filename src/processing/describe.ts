// Plain-language wording for a session's lifecycle and stages, shared by the
// Home and the processing view so each state reads the same everywhere.

import type { Readiness, SessionDetail, SessionEntry } from "../bindings";

export const STAGES = ["assemble", "diarize", "transcribe", "join", "verify", "summarize", "save"] as const;
export type StageName = (typeof STAGES)[number];

const stageLabels: Record<StageName, string> = {
  assemble: "Preparing the audio",
  diarize: "Telling speakers apart",
  transcribe: "Transcribing",
  join: "Assembling the transcript",
  verify: "Checking the transcript",
  summarize: "Summarizing",
  save: "Saving to your folder",
};

export function stageLabel(stage: string): string {
  return stageLabels[stage as StageName] ?? stage;
}

/// Sessions that Home shows as the recording row, not in its list.
export function isOpen(state: string): boolean {
  return state === "capturing" || state === "interrupted";
}

/// "Stopped while checking the transcript".
export function stoppedAt(stage: string | null): string {
  return stage ? `Stopped while ${stageLabel(stage).toLowerCase()}` : "Stopped";
}

/// A backend message as a sentence: they start in lower case so they can
/// follow a colon in a log.
export function sentence(message: string): string {
  const text = message.trim();
  return text.charAt(0).toUpperCase() + text.slice(1) + (/[.!?]$/.test(text) ? "" : ".");
}

/// A session's status on Home. `live` is the stage an event
/// named since the list was read.
export function entryStatus(entry: SessionEntry, live: string | null): string {
  const stage = live ?? entry.stage;
  switch (entry.state) {
    case "queued":
      return "Waiting to process";
    case "processing":
      return stage ? stageLabel(stage) : "Processing";
    case "blocked":
      return "Needs the keychain";
    case "processing_failed":
      // The view says where it stopped; Home says what it needs,
      // like its neighbours.
      return "Needs a retry";
    case "awaiting_destination":
      return "Needs a folder";
    case "saved":
      return "Ready";
    default:
      return "";
  }
}

export type Tone = "positive" | "caution" | "negative" | "neutral";

export function entryTone(state: string): Tone | null {
  switch (state) {
    case "processing_failed":
      return "negative";
    case "blocked":
    case "awaiting_destination":
      return "caution";
    default:
      return null;
  }
}

export type StageRow = {
  stage: StageName;
  label: string;
  status: "done" | "running" | "failed" | "pending";
};

/// Every stage with where it stands. A failed session's stage is the failed
/// one; a processing session's is the one it last started.
export function stageRows(detail: SessionDetail): StageRow[] {
  const done = new Set(detail.done);
  const failed = detail.entry.state === "processing_failed" ? detail.failed_stage : null;
  const running = detail.entry.state === "processing" ? detail.current : null;
  return STAGES.map((stage) => ({
    stage,
    label: stageLabel(stage),
    status:
      stage === failed
        ? "failed"
        : done.has(stage)
          ? "done"
          : stage === running
            ? "running"
            : "pending",
  }));
}

/// "Today, 14:05", "Yesterday, 09:10", or "Mon 21 Sep, 14:05".
export function sessionTitle(startedIso: string, now: Date = new Date()): string {
  const started = new Date(startedIso);
  const time = started.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const day = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((day(now) - day(started)) / 86_400_000);
  if (days === 0) return `Today, ${time}`;
  if (days === 1) return `Yesterday, ${time}`;
  const date = started.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });
  return `${date}, ${time}`;
}

/// "Thursday 24 September 2026, 14:05": the date and start time in full.
export function startedAt(iso: string): string {
  return new Date(iso).toLocaleString(undefined, {
    weekday: "long",
    day: "numeric",
    month: "long",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/// What stands between the shortcut and a recording (R1a).
export function missingPreconditions(readiness: Readiness): string {
  const missing: string[] = [];
  if (!readiness.microphone_authorized) missing.push("microphone access");
  if (readiness.destination.kind === "unset") missing.push("a destination folder");
  if (!readiness.shortcut.registered) missing.push("the shortcut could not be registered");
  return missing.join(", ");
}

/// "1 h 12 min", "42 min", "under a minute".
export function formatDuration(ms: number): string {
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 1) return "under a minute";
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h === 0) return `${m} min`;
  return m === 0 ? `${h} h` : `${h} h ${m} min`;
}
