import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import type { Readiness, Settings } from "../bindings";
import { mockTauri, shortcutStatuses } from "../test/tauri";
import { StatusPane } from "./Status";
import { features } from "../lib/features";

const readiness: Readiness = {
  microphone_authorized: true,
  system_audio: "allowed",
  destination: { kind: "ready", path: "/x", free_bytes: null },
  armed: true,
  shortcut: shortcutStatuses.recording,
  missing_models: [],
  endpoint_configured: true,
};

const baseSettings: Settings = {
  settings_schema_version: 4,
  interface_zoom_percent: 100,
  shortcut: "Alt+Shift+L",
  show_window_shortcut: null,
  storage_folder: "/notes",
  endpoint_base_url: null,
  endpoint_model: "",
  context_limit_tokens: null,
  default_speaker_count: null,
  asr_model: "a",
  live_transcript: true,
  speaker_source: "pyannote",
  speaker_processing_mode: "after_stop",
  recognition_language: null,
  chosen_recognition: { asr_model: "a", live_transcript: true, speaker_source: "pyannote", speaker_processing_mode: "after_stop", recognition_language: null, default_speaker_count: null },
  recognition: { differs: false, chosen_not_known: false, waiting_for: [], failure: null },
  microphone: null,
  system_audio_default: false,
  hide_from_dock: false,
  check_updates_automatically: true,
  learned_context_limits: {},
  summary_prompts: [],
  active_summary_prompt: "topic-summary",
  shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [] },
};

/// The pane with the settings state that the Settings window holds. A
/// successful setter replaces the settings, as a refresh does.
function Harness({
  readiness: r,
  onRefresh = async () => {},
  initial = baseSettings,
}: {
  readiness: Readiness;
  onRefresh?: () => Promise<void>;
  initial?: Settings;
}) {
  const [settings, setSettings] = useState(initial);
  return <StatusPane readiness={r} settings={settings} onSettings={setSettings} onRefresh={onRefresh} onNavigate={() => {}} />;
}

function renderPane(playResult: () => unknown = () => null) {
  const tauri = mockTauri({
    shortcut_status: () => shortcutStatuses,
    login_item_status: () => "disabled",
    play_alert_sound: playResult,
  });
  render(<Harness readiness={readiness} />);
  const row = () => screen.getByText("Alert sound").closest(".settings-row") as HTMLElement;
  return { tauri, row };
}

describe("the alert sound row", () => {
  test("says what the sound is for and asks for no test", () => {
    const { row } = renderPane();
    expect(row().textContent).toContain("Plays when recording stops and cannot restart");
    expect(document.body.textContent).not.toMatch(/Not tested|Heard at setup|Tested|Do Not Disturb|silence the notification/);
  });

  test("Play Sound plays the sound and asks no question", async () => {
    const { tauri, row } = renderPane();
    await userEvent.setup().click(within(row()).getByRole("button", { name: "Play Sound" }));
    expect(tauri.calls.filter((c) => c.cmd === "play_alert_sound")).toHaveLength(1);
    expect(screen.queryByText("Did you hear it?")).toBeNull();
    expect(screen.queryByRole("button", { name: "Yes" })).toBeNull();
    expect(screen.queryByRole("button", { name: "No" })).toBeNull();
    expect(within(row()).getByRole("button", { name: "Play Sound" })).toBeTruthy();
    expect(tauri.calls.some((c) => c.cmd === "confirm_alert_sound")).toBe(false);
  });

  test("a sound that cannot play shows the error", async () => {
    const { row } = renderPane(() => {
      throw "no output device";
    });
    await userEvent.setup().click(within(row()).getByRole("button", { name: "Play Sound" }));
    expect(await within(row()).findByText("no output device")).toBeTruthy();
  });
});

function renderSystemAudio(
  state: Readiness["system_audio"],
  handlers: Record<string, (args: Record<string, unknown>) => unknown> = {},
) {
  const tauri = mockTauri({ shortcut_status: () => shortcutStatuses,
    login_item_status: () => "disabled", ...handlers });
  let refreshed = 0;
  render(
    <Harness
      readiness={{ ...readiness, system_audio: state }}
      onRefresh={async () => {
        refreshed += 1;
      }}
    />,
  );
  const row = () => screen.getByText("Computer audio").closest(".settings-row") as HTMLElement;
  return { tauri, row, refreshed: () => refreshed };
}

