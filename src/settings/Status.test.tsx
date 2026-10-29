import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import type { Readiness, Settings } from "../bindings";
import { mockTauri, shortcutStatuses } from "../test/tauri";
import { type PaneId, StatusPane } from "./Status";

const readiness: Readiness = {
  microphone_authorized: true,
  system_audio: "allowed",
  destination: { kind: "ready", path: "/x", free_bytes: null },
  armed: true,
  shortcut: shortcutStatuses.recording,
  missing_models: [],
  endpoint_configured: true, can_start_note: true, note_start_refusal: null,
};

const baseSettings: Settings = {
  settings_schema_version: 4,
  interface_zoom_percent: 100,
  shortcut: "Alt+Shift+L",
  show_window_shortcut: null,
  pause_shortcut: null,
  widget_shortcut: null,
  storage_folder: "/notes",
  active_provider: null,
  providers: {
    openai: { address: null, model: "", context_limit_tokens: null },
    anthropic: { address: null, model: "", context_limit_tokens: null },
    gemini: { address: null, model: "", context_limit_tokens: null },
    openrouter: { address: null, model: "", context_limit_tokens: null },
    ollama: { address: null, model: "", context_limit_tokens: null },
    lm_studio: { address: null, model: "", context_limit_tokens: null },
    custom: { address: null, model: "", context_limit_tokens: null },
  },
  default_speaker_count: null,
  asr_model: "a",
  live_transcript: true,
  speaker_source: "pyannote",
  speaker_processing_mode: "after_stop",
  recognition_language: null,
  chosen_recognition: { asr_model: "a", live_transcript: true, speaker_source: "pyannote", speaker_processing_mode: "after_stop", recognition_language: null, default_speaker_count: null },
  recognition: { differs: false, chosen_not_known: false, waiting_for: [], failure: null },
  microphone: null,
  apps: [],
  new_note_records: true,
  alert_sound: true,
  show_recording_widget: true,
  hide_from_dock: false,
  keep_running_in_tray: false,
  watched_folder: null,
  check_updates_automatically: true,
  audio_removal: { kind: "after_days", days: 30 },
  summary_prompts: [],
  active_summary_prompt: "topic-summary",
  shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [], files_writable: true, damaged_file: null },
  background: "",
  background_limit: 4000,
  background_read_only: null,
};

/// The pane with the settings state that the Settings window holds. A
/// successful setter replaces the settings, as a refresh does.
function Harness({
  readiness: r,
  onRefresh = async () => {},
  onNavigate = () => {},
  initial = baseSettings,
}: {
  readiness: Readiness;
  onRefresh?: () => Promise<void>;
  initial?: Settings;
  onNavigate?: (p: PaneId) => void;
}) {
  const [settings, setSettings] = useState(initial);
  return <StatusPane readiness={r} settings={settings} onSettings={setSettings} onRefresh={onRefresh} onNavigate={onNavigate} />;
}

function setUserAgent(value: string) {
  Object.defineProperty(navigator, "userAgent", { value, configurable: true });
}

function renderWindowGroup(handlers: Record<string, (args: Record<string, unknown>) => unknown> = {}) {
  const tauri = mockTauri({ shortcut_status: () => shortcutStatuses, login_item_status: () => "disabled", ...handlers });
  render(<Harness readiness={readiness} />);
  const group = () => screen.getByRole("heading", { name: "Window" }).closest("section") as HTMLElement;
  return { tauri, group };
}

function renderSetup(r: Readiness, initial: Settings = baseSettings) {
  mockTauri({ shortcut_status: () => shortcutStatuses, login_item_status: () => "disabled" });
  const opened: PaneId[] = [];
  render(<Harness readiness={r} initial={initial} onNavigate={(p) => opened.push(p)} />);
  const group = () => screen.getByRole("heading", { name: "Setup" }).closest("section") as HTMLElement;
  const rows = () => Array.from(group().querySelectorAll(".settings-row")).map((el) => el.textContent ?? "");
  return { group, rows, opened };
}

test("the General pane shows its groups in the order Setup, Window, Updates", () => {
  mockTauri({ shortcut_status: () => shortcutStatuses, login_item_status: () => "disabled" });
  render(<Harness readiness={readiness} />);
  expect(screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent)).toEqual([
    "Setup",
    "Window",
    "Updates",
  ]);
});

