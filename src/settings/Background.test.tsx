import { describe, expect, test } from "bun:test";
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
  storage_folder: "/notes",
  endpoint_base_url: "",
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
  new_note_records: true,
  hide_from_dock: false,
  check_updates_automatically: true,
  audio_removal: { kind: "after_days", days: 30 },
  learned_context_limits: {},
  summary_prompts: [],
  active_summary_prompt: "topic-summary",
  shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [] },
  background: "",
  background_limit: 4000,
  background_read_only: null,
};

/// The setter of the settings that the harness shows. A test calls it to
/// show a change from another computer.
let setShown: (f: (s: Settings) => Settings) => void = () => {};

function Harness({ initial }: { initial: Settings }) {
  const [settings, setSettings] = useState(initial);
  setShown = setSettings;
  return <Background settings={settings} onSettings={setSettings} />;
}

function setup(settings: Partial<Settings> = {}, over: Record<string, CommandHandler> = {}) {
  let current: Settings = { ...baseSettings, ...settings };
  const tauri = mockTauri({
    set_background: ({ text }) => {
      current = { ...current, background: text as string };
      return current;
    },
    ...over,
  });
  mockWindows("settings");
  const { unmount } = render(<Harness initial={current} />);
  // Another computer changes the saved text.
  const remote = (text: string) => {
    current = { ...current, background: text };
    act(() => setShown((s) => ({ ...s, background: text })));
  };
  return { tauri, remote, unmount, user: userEvent.setup() };
}

const box = () => screen.getByRole("textbox", { name: "Background" });
const saves = (tauri: TauriMock) => tauri.calls.filter((c) => c.cmd === "set_background").map((c) => c.args.text);
const pause = (ms: number) => act(() => new Promise<void>((r) => setTimeout(r, ms)));

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
    const { tauri, user } = setup();
    await user.click(box());
    await user.keyboard("Erv");
    await pause(500);
    expect(saves(tauri)).toEqual([]);
    await waitFor(() => expect(saves(tauri)).toEqual(["Erv"]), { timeout: 1500 });
    await pause(1100);
    expect(saves(tauri)).toEqual(["Erv"]);
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
      shared: { availability: { kind: "read_only", cause: "folder_not_available" }, newer_version: false, unknown_values: [] },
    });
    expect(box().getAttribute("aria-readonly")).toBe("true");
    expect(screen.getByText(/The storage folder is not available\./)).toBeTruthy();
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
    await pause(10);
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
    await pause(10);
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
});
