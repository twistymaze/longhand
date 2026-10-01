import { describe, expect, test } from "bun:test";
import { features } from "../lib/features";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { MicrophoneList, Settings } from "../bindings";
import { mockTauri, shortcutStatuses } from "../test/tauri";
import { SettingsWindow } from "./SettingsWindow";

function settings(extra: Partial<Settings> = {}): Settings {
  return {
    settings_schema_version: 4,
    interface_zoom_percent: 100,
    shortcut: "CmdOrCtrl+Shift+R",
    show_window_shortcut: null,
    storage_folder: "/notes",
    endpoint_base_url: "",
    endpoint_model: "",
    context_limit_tokens: null,
    default_speaker_count: null,
    asr_model: "parakeet-unified-en-0.6b-q80",
    live_transcript: true,
    speaker_source: "pyannote",
    speaker_processing_mode: "after_stop",
    recognition_language: null,
    chosen_recognition: { asr_model: "parakeet-unified-en-0.6b-q80", live_transcript: true, speaker_source: "pyannote", speaker_processing_mode: "after_stop", recognition_language: null, default_speaker_count: null },
    recognition: { differs: false, chosen_not_known: false, waiting_for: [], failure: null },
    microphone: null,
    system_audio_default: false,
    hide_from_dock: false,
    check_updates_automatically: true,
    audio_removal: { kind: "after_days", days: 30 },
    learned_context_limits: {},
    summary_prompts: [],
    active_summary_prompt: "topic-summary",
    shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [] },
    ...extra,
  };
}

const twoInputs: MicrophoneList = {
  default_id: "builtin",
  inputs: [
    { id: "builtin", name: "MacBook Pro Microphone" },
    { id: "usb", name: "USB Audio" },
  ],
};

/// Mock the backend with mutable state, render the Settings window and open
/// the Recording pane.
async function open({
  extra = {},
  microphones = twoInputs,
}: {
  extra?: Partial<Settings>;
  microphones?: MicrophoneList;
} = {}) {
  const state = { settings: settings(extra), microphones };
  const mock = mockTauri({
    get_settings: () => state.settings,
    move_status: () => null,
    readiness: () => ({
      microphone_authorized: true,
      system_audio: "allowed",
      destination: { kind: "ready", path: "/notes", free_bytes: null },
      armed: false,
      shortcut: { chord: "CmdOrCtrl+Shift+R", registered: true, error: null },
      missing_models: [],
      endpoint_configured: false, can_start_note: true, note_start_refusal: null,
    }),
    shortcut_status: () => shortcutStatuses,
    login_item_status: () => "disabled",
    model_statuses: () => [],
    speaker_models_status: () => ({ ids: [], size_bytes: 0, state: { state: "present" } }),
    list_microphones: () => state.microphones,
    set_microphone: ({ microphone }) => {
      state.settings = { ...state.settings, microphone: microphone as Settings["microphone"] };
      return state.settings;
    },
    set_system_audio_default: ({ enabled }) => {
      state.settings = { ...state.settings, system_audio_default: enabled as boolean };
      return state.settings;
    },
  });
  const user = userEvent.setup();
  render(<SettingsWindow />);
  await user.click(await screen.findByRole("tab", { name: /Recording/ }));
  const group = await screen.findByRole("radiogroup", { name: "Microphone" });
  await waitFor(() => expect(within(group).getAllByRole("radio").length).toBeGreaterThan(1));
  return { mock, user, state, group };
}

const calls = (mock: { calls: { cmd: string; args: Record<string, unknown> }[] }, cmd: string) =>
  mock.calls.filter((c) => c.cmd === cmd).map((c) => c.args);

const checked = (group: HTMLElement) =>
  within(group)
    .getAllByRole("radio")
    .filter((r) => r.getAttribute("aria-checked") === "true")
    .map((r) => r.getAttribute("aria-label"));

