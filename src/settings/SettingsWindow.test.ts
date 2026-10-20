import { describe, expect, test } from "vitest";
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
  shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [], files_writable: true, damaged_file: null },
} as unknown as Settings;

const tones = (r: Readiness) => Object.fromEntries(paneOrder.map((id) => [id, paneTone(id, r, settings)]));

describe("the sidebar", () => {
  test("lists the panes in the order General, Storage, Recording, Transcription, AI Provider, Summaries, Types and Tags, About", () => {
    expect(paneOrder).toEqual(["status", "destination", "recording", "recognition", "provider", "summarization", "types", "about"]);
  });

  test("all ready: no pane has a mark", () => {
    expect(Object.values(tones(ready)).every((t) => t === null)).toBe(true);
  });

  test("Covers AE3. Microphone not allowed: General and Recording are caution", () => {
    const t = tones({ ...ready, microphone_authorized: false, armed: false });
    expect(t.status).toBe("caution");
    expect(t.recording).toBe("caution");
  });

  test("storage and provider marks follow the setup items", () => {
    const t = tones({
      ...ready,
      destination: { kind: "low_space", path: "/x", free_bytes: 1, floor_bytes: 2 },
      endpoint_configured: false,
    });
    expect(t.destination).toBe("caution");
    expect(t.provider).toBe("neutral");
    expect(t.summarization).toBeNull();
    expect(t.status).toBe("neutral");
  });

  test("Covers AE2. Only models outstanding: General and Transcription are neutral, Recording has no mark", () => {
    const t = tones({ ...ready, missing_models: ["a"] });
    expect(t.status).toBe("neutral");
    expect(t.recognition).toBe("neutral");
    expect(t.recording).toBeNull();
  });
});