test("the General pane shows no setting that belongs to another pane", () => {
  mockTauri({ shortcut_status: () => shortcutStatuses, login_item_status: () => "disabled" });
  // Not ready, with app audio not allowed and no app on the list: only the
  // setup items can name these topics, and each item has one row.
  render(
    <Harness
      readiness={{ ...readiness, microphone_authorized: false, armed: false, system_audio: "denied" }}
      initial={{ ...baseSettings, apps: [] }}
    />,
  );
  const text = document.body.textContent ?? "";
  expect(screen.getAllByText("Microphone access")).toHaveLength(1);
  for (const moved of ["Recording shortcut", "App audio", "Models", "Summaries", "Alert sound", "Play Sound", "Test Sound"]) {
    expect(text).not.toContain(moved);
  }
  expect(screen.queryByRole("radiogroup", { name: "Microphone" })).toBeNull();
});

describe("the setup group", () => {
  afterEach(() => {
    delete (navigator as { userAgent?: string }).userAgent;
  });

  test("Covers AE1. All ready: only the Ready to record row", () => {
    const { rows, group } = renderSetup(readiness);
    expect(rows()).toEqual(["Ready to record"]);
    expect(group().textContent).not.toContain("processed once");
  });

  test("Covers AE2. Models and provider missing: ready to record, with two items that open their panes", async () => {
    const { rows, group, opened } = renderSetup({ ...readiness, missing_models: ["a", "b"], endpoint_configured: false });
    expect(rows()).toHaveLength(3);
    expect(rows()[0]).toContain("Ready to record");
    expect(rows()[0]).toContain("The items below do not stop a recording");
    expect(rows()[1]).toContain("2 still to download");
    expect(rows()[2]).toContain("Not set up yet");
    const user = userEvent.setup();
    await user.click(within(group()).getByRole("button", { name: "Download…" }));
    await user.click(within(group()).getByRole("button", { name: "Set Up…" }));
    expect(opened).toEqual(["recognition", "provider"]);
  });

  test("Covers AE3. Microphone not allowed on macOS: not ready, with Allow in place", () => {
    setUserAgent("Mozilla/5.0 (Macintosh; Mac OS X)");
    const { rows, group } = renderSetup({ ...readiness, microphone_authorized: false, armed: false });
    expect(rows()[0]).toContain("Not ready to record");
    expect(rows()[1]).toContain("Microphone access");
    expect(within(group()).getByRole("button", { name: "Allow…" })).toBeTruthy();
  });

  test("on another platform, the microphone item has no button", () => {
    setUserAgent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)");
    const { group } = renderSetup({ ...readiness, microphone_authorized: false, armed: false });
    expect(group().textContent).toContain("Microphone access");
    expect(within(group()).queryAllByRole("button")).toHaveLength(0);
  });

  test("a storage folder low on space is listed and opens Storage", async () => {
    const { group, opened } = renderSetup({
      ...readiness,
      destination: { kind: "low_space", path: "/x", free_bytes: 1000, floor_bytes: 5000 },
    });
    expect(group().textContent).toContain("Storage");
    expect(group().textContent).toContain("free");
    await userEvent.setup().click(within(group()).getByRole("button", { name: "Fix…" }));
    expect(opened).toEqual(["destination"]);
  });

  test("a read-only storage folder is listed with its reason", () => {
    const readOnly: Settings = {
      ...baseSettings,
      shared: { ...baseSettings.shared, availability: { kind: "read_only", cause: "folder_not_available" } },
    };
    const { group } = renderSetup(readiness, readOnly);
    expect(group().textContent).toContain("The storage folder is not available.");
  });

  test("a recording shortcut that did not register is listed and opens Recording", async () => {
    const { group, opened } = renderSetup({
      ...readiness,
      armed: false,
      shortcut: { chord: "Alt+Shift+L", registered: false, error: "Another app uses this shortcut" },
    });
    expect(group().textContent).toContain("Another app uses this shortcut");
    await userEvent.setup().click(within(group()).getByRole("button", { name: "Fix…" }));
    expect(opened).toEqual(["recording"]);
  });

  test("app audio access is listed only while the app list is not empty", () => {
    const denied = { ...readiness, system_audio: "denied" as const };
    const first = renderSetup(denied);
    expect(first.group().textContent).not.toContain("App audio access");
    cleanup();
    const second = renderSetup(denied, { ...baseSettings, apps: [{ id: "us.zoom.xos", name: "zoom.us", path: "/Applications/zoom.us.app" }] });
    expect(second.group().textContent).toContain("App audio access");
    expect(within(second.group()).getByRole("button", { name: "Open System Settings" })).toBeTruthy();
  });
});

