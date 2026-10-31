import { describe, expect, test, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockWindows } from "@tauri-apps/api/mocks";
import { useState } from "react";
import type { Settings } from "../bindings";
import { type CommandHandler, type TauriMock, mockTauri } from "../test/tauri";
import { Background } from "./Background";

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
    openai: { address: null, model: "", context_limit_tokens: null, thinking: null, requests_at_once: null },
    anthropic: { address: null, model: "", context_limit_tokens: null, thinking: null, requests_at_once: null },
    gemini: { address: null, model: "", context_limit_tokens: null, thinking: null, requests_at_once: null },
    openrouter: { address: null, model: "", context_limit_tokens: null, thinking: null, requests_at_once: null },
    ollama: { address: null, model: "", context_limit_tokens: null, thinking: null, requests_at_once: null },
    lm_studio: { address: null, model: "", context_limit_tokens: null, thinking: null, requests_at_once: null },
    custom: { address: null, model: "", context_limit_tokens: null, thinking: null, requests_at_once: null },
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
  transcript_removal: { kind: "off" },
  summary_prompts: [],
  active_summary_prompt: "topic-summary",
  shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [], files_writable: true, damaged_file: null },
  background: "",
  background_limit: 4000,
  background_read_only: null,
};

/// The setter of the settings that the harness shows. A test calls it to
/// show a change from another computer.
let setShown: (f: (s: Settings) => Settings) => void = () => {};

/// With `lag`, a save result does not change the settings that the
/// harness shows: as in the Settings window, only a later read does.
function Harness({ initial, lag }: { initial: Settings; lag: boolean }) {
  const [settings, setSettings] = useState(initial);
  setShown = setSettings;
  return <Background settings={settings} onSettings={lag ? () => {} : setSettings} />;
}

function setup(settings: Partial<Settings> = {}, over: Record<string, CommandHandler> = {}, lag = false) {
  let current: Settings = { ...baseSettings, ...settings };
  const tauri = mockTauri({
    set_background: ({ text }) => {
      current = { ...current, background: text as string };
      return current;
    },
    ...over,
  });
  mockWindows("settings");
  const { unmount } = render(<Harness initial={current} lag={lag} />);
  // Another computer changes the saved text.
  const remote = (text: string) => {
    current = { ...current, background: text };
    act(() => setShown((s) => ({ ...s, background: text })));
  };
  return { tauri, remote, unmount, user: userEvent.setup() };
}

/// A `set_background` handler whose saves wait until the test lets them go.
/// `done` counts the saves that returned.
function heldSaves() {
  let release: () => void = () => {};
  const held = new Promise<void>((r) => (release = r));
  const state = { done: 0 };
  let saved = "";
  const handler: CommandHandler = async ({ text }) => {
    await held;
    saved = text as string;
    state.done += 1;
    return { ...baseSettings, background: saved };
  };
  return { handler, release: () => act(async () => release()), state, saved: () => saved };
}

const box = () => screen.getByRole("textbox", { name: "Background" });
const saves = (tauri: TauriMock) => tauri.calls.filter((c) => c.cmd === "set_background").map((c) => c.args.text);
const pause = (ms: number) => act(() => new Promise<void>((r) => setTimeout(r, ms)));
/// Moves the fake clock, then lets a queued save run.
const advance = (ms: number) =>
  act(async () => {
    vi.advanceTimersByTime(ms);
  });

