import { describe, expect, test } from "bun:test";
import type { Readiness, SessionDetail } from "../bindings";
import {
  entryStatus,
  formatDuration,
  missingPreconditions,
  sentence,
  sessionTitle,
  speakerMeter,
  stageRows,
  stoppedAt,
} from "./describe";

const detail = (over: Partial<SessionDetail>): SessionDetail => ({
  entry: { id: "s", started: "2026-09-22T14:05:00Z", duration_ms: 0, state: "processing", stage: null, title: null, found_speakers: null, storage_unavailable: false, remote: null },
  done: [],
  current: null,
  windows_done: null,
  windows_total: null,
  speakers: null,
  failed_stage: null,
  message: null,
  summary_problem: null,
  resumable: false,
  record: null,
  home_path: null,
  endpoint: null,
  audio_kept: true,
  transcript_kept: false,
  audio: "present",
  asr_model: null,
  live_transcript: false,
  transcribe_again: false,
  transcribe_again_error: null,
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
        entry: { id: "s", started: "2026-09-22T14:05:00Z", duration_ms: 0, state: "processing_failed", stage: "verify", title: null, found_speakers: null, storage_unavailable: false, remote: null },
        done: ["assemble", "diarize", "transcribe", "join"],
        current: "verify",
        failed_stage: "verify",
      }),
    );
    expect(rows.find((r) => r.status === "failed")?.stage).toBe("verify");
    expect(rows.some((r) => r.status === "running")).toBe(false);
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
    const entry = { id: "s", started: "", duration_ms: 0, state: "processing", stage: "diarize", title: null, found_speakers: null, storage_unavailable: false, remote: null };
    expect(entryStatus(entry, null)).toBe("Finding speakers");
    expect(stoppedAt("diarize")).toBe("Stopped while finding speakers");
  });

  test("a blocked session says what it needs, not that it failed", () => {
    const status = entryStatus({ id: "s", started: "", duration_ms: 0, state: "blocked", stage: null, title: null, found_speakers: null, storage_unavailable: false, remote: null }, null);
    expect(status).toBe("Needs the keychain");
    expect(status.toLowerCase()).not.toContain("fail");
  });

  test("a failed session says where it stopped, and a reload keeps the stage", () => {
    expect(stoppedAt("verify")).toBe("Stopped while checking the transcript");
    const entry = { id: "s", started: "", duration_ms: 0, state: "processing", stage: "transcribe", title: null, found_speakers: null, storage_unavailable: false, remote: null };
    expect(entryStatus(entry, null)).toBe("Transcribing");
    expect(entryStatus({ ...entry, state: "processing_failed" }, null)).toBe("Needs a retry");
  });

  test("backend messages read as sentences", () => {
    expect(sentence("the destination folder is missing")).toBe("The destination folder is missing.");
    expect(sentence("Locked.")).toBe("Locked.");
  });

  test("titles name today and yesterday", () => {
    const now = new Date(2026, 8, 22, 18, 0);
    expect(sessionTitle(new Date(2026, 8, 22, 14, 5).toISOString(), now)).toStartWith("Today, ");
    expect(sessionTitle(new Date(2026, 8, 21, 9, 10).toISOString(), now)).toStartWith("Yesterday, ");
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
