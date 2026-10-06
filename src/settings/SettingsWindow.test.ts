import { describe, expect, test } from "bun:test";
import type { Readiness, Settings } from "../bindings";
import { paneOrder, paneTone } from "./SettingsWindow";

const ready: Readiness = {
  microphone_authorized: true,
  system_audio: "allowed",
  destination: { kind: "ready", path: "/x", free_bytes: null },
  armed: true,
  shortcut: { chord: "Alt+Shift+L", registered: true, error: null },
  missing_models: [],
  endpoint_configured: true,
  can_start_note: true,
  note_start_refusal: null,
};

const settings = {
  system_audio_default: false,
  shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [] },
} as unknown as Settings;

const tones = (r: Readiness) => Object.fromEntries(paneOrder.map((id) => [id, paneTone(id, r, settings)]));

describe("the sidebar", () => {
  test("lists the panes in the order General, Storage, Recording, Transcription, Summaries, About", () => {
    expect(paneOrder).toEqual(["status", "destination", "recording", "recognition", "summarization", "about"]);
  });

  test("all ready: no pane has a mark", () => {
    expect(Object.values(tones(ready)).every((t) => t === null)).toBe(true);
  });

  test("Covers AE3. Microphone not allowed: General and Recording are caution", () => {
    const t = tones({ ...ready, microphone_authorized: false, armed: false });
    expect(t.status).toBe("caution");
    expect(t.recording).toBe("caution");
  });

  test("Covers AE2. Only models outstanding: General and Transcription are neutral, Recording has no mark", () => {
    const t = tones({ ...ready, missing_models: ["a"] });
    expect(t.status).toBe("neutral");
    expect(t.recognition).toBe("neutral");
    expect(t.recording).toBeNull();
  });
});
