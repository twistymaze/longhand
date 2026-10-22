import { describe, expect, test } from "vitest";
import type { Readiness, Settings } from "../bindings";
import { outstandingItems } from "./setup";

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

// Only the fields that the function reads.
const settings = {
  apps: [],
  shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [], files_writable: true, damaged_file: null },
} as unknown as Settings;

const ids = (r: Readiness, s: Settings = settings, systemAudio = false) =>
  outstandingItems(r, s, { systemAudio }).map((i) => i.id);

describe("the outstanding setup items", () => {
  test("Covers AE1. Nothing is outstanding when everything is ready", () => {
    expect(ids(ready)).toEqual([]);
  });

  test("Covers AE2. Models and endpoint are outstanding and do not stop a recording", () => {
    const items = outstandingItems({ ...ready, missing_models: ["a", "b"], endpoint_configured: false }, settings, {
      systemAudio: false,
    });
    expect(items.map((i) => i.id)).toEqual(["models", "provider"]);
    expect(items.every((i) => !i.blocksRecording)).toBe(true);
  });

  test("Covers AE3. A microphone that is not allowed comes first and stops a recording", () => {
    const items = outstandingItems({ ...ready, microphone_authorized: false, armed: false, endpoint_configured: false }, settings, {
      systemAudio: false,
    });
    expect(items.map((i) => i.id)).toEqual(["microphone", "provider"]);
    expect(items[0].blocksRecording).toBe(true);
  });

  test("a shortcut that did not register is outstanding; a cleared shortcut is not", () => {
    expect(ids({ ...ready, armed: false, shortcut: { chord: "Alt+Shift+L", registered: false, error: "taken" } })).toEqual([
      "shortcut",
    ]);
    expect(ids({ ...ready, shortcut: { chord: null, registered: false, error: null } })).toEqual([]);
  });

  test("a storage folder that is missing, low on space, or read-only is outstanding", () => {
    expect(ids({ ...ready, destination: { kind: "missing", path: "/x" } } as Readiness)).toEqual(["storage"]);
    expect(ids({ ...ready, destination: { kind: "low_space", path: "/x", free_bytes: 1 } } as Readiness)).toEqual(["storage"]);
    const readOnly = {
      ...settings,
      shared: { availability: { kind: "read_only", cause: "folder_not_available" }, newer_version: false, unknown_values: [], files_writable: false, damaged_file: null },
    } as unknown as Settings;
    expect(ids(ready, readOnly)).toEqual(["storage"]);
  });

  test("Covers AE4. Computer audio access is outstanding only when new notes start with Computer audio on", () => {
    const notAllowed = { ...ready, system_audio: "denied" as const };
    expect(ids(notAllowed, settings, true)).toEqual([]);
    expect(ids(notAllowed, { ...settings, apps: [{ id: "us.zoom.xos", name: "zoom.us", path: "/Applications/zoom.us.app" }] }, true)).toEqual(["system_audio"]);
    expect(ids(notAllowed, { ...settings, apps: [{ id: "us.zoom.xos", name: "zoom.us", path: "/Applications/zoom.us.app" }] }, false)).toEqual([]);
  });

  test("all items come in the order microphone, shortcut, storage, Computer audio, models, endpoint", () => {
    const all: Readiness = {
      ...ready,
      microphone_authorized: false,
      armed: false,
      shortcut: { chord: "Alt+Shift+L", registered: false, error: null },
      destination: { kind: "missing", path: "/x" } as Readiness["destination"],
      system_audio: "not_asked",
      missing_models: ["a"],
      endpoint_configured: false,
    };
    expect(ids(all, { ...settings, apps: [{ id: "us.zoom.xos", name: "zoom.us", path: "/Applications/zoom.us.app" }] }, true)).toEqual([
      "microphone",
      "shortcut",
      "storage",
      "system_audio",
      "models",
      "provider",
    ]);
  });
});