test("while Computer audio is hidden, the General pane has no Computer audio row", () => {
  mockTauri({ shortcut_status: () => shortcutStatuses, login_item_status: () => "disabled" });
  render(<Harness readiness={{ ...readiness, system_audio: "not_asked" }} />);
  expect(screen.queryByText("Computer audio")).toBeNull();
});

describe("the Computer audio permission row", () => {
  beforeEach(() => {
    features.systemAudio = true;
  });
  afterEach(() => {
    features.systemAudio = false;
  });

  test("not asked: says so and offers Allow…", () => {
    const { row } = renderSystemAudio("not_asked");
    expect(row().textContent).toContain("Not allowed yet");
    expect(within(row()).getAllByRole("button").map((b) => b.textContent)).toEqual(["Allow…"]);
  });

  test("denied: says so and opens System Settings", async () => {
    const { tauri, row } = renderSystemAudio("denied", { open_system_audio_settings: () => null });
    expect(row().textContent).toContain("Not allowed");
    expect(row().textContent).not.toContain("Not allowed yet");
    const buttons = within(row()).getAllByRole("button");
    expect(buttons.map((b) => b.textContent)).toEqual(["Open System Settings"]);
    await userEvent.setup().click(buttons[0]);
    expect(tauri.calls.filter((c) => c.cmd === "open_system_audio_settings")).toHaveLength(1);
  });

  test("unknown: cannot check, and opens System Settings", () => {
    const { row } = renderSystemAudio("unknown");
    expect(row().textContent).toContain("Cannot check");
    expect(within(row()).getAllByRole("button").map((b) => b.textContent)).toEqual(["Open System Settings"]);
  });

  test("allowed: says so and shows no button", () => {
    const { row } = renderSystemAudio("allowed");
    expect(row().textContent).toContain("Allowed");
    expect(within(row()).queryAllByRole("button")).toHaveLength(0);
  });

  test("Allow… asks, checks the status again, and refreshes the pane", async () => {
    let checks = 0;
    const { tauri, row, refreshed } = renderSystemAudio("not_asked", {
      request_system_audio_permission: () => "not_asked",
      system_audio_permission: () => {
        checks += 1;
        return checks < 2 ? "not_asked" : "allowed";
      },
    });
    await userEvent.setup().click(within(row()).getByRole("button", { name: "Allow…" }));
    await waitFor(() => expect(refreshed()).toBe(1), { timeout: 3000 });
    expect(tauri.calls.filter((c) => c.cmd === "request_system_audio_permission")).toHaveLength(1);
    expect(checks).toBe(2);
  });
});

function setUserAgent(value: string) {
  Object.defineProperty(navigator, "userAgent", { value, configurable: true });
}

function renderWindowGroup(handlers: Record<string, (args: Record<string, unknown>) => unknown> = {}) {
  const tauri = mockTauri({ shortcut_status: () => shortcutStatuses, login_item_status: () => "disabled", ...handlers });
  render(<Harness readiness={readiness} />);
  const group = () => screen.getByRole("heading", { name: "Window" }).closest("section") as HTMLElement;
  return { tauri, group };
}

test("the General pane shows its groups in the order Window, Recording, Processing, Alerts", () => {
  mockTauri({ shortcut_status: () => shortcutStatuses, login_item_status: () => "disabled" });
  render(<Harness readiness={readiness} />);
  expect(screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent)).toEqual([
    "Window",
    "Recording",
    "Processing",
    "Alerts",
  ]);
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

  test("shows after Processing, and the switch is on by default", () => {
    const { group } = renderUpdatesGroup();
    expect(screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent)).toEqual([
      "Window",
      "Recording",
      "Processing",
      "Updates",
      "Alerts",
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

describe("the Window group on other platforms", () => {
  beforeEach(() => setUserAgent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"));
  afterEach(() => {
    delete (navigator as { userAgent?: string }).userAgent;
  });

  test("has no Hide from Dock switch, and the footer says system tray", () => {
    const { group } = renderWindowGroup();
    expect(within(group()).queryByRole("switch", { name: "Hide from Dock" })).toBeNull();
    expect(group().textContent).toContain("Closing the window keeps Longhand running in the system tray.");
    expect(group().textContent).not.toContain("menu bar");
  });
});
