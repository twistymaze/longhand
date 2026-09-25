import { describe, expect, test } from "bun:test";
import type { SessionDetail } from "../bindings";
import { entryStatus, formatDuration, sentence, sessionTitle, stageRows, stoppedAt } from "./describe";

const detail = (over: Partial<SessionDetail>): SessionDetail => ({
  entry: { id: "s", started: "2026-09-22T14:05:00Z", duration_ms: 0, state: "processing", stage: null, title: null, found_speakers: null },
  done: [],
  current: null,
  windows_done: null,
  windows_total: null,
  failed_stage: null,
  message: null,
  summary_problem: null,
  resumable: false,
  home_path: null,
  endpoint: null,
  audio_kept: true,
  transcript_kept: false,
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
        entry: { id: "s", started: "2026-09-22T14:05:00Z", duration_ms: 0, state: "processing_failed", stage: "verify", title: null, found_speakers: null },
        done: ["assemble", "diarize", "transcribe", "join"],
        current: "verify",
        failed_stage: "verify",
      }),
    );
    expect(rows.find((r) => r.status === "failed")?.stage).toBe("verify");
    expect(rows.some((r) => r.status === "running")).toBe(false);
  });
});

describe("wording", () => {
  test("a blocked session says what it needs, not that it failed", () => {
    const status = entryStatus({ id: "s", started: "", duration_ms: 0, state: "blocked", stage: null, title: null, found_speakers: null }, null);
    expect(status).toBe("Needs the keychain");
    expect(status.toLowerCase()).not.toContain("fail");
  });

  test("a failed session says where it stopped, and a reload keeps the stage", () => {
    expect(stoppedAt("verify")).toBe("Stopped while checking the transcript");
    const entry = { id: "s", started: "", duration_ms: 0, state: "processing", stage: "transcribe", title: null, found_speakers: null };
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
