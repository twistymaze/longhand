import { describe, expect, test } from "vitest";
import type { Readiness, SessionDetail, SessionEntry } from "../bindings";
import {
  entryStatus,
  entryTone,
  formatDuration,
  missingPreconditions,
  sentence,
  sessionTitle,
  speakerMeter,
  stageRows,
  stoppedAt,
  waitText,
} from "./describe";

const detail = (over: Partial<SessionDetail>): SessionDetail => ({
  entry: { id: "s", started: "2026-09-22T14:05:00Z", duration_ms: 0, state: "processing", stage: null, title: null, found_speakers: null, storage_unavailable: false, problem: null, remote: null, note_type: null, tags: [], waits_for: null },
  done: [],
  current: null,
  windows_done: null,
  windows_total: null,
  speakers: null,
  failed_stage: null,
  message: null,
  summary_problem: null,
  low_coverage: false,
  resumable: false,
  record: null,
  home_path: null,
  endpoint: null,
  audio_kept: true,
  transcript_kept: false,
  audio: "present",
  transcript: "present",
  asr_model: null,
  live_transcript: false,
  transcribe_again: false,
  regenerating: false,
  transcribe_again_error: null,
  retry_starts_over: false,
  summary_not_updated: null,
  waiting: [],
  conflict_files: [],
  not_saved_yet: null,
  ...over,
});

describe("stage rows", () => {
  test("a session at transcribe shows earlier stages done and the rest pending", () => {
    const rows = stageRows(detail({ done: ["assemble", "diarize"], current: "transcribe" }));
    expect(rows.map((r) => r.status)).toEqual(["done", "done", "running", "pending", "pending", "pending", "pending"]);
  });

  test("a failed verify names the stage and runs nothing", () => {
    const rows = stageRows(
      detail({
        entry: { id: "s", started: "2026-09-22T14:05:00Z", duration_ms: 0, state: "processing_failed", stage: "verify", title: null, found_speakers: null, storage_unavailable: false, problem: null, remote: null, note_type: null, tags: [], waits_for: null },
        done: ["assemble", "diarize", "transcribe", "join"],
        current: "verify",
        failed_stage: "verify",
      }),
    );
    expect(rows.find((r) => r.status === "failed")?.stage).toBe("verify");
    expect(rows.some((r) => r.status === "running")).toBe(false);
  });

  test("a claimed note that waits before its first stage starts runs no stage", () => {
    // At the claim, the job row names Assemble before Assemble starts.
    const claimed = detail({ current: "assemble" });
    claimed.entry = { ...claimed.entry, stage: "assemble", waits_for: "another_note" };
    const rows = stageRows(claimed);
    expect(rows.map((r) => r.status)).toEqual(["pending", "pending", "pending", "pending", "pending", "pending", "pending"]);
  });

  test("a note that waits for its summary turn shows the summarize row as to do", () => {
    const waiting = detail({ done: ["assemble", "diarize", "transcribe", "join", "verify"], current: "summarize" });
    waiting.entry = { ...waiting.entry, stage: "summarize", waits_for: "summary_turn" };
    const rows = stageRows(waiting);
    expect(rows.find((r) => r.stage === "summarize")?.status).toBe("pending");
    expect(rows.some((r) => r.status === "running")).toBe(false);
    // The turn comes: the summarize row runs.
    waiting.entry = { ...waiting.entry, waits_for: null };
    expect(stageRows(waiting).find((r) => r.stage === "summarize")?.status).toBe("running");
  });

  test("a note paused by a recording keeps its done steps and runs none", () => {
    const paused = detail({ done: ["assemble", "diarize"], current: "transcribe" });
    paused.entry = { ...paused.entry, stage: "transcribe", waits_for: "recording" };
    expect(stageRows(paused).map((r) => r.status)).toEqual(["done", "done", "pending", "pending", "pending", "pending", "pending"]);
  });
});

describe("speaker meter", () => {
  const running = (speakers: SessionDetail["speakers"]) => detail({ done: ["assemble"], current: "diarize", speakers });
  const progress = (attempt: string, percent: number | null, measuring: boolean) => ({ attempt, percent, measuring });

  test("a view opened mid-stage shows the saved share as active work until an event arrives", () => {
    expect(speakerMeter(running(progress("a", 40, true)), undefined)).toEqual({ percent: 40, measuring: false });
  });

  test("a newer event of the same attempt advances the value", () => {
    expect(speakerMeter(running(progress("a", 40, true)), progress("a", 55, true))).toEqual({ percent: 55, measuring: true });
  });

  test("an older event does not move the value back", () => {
    expect(speakerMeter(running(progress("a", 60, true)), progress("a", 55, true))).toEqual({ percent: 60, measuring: false });
  });

  test("saved progress of an earlier attempt does not count", () => {
    expect(speakerMeter(running(progress("a", 80, true)), progress("b", 10, true))).toEqual({ percent: 10, measuring: true });
  });

  test("work with no fraction keeps the value and shows activity", () => {
    expect(speakerMeter(running(progress("a", 99, true)), progress("a", 99, false))).toEqual({ percent: 99, measuring: false });
    expect(speakerMeter(running(null), progress("a", null, false))).toEqual({ percent: null, measuring: false });
  });

  test("a done, failed, or other stage has no meter", () => {
    expect(speakerMeter(detail({ done: ["assemble", "diarize"], current: "diarize" }), progress("a", 99, false))).toBeNull();
    expect(speakerMeter(detail({ current: "transcribe" }), undefined)).toBeNull();
    const failed = running(progress("a", 50, true));
    failed.entry = { ...failed.entry, state: "processing_failed" };
    expect(speakerMeter(failed, progress("a", 50, true))).toBeNull();
  });
});

