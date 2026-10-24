// Plain-language wording for a session's lifecycle and stages, shared by the
// Home and the processing view so each state reads the same everywhere.

import type { Readiness, SessionDetail, SessionEntry, SpeakerProgressView, WaitView } from "../bindings";

export const STAGES = ["assemble", "diarize", "transcribe", "join", "verify", "summarize", "save"] as const;
export type StageName = (typeof STAGES)[number];

const stageLabels: Record<StageName, string> = {
  assemble: "Preparing the audio",
  diarize: "Finding speakers",
  transcribe: "Transcribing",
  join: "Assembling the transcript",
  verify: "Checking the transcript",
  summarize: "Summarizing",
  save: "Saving to your folder",
};

export function stageLabel(stage: string): string {
  return stageLabels[stage as StageName] ?? stage;
}

/// The step of a summary in parts: "Summarizing part 5 of 16", then
/// "Combining summaries 3 of 9" for the calls that combine the part
/// summaries in groups, then "Writing the final summary". `done` counts the
/// calls of the step that are done, so the call that runs is the next one.
export function summaryStepLabel({
  summary_step,
  done,
  total,
}: {
  summary_step: string | null;
  done: number;
  total: number;
}): string {
  const current = Math.min(done + 1, total);
  switch (summary_step) {
    case "parts":
      return total > 1 ? `Summarizing part ${current} of ${total}` : stageLabel("summarize");
    case "groups":
      return total > 1 ? `Combining summaries ${current} of ${total}` : "Combining summaries";
    case "final":
      return "Writing the final summary";
    default:
      return stageLabel("summarize");
  }
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

const waitTexts: Record<WaitView, string> = {
  recording: "Waiting for the recording to end",
  another_note: "Waiting for another note to finish",
  summary_turn: "Waiting to summarize",
  model: "Waiting for the model to download",
};

/// What a note waits for, as Home and the note page show it (KTD10).
export function waitText(wait: WaitView): string {
  return waitTexts[wait];
}

/// A session's status on Home. `live` is the stage an event
/// named since the list was read. A queued or processing note that waits
/// says what it waits for (KTD10).
export function entryStatus(entry: SessionEntry, live: string | null): string {
  const stage = live ?? entry.stage;
  if (entry.waits_for && (entry.state === "queued" || entry.state === "processing")) return waitText(entry.waits_for);
  switch (entry.state) {
    // An imported file downloads and converts: a quiet state, not a
    // message (R21 of the audio import plan).
    case "importing":
      return "Preparing";
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
/// one; a processing session's is the one it last started. While the note
/// waits (`waits`, from the runner), no stage runs: the job row can name a
/// stage that has not started (KTD10).
export function stageRows(detail: SessionDetail, waits: WaitView | null = detail.entry.waits_for): StageRow[] {
  const done = new Set(detail.done);
  const failed = detail.entry.state === "processing_failed" ? detail.failed_stage : null;
  const running = detail.entry.state === "processing" && !waits ? detail.current : null;
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

/// The speaker stage's meter: the measured whole-note percent, if any, and
/// whether work runs that the percent does not show.
export type SpeakerMeter = { percent: number | null; measuring: boolean };

/// The speaker meter from the saved progress and the latest event. The
/// latest event comes from the current attempt, so saved progress of another
/// attempt does not count. The percent does not go back, and it reaches 100
/// only when the stage is done, which removes the meter (R5).
export function speakerMeter(detail: SessionDetail, live: SpeakerProgressView | undefined): SpeakerMeter | null {
  const running = detail.entry.state === "processing" && detail.current === "diarize" && !detail.done.includes("diarize");
  if (!running) return null;
  const current = live;
  const saved = detail.speakers && (!live || live.attempt === detail.speakers.attempt) ? detail.speakers : null;
  const percents = [saved?.percent, current?.percent].filter((p): p is number => p != null);
  const percent = percents.length > 0 ? Math.min(99, Math.max(...percents)) : null;
  // The newest source that reached this percent says whether it is measured.
  const measuring = current && (current.percent ?? -1) >= (saved?.percent ?? -1) ? current.measuring : false;
  return { percent, measuring };
}

/// "14:05": the local clock time of `iso`.
export function clockTime(iso: string): string {
  return new Date(iso).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

/// The number of local calendar days from `date` to `now`: 0 on the same day.
export function daysBefore(date: Date, now: Date): number {
  const midnight = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  return Math.round((midnight(now) - midnight(date)) / 86_400_000);
}

/// "Today, 14:05", "Yesterday, 09:10", or "Mon 21 Sep, 14:05".
export function sessionTitle(startedIso: string, now: Date = new Date()): string {
  const started = new Date(startedIso);
  const time = clockTime(startedIso);
  const days = daysBefore(started, now);
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
  if (!readiness.microphone_authorized) missing.push("allow microphone access");
  // A cleared shortcut is a choice; only one that failed to register is missing.
  if (readiness.shortcut.chord && !readiness.shortcut.registered) missing.push("choose a different shortcut");
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