describe("Background", () => {
  test("shows the saved text, the hint and the count, and does not take the focus", () => {
    setup({ background: "I am Erv." });
    expect(box().textContent).toBe("I am Erv.");
    const hint = document.getElementById(box().getAttribute("aria-describedby")!);
    expect(hint?.textContent).toContain("words the transcript often gets wrong");
    expect(screen.getByText("9 / 4,000")).toBeTruthy();
    expect(document.activeElement).not.toBe(box());
  });

  test("typing saves once, one second after the last change", async () => {
    vi.useFakeTimers();
    try {
      const { tauri } = setup();
      // No delay between keys: the test moves the clock itself.
      const user = userEvent.setup({ delay: null });
      await user.click(box());
      await user.keyboard("Er");
      // A change before the delay ends starts the delay again.
      await advance(800);
      await user.keyboard("v");
      await advance(950);
      expect(saves(tauri)).toEqual([]);
      await advance(50);
      expect(saves(tauri)).toEqual(["Erv"]);
      await advance(2000);
      expect(saves(tauri)).toEqual(["Erv"]);
    } finally {
      vi.useRealTimers();
    }
  });

  test("leaving the field saves at once", async () => {
    const { tauri, user } = setup();
    await user.click(box());
    await user.keyboard("Erv");
    act(() => void fireEvent.blur(box()));
    await waitFor(() => expect(saves(tauri)).toEqual(["Erv"]), { timeout: 300 });
  });

  test("an own save does not remount the editor, and the focus stays in the field", async () => {
    const { tauri, user } = setup();
    const before = box();
    await user.click(before);
    await user.keyboard("Erv");
    await waitFor(() => expect(saves(tauri)).toEqual(["Erv"]), { timeout: 1500 });
    await pause(20);
    expect(box()).toBe(before);
    expect(document.activeElement).toBe(before);
    expect(screen.queryByText(/changed on another computer/)).toBeNull();
  });

  test("a text over the limit shows a red count and frame and is not saved", async () => {
    const { tauri, user } = setup({ background_limit: 5 });
    await user.click(box());
    await user.keyboard("abcdef");
    const count = screen.getByText("6 / 5");
    expect(count.className).toContain("text-negative");
    expect((box().closest("[data-markdown-field]") as HTMLElement).className).toContain("border-negative");
    act(() => void fireEvent.blur(box()));
    await pause(1200);
    expect(saves(tauri)).toEqual([]);
  });

  test("the count counts characters, not UTF-16 units", () => {
    setup({ background: "😀😀" });
    expect(screen.getByText("2 / 4,000")).toBeTruthy();
  });

  test("clearing the text saves an empty Background", async () => {
    const { tauri, user } = setup({ background: "ab" });
    await user.click(box());
    // Select all, then delete the selection.
    await user.keyboard("{Control>}a{/Control}{Backspace}");
    act(() => void fireEvent.blur(box()));
    await waitFor(() => expect(saves(tauri)).toEqual([""]), { timeout: 300 });
  });

  test("a save error shows under the field and keeps the text", async () => {
    const { user } = setup({}, {
      set_background: () => {
        throw "The storage folder is not available.";
      },
    });
    await user.click(box());
    await user.keyboard("Erv");
    act(() => void fireEvent.blur(box()));
    expect(await screen.findByText("The storage folder is not available.")).toBeTruthy();
    expect(box().textContent).toBe("Erv");
  });

  test("a read-only Background shows the text read-only with the reason", async () => {
    const { tauri, user } = setup({
      background: "I am Erv.",
      background_read_only: "The Background file is over the limit.",
    });
    expect(box().getAttribute("aria-readonly")).toBe("true");
    expect(screen.getByText(/The Background file is over the limit\./)).toBeTruthy();
    await user.click(box());
    await user.keyboard("x");
    act(() => void fireEvent.blur(box()));
    await pause(20);
    expect(saves(tauri)).toEqual([]);
  });

  test("a read-only shared state makes the Background read-only", () => {
    setup({
      shared: { availability: { kind: "read_only", cause: "folder_not_available" }, newer_version: false, unknown_values: [], files_writable: false, damaged_file: null },
    });
    expect(box().getAttribute("aria-readonly")).toBe("true");
    expect(screen.getByText(/The storage folder is not available\./)).toBeTruthy();
  });

  test("a settings file that does not parse leaves the Background editable", () => {
    // R22: the Background is in another file.
    setup({
      shared: { availability: { kind: "read_only", cause: "settings_file_damaged" }, newer_version: false, unknown_values: [], files_writable: true, damaged_file: { path: ".longhand/settings.json", line: 3 } },
    });
    expect(box().getAttribute("aria-readonly")).toBeNull();
  });

  test("a change from another computer with no draft shows the new text", async () => {
    const { remote } = setup({ background: "Old text" });
    remote("Text from the other computer");
    await waitFor(() => expect(box().textContent).toBe("Text from the other computer"));
    expect(screen.queryByText(/changed on another computer/)).toBeNull();
  });

  test("a change from another computer with a draft shows the notice and keeps the text and the focus", async () => {
    const { tauri, remote, user } = setup({ background: "Old" });
    const before = box();
    await user.click(before);
    await user.keyboard("My ");
    remote("Text from the other computer");
    expect(await screen.findByText("The Background changed on another computer.")).toBeTruthy();
    expect(box()).toBe(before);
    expect(box().textContent).toBe("My Old");
    expect(document.activeElement).toBe(before);
    // The draft does not replace the text of the other computer.
    await pause(1200);
    act(() => void fireEvent.blur(box()));
    await pause(20);
    expect(saves(tauri)).toEqual([]);
  });

  test("Reload shows the saved text and drops the draft", async () => {
    const { tauri, remote, user } = setup({ background: "Old" });
    await user.click(box());
    await user.keyboard("My ");
    remote("Text from the other computer");
    await user.click(await screen.findByRole("button", { name: "Reload" }));
    await waitFor(() => expect(box().textContent).toBe("Text from the other computer"));
    expect(screen.queryByText(/changed on another computer/)).toBeNull();
    expect(saves(tauri)).toEqual([]);
  });

  test("Keep My Text saves the draft", async () => {
    const { tauri, remote, user } = setup({ background: "Old" });
    await user.click(box());
    await user.keyboard("My ");
    remote("Text from the other computer");
    await user.click(await screen.findByRole("button", { name: "Keep My Text" }));
    await waitFor(() => expect(saves(tauri)).toEqual(["My Old"]));
    await waitFor(() => expect(screen.queryByText(/changed on another computer/)).toBeNull());
    expect(box().textContent).toBe("My Old");
  });

  test("a window close at once after typing saves before the window closes", async () => {
    let finish: () => void = () => {};
    const held = new Promise<void>((r) => (finish = r));
    const { tauri, user } = setup({}, {
      set_background: async ({ text }) => {
        await held;
        return { ...baseSettings, background: text as string };
      },
      "plugin:window|destroy": () => null,
      log_from_webview: () => null,
    });
    // The mount registers the close listener: the event mock adds it when
    // the effect runs, and render runs the effects.
    await user.click(box());
    await user.keyboard("Erv");
    await tauri.emit("tauri://close-requested");
    await waitFor(() => expect(saves(tauri)).toEqual(["Erv"]));
    await pause(20);
    expect(tauri.calls.some((c) => c.cmd === "plugin:window|destroy")).toBe(false);
    await act(async () => finish());
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "plugin:window|destroy")).toBe(true));
  });

  test("a window close during the save that a pane change started waits for that save", async () => {
    let finish: () => void = () => {};
    const held = new Promise<void>((r) => (finish = r));
    const { tauri, unmount, user } = setup({}, {
      set_background: async ({ text }) => {
        await held;
        return { ...baseSettings, background: text as string };
      },
      "plugin:window|destroy": () => null,
      log_from_webview: () => null,
    });
    await user.click(box());
    await user.keyboard("Erv");
    unmount();
    await waitFor(() => expect(saves(tauri)).toEqual(["Erv"]));
    await tauri.emit("tauri://close-requested");
    await pause(20);
    expect(tauri.calls.some((c) => c.cmd === "plugin:window|destroy")).toBe(false);
    await act(async () => finish());
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "plugin:window|destroy")).toBe(true));
  });

  test("text typed during a save is saved when the save result comes before the settings read", async () => {
    const h = heldSaves();
    const { tauri, remote, user } = setup({}, { set_background: h.handler }, true);
    await user.click(box());
    await user.keyboard("A");
    await waitFor(() => expect(saves(tauri)).toEqual(["A"]), { timeout: 1500 });
    await user.keyboard("B");
    await h.release();
    await waitFor(() => expect(h.state.done).toBe(1));
    await act(async () => {});
    expect(screen.queryByText(/changed on another computer/)).toBeNull();
    act(() => void fireEvent.blur(box()));
    await waitFor(() => expect(saves(tauri)).toEqual(["A", "AB"]));
    // The settings read then shows the own saves, with no notice.
    await waitFor(() => expect(h.state.done).toBe(2));
    remote(h.saved());
    expect(screen.queryByText(/changed on another computer/)).toBeNull();
    expect(box().textContent).toBe("AB");
  });

  test("a change on another computer back to a text that this window saved before shows the text", async () => {
    const { tauri, remote, user } = setup();
    await user.click(box());
    await user.keyboard("Alice");
    await waitFor(() => expect(saves(tauri)).toEqual(["Alice"]), { timeout: 1500 });
    await user.keyboard(" Smith");
    await waitFor(() => expect(saves(tauri)).toEqual(["Alice", "Alice Smith"]), { timeout: 1500 });
    await waitFor(() => expect(screen.getByText("11 / 4,000")).toBeTruthy());
    remote("Alice");
    await waitFor(() => expect(box().textContent).toBe("Alice"));
  });

  test("a change on another computer back to a text that this window saved before shows the notice over a draft", async () => {
    const { tauri, remote, user } = setup();
    await user.click(box());
    await user.keyboard("Alice");
    await waitFor(() => expect(saves(tauri)).toEqual(["Alice"]), { timeout: 1500 });
    await user.keyboard(" Smith");
    await waitFor(() => expect(saves(tauri)).toEqual(["Alice", "Alice Smith"]), { timeout: 1500 });
    await waitFor(() => expect(screen.getByText("11 / 4,000")).toBeTruthy());
    await user.keyboard("!");
    remote("Alice");
    await waitFor(() => expect(screen.getByText("The Background changed on another computer.")).toBeTruthy());
    expect(box().textContent).toBe("Alice Smith!");
  });

  test("a window close after a pane change waits for a save that was queued behind a running save", async () => {
    const h = heldSaves();
    const { tauri, unmount, user } = setup({}, {
      set_background: h.handler,
      "plugin:window|destroy": () => null,
      log_from_webview: () => null,
    });
    await user.click(box());
    await user.keyboard("A");
    await waitFor(() => expect(saves(tauri)).toEqual(["A"]), { timeout: 1500 });
    await user.keyboard("B");
    // The blur queues the save of "AB" behind the running save.
    act(() => void fireEvent.blur(box()));
    unmount();
    await tauri.emit("tauri://close-requested");
    await h.release();
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "plugin:window|destroy")).toBe(true));
    expect(saves(tauri)).toEqual(["A", "AB"]);
  });
});