describe("wording", () => {
  test("the speaker stage finds speakers and does not name people", () => {
    const entry = { id: "s", started: "", duration_ms: 0, state: "processing", stage: "diarize", title: null, found_speakers: null, storage_unavailable: false, problem: null, remote: null, note_type: null, tags: [], waits_for: null };
    expect(entryStatus(entry, null)).toBe("Finding speakers");
    expect(stoppedAt("diarize")).toBe("Stopped while finding speakers");
  });

  test("an importing note says Preparing, with no message (R21)", () => {
    const entry = { id: "s", started: "", duration_ms: 0, state: "importing", stage: null, title: null, found_speakers: null, storage_unavailable: false, problem: null, remote: null, note_type: null, tags: [], waits_for: null };
    expect(entryStatus(entry, null)).toBe("Preparing");
    expect(entryTone("importing")).toBeNull();
  });

  test("a blocked session says what it needs, not that it failed", () => {
    const status = entryStatus({ id: "s", started: "", duration_ms: 0, state: "blocked", stage: null, title: null, found_speakers: null, storage_unavailable: false, problem: null, remote: null, note_type: null, tags: [], waits_for: null }, null);
    expect(status).toBe("Needs the keychain");
    expect(status.toLowerCase()).not.toContain("fail");
  });

  test("a failed session says where it stopped, and a reload keeps the stage", () => {
    expect(stoppedAt("verify")).toBe("Stopped while checking the transcript");
    const entry = { id: "s", started: "", duration_ms: 0, state: "processing", stage: "transcribe", title: null, found_speakers: null, storage_unavailable: false, problem: null, remote: null, note_type: null, tags: [], waits_for: null };
    expect(entryStatus(entry, null)).toBe("Transcribing");
    expect(entryStatus({ ...entry, state: "processing_failed" }, null)).toBe("Needs a retry");
  });

  test("a note that waits says what it waits for, not its stage", () => {
    const entry: SessionEntry = { id: "s", started: "", duration_ms: 0, state: "processing", stage: "assemble", title: null, found_speakers: null, storage_unavailable: false, problem: null, remote: null, note_type: null, tags: [], waits_for: null };
    expect(entryStatus({ ...entry, waits_for: "recording" }, null)).toBe("Waiting for the recording to end");
    expect(entryStatus({ ...entry, state: "queued", waits_for: "recording" }, null)).toBe("Waiting for the recording to end");
    expect(entryStatus({ ...entry, waits_for: "another_note" }, null)).toBe("Waiting for another note to finish");
    expect(entryStatus({ ...entry, state: "queued", waits_for: "another_note" }, null)).toBe("Waiting for another note to finish");
    expect(entryStatus({ ...entry, stage: "summarize", waits_for: "summary_turn" }, "verify")).toBe("Waiting to summarize");
    expect(entryStatus({ ...entry, state: "queued", waits_for: "model" }, null)).toBe("Waiting for the model to download");
    expect(entryStatus(entry, null)).toBe("Preparing the audio");
    expect(entryStatus({ ...entry, state: "queued" }, null)).toBe("Waiting to process");
    // A note that needs an action says what it needs.
    expect(entryStatus({ ...entry, state: "blocked", waits_for: "recording" }, null)).toBe("Needs the keychain");
  });

  test("each wait has one text, on Home and on the note page", () => {
    expect(waitText("recording")).toBe("Waiting for the recording to end");
    expect(waitText("another_note")).toBe("Waiting for another note to finish");
    expect(waitText("summary_turn")).toBe("Waiting to summarize");
    expect(waitText("model")).toBe("Waiting for the model to download");
  });

  test("backend messages read as sentences", () => {
    expect(sentence("the destination folder is missing")).toBe("The destination folder is missing.");
    expect(sentence("Locked.")).toBe("Locked.");
  });

  test("titles name today and yesterday", () => {
    const now = new Date(2026, 8, 22, 18, 0);
    expect(sessionTitle(new Date(2026, 8, 22, 14, 5).toISOString(), now)).toMatch(/^Today, /);
    expect(sessionTitle(new Date(2026, 8, 21, 9, 10).toISOString(), now)).toMatch(/^Yesterday, /);
  });

  test("durations read in hours and minutes", () => {
    expect(formatDuration(72 * 60_000)).toBe("1 h 12 min");
    expect(formatDuration(42 * 60_000)).toBe("42 min");
    expect(formatDuration(20_000)).toBe("under a minute");
  });
});

describe("missingPreconditions", () => {
  const readiness = (over: Partial<Readiness>): Readiness => ({
    microphone_authorized: true,
    system_audio: "allowed",
    destination: { kind: "ready", path: "/notes", free_bytes: null },
    armed: true,
    shortcut: { chord: "Super+KeyR", registered: true, error: null },
    missing_models: [],
    endpoint_configured: true, can_start_note: true, note_start_refusal: null,
    ...over,
  });

  test("a cleared shortcut is not a missing step", () => {
    const r = readiness({ microphone_authorized: false, shortcut: { chord: null, registered: false, error: null } });
    expect(missingPreconditions(r)).toBe("allow microphone access");
  });

  test("a shortcut that failed to register is a missing step", () => {
    const r = readiness({
      shortcut: { chord: "Super+KeyR", registered: false, error: "Another app uses these keys." },
    });
    expect(missingPreconditions(r)).toBe("choose a different shortcut");
  });
});