describe("Recording pane", () => {
  test("with two inputs and no choice, System default is selected", async () => {
    const { group } = await open();
    const radios = within(group).getAllByRole("radio");
    expect(radios.map((r) => r.getAttribute("aria-label"))).toEqual([
      "System default",
      "MacBook Pro Microphone",
      "USB Audio",
    ]);
    expect(checked(group)).toEqual(["System default"]);
    // The default row names the input that the system uses now.
    expect(within(group).getByText("Currently MacBook Pro Microphone")).toBeTruthy();
  });

  test("choosing an input calls the set-microphone command and marks it selected", async () => {
    const { mock, user, group } = await open();
    await user.click(within(group).getByRole("radio", { name: "USB Audio" }));
    expect(calls(mock, "set_microphone")).toEqual([{ microphone: { id: "usb", label: "USB Audio" } }]);
    await waitFor(() => expect(checked(group)).toEqual(["USB Audio"]));

    await user.click(within(group).getByRole("radio", { name: "System default" }));
    expect(calls(mock, "set_microphone").slice(-1)[0]).toEqual({ microphone: null });
    await waitFor(() => expect(checked(group)).toEqual(["System default"]));
  });

  test("a click gives the radio focus, and the arrow keys move the choice", async () => {
    const { mock, user, group } = await open();
    const usb = within(group).getByRole("radio", { name: "USB Audio" });
    await user.click(usb);
    // WKWebView does not focus a clicked button; the row does it.
    expect(document.activeElement).toBe(usb);
    await user.keyboard("{ArrowUp}");
    expect(calls(mock, "set_microphone").slice(-1)[0]).toEqual({
      microphone: { id: "builtin", label: "MacBook Pro Microphone" },
    });
    await waitFor(() => expect(checked(group)).toEqual(["MacBook Pro Microphone"]));
    expect(document.activeElement).toBe(within(group).getByRole("radio", { name: "MacBook Pro Microphone" }));
  });

  test("two inputs with the same name get different labels, and the second sends its own ID", async () => {
    const { mock, user, group } = await open({
      microphones: {
        default_id: "builtin",
        inputs: [
          { id: "builtin", name: "MacBook Pro Microphone" },
          { id: "usb-a", name: "USB Audio" },
          { id: "usb-b", name: "USB Audio" },
        ],
      },
    });
    expect(within(group).getAllByRole("radio").map((r) => r.getAttribute("aria-label"))).toEqual([
      "System default",
      "MacBook Pro Microphone",
      "USB Audio 1",
      "USB Audio 2",
    ]);
    await user.click(within(group).getByRole("radio", { name: "USB Audio 2" }));
    expect(calls(mock, "set_microphone")).toEqual([{ microphone: { id: "usb-b", label: "USB Audio" } }]);
    await waitFor(() => expect(checked(group)).toEqual(["USB Audio 2"]));
  });

  test("a chosen input that is not in the list shows Not connected and stays selected (AE4)", async () => {
    const { group } = await open({ extra: { microphone: { id: "gone", label: "Yeti Stereo Microphone" } } });
    const radio = within(group).getByRole("radio", { name: "Yeti Stereo Microphone" });
    expect(radio.getAttribute("aria-checked")).toBe("true");
    expect(within(radio).getByText("Not connected")).toBeTruthy();
    expect(checked(group)).toEqual(["Yeti Stereo Microphone"]);
  });

  test("the list refreshes when the Settings window gets focus", async () => {
    const { mock, state, group } = await open();
    const before = calls(mock, "list_microphones").length;
    state.microphones = {
      default_id: "builtin",
      inputs: [...twoInputs.inputs, { id: "headset", name: "AirPods Pro" }],
    };
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
    });
    await within(group).findByRole("radio", { name: "AirPods Pro" });
    expect(calls(mock, "list_microphones").length).toBeGreaterThan(before);
  });

  test("while Computer audio is hidden, the pane has no Computer audio switch", async () => {
    await open();
    expect(screen.queryByRole("switch", { name: "Computer audio" })).toBeNull();
  });

  test("turning the Computer audio switch on calls the command and shows it on", async () => {
    features.systemAudio = true;
    try {
      const { mock, user } = await open();
      const toggle = screen.getByRole("switch", { name: "Computer audio" });
      expect(toggle.getAttribute("aria-checked")).toBe("false");
      await user.click(toggle);
      expect(calls(mock, "set_system_audio_default")).toEqual([{ enabled: true }]);
      await waitFor(() => expect(screen.getByRole("switch", { name: "Computer audio" }).getAttribute("aria-checked")).toBe("true"));
      expect(screen.getByText(/all sound that the computer plays/)).toBeTruthy();
    } finally {
      features.systemAudio = false;
    }
  });

  test("the pane is after General in the sidebar and opens with the arrow keys", async () => {
    const { user } = await open();
    const tabs = screen.getAllByRole("tab").map((t) => t.getAttribute("data-pane"));
    expect(tabs.slice(0, 3)).toEqual(["status", "recording", "destination"]);

    await user.click(screen.getByRole("tab", { name: /General/ }));
    screen.getByRole("tab", { name: /General/ }).focus();
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("tab", { name: /Recording/ }).getAttribute("aria-selected")).toBe("true");
    expect(await screen.findByRole("radiogroup", { name: "Microphone" })).toBeTruthy();
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("tab", { name: /Storage/ }).getAttribute("aria-selected")).toBe("true");
  });
});