describe("the Hide from Dock switch on macOS", () => {
  beforeEach(() => setUserAgent("Mozilla/5.0 (Macintosh; Mac OS X)"));
  afterEach(() => {
    delete (navigator as { userAgent?: string }).userAgent;
  });

  test("shows, is off by default, and a click turns it on", async () => {
    const { tauri, group } = renderWindowGroup({
      set_hide_from_dock: (args) => ({ ...baseSettings, hide_from_dock: args.enabled as boolean }),
    });
    const toggle = within(group()).getByRole("switch", { name: "Hide from Dock" });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    await userEvent.setup().click(toggle);
    expect(tauri.calls.filter((c) => c.cmd === "set_hide_from_dock").map((c) => c.args)).toEqual([{ enabled: true }]);
    await waitFor(() => expect(toggle.getAttribute("aria-checked")).toBe("true"));
  });

  test("a failed change shows the error and keeps the previous value", async () => {
    const { group } = renderWindowGroup({
      set_hide_from_dock: () => {
        throw "could not save settings";
      },
    });
    const toggle = within(group()).getByRole("switch", { name: "Hide from Dock" });
    await userEvent.setup().click(toggle);
    expect(await within(group()).findByText(/could not save settings/i)).toBeTruthy();
    expect(toggle.getAttribute("aria-checked")).toBe("false");
  });

  test("the switch is off while a change is in progress", async () => {
    let finish: () => void = () => {};
    const { tauri, group } = renderWindowGroup({
      set_hide_from_dock: (args) =>
        new Promise<Settings>((resolve) => {
          finish = () => resolve({ ...baseSettings, hide_from_dock: args.enabled as boolean });
        }),
    });
    const toggle = within(group()).getByRole("switch", { name: "Hide from Dock" }) as HTMLButtonElement;
    const user = userEvent.setup();
    await user.click(toggle);
    await waitFor(() => expect(toggle.disabled).toBe(true));
    await user.click(toggle);
    expect(tauri.calls.filter((c) => c.cmd === "set_hide_from_dock")).toHaveLength(1);
    finish();
    await waitFor(() => expect(toggle.disabled).toBe(false));
    expect(toggle.getAttribute("aria-checked")).toBe("true");
  });

  test("the footer follows the switch value", async () => {
    const { group } = renderWindowGroup({
      set_hide_from_dock: (args) => ({ ...baseSettings, hide_from_dock: args.enabled as boolean }),
    });
    expect(group().textContent).toContain("Closing the window keeps Longhand in the Dock and the menu bar.");
    await userEvent.setup().click(within(group()).getByRole("switch", { name: "Hide from Dock" }));
    await waitFor(() =>
      expect(group().textContent).toContain("Closing the window keeps Longhand running only in the menu bar."),
    );
    expect(group().textContent).not.toContain("Dock and the menu bar");
  });
});

function renderUpdatesGroup(handlers: Record<string, (args: Record<string, unknown>) => unknown> = {}) {
  const tauri = mockTauri({ shortcut_status: () => shortcutStatuses, login_item_status: () => "disabled", ...handlers });
  render(<Harness readiness={readiness} />);
  const group = () => screen.getByRole("heading", { name: "Updates" }).closest("section") as HTMLElement;
  return { tauri, group };
}

