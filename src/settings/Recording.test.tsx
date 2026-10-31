import { afterEach, describe, expect, test } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ListedApp, MicrophoneList, Readiness, RunningApp, Settings } from "../bindings";
import { mockTauri, shortcutStatuses } from "../test/tauri";
import { SettingsWindow } from "./SettingsWindow";

function settings(extra: Partial<Settings> = {}): Settings {
  return {
    settings_schema_version: 4,
    interface_zoom_percent: 100,
    shortcut: "CmdOrCtrl+Shift+R",
    show_window_shortcut: null,
    pause_shortcut: null,
    widget_shortcut: null,
    storage_folder: "/notes",
    active_provider: null,
    providers: {
      openai: { address: null, model: "", context_limit_tokens: null, thinking: null, requests_at_once: null },
      anthropic: { address: null, model: "", context_limit_tokens: null, thinking: null, requests_at_once: null },
      gemini: { address: null, model: "", context_limit_tokens: null, thinking: null, requests_at_once: null },
      openrouter: { address: null, model: "", context_limit_tokens: null, thinking: null, requests_at_once: null },
      ollama: { address: null, model: "", context_limit_tokens: null, thinking: null, requests_at_once: null },
      lm_studio: { address: null, model: "", context_limit_tokens: null, thinking: null, requests_at_once: null },
      custom: { address: null, model: "", context_limit_tokens: null, thinking: null, requests_at_once: null },
    },
    default_speaker_count: null,
    asr_model: "parakeet-unified-en-0.6b-q80",
    live_transcript: true,
    speaker_source: "pyannote",
    speaker_processing_mode: "after_stop",
    recognition_language: null,
    chosen_recognition: { asr_model: "parakeet-unified-en-0.6b-q80", live_transcript: true, speaker_source: "pyannote", speaker_processing_mode: "after_stop", recognition_language: null, default_speaker_count: null },
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
    transcript_removal: { kind: "off" },
    summary_prompts: [],
    active_summary_prompt: "topic-summary",
    shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [], files_writable: true, damaged_file: null },
    background: "",
    background_limit: 4000,
    background_read_only: null,
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
  running = [],
  ready = {},
  handlers = {},
}: {
  extra?: Partial<Settings>;
  microphones?: MicrophoneList;
  /// The running apps with a window.
  running?: RunningApp[];
  ready?: Partial<Readiness>;
  handlers?: Record<string, (args: Record<string, unknown>) => unknown>;
} = {}) {
  const state = { settings: settings(extra), microphones, running };
  const listed = (id: string) => state.settings.apps.some((a) => a.id === id);
  const entry = (app: RunningApp): ListedApp => ({ id: app.id, name: app.name, path: app.path });
  const mock = mockTauri({
    get_settings: () => state.settings,
    move_status: () => null,
    agent_instructions_status: () => ({ kind: "not_added" }),
    readiness: () => ({
      microphone_authorized: true,
      system_audio: "allowed",
      destination: { kind: "ready", path: "/notes", free_bytes: null },
      armed: false,
      shortcut: { chord: "CmdOrCtrl+Shift+R", registered: true, error: null },
      missing_models: [],
      endpoint_configured: false, can_start_note: true, note_start_refusal: null,
      ...ready,
    }),
    shortcut_status: () => shortcutStatuses,
    login_item_status: () => "disabled",
    model_statuses: () => [],
    speaker_models_status: () => ({ ids: [], size_bytes: 0, state: { state: "present" } }),
    list_microphones: () => state.microphones,
    app_audio_available: () => true,
    list_running_apps: ({ all }) => state.running.filter((a) => all || !listed(a.id)),
    app_icon: () => null,
    add_app: ({ entry }) => {
      state.settings = { ...state.settings, apps: [...state.settings.apps, entry as ListedApp] };
      return state.settings;
    },
    add_call_apps: () => {
      const added = state.running.filter((a) => a.call_app && !listed(a.id)).map(entry);
      state.settings = { ...state.settings, apps: [...state.settings.apps, ...added] };
      return state.settings;
    },
    remove_app: ({ id }) => {
      state.settings = { ...state.settings, apps: state.settings.apps.filter((a) => a.id !== id) };
      return state.settings;
    },
    set_microphone: ({ microphone }) => {
      state.settings = { ...state.settings, microphone: microphone as Settings["microphone"] };
      return state.settings;
    },
    set_new_note_records: ({ enabled }) => {
      state.settings = { ...state.settings, new_note_records: enabled as boolean };
      return state.settings;
    },
    set_alert_sound: ({ enabled }) => {
      state.settings = { ...state.settings, alert_sound: enabled as boolean };
      return state.settings;
    },
    set_show_recording_widget: ({ enabled }) => {
      state.settings = { ...state.settings, show_recording_widget: enabled as boolean };
      return state.settings;
    },
    ...handlers,
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

  test("Covers R18. The pane has no Computer audio switch", async () => {
    await open();
    expect(screen.queryByRole("switch", { name: "Computer audio" })).toBeNull();
  });

  // Covers R8.
  test("the New Note switch shows the setting and turning it off saves the change", async () => {
    const { mock, user } = await open();
    const toggle = screen.getByRole("switch", { name: "New Note starts recording" });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    await user.click(toggle);
    expect(calls(mock, "set_new_note_records")).toEqual([{ enabled: false }]);
    await waitFor(() =>
      expect(screen.getByRole("switch", { name: "New Note starts recording" }).getAttribute("aria-checked")).toBe("false"),
    );
  });

  test("the recording shortcut is on this pane, and a change refreshes readiness", async () => {
    const { mock, user } = await open({
      handlers: {
        set_shortcut: () => shortcutStatuses,
        suspend_shortcuts: () => null,
        resume_shortcuts: () => null,
      },
    });
    const row = screen.getByText("Recording shortcut").closest(".settings-row") as HTMLElement;
    expect(row.textContent).toContain("Starts and stops a recording from any app");
    const before = calls(mock, "readiness").length;
    await user.click(within(row).getByRole("button", { name: "Clear" }));
    await waitFor(() => expect(calls(mock, "readiness").length).toBeGreaterThan(before));
  });

  test("with New Note recording off and no recording shortcut, the shortcut row says only Off", async () => {
    await open({
      extra: { new_note_records: false, shortcut: null },
      ready: { shortcut: { chord: null, registered: false, error: null } },
    });
    const row = screen.getByText("Recording shortcut").closest(".settings-row") as HTMLElement;
    expect(within(row).getByText("Off")).toBeTruthy();
    expect(row.textContent).not.toContain("New Note still starts a recording");
  });

  test("with New Note recording on and no recording shortcut, the shortcut row says New Note still records", async () => {
    await open({
      extra: { new_note_records: true, shortcut: null },
      ready: { shortcut: { chord: null, registered: false, error: null } },
    });
    const row = screen.getByText("Recording shortcut").closest(".settings-row") as HTMLElement;
    expect(within(row).getByText("Off — New Note still starts a recording")).toBeTruthy();
  });

  // Covers R16: a new install has no Pause chord.
  test("the Pause shortcut is under the recording shortcut and shows Off when it has no chord", async () => {
    await open();
    const rows = screen.getAllByText(/^(Recording shortcut|Pause shortcut)$/).map((el) => el.textContent);
    expect(rows).toEqual(["Recording shortcut", "Pause shortcut"]);
    const row = screen.getByText("Pause shortcut").closest(".settings-row") as HTMLElement;
    expect(row.textContent).toContain("Off");
    expect(within(row).queryByRole("button", { name: "Clear" })).toBeNull();
  });

  test("a registered Pause shortcut shows what it does, and Clear sends the pause kind", async () => {
    const pause = { chord: "Alt+Shift+P", registered: true, error: null };
    const { mock, user } = await open({
      handlers: {
        shortcut_status: () => ({ ...shortcutStatuses, pause }),
        set_shortcut: () => shortcutStatuses,
        suspend_shortcuts: () => null,
        resume_shortcuts: () => null,
      },
    });
    const row = (await screen.findByText("Pauses and continues a recording from any app")).closest(
      ".settings-row",
    ) as HTMLElement;
    expect(within(row).getByText("Pause shortcut")).toBeTruthy();
    await user.click(within(row).getByRole("button", { name: "Clear" }));
    expect(calls(mock, "set_shortcut")).toEqual([{ kind: "pause", chord: null }]);
    await waitFor(() => expect(row.textContent).toContain("Off"));
  });

  test("the recording widget switch is on by default, and turning it off saves the change", async () => {
    const { mock, user } = await open();
    const toggle = screen.getByRole("switch", { name: "Show the recording widget" });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(screen.getByText("A change applies from the next note")).toBeTruthy();
    await user.click(toggle);
    expect(calls(mock, "set_show_recording_widget")).toEqual([{ enabled: false }]);
    await waitFor(() => expect(toggle.getAttribute("aria-checked")).toBe("false"));
  });

  // Safe agent edits U7: the two switches are shared settings, so they are
  // read-only while the shared settings are read-only.
  test("the New Note and widget switches are read-only while the shared settings are", async () => {
    await open({
      extra: { shared: { availability: { kind: "read_only", cause: "folder_not_available" }, newer_version: false, unknown_values: [], files_writable: false, damaged_file: null } },
    });
    for (const name of ["New Note starts recording", "Show the recording widget"]) {
      const toggle = screen.getByRole("switch", { name });
      expect(toggle.hasAttribute("disabled")).toBe(true);
      const row = toggle.closest(".settings-row") as HTMLElement;
      expect(row.textContent).toContain("The storage folder is not available. This setting can’t be changed now.");
    }
  });

  test("the widget shortcut shows Off with no chord", async () => {
    await open();
    const row = screen.getByText("Widget shortcut").closest(".settings-row") as HTMLElement;
    expect(row.textContent).toContain("Off");
    expect(within(row).queryByRole("button", { name: "Clear" })).toBeNull();
  });

  test("a registered widget shortcut shows what it does, and Clear sends the widget kind", async () => {
    const widget = { chord: "Alt+Shift+W", registered: true, error: null };
    const { mock, user } = await open({
      handlers: {
        shortcut_status: () => ({ ...shortcutStatuses, widget }),
        set_shortcut: () => shortcutStatuses,
        suspend_shortcuts: () => null,
        resume_shortcuts: () => null,
      },
    });
    const row = (await screen.findByText("Shows or hides the widget for the open note")).closest(
      ".settings-row",
    ) as HTMLElement;
    expect(within(row).getByText("Widget shortcut")).toBeTruthy();
    await user.click(within(row).getByRole("button", { name: "Clear" }));
    expect(calls(mock, "set_shortcut")).toEqual([{ kind: "widget", chord: null }]);
    await waitFor(() => expect(row.textContent).toContain("Off"));
  });

  test("with microphone access not allowed on macOS, the Microphone group offers Allow", async () => {
    Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (Macintosh; Mac OS X)", configurable: true });
    try {
      const { group } = await open({ ready: { microphone_authorized: false } });
      const section = group.closest("section") as HTMLElement;
      const row = within(section).getByText("Microphone access").closest(".settings-row") as HTMLElement;
      expect(within(row).getByRole("button", { name: "Allow…" })).toBeTruthy();
    } finally {
      delete (navigator as { userAgent?: string }).userAgent;
    }
  });

  describe("the app list", () => {
    const zoom: RunningApp = { id: "us.zoom.xos", name: "zoom.us", path: "/Applications/zoom.us.app", call_app: true };
    const discord: RunningApp = { id: "com.hnc.Discord", name: "Discord", path: "/Applications/Discord.app", call_app: true };
    const chrome: RunningApp = { id: "com.google.Chrome", name: "Google Chrome", path: "/Applications/Google Chrome.app", call_app: false };
    const teams: ListedApp = { id: "ms-teams", name: "Microsoft Teams", path: "C:\\Program Files\\WindowsApps\\ms-teams.exe" };
    const entryOf = (app: RunningApp): ListedApp => ({ id: app.id, name: app.name, path: app.path });
    const group = () => screen.getByRole("heading", { name: "App Audio" }).closest("section") as HTMLElement;
    const addButton = () => within(group()).getByRole("button", { name: /Add App/ }) as HTMLButtonElement;
    const callButton = () => within(group()).getByRole("button", { name: "Add Call Apps" }) as HTMLButtonElement;
    const reasonOf = (button: HTMLElement) => button.closest("[title]")?.getAttribute("title");

    afterEach(() => {
      delete (navigator as { userAgent?: string }).userAgent;
    });

    test("Covers R1. An empty list shows no app rows and no message", async () => {
      const { mock } = await open({ running: [chrome] });
      await waitFor(() => expect(addButton().disabled).toBe(false));
      expect(within(group()).queryAllByRole("button", { name: /^Remove/ })).toHaveLength(0);
      expect(within(group()).queryByRole("status")).toBeNull();
      expect(within(group()).queryByText("App audio access")).toBeNull();
      expect(calls(mock, "list_running_apps")[0]).toEqual({ all: false });
    });

    test("Covers R2. The add menu lists the running apps that are not on the list, and adds the chosen app", async () => {
      const { mock, user } = await open({ running: [discord, chrome], extra: { apps: [entryOf(discord)] } });
      await waitFor(() => expect(addButton().disabled).toBe(false));
      const before = calls(mock, "list_running_apps").length;
      await user.click(addButton());
      // The menu reads the running apps again when it opens.
      expect(calls(mock, "list_running_apps").length).toBe(before + 1);
      expect(screen.getAllByRole("menuitem").map((i) => i.textContent)).toEqual(["Google Chrome"]);
      await user.click(screen.getByRole("menuitem", { name: "Google Chrome" }));
      expect(calls(mock, "add_app")).toEqual([{ entry: entryOf(chrome) }]);
      expect(await within(group()).findByRole("button", { name: "Remove Google Chrome" })).toBeTruthy();
    });

    test("Covers R2. When no app remains to add, Add App is disabled with its reason", async () => {
      await open({ running: [chrome], extra: { apps: [entryOf(chrome)] } });
      await waitFor(() => expect(addButton().disabled).toBe(true));
      expect(reasonOf(addButton())).toBe("No other apps are running");
      expect(group().textContent).toContain("No other apps are running");
    });

    test("Covers R3. Add Call Apps is disabled with its reason when no call app runs that is not on the list", async () => {
      await open({ running: [zoom, chrome], extra: { apps: [entryOf(zoom)] } });
      await waitFor(() => expect(addButton().disabled).toBe(false));
      expect(callButton().disabled).toBe(true);
      expect(reasonOf(callButton())).toBe("No other call apps are running");
    });

    test("Covers R3. Add Call Apps adds the running call apps", async () => {
      const { mock, user } = await open({ running: [zoom, chrome] });
      await waitFor(() => expect(callButton().disabled).toBe(false));
      await user.click(callButton());
      expect(calls(mock, "add_call_apps")).toHaveLength(1);
      expect(await within(group()).findByRole("button", { name: "Remove zoom.us" })).toBeTruthy();
      expect(within(group()).queryByRole("button", { name: "Remove Google Chrome" })).toBeNull();
    });

    test("Covers R4. Remove takes the app off the list", async () => {
      const { mock, user } = await open({ extra: { apps: [entryOf(zoom)] } });
      await user.click(within(group()).getByRole("button", { name: "Remove zoom.us" }));
      expect(calls(mock, "remove_app")).toEqual([{ id: "us.zoom.xos" }]);
      await waitFor(() => expect(within(group()).queryByRole("button", { name: "Remove zoom.us" })).toBeNull());
    });

    test("Covers R5. A row of an app that is not running shows its name and icon", async () => {
      const icon = "data:image/png;base64,AAAA";
      await open({
        extra: { apps: [entryOf(zoom), entryOf(discord)] },
        handlers: { app_icon: ({ path }) => (path === zoom.path ? icon : null) },
      });
      const zoomRow = within(group()).getByText("zoom.us").closest(".settings-row") as HTMLElement;
      await waitFor(() => expect(zoomRow.querySelector("img")?.getAttribute("src")).toBe(icon));
      // Longhand cannot read an icon for Discord: the row shows the generic icon.
      const discordRow = within(group()).getByText("Discord").closest(".settings-row") as HTMLElement;
      expect(discordRow.querySelector("img")).toBeNull();
      expect(discordRow.querySelector("[data-generic-icon]")).toBeTruthy();
    });

    test("Covers R20. On Windows, the Teams row shows the secondary line and other rows do not", async () => {
      Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (Windows NT 10.0; Win64; x64)", configurable: true });
      const line = "Windows can’t record Teams by itself, so Longhand records all computer sound while Teams plays.";
      await open({ extra: { apps: [teams, entryOf(zoom)] } });
      const teamsRow = within(group()).getByText("Microsoft Teams").closest(".settings-row") as HTMLElement;
      const zoomRow = within(group()).getByText("zoom.us").closest(".settings-row") as HTMLElement;
      expect(teamsRow.textContent).toContain(line);
      expect(zoomRow.textContent).not.toContain(line);
    });

    test("on macOS, the Teams row shows no secondary line", async () => {
      Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (Macintosh; Mac OS X)", configurable: true });
      await open({ extra: { apps: [teams] } });
      expect(group().textContent).not.toContain("Windows can’t record Teams");
    });

    test("Covers R17. On older Windows, the group is disabled with its reason", async () => {
      await open({
        running: [zoom, chrome],
        extra: { apps: [entryOf(discord)] },
        handlers: { app_audio_available: () => false },
      });
      await waitFor(() => expect(group().textContent).toContain("App audio needs Windows 11"));
      expect(addButton().disabled).toBe(true);
      expect(callButton().disabled).toBe(true);
      expect(reasonOf(addButton())).toBe("App audio needs Windows 11");
      expect((within(group()).getByRole("button", { name: "Remove Discord" }) as HTMLButtonElement).disabled).toBe(true);
    });

    test("Covers R15. On macOS, the rows are access, Add apps, then the list", async () => {
      Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (Macintosh; Mac OS X)", configurable: true });
      await open({ ready: { system_audio: "allowed" }, extra: { apps: [entryOf(zoom)] } });
      const labels = Array.from(group().querySelectorAll(".settings-row")).map((row) =>
        ["App audio access", "Add apps", "zoom.us"].find((label) => row.textContent?.includes(label)),
      );
      expect(labels).toEqual(["App audio access", "Add apps", "zoom.us"]);
    });

    test("Covers R15. On macOS, an empty list shows the access row with Allow", async () => {
      Object.defineProperty(navigator, "userAgent", { value: "Mozilla/5.0 (Macintosh; Mac OS X)", configurable: true });
      await open({ ready: { system_audio: "not_asked" } });
      const row = within(group()).getByText("App audio access").closest(".settings-row") as HTMLElement;
      expect(row.textContent).toContain("Not allowed yet");
      expect(within(row).getByRole("button", { name: "Allow…" })).toBeTruthy();
    });

    test("a running-apps reply that arrives after a newer reply is dropped", async () => {
      // Each read waits until the test answers it.
      const pending: ((apps: RunningApp[]) => void)[] = [];
      const { user } = await open({
        handlers: {
          list_running_apps: () => new Promise<RunningApp[]>((resolve) => pending.push(resolve)),
        },
      });
      await waitFor(() => expect(pending.length).toBe(1));
      // The first reply gives a list with an app, so Add App can open.
      await act(async () => pending.shift()!([zoom]));
      await waitFor(() => expect(addButton().disabled).toBe(false));
      // Two reads are open: the focus read and the menu read. The newer
      // reply comes first, then the older one.
      await act(async () => {
        window.dispatchEvent(new Event("focus"));
      });
      await user.click(addButton());
      await waitFor(() => expect(pending.length).toBe(2));
      const [older, newer] = pending.splice(0);
      await act(async () => newer([discord]));
      await act(async () => older([chrome]));
      expect(screen.getAllByRole("menuitem").map((i) => i.textContent)).toEqual(["Discord"]);
    });
  });

  test("Covers AE5. The alert switch is on by default, and turning it off saves the change", async () => {
    const { mock, user } = await open();
    const toggle = screen.getByRole("switch", { name: "Play a sound if recording stops" });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    await user.click(toggle);
    expect(calls(mock, "set_alert_sound")).toEqual([{ enabled: false }]);
    await waitFor(() => expect(toggle.getAttribute("aria-checked")).toBe("false"));
  });

  test("a failed alert switch change shows the error and keeps the value", async () => {
    const { user } = await open({
      handlers: {
        set_alert_sound: () => {
          throw "could not save settings";
        },
      },
    });
    const toggle = screen.getByRole("switch", { name: "Play a sound if recording stops" });
    await user.click(toggle);
    expect(await screen.findByText(/could not save settings/i)).toBeTruthy();
    expect(toggle.getAttribute("aria-checked")).toBe("true");
  });

  test("Test Sound plays the sound while the alert switch is off", async () => {
    const { mock, user } = await open({ extra: { alert_sound: false }, handlers: { play_alert_sound: () => null } });
    await user.click(screen.getByRole("button", { name: "Test Sound" }));
    expect(calls(mock, "play_alert_sound")).toHaveLength(1);
  });

  test("a sound that cannot play shows the error", async () => {
    const { user } = await open({
      handlers: {
        play_alert_sound: () => {
          throw "no output device";
        },
      },
    });
    await user.click(screen.getByRole("button", { name: "Test Sound" }));
    expect(await screen.findByText("no output device")).toBeTruthy();
  });

  test("the pane is after Storage in the sidebar and opens with the arrow keys", async () => {
    const { user } = await open();
    const tabs = screen.getAllByRole("tab").map((t) => t.getAttribute("data-pane"));
    expect(tabs.slice(0, 3)).toEqual(["status", "destination", "recording"]);

    await user.click(screen.getByRole("tab", { name: /Storage/ }));
    screen.getByRole("tab", { name: /Storage/ }).focus();
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("tab", { name: /Recording/ }).getAttribute("aria-selected")).toBe("true");
    expect(await screen.findByRole("radiogroup", { name: "Microphone" })).toBeTruthy();
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("tab", { name: /Transcription/ }).getAttribute("aria-selected")).toBe("true");
  });
});