describe("the Updates group on macOS", () => {
  beforeEach(() => setUserAgent("Mozilla/5.0 (Macintosh; Mac OS X)"));
  afterEach(() => {
    delete (navigator as { userAgent?: string }).userAgent;
  });

  test("shows after Window, and the switch is on by default", () => {
    const { group } = renderUpdatesGroup();
    expect(screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent)).toEqual([
      "Setup",
      "Window",
      "Updates",
    ]);
    const toggle = within(group()).getByRole("switch", { name: "Check for updates automatically" });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(group().textContent).toContain("Longhand asks before it installs a new version.");
  });

  test("a click turns automatic checks off", async () => {
    const { tauri, group } = renderUpdatesGroup({
      set_check_updates_automatically: (args) => ({ ...baseSettings, check_updates_automatically: args.enabled as boolean }),
    });
    const toggle = within(group()).getByRole("switch", { name: "Check for updates automatically" });
    await userEvent.setup().click(toggle);
    expect(tauri.calls.filter((c) => c.cmd === "set_check_updates_automatically").map((c) => c.args)).toEqual([
      { enabled: false },
    ]);
    await waitFor(() => expect(toggle.getAttribute("aria-checked")).toBe("false"));
  });

  test("a failed change shows the error and keeps the previous value", async () => {
    const { group } = renderUpdatesGroup({
      set_check_updates_automatically: () => {
        throw "could not save settings";
      },
    });
    const toggle = within(group()).getByRole("switch", { name: "Check for updates automatically" });
    await userEvent.setup().click(toggle);
    expect(await within(group()).findByText(/could not save settings/i)).toBeTruthy();
    expect(toggle.getAttribute("aria-checked")).toBe("true");
  });

  test("the switch is read-only while the storage folder is not available", () => {
    mockTauri({ shortcut_status: () => shortcutStatuses, login_item_status: () => "disabled" });
    const readOnly: Settings = {
      ...baseSettings,
      shared: { ...baseSettings.shared, availability: { kind: "read_only", cause: "folder_not_available" } },
    };
    render(<Harness readiness={readiness} initial={readOnly} />);
    const group = screen.getByRole("heading", { name: "Updates" }).closest("section") as HTMLElement;
    const toggle = within(group).getByRole("switch", { name: "Check for updates automatically" }) as HTMLButtonElement;
    expect(toggle.disabled).toBe(true);
    expect(group.textContent).toContain("The storage folder is not available.");
  });
});

describe("the Updates group on Windows", () => {
  beforeEach(() => setUserAgent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"));
  afterEach(() => {
    delete (navigator as { userAgent?: string }).userAgent;
  });

  test("shows after Window, and the switch is on by default", () => {
    const { group } = renderUpdatesGroup();
    expect(screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent)).toEqual([
      "Setup",
      "Window",
      "Updates",
    ]);
    const toggle = within(group()).getByRole("switch", { name: "Check for updates automatically" });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(group().textContent).toContain("Longhand asks before it installs a new version.");
  });

  test("the switch is read-only while the storage folder is not available", () => {
    mockTauri({ shortcut_status: () => shortcutStatuses, login_item_status: () => "disabled" });
    const readOnly: Settings = {
      ...baseSettings,
      shared: { ...baseSettings.shared, availability: { kind: "read_only", cause: "folder_not_available" } },
    };
    render(<Harness readiness={readiness} initial={readOnly} />);
    const group = screen.getByRole("heading", { name: "Updates" }).closest("section") as HTMLElement;
    const toggle = within(group).getByRole("switch", { name: "Check for updates automatically" }) as HTMLButtonElement;
    expect(toggle.disabled).toBe(true);
    expect(group.textContent).toContain("The storage folder is not available.");
  });
});

describe("the Window group on other platforms", () => {
  beforeEach(() => setUserAgent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"));
  afterEach(() => {
    delete (navigator as { userAgent?: string }).userAgent;
  });

  test("has no Hide from Dock switch, and the footer says that a close quits", () => {
    const { group } = renderWindowGroup();
    expect(within(group()).queryByRole("switch", { name: "Hide from Dock" })).toBeNull();
    expect(group().textContent).toContain("Closing the window quits Longhand, except during a recording.");
    expect(group().textContent).not.toContain("menu bar");
  });

  test("the system tray switch is off by default, and the footer follows it", async () => {
    const { tauri, group } = renderWindowGroup({
      set_keep_running_in_tray: (args) => ({ ...baseSettings, keep_running_in_tray: args.enabled as boolean }),
    });
    const toggle = within(group()).getByRole("switch", { name: "Keep running in the system tray" });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    await userEvent.setup().click(toggle);
    expect(tauri.calls.filter((c) => c.cmd === "set_keep_running_in_tray").map((c) => c.args)).toEqual([
      { enabled: true },
    ]);
    await waitFor(() =>
      expect(group().textContent).toContain("Closing the window keeps Longhand running in the system tray."),
    );
  });
});
