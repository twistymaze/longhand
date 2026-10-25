import { describe, expect, test, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockWindows } from "@tauri-apps/api/mocks";
import type { Readiness, Settings, SummaryPrompt } from "../bindings";
import { type CommandHandler, type TauriMock, mockTauri, shortcutStatuses } from "../test/tauri";
import { SettingsWindow } from "./SettingsWindow";
import { Background } from "./Background";
import { SummaryPrompts } from "./SummaryPrompts";

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

const readiness: Readiness = {
  microphone_authorized: true,
  system_audio: "allowed",
  destination: { kind: "ready", path: "/x", free_bytes: null },
  armed: true,
  shortcut: { chord: "Alt+Shift+L", registered: true, error: null },
  missing_models: [],
  endpoint_configured: true, can_start_note: true, note_start_refusal: null,
};

/// A fake of the prompt commands that keeps its own list, as the backend does.
/// The backend lists the custom prompts by name.
function backend(over: Record<string, CommandHandler> = {}) {
  const builtIns = [
    { id: "topic-summary", name: "Topic summary", text: "Topic text", title_text: "Built-in title text", built_in: true, over_limit: false, too_large: false },
    { id: "play-by-play", name: "Play-by-play summary", text: "Play text", title_text: "Built-in title text", built_in: true, over_limit: false, too_large: false },
  ];
  let custom: SummaryPrompt[] = [{ id: "c1", name: "Team sync", text: "Sync text", title_text: "Sync title" }];
  let active = "topic-summary";
  let next = 2;
  const byName = () => [...custom].sort((a, b) => a.name.localeCompare(b.name));
  const settings = (): Settings => ({
    ...baseSettings,
    summary_prompts: byName().map((p) => ({ ...p })),
    active_summary_prompt: active,
  });
  const find = (id: unknown) => [...builtIns, ...custom].find((p) => p.id === id)!;
  return mockTauri({
    summary_prompts: () => ({
      prompts: [...builtIns, ...byName().map((p) => ({ ...p, built_in: false, over_limit: false, too_large: false }))],
      active,
      text_limit: 4000,
      title_text_limit: 1000,
    }),
    create_summary_prompt: () => {
      custom.push({ id: `c${next++}`, name: "New prompt", text: "Topic text", title_text: "Built-in title text" });
      return settings();
    },
    duplicate_summary_prompt: ({ id }) => {
      const from = find(id);
      custom.push({ id: `c${next++}`, name: `${from.name} copy`, text: from.text, title_text: from.title_text });
      return settings();
    },
    rename_summary_prompt: ({ id, name }) => {
      custom = custom.map((p) => (p.id === id ? { ...p, name: name as string } : p));
      return settings();
    },
    set_summary_prompt_text: ({ id, text }) => {
      custom = custom.map((p) => (p.id === id ? { ...p, text: text as string } : p));
      return settings();
    },
    set_summary_prompt_title_text: ({ id, text }) => {
      custom = custom.map((p) => (p.id === id ? { ...p, title_text: text as string } : p));
      return settings();
    },
    delete_summary_prompt: ({ id }) => {
      custom = custom.filter((p) => p.id !== id);
      if (active === id) active = "topic-summary";
      return settings();
    },
    activate_summary_prompt: ({ id }) => {
      active = id as string;
      return settings();
    },
    ...over,
  });
}

function renderPrompts(closeSaveLimit?: number) {
  mockWindows("settings");
  const onSettings: Settings[] = [];
  render(<SummaryPrompts onSettings={(s) => onSettings.push(s)} closeSaveLimit={closeSaveLimit} />);
  return onSettings;
}

const called = (tauri: TauriMock, cmd: string) => tauri.calls.filter((c) => c.cmd === cmd).length;
const logLines = (tauri: TauriMock) =>
  tauri.calls.filter((c) => c.cmd === "log_from_webview").map((c) => `${c.args.level}: ${c.args.message}`);

const row = (name: string) => screen.getByRole("radio", { name: new RegExp(`^${name}`) });
// The two prompt editors, found by their accessible names.
const textArea = () => screen.getByRole("textbox", { name: "Prompt text" });
const button = (name: string | RegExp) => screen.getByRole("button", { name });
const textCalls = (tauri: TauriMock) =>
  tauri.calls.filter((c) => c.cmd === "set_summary_prompt_text").map((c) => c.args);
const titleArea = () => screen.getByRole("textbox", { name: "Title instructions" });

const selectAll = /Mac/.test(navigator.platform) ? "{Meta>}a{/Meta}" : "{Control>}a{/Control}";
/// Replaces the whole text of an editor: a click, select all, then a paste
/// of `text`, or a delete when `text` is empty. A paste gives one change, as
/// one value change of the old text area did. A test with fake timers gives
/// its own `user`, which has no delay between keys.
async function setText(box: HTMLElement, text: string, user = userEvent.setup()) {
  await user.click(box);
  await user.keyboard(selectAll);
  if (text === "") await user.keyboard("{Backspace}");
  else await user.paste(text);
}
const pause = (ms: number) => act(() => new Promise<void>((r) => setTimeout(r, ms)));
/// Moves the fake clock, then lets a queued save run.
const advance = (ms: number) =>
  act(async () => {
    vi.advanceTimersByTime(ms);
  });
const titleCalls = (tauri: TauriMock) =>
  tauri.calls.filter((c) => c.cmd === "set_summary_prompt_title_text").map((c) => c.args);

async function selectCustom(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("radio", { name: /^Team sync/ }));
  await waitFor(() => expect(textArea().textContent).toBe("Sync text"));
}

describe("SummaryPrompts", () => {
  test("the list shows the built-in prompts first and marks the one in use", async () => {
    backend();
    renderPrompts();
    const radios = await screen.findAllByRole("radio");
    expect(radios.map((r) => r.textContent)).toEqual([
      expect.stringContaining("Topic summary"),
      expect.stringContaining("Play-by-play summary"),
      expect.stringContaining("Team sync"),
    ]);
    const group = screen.getByRole("radiogroup", { name: "Summary prompts" });
    expect(within(group).getAllByText("Default")).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Make default Topic summary" })).toBeNull();
    expect(button("Make default Play-by-play summary")).toBeTruthy();
    expect(button("Make default Team sync")).toBeTruthy();
  });

  test("a built-in prompt shows read-only text and Duplicate, with no Delete or count", async () => {
    backend();
    renderPrompts();
    await waitFor(() => expect(textArea().textContent).toBe("Topic text"));
    expect(row("Topic summary").getAttribute("aria-checked")).toBe("true");
    expect(textArea().getAttribute("aria-readonly")).toBe("true");
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).disabled).toBe(true);
    expect(screen.getByText(/Built-in prompts can’t be changed/)).toBeTruthy();
    expect(button("Duplicate")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
    expect(screen.queryByText(/ \/ 4,000$/)).toBeNull();
  });

  test("New calls the create command and selects the new prompt", async () => {
    const tauri = backend();
    const onSettings = renderPrompts();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "New Prompt" }));
    await waitFor(() => expect(row("New prompt").getAttribute("aria-checked")).toBe("true"));
    expect(tauri.calls.some((c) => c.cmd === "create_summary_prompt")).toBe(true);
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("New prompt");
    expect(textArea().getAttribute("aria-readonly")).toBeNull();
    expect(onSettings).toHaveLength(1);
  });

  test("Duplicate calls the duplicate command and selects the copy", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await waitFor(() => expect(textArea().textContent).toBe("Topic text"));
    await user.click(button("Duplicate"));
    await waitFor(() => expect(row("Topic summary copy").getAttribute("aria-checked")).toBe("true"));
    expect(tauri.calls.find((c) => c.cmd === "duplicate_summary_prompt")?.args).toEqual({ id: "topic-summary" });
  });

  test("typing saves the text one second after the last change, in one call", async () => {
    vi.useFakeTimers();
    try {
      const tauri = backend();
      renderPrompts();
      // No delay between keys: the test moves the clock itself.
      const user = userEvent.setup({ delay: null });
      await selectCustom(user);
      // Each change before the delay ends starts the delay again.
      await setText(textArea(), "Short", user);
      await advance(800);
      await setText(textArea(), "Short re", user);
      await advance(800);
      await setText(textArea(), "Short note", user);
      await advance(950);
      expect(textCalls(tauri)).toEqual([]);
      await advance(50);
      expect(textCalls(tauri)).toEqual([{ id: "c1", text: "Short note" }]);
      await advance(2000);
      expect(textCalls(tauri)).toEqual([{ id: "c1", text: "Short note" }]);
      expect(textArea().textContent).toBe("Short note");
    } finally {
      vi.useRealTimers();
    }
  });

  test("blur saves the text at once", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "Short note");
    fireEvent.blur(textArea());
    await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "Short note" }]), { timeout: 500 });
  });

  test("selecting another prompt saves the text first and asks nothing", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "Changed");
    await user.click(row("Topic summary"));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    await waitFor(() => expect(textArea().textContent).toBe("Topic text"));
    await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "Changed" }]), { timeout: 500 });

    await user.click(row("Team sync"));
    await waitFor(() => expect(textArea().textContent).toBe("Changed"));
  });

  test("empty text is not saved and the editor says why", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    expect(screen.queryByText("A prompt can’t be empty.")).toBeNull();
    await setText(textArea(), "   ");
    expect(screen.getByText("A prompt can’t be empty.")).toBeTruthy();
    fireEvent.blur(textArea());
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(textCalls(tauri)).toEqual([]);
    expect(textArea().textContent).toBe("   ");

    // A change of prompt drops the text that is not valid.
    await user.click(row("Topic summary"));
    await user.click(row("Team sync"));
    await waitFor(() => expect(textArea().textContent).toBe("Sync text"));
  });

  test("a save error shows inline and keeps the edit", async () => {
    backend({
      set_summary_prompt_text: () => {
        throw "The prompt text cannot be saved.";
      },
    });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "Short note");
    fireEvent.blur(textArea());
    expect(await screen.findByText("The prompt text cannot be saved.")).toBeTruthy();
    expect(textArea().textContent).toBe("Short note");
  });

  test("a failed save on a change of prompt keeps the text and the error with its prompt", async () => {
    let fail = true;
    const tauri = backend({
      set_summary_prompt_text: () => {
        if (fail) throw "The prompt text cannot be saved.";
        return baseSettings;
      },
    });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "Changed");
    await user.click(row("Topic summary"));
    await waitFor(() => expect(textCalls(tauri)).toHaveLength(1));
    await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(textArea().textContent).toBe("Topic text");
    expect(screen.queryByText("The prompt text cannot be saved.")).toBeNull();

    await user.click(row("Team sync"));
    expect(textArea().textContent).toBe("Changed");
    expect(screen.getByText("The prompt text cannot be saved.")).toBeTruthy();

    // A later edit saves again.
    fail = false;
    await setText(textArea(), "Changed again");
    fireEvent.blur(textArea());
    await waitFor(() => expect(textCalls(tauri)[1]).toEqual({ id: "c1", text: "Changed again" }));
    await waitFor(() => expect(screen.queryByText("The prompt text cannot be saved.")).toBeNull());
  });

  test("New and Duplicate stop when the save of the selected prompt fails", async () => {
    const tauri = backend({
      set_summary_prompt_text: () => {
        throw "The prompt text cannot be saved.";
      },
    });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "Changed");
    await user.click(screen.getByRole("button", { name: "New Prompt" }));
    expect(await screen.findByText("The prompt text cannot be saved.")).toBeTruthy();
    await user.click(button("Duplicate"));
    await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(tauri.calls.some((c) => c.cmd === "create_summary_prompt")).toBe(false);
    expect(tauri.calls.some((c) => c.cmd === "duplicate_summary_prompt")).toBe(false);
    expect(row("Team sync").getAttribute("aria-checked")).toBe("true");
    expect(textArea().textContent).toBe("Changed");
  });

  test("pagehide saves the text before the timer", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "Short note");
    act(() => void window.dispatchEvent(new Event("pagehide")));
    await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "Short note" }]), { timeout: 500 });
  });

  test("beforeunload saves the text before the timer", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "Short note");
    act(() => void window.dispatchEvent(new Event("beforeunload")));
    await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "Short note" }]), { timeout: 500 });
  });

  test("a window close waits for the save, then closes the window and asks nothing", async () => {
    // The save command waits until the test calls `finish`.
    let finish: () => void = () => {};
    const saved = new Promise<void>((r) => (finish = r));
    const tauri = backend({
      set_summary_prompt_text: async () => {
        await saved;
        return baseSettings;
      },
      "plugin:window|destroy": () => null,
    });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "Changed");
    // The mock handles `plugin:event|listen` itself, so the call is not in
    // `calls`. Let the listener register.
    await act(() => new Promise((r) => setTimeout(r, 10)));

    await tauri.emit("tauri://close-requested");
    await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "Changed" }]));
    await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(tauri.calls.some((c) => c.cmd === "plugin:window|destroy")).toBe(false);
    expect(screen.queryByRole("alertdialog")).toBeNull();

    await act(async () => finish());
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "plugin:window|destroy")).toBe(true));
  });

  test("a window close still closes the window when the save fails", async () => {
    const tauri = backend({
      set_summary_prompt_text: () => {
        throw "The prompt text cannot be saved.";
      },
      "plugin:window|destroy": () => null,
    });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "Changed");
    await act(() => new Promise((r) => setTimeout(r, 10)));

    await tauri.emit("tauri://close-requested");
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "plugin:window|destroy")).toBe(true));
    expect(textCalls(tauri)).toHaveLength(1);
  });

  test("a window close closes the window after the time limit when the save does not complete", async () => {
    // The save does not complete until the end of the test. A save that never
    // completes would keep its close registration into the next tests.
    let finish: () => void = () => {};
    const held = new Promise<void>((r) => (finish = r));
    const tauri = backend({
      set_summary_prompt_text: async () => {
        await held;
        throw "The test ended.";
      },
      "plugin:window|destroy": () => null,
      log_from_webview: () => null,
    });
    renderPrompts(50);
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "Changed");
    await act(() => new Promise((r) => setTimeout(r, 10)));

    await tauri.emit("tauri://close-requested");
    await waitFor(() => expect(textCalls(tauri)).toHaveLength(1));
    expect(called(tauri, "plugin:window|destroy")).toBe(0);
    await waitFor(() => expect(called(tauri, "plugin:window|destroy")).toBe(1));
    expect(logLines(tauri)).toEqual([
      "info: close requested",
      "warn: the save did not complete in 50 ms; the window closes",
      "info: destroy called",
    ]);
    expect(screen.queryByRole("alertdialog")).toBeNull();
    await act(async () => finish());
    await screen.findByText("The test ended.");
  });

  test("a window close removes its listener before the destroy", async () => {
    // Tauri keeps the listener record of a destroyed webview, and the record
    // stops the close of the next Settings window. A second close request
    // after the destroy thus reaches no listener.
    const tauri = backend({ "plugin:window|destroy": () => null, log_from_webview: () => null });
    renderPrompts();
    await screen.findByRole("radio", { name: /^Team sync/ });
    await act(() => new Promise((r) => setTimeout(r, 10)));

    await tauri.emit("tauri://close-requested");
    await waitFor(() => expect(called(tauri, "plugin:window|destroy")).toBe(1));
    expect(logLines(tauri)).toEqual(["info: close requested", "info: the save is complete", "info: destroy called"]);

    // The mock keeps its record of a removed listener too (it reads the
    // wrong argument name), and it warns when an event reaches the removed
    // callback. That warning is the expected result here.
    const warn = console.warn;
    const warnings: unknown[][] = [];
    console.warn = (...args: unknown[]) => void warnings.push(args);
    try {
      await tauri.emit("tauri://close-requested");
      await act(() => new Promise((r) => setTimeout(r, 20)));
    } finally {
      console.warn = warn;
    }
    expect(called(tauri, "plugin:window|destroy")).toBe(1);
    expect(logLines(tauri)).toHaveLength(3);
  });

  test("a failed destroy falls back to the backend close and logs the failure", async () => {
    const tauri = backend({
      "plugin:window|destroy": () => {
        throw new Error("destroy refused");
      },
      close_settings_window: () => null,
      log_from_webview: () => null,
    });
    renderPrompts();
    await screen.findByRole("radio", { name: /^Team sync/ });
    await act(() => new Promise((r) => setTimeout(r, 10)));

    await tauri.emit("tauri://close-requested");
    await waitFor(() => expect(called(tauri, "close_settings_window")).toBe(1));
    const lines = logLines(tauri);
    expect(lines.slice(0, 3)).toEqual(["info: close requested", "info: the save is complete", "info: destroy called"]);
    expect(lines[3]).toMatch(/^error: destroy failed: Error: destroy refused/);
    expect(lines).toHaveLength(4);
  });

  test("a window close waits for a Background save and a prompt save that are both pending", async () => {
    let finishPrompt: () => void = () => {};
    const promptHeld = new Promise<void>((r) => (finishPrompt = r));
    let finishBackground: () => void = () => {};
    const backgroundHeld = new Promise<void>((r) => (finishBackground = r));
    const tauri = backend({
      set_summary_prompt_text: async () => {
        await promptHeld;
        return baseSettings;
      },
      set_background: async ({ text }) => {
        await backgroundHeld;
        return { ...baseSettings, background: text as string };
      },
      "plugin:window|destroy": () => null,
      log_from_webview: () => null,
    });
    mockWindows("settings");
    render(
      <>
        <Background settings={baseSettings} onSettings={() => {}} />
        <SummaryPrompts onSettings={() => {}} />
      </>,
    );
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "Changed");
    await user.click(screen.getByRole("textbox", { name: "Background" }));
    await user.keyboard("Erv");
    await act(() => new Promise((r) => setTimeout(r, 10)));

    await tauri.emit("tauri://close-requested");
    await waitFor(() => expect(called(tauri, "set_background")).toBe(1));
    await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "Changed" }]));
    await act(async () => finishBackground());
    await act(() => new Promise((r) => setTimeout(r, 20)));
    expect(called(tauri, "plugin:window|destroy")).toBe(0);
    await act(async () => finishPrompt());
    await waitFor(() => expect(called(tauri, "plugin:window|destroy")).toBe(1));
    expect(called(tauri, "set_background")).toBe(1);
    expect(logLines(tauri)).toEqual(["info: close requested", "info: the save is complete", "info: destroy called"]);
  });

  test("a log failure does not stop the close", async () => {
    // With no log handler, each log call fails.
    const tauri = backend({ "plugin:window|destroy": () => null });
    renderPrompts();
    await screen.findByRole("radio", { name: /^Team sync/ });
    await act(() => new Promise((r) => setTimeout(r, 10)));

    await tauri.emit("tauri://close-requested");
    await waitFor(() => expect(called(tauri, "plugin:window|destroy")).toBe(1));
  });

  test("Delete waits for a save of the same prompt that already started", async () => {
    // The save command waits until the test calls `finish`.
    let finish: () => void = () => {};
    const saved = new Promise<void>((r) => (finish = r));
    let done = false;
    let savedFirst = false;
    const tauri = backend({
      set_summary_prompt_text: async () => {
        await saved;
        done = true;
        return baseSettings;
      },
      delete_summary_prompt: () => {
        savedFirst = done;
        return baseSettings;
      },
    });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "Changed");
    fireEvent.blur(textArea());
    await user.click(button("Delete"));
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }));
    await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(textCalls(tauri)).toHaveLength(1);
    expect(tauri.calls.some((c) => c.cmd === "delete_summary_prompt")).toBe(false);

    await act(async () => finish());
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "delete_summary_prompt")).toBe(true));
    expect(savedFirst).toBe(true);
  });

  test("two saves apply in the order typed", async () => {
    // The first save waits until the test calls `release`.
    let release: () => void = () => {};
    const held = new Promise<void>((r) => (release = r));
    const done: string[] = [];
    let text = "Sync text";
    const tauri = backend({
      set_summary_prompt_text: async (args) => {
        if (done.length === 0) await held;
        text = args.text as string;
        done.push(text);
        return { ...baseSettings, summary_prompts: [{ id: "c1", name: "Team sync", text }] };
      },
      summary_prompts: () => ({
        prompts: [
          { id: "topic-summary", name: "Topic summary", text: "Topic text", title_text: "Built-in title text", built_in: true, over_limit: false, too_large: false },
          { id: "play-by-play", name: "Play-by-play summary", text: "Play text", title_text: "Built-in title text", built_in: true, over_limit: false, too_large: false },
          { id: "c1", name: "Team sync", text, title_text: "Sync title", built_in: false, over_limit: false, too_large: false },
        ],
        active: "topic-summary",
        text_limit: 4000,
        title_text_limit: 1000,
      }),
    });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "First");
    fireEvent.blur(textArea());
    await setText(textArea(), "First, then second");
    fireEvent.blur(textArea());
    await act(() => new Promise((r) => setTimeout(r, 50)));
    // The second save waits for the first.
    expect(textCalls(tauri)).toHaveLength(1);

    await act(async () => release());
    await waitFor(() => expect(done).toEqual(["First", "First, then second"]));
    expect(textCalls(tauri)[1]).toEqual({ id: "c1", text: "First, then second" });
    await waitFor(() => expect(text).toBe("First, then second"));
    expect(textArea().textContent).toBe("First, then second");
  });

  test("text typed while a save runs stays in the editor and the next save saves it", async () => {
    // The save command waits until the test calls `finish`.
    let finish: () => void = () => {};
    const saved = new Promise<void>((r) => (finish = r));
    let text = "Sync text";
    backend({
      set_summary_prompt_text: async (args) => {
        await saved;
        text = args.text as string;
        return { ...baseSettings, summary_prompts: [{ id: "c1", name: "Team sync", text }] };
      },
      summary_prompts: () => ({
        prompts: [
          { id: "topic-summary", name: "Topic summary", text: "Topic text", title_text: "Built-in title text", built_in: true, over_limit: false, too_large: false },
          { id: "play-by-play", name: "Play-by-play summary", text: "Play text", title_text: "Built-in title text", built_in: true, over_limit: false, too_large: false },
          { id: "c1", name: "Team sync", text, title_text: "Sync title", built_in: false, over_limit: false, too_large: false },
        ],
        active: "topic-summary",
        text_limit: 4000,
        title_text_limit: 1000,
      }),
    });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "Short note");
    fireEvent.blur(textArea());
    await setText(textArea(), "Short note, longer");
    await act(async () => finish());
    await waitFor(() => expect(text).toBe("Short note"));
    await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(textArea().textContent).toBe("Short note, longer");

    fireEvent.blur(textArea());
    await waitFor(() => expect(text).toBe("Short note, longer"));
    expect(textArea().textContent).toBe("Short note, longer");
  });

  test("a list read error shows inline", async () => {
    backend({
      summary_prompts: () => {
        throw "The settings file could not be read.";
      },
    });
    renderPrompts();
    expect(await screen.findByText("The settings file could not be read.")).toBeTruthy();
  });

  test("a rename error shows inline", async () => {
    backend({
      rename_summary_prompt: () => {
        throw "A prompt with that name already exists.";
      },
    });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    const name = screen.getByRole("textbox", { name: "Name" });
    await user.clear(name);
    await user.type(name, "Topic summary{Enter}");
    expect(await screen.findByText("A prompt with that name already exists.")).toBeTruthy();
  });

  test("a rename calls the rename command", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    const name = screen.getByRole("textbox", { name: "Name" });
    await user.clear(name);
    await user.type(name, "1:1{Enter}");
    await waitFor(() => expect(row("1:1")).toBeTruthy());
    expect(tauri.calls.find((c) => c.cmd === "rename_summary_prompt")?.args).toEqual({ id: "c1", name: "1:1" });
  });

  test("rows select with Enter and Space from the keyboard", async () => {
    backend();
    renderPrompts();
    const user = userEvent.setup();
    await waitFor(() => expect(textArea().textContent).toBe("Topic text"));
    row("Play-by-play summary").focus();
    await user.keyboard("{Enter}");
    await waitFor(() => expect(textArea().textContent).toBe("Play text"));
    row("Team sync").focus();
    await user.keyboard(" ");
    await waitFor(() => expect(textArea().textContent).toBe("Sync text"));
  });

  test("Delete asks for confirmation, then calls delete", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await user.click(button("Delete"));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText("Delete “Team sync”?")).toBeTruthy();
    expect(within(dialog).getByText("This can’t be undone.")).toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(tauri.calls.some((c) => c.cmd === "delete_summary_prompt")).toBe(false);

    await user.click(button("Delete"));
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(screen.queryByRole("radio", { name: /^Team sync/ })).toBeNull());
    expect(tauri.calls.find((c) => c.cmd === "delete_summary_prompt")?.args).toEqual({ id: "c1" });
    expect(row("Topic summary").getAttribute("aria-checked")).toBe("true");
  });

  const typeEntry = (name: string, prompt_choice: string | null, summary_prompt_id = prompt_choice ?? "topic-summary") => ({
    name, valid: true, shared: true, notes: 0, general: false, prompt_choice, prompt_name: null, summary_prompt_id,
  });
  const noteTypes = (types: ReturnType<typeof typeEntry>[]) => ({ note_types: () => ({ types, default_type: null }) });
  const deleteCustom = async () => {
    const user = userEvent.setup();
    await selectCustom(user);
    await user.click(button("Delete"));
    return within(await screen.findByRole("alertdialog"));
  };

  test("Delete of a prompt that one type uses says that one type uses it", async () => {
    backend(noteTypes([typeEntry("Standup", "c1"), typeEntry("Review", null, "c1"), typeEntry("Plan", "play-by-play")]));
    renderPrompts();
    const dialog = await deleteCustom();
    expect(dialog.getByText(/^1 type uses this prompt\. It changes to Default\./)).toBeTruthy();
  });

  test("Delete of a prompt that no type uses shows no type count", async () => {
    backend(noteTypes([typeEntry("Review", null, "c1"), typeEntry("Plan", "play-by-play")]));
    renderPrompts();
    const dialog = await deleteCustom();
    expect(dialog.getByText("This can’t be undone.")).toBeTruthy();
    expect(dialog.queryByText(/type/)).toBeNull();
  });

  test("Delete of a prompt that two types use gives the count", async () => {
    backend(noteTypes([typeEntry("Standup", "c1"), typeEntry("Review", "c1")]));
    renderPrompts();
    const dialog = await deleteCustom();
    expect(dialog.getByText(/^2 types use this prompt\. They change to Default\./)).toBeTruthy();
  });

  test("the footer says that the default prompt applies to each type set to Default", async () => {
    backend();
    renderPrompts();
    expect(await screen.findByText(/The default prompt writes the summary of each type set to “Default”/)).toBeTruthy();
  });

  test("clicking a row only selects it, and its Use button makes it the prompt in use", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    expect(tauri.calls.some((c) => c.cmd === "activate_summary_prompt")).toBe(false);
    expect(within(row("Topic summary").parentElement!).getByText("Default")).toBeTruthy();

    await user.click(button("Make default Team sync"));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Make default Team sync" })).toBeNull());
    expect(tauri.calls.find((c) => c.cmd === "activate_summary_prompt")?.args).toEqual({ id: "c1" });
    expect(button("Make default Topic summary")).toBeTruthy();
  });

  test("an activate error shows inline in the list", async () => {
    backend({
      activate_summary_prompt: () => {
        throw "The settings could not be saved.";
      },
    });
    renderPrompts();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Make default Team sync" }));
    expect(await screen.findByText("The settings could not be saved.")).toBeTruthy();
  });

  test("text of 4,001 characters shows the count in the error colour and is not saved", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "a".repeat(4001));
    expect(screen.getByText("4,001 / 4,000").className).toContain("text-negative");
    fireEvent.blur(textArea());
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(textCalls(tauri)).toEqual([]);
    expect(textArea().textContent).toHaveLength(4001);

    await setText(textArea(), "a".repeat(4000));
    expect(screen.getByText("4,000 / 4,000").className).not.toContain("text-negative");
    fireEvent.blur(textArea());
    await waitFor(() => expect(textCalls(tauri)).toHaveLength(1));
  });

  // Safe agent edits U6, R21: an outside edit made a prompt too long.
  const overLimitList = (custom: Record<string, unknown>[]) => () => ({
    prompts: [
      { id: "topic-summary", name: "Topic summary", text: "Topic text", title_text: "Built-in title text", built_in: true, over_limit: false, too_large: false },
      ...custom,
    ],
    active: "topic-summary",
    text_limit: 4000,
    title_text_limit: 1000,
  });

  test("a prompt over the limit shows its counts in the list and saves only at the limit", async () => {
    const long = "b".repeat(5000);
    const tauri = backend({
      summary_prompts: overLimitList([
        { id: "c1", name: "Team sync", text: long, title_text: "", built_in: false, over_limit: true, too_large: false },
      ]),
    });
    renderPrompts();
    const user = userEvent.setup();
    expect((await screen.findByRole("radio", { name: /^Team sync/ })).textContent).toContain("5,000 / 4,000 characters");
    await user.click(row("Team sync"));
    await waitFor(() => expect(textArea().textContent).toBe(long));
    expect(screen.getByText("5,000 / 4,000").className).toContain("text-negative");
    expect(screen.getByText(/The prompt text is over the limit. Summaries use the whole text./)).toBeTruthy();
    // Rename, delete and Make default stay available.
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).disabled).toBe(false);
    expect((button("Delete") as HTMLButtonElement).disabled).toBe(false);
    expect((button("Make default Team sync") as HTMLButtonElement).disabled).toBe(false);
    fireEvent.blur(textArea());
    await pause(50);
    expect(textCalls(tauri)).toEqual([]);
    await setText(textArea(), "a".repeat(4000));
    expect(screen.queryByText(/is over the limit/)).toBeNull();
    fireEvent.blur(textArea());
    await waitFor(() => expect(textCalls(tauri)).toHaveLength(1));
  });

  test("title instructions over the limit show the title counts in the list", async () => {
    backend({
      summary_prompts: overLimitList([
        { id: "c1", name: "Team sync", text: "Sync text", title_text: "t".repeat(1200), built_in: false, over_limit: true, too_large: false },
      ]),
    });
    renderPrompts();
    const user = userEvent.setup();
    expect((await screen.findByRole("radio", { name: /^Team sync/ })).textContent).toContain("Title 1,200 / 1,000 characters");
    await user.click(row("Team sync"));
    expect(await screen.findByText(/The title instructions are over the limit/)).toBeTruthy();
  });

  test("a prompt whose file is too large shows no text and offers rename and delete", async () => {
    backend({
      summary_prompts: overLimitList([
        { id: "c1", name: "Team sync", text: "", title_text: "", built_in: false, over_limit: true, too_large: true },
      ]),
    });
    renderPrompts();
    const user = userEvent.setup();
    expect((await screen.findByRole("radio", { name: /^Team sync/ })).textContent).toContain("File too large");
    await user.click(row("Team sync"));
    expect(await screen.findByText(/The file of this prompt is too large to show/)).toBeTruthy();
    expect(textArea().getAttribute("aria-readonly")).toBe("true");
    expect(screen.queryByText("A prompt can’t be empty.")).toBeNull();
    expect((button("Duplicate") as HTMLButtonElement).disabled).toBe(true);
    expect((button("Delete") as HTMLButtonElement).disabled).toBe(false);
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).disabled).toBe(false);
  });

  test("with only the settings file read-only, prompts change and Make default does not", async () => {
    // R22: the default prompt is in the settings file, the prompts are not.
    backend();
    mockWindows("settings");
    render(
      <SummaryPrompts
        onSettings={() => {}}
        readOnly={null}
        settingsReadOnly="The settings file in the storage folder has an error."
      />,
    );
    const user = userEvent.setup();
    await selectCustom(user);
    expect((button("Make default Team sync") as HTMLButtonElement).disabled).toBe(true);
    expect(textArea().getAttribute("aria-readonly")).toBeNull();
    expect((button("New Prompt") as HTMLButtonElement).disabled).toBe(false);
    expect(screen.getByText(/has an error. The default prompt can’t be changed now./)).toBeTruthy();
  });

  test("the count counts characters, not UTF-16 units", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "😀".repeat(4000));
    expect(screen.getByText("4,000 / 4,000")).toBeTruthy();
    fireEvent.blur(textArea());
    await waitFor(() => expect(textCalls(tauri)).toHaveLength(1));
  });

  test("a built-in prompt shows its title text read-only, with no count", async () => {
    backend();
    renderPrompts();
    await waitFor(() => expect(titleArea().textContent).toBe("Built-in title text"));
    expect(titleArea().getAttribute("aria-readonly")).toBe("true");
    expect(screen.queryByText(/ \/ 1,000$/)).toBeNull();
  });

  test("typing in the title field saves the title text with the title command only", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    expect(titleArea().textContent).toBe("Sync title");
    expect(screen.getByText("10 / 1,000")).toBeTruthy();
    await setText(titleArea(), "Type: topics");
    await waitFor(() => expect(titleCalls(tauri)).toEqual([{ id: "c1", text: "Type: topics" }]), { timeout: 3000 });
    expect(textCalls(tauri)).toEqual([]);
    expect(titleArea().textContent).toBe("Type: topics");
    expect(textArea().textContent).toBe("Sync text");
  });

  test("an empty title field saves and shows no error", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(titleArea(), "");
    fireEvent.blur(titleArea());
    await waitFor(() => expect(titleCalls(tauri)).toEqual([{ id: "c1", text: "" }]), { timeout: 500 });
    expect(screen.queryByText("A prompt can’t be empty.")).toBeNull();
    expect(screen.getByText("0 / 1,000").className).not.toContain("text-negative");
  });

  test("a title text over the limit shows the count in the error colour and is not saved", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(titleArea(), "a".repeat(1001));
    expect(screen.getByText("1,001 / 1,000").className).toContain("text-negative");
    fireEvent.blur(titleArea());
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(titleCalls(tauri)).toEqual([]);

    await setText(titleArea(), "a".repeat(1000));
    expect(screen.getByText("1,000 / 1,000").className).not.toContain("text-negative");
    fireEvent.blur(titleArea());
    await waitFor(() => expect(titleCalls(tauri)).toHaveLength(1));
  });

  test("a save error in the title field names that field, and the summary text still saves", async () => {
    const tauri = backend({
      set_summary_prompt_title_text: () => {
        throw "The prompt file cannot be written: disk full";
      },
    });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(titleArea(), "New title text");
    fireEvent.blur(titleArea());
    expect(
      await screen.findByText("The title instructions could not be saved. The prompt file cannot be written: disk full"),
    ).toBeTruthy();
    expect(titleArea().textContent).toBe("New title text");

    await setText(textArea(), "New summary text");
    fireEvent.blur(textArea());
    await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "New summary text" }]));
    expect(screen.getByText(/^The title instructions could not be saved/)).toBeTruthy();
  });

  test("an edit in one field does not drop the waiting save of the other field", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(titleArea(), "New title text");
    await setText(textArea(), "New summary text");
    await waitFor(
      () => {
        expect(titleCalls(tauri)).toEqual([{ id: "c1", text: "New title text" }]);
        expect(textCalls(tauri)).toEqual([{ id: "c1", text: "New summary text" }]);
      },
      { timeout: 3000 },
    );
  });

  test("a change of prompt drops a title text over the limit and saves a valid one", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(titleArea(), "a".repeat(1001));
    await user.click(row("Topic summary"));
    await user.click(row("Team sync"));
    await waitFor(() => expect(titleArea().textContent).toBe("Sync title"));
    expect(titleCalls(tauri)).toEqual([]);

    await setText(titleArea(), "Changed title");
    await user.click(row("Topic summary"));
    await waitFor(() => expect(titleCalls(tauri)).toEqual([{ id: "c1", text: "Changed title" }]), { timeout: 500 });
  });

  test("a built-in prompt shows its formatted text in a read-only editor with no toolbar (AE6)", async () => {
    const tauri = backend({
      summary_prompts: () => ({
        prompts: [
          { id: "topic-summary", name: "Topic summary", text: "## Topics\n\n- one\n- two", title_text: "Built-in title text", built_in: true, over_limit: false, too_large: false },
          { id: "c1", name: "Team sync", text: "Sync text", title_text: "Sync title", built_in: false, over_limit: false, too_large: false },
        ],
        active: "topic-summary",
        text_limit: 4000,
        title_text_limit: 1000,
      }),
    });
    renderPrompts();
    const user = userEvent.setup();
    await waitFor(() => expect(textArea().querySelector("h2")?.textContent).toBe("Topics"));
    expect(textArea().querySelectorAll("li")).toHaveLength(2);
    expect(textArea().getAttribute("aria-readonly")).toBe("true");
    await user.click(textArea());
    await user.keyboard(selectAll);
    expect(screen.queryByRole("toolbar", { name: "Format selection" })).toBeNull();
    await user.keyboard("x");
    act(() => void fireEvent.blur(textArea()));
    await pause(20);
    expect(textArea().textContent).not.toContain("x");
    expect(textCalls(tauri)).toEqual([]);
  });

  test("selecting another prompt shows that prompt's texts", async () => {
    backend();
    renderPrompts();
    const user = userEvent.setup();
    await waitFor(() => expect(textArea().textContent).toBe("Topic text"));
    await selectCustom(user);
    expect(titleArea().textContent).toBe("Sync title");
    await user.click(row("Play-by-play summary"));
    await waitFor(() => expect(textArea().textContent).toBe("Play text"));
    expect(titleArea().textContent).toBe("Built-in title text");
    expect(textArea().getAttribute("aria-readonly")).toBe("true");
  });

  test("a title text whose save failed comes back when the user selects its prompt again", async () => {
    const tauri = backend({
      set_summary_prompt_title_text: () => {
        throw "disk full";
      },
    });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(titleArea(), "Changed title");
    await user.click(row("Topic summary"));
    await waitFor(() => expect(titleCalls(tauri)).toHaveLength(1));
    await pause(10);
    expect(titleArea().textContent).toBe("Built-in title text");

    await user.click(row("Team sync"));
    await waitFor(() => expect(titleArea().textContent).toBe("Changed title"));
    expect(screen.getByText("The title instructions could not be saved. disk full")).toBeTruthy();
    expect(textArea().textContent).toBe("Sync text");
  });

  test("the title marker line in the prompt text shows the backend error and keeps the text", async () => {
    const marker = "<!-- longhand: title instructions -->";
    const error = `The prompt text can't contain the line ${marker}.`;
    // The backend refuses a summary text with the marker line.
    const tauri = backend({
      set_summary_prompt_text: ({ text }) => {
        if ((text as string).split("\n").some((line) => line.trim() === marker)) throw error;
        return baseSettings;
      },
    });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await user.click(textArea());
    await user.keyboard(selectAll);
    await user.keyboard(`Summary.{Enter}${marker}`);
    act(() => void fireEvent.blur(textArea()));
    expect(await screen.findByText(error)).toBeTruthy();
    expect(textCalls(tauri)).toEqual([{ id: "c1", text: `Summary.\n\n${marker}` }]);
    expect(textArea().textContent).toBe(`Summary.${marker}`);
  });

  test("an own save does not remount the editor, and the focus stays in the field", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    const before = textArea();
    await setText(before, "Short note");
    await waitFor(() => expect(textCalls(tauri)).toHaveLength(1), { timeout: 3000 });
    await pause(20);
    expect(textArea()).toBe(before);
    expect(document.activeElement).toBe(before);
    expect(textArea().textContent).toBe("Short note");
  });

  test("a change to the read-only state does not remount the editor", async () => {
    backend();
    mockWindows("settings");
    const view = render(<SummaryPrompts onSettings={() => {}} />);
    await selectCustom(userEvent.setup());
    const before = textArea();
    expect(before.getAttribute("aria-readonly")).toBeNull();
    view.rerender(<SummaryPrompts onSettings={() => {}} readOnly="The storage folder is not available." />);
    expect(textArea()).toBe(before);
    expect(textArea().getAttribute("aria-readonly")).toBe("true");
  });
});


test("a pane change in the Settings window saves the prompt text and asks nothing", async () => {
  mockWindows("settings");
  const tauri = backend({
    get_settings: () => baseSettings,
    readiness: () => readiness,
    shortcut_status: () => shortcutStatuses,
    login_item_status: () => "disabled",
    model_statuses: () => [],
    speaker_models_status: () => ({ ids: [], size_bytes: 0, state: { state: "present" } }),
  });
  render(<SettingsWindow />);
  const user = userEvent.setup();
  await user.click(await screen.findByRole("tab", { name: "Summaries" }));
  await selectCustom(user);
  await setText(textArea(), "Changed");
  await user.click(screen.getByRole("tab", { name: "Transcription" }));
  expect(screen.queryByRole("alertdialog")).toBeNull();
  expect(screen.getByRole("tab", { name: "Transcription" }).getAttribute("aria-selected")).toBe("true");
  await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "Changed" }]), { timeout: 500 });
});

describe("shared prompts", () => {
  /// A backend whose prompts another computer can change: `remote` changes
  /// the saved values, and then sends `settings-changed`.
  function sharedBackend() {
    const state = { active: "topic-summary", text: "Sync text", title_text: "Sync title" };
    const list = () => ({
      prompts: [
        { id: "topic-summary", name: "Topic summary", text: "Topic text", title_text: "Built-in title text", built_in: true, over_limit: false, too_large: false },
        { id: "c1", name: "Team sync", text: state.text, title_text: state.title_text, built_in: false, over_limit: false, too_large: false },
      ],
      active: state.active,
      text_limit: 4000,
      title_text_limit: 1000,
    });
    const tauri = backend({
      summary_prompts: list,
      set_summary_prompt_text: ({ text }) => {
        state.text = text as string;
        return baseSettings;
      },
      set_summary_prompt_title_text: ({ text }) => {
        state.title_text = text as string;
        return baseSettings;
      },
    });
    const remote = async (change: Partial<typeof state>) => {
      Object.assign(state, change);
      await act(() => tauri.emit("settings-changed", { settings: baseSettings }));
    };
    return { tauri, remote };
  }

  const inUseRow = () =>
    within(screen.getByRole("radiogroup", { name: "Summary prompts" })).getByText("Default").closest(".settings-row")
      ?.textContent;

  test("a change from another computer refreshes the prompt in use", async () => {
    const { remote } = sharedBackend();
    renderPrompts();
    await waitFor(() => expect(inUseRow()).toContain("Topic summary"));
    await remote({ active: "c1" });
    await waitFor(() => expect(inUseRow()).toContain("Team sync"));
  });

  test("a change from another computer shows the new text when the editor has no unsaved text", async () => {
    const { remote } = sharedBackend();
    renderPrompts();
    await selectCustom(userEvent.setup());
    await remote({ text: "Text from the other computer" });
    await waitFor(() => expect(textArea().textContent).toBe("Text from the other computer"));
    expect(screen.queryByText(/changed on another computer/)).toBeNull();
  });

  test("a change from another computer to a prompt with unsaved text keeps the text and offers Reload", async () => {
    const { tauri, remote } = sharedBackend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "My unsaved text");
    await remote({ text: "Text from the other computer" });
    expect(await screen.findByText("The prompt text changed on another computer.")).toBeTruthy();
    expect(textArea().textContent).toBe("My unsaved text");
    // The unsaved text does not replace the text of the other computer.
    await new Promise((r) => setTimeout(r, 1200));
    fireEvent.blur(textArea());
    await new Promise((r) => setTimeout(r, 20));
    expect(textCalls(tauri)).toEqual([]);
    await user.click(button("Reload"));
    expect(textArea().textContent).toBe("Text from the other computer");
    expect(screen.queryByText("The prompt text changed on another computer.")).toBeNull();
    expect(textCalls(tauri)).toEqual([]);
  });

  test("text typed after the change from another computer is not saved until Keep My Text", async () => {
    const { tauri, remote } = sharedBackend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "My unsaved text");
    await remote({ text: "Text from the other computer" });
    expect(await screen.findByText("The prompt text changed on another computer.")).toBeTruthy();
    await setText(textArea(), "My unsaved text and more");
    expect(screen.getByText("The prompt text changed on another computer.")).toBeTruthy();
    await new Promise((r) => setTimeout(r, 1200));
    fireEvent.blur(textArea());
    await new Promise((r) => setTimeout(r, 20));
    expect(textCalls(tauri)).toEqual([]);
    await user.click(button("Keep My Text"));
    await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "My unsaved text and more" }]));
    await waitFor(() => expect(screen.queryByText("The prompt text changed on another computer.")).toBeNull());
  });

  test("Keep My Text saves the unsaved text over the change from another computer", async () => {
    const { tauri, remote } = sharedBackend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "My unsaved text");
    await remote({ text: "Text from the other computer" });
    await user.click(await screen.findByRole("button", { name: "Keep My Text" }));
    await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "My unsaved text" }]));
    await waitFor(() => expect(screen.queryByText("The prompt text changed on another computer.")).toBeNull());
    expect(textArea().textContent).toBe("My unsaved text");
  });

  test("text typed after this computer's own save shows no notice", async () => {
    const { tauri } = sharedBackend();
    renderPrompts();
    await selectCustom(userEvent.setup());
    await setText(textArea(), "First");
    fireEvent.blur(textArea());
    await waitFor(() => expect(textCalls(tauri)).toHaveLength(1));
    await setText(textArea(), "First and more");
    await act(() => tauri.emit("settings-changed", { settings: baseSettings }));
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByText(/changed on another computer/)).toBeNull();
    expect(textArea().textContent).toBe("First and more");
  });

  test("a change from another computer to the title text shows the notice for the title field only", async () => {
    const { tauri, remote } = sharedBackend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(titleArea(), "My title text");
    await remote({ title_text: "Title from the other computer" });
    expect(await screen.findByText("The title instructions changed on another computer.")).toBeTruthy();
    expect(screen.queryByText("The prompt text changed on another computer.")).toBeNull();
    expect(titleArea().textContent).toBe("My title text");

    // An edit in the summary field still saves.
    await setText(textArea(), "New summary text");
    fireEvent.blur(textArea());
    await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "New summary text" }]));
    fireEvent.blur(titleArea());
    await new Promise((r) => setTimeout(r, 20));
    expect(titleCalls(tauri)).toEqual([]);
    expect(screen.getByText("The title instructions changed on another computer.")).toBeTruthy();

    await user.click(button("Keep My Text title instructions"));
    await waitFor(() => expect(titleCalls(tauri)).toEqual([{ id: "c1", text: "My title text" }]));
    await waitFor(() => expect(screen.queryByText(/changed on another computer/)).toBeNull());
  });

  test("Reload in the title notice shows the title text of the other computer", async () => {
    const { tauri, remote } = sharedBackend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(titleArea(), "My title text");
    await remote({ title_text: "Title from the other computer" });
    await user.click(await screen.findByRole("button", { name: "Reload title instructions" }));
    expect(titleArea().textContent).toBe("Title from the other computer");
    expect(screen.queryByText(/changed on another computer/)).toBeNull();
    expect(titleCalls(tauri)).toEqual([]);
  });

  test("a read-only state disables the prompt editor and the prompt in use", async () => {
    sharedBackend();
    mockWindows("settings");
    render(<SummaryPrompts onSettings={() => {}} readOnly="The storage folder is not available." />);
    await selectCustom(userEvent.setup());
    expect(screen.getByText("The storage folder is not available. Prompts can’t be changed now.")).toBeTruthy();
    expect(textArea().getAttribute("aria-readonly")).toBe("true");
    expect(titleArea().getAttribute("aria-readonly")).toBe("true");
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).disabled).toBe(true);
    expect((button("New Prompt") as HTMLButtonElement).disabled).toBe(true);
    expect((button("Make default Team sync") as HTMLButtonElement).disabled).toBe(true);
    expect((button("Duplicate") as HTMLButtonElement).disabled).toBe(true);
    expect((button("Delete") as HTMLButtonElement).disabled).toBe(true);
  });
});

describe("own saves of prompts", () => {
  /// A backend whose list reads and saves can wait until the test lets them
  /// go. `remote` changes the saved text on another computer and sends
  /// `settings-changed`.
  function heldBackend() {
    const state = { text: "Sync text", fail: false, present: true };
    const restored: { id: string; name: string; text: string; title_text: string }[] = [];
    const gates = { list: [] as (() => void)[], save: null as Promise<void> | null };
    const reads = { holdNext: false, done: 0 };
    const saves = { done: 0 };
    const custom = (p: { id: string; name: string; text: string; title_text: string }) => ({
      ...p,
      built_in: false,
      over_limit: false,
      too_large: false,
    });
    const list = () => ({
      prompts: [
        { id: "topic-summary", name: "Topic summary", text: "Topic text", title_text: "Built-in title text", built_in: true, over_limit: false, too_large: false },
        ...(state.present ? [custom({ id: "c1", name: "Team sync", text: state.text, title_text: "Sync title" })] : []),
        ...restored.map(custom),
      ],
      active: "topic-summary",
      text_limit: 4000,
      title_text_limit: 1000,
    });
    const tauri = backend({
      // The list is made when the read starts, as the backend reads the
      // files then.
      summary_prompts: async () => {
        const result = list();
        if (reads.holdNext) {
          reads.holdNext = false;
          await new Promise<void>((r) => gates.list.push(r));
        }
        reads.done += 1;
        return result;
      },
      set_summary_prompt_text: async ({ text }) => {
        if (gates.save) await gates.save;
        saves.done += 1;
        if (!state.present) throw "No such prompt.";
        if (state.fail) {
          state.fail = false;
          throw "The prompt file could not be written.";
        }
        state.text = text as string;
        return baseSettings;
      },
      restore_summary_prompt: ({ name, text, titleText }) => {
        const id = `r${restored.length + 1}`;
        restored.push({ id, name: name as string, text: text as string, title_text: titleText as string });
        return { id, settings: baseSettings };
      },
      delete_summary_prompt: () => {
        state.present = false;
        return baseSettings;
      },
      "plugin:window|destroy": () => null,
      log_from_webview: () => null,
    });
    const remote = async (text: string) => {
      state.text = text;
      await act(() => tauri.emit("settings-changed", { settings: baseSettings }));
    };
    // Another computer deletes the prompt "Team sync".
    const remoteDelete = async () => {
      state.present = false;
      await act(() => tauri.emit("settings-changed", { settings: baseSettings }));
    };
    const holdSaves = () => {
      let release: () => void = () => {};
      gates.save = new Promise<void>((r) => (release = r));
      return () => act(async () => release());
    };
    const releaseRead = () => act(async () => gates.list.shift()?.());
    return { tauri, state, reads, saves, remote, remoteDelete, holdSaves, releaseRead };
  }

  const deletedNotice = "“Team sync” was deleted on another computer.";
  const restoreCalls = (tauri: TauriMock) =>
    tauri.calls.filter((c) => c.cmd === "restore_summary_prompt").map((c) => c.args);

  test("a prompt deleted on another computer keeps the unsaved text, and Save as New Prompt saves it", async () => {
    const b = heldBackend();
    renderPrompts();
    await selectCustom(userEvent.setup());
    await setText(textArea(), "Mine");
    await b.remoteDelete();
    expect(await screen.findByText(deletedNotice)).toBeTruthy();
    expect(screen.getByText("Save as New Prompt keeps your text. Discard removes it.")).toBeTruthy();
    await act(async () => void fireEvent.click(button(/^Save as New Prompt/)));
    await waitFor(() => expect(restoreCalls(b.tauri)).toEqual([{ name: "Team sync", text: "Mine", titleText: "Sync title" }]));
    await waitFor(() => expect(textArea().textContent).toBe("Mine"));
    expect(row("Team sync").getAttribute("aria-checked")).toBe("true");
    expect(screen.queryByText(deletedNotice)).toBeNull();
    expect(textCalls(b.tauri)).toEqual([]);
  });

  test("a prompt deleted on another computer keeps an unsaved name", async () => {
    const b = heldBackend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await user.type(screen.getByRole("textbox", { name: "Name" }), " notes");
    await b.remoteDelete();
    expect(await screen.findByText("“Team sync notes” was deleted on another computer.")).toBeTruthy();
    expect(called(b.tauri, "rename_summary_prompt")).toBe(0);
    await act(async () => void fireEvent.click(button(/^Save as New Prompt/)));
    await waitFor(() =>
      expect(restoreCalls(b.tauri)).toEqual([{ name: "Team sync notes", text: "Sync text", titleText: "Sync title" }]),
    );
  });

  test("a deleted prompt whose Name field was cleared keeps its saved name", async () => {
    const b = heldBackend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "Mine");
    await user.clear(screen.getByRole("textbox", { name: "Name" }));
    await b.remoteDelete();
    expect(await screen.findByText(deletedNotice)).toBeTruthy();
    await act(async () => void fireEvent.click(button(/^Save as New Prompt/)));
    await waitFor(() => expect(restoreCalls(b.tauri)).toEqual([{ name: "Team sync", text: "Mine", titleText: "Sync title" }]));
  });

  test("an invalid unsaved text of a deleted prompt becomes the unsaved text of the new prompt", async () => {
    const b = heldBackend();
    renderPrompts();
    await selectCustom(userEvent.setup());
    await setText(textArea(), "");
    await b.remoteDelete();
    expect(await screen.findByText(deletedNotice)).toBeTruthy();
    await act(async () => void fireEvent.click(button(/^Save as New Prompt/)));
    await waitFor(() => expect(restoreCalls(b.tauri)).toEqual([{ name: "Team sync", text: "Sync text", titleText: "Sync title" }]));
    await waitFor(() => expect(row("Team sync").getAttribute("aria-checked")).toBe("true"));
    expect(textArea().textContent).toBe("");
    expect(screen.getByText("A prompt can’t be empty.")).toBeTruthy();
  });

  test("Discard asks first and then removes the unsaved text of a deleted prompt", async () => {
    const b = heldBackend();
    renderPrompts();
    await selectCustom(userEvent.setup());
    await setText(textArea(), "Mine");
    await b.remoteDelete();
    expect(await screen.findByText(deletedNotice)).toBeTruthy();
    await act(async () => void fireEvent.click(button(/^Discard/)));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("Discard your changes to “Team sync”?");
    await act(async () => void fireEvent.click(within(dialog).getByRole("button", { name: "Discard" })));
    await waitFor(() => expect(screen.queryByText(deletedNotice)).toBeNull());
    expect(restoreCalls(b.tauri)).toEqual([]);
  });

  test("a delete in this window shows no notice", async () => {
    const b = heldBackend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await setText(textArea(), "Mine");
    await act(async () => void fireEvent.click(button("Delete")));
    const dialog = await screen.findByRole("alertdialog");
    await act(async () => void fireEvent.click(within(dialog).getByRole("button", { name: "Delete" })));
    await waitFor(() => expect(called(b.tauri, "delete_summary_prompt")).toBe(1));
    await waitFor(() => expect(screen.queryByRole("radio", { name: /^Team sync/ })).toBeNull());
    expect(screen.queryByText(deletedNotice)).toBeNull();
  });

  test("a save that fails because another computer deleted the prompt shows no error", async () => {
    const b = heldBackend();
    renderPrompts();
    await selectCustom(userEvent.setup());
    const release = b.holdSaves();
    await setText(textArea(), "Mine");
    await waitFor(() => expect(textCalls(b.tauri)).toHaveLength(1), { timeout: 1500 });
    await b.remoteDelete();
    expect(await screen.findByText(deletedNotice)).toBeTruthy();
    await release();
    await waitFor(() => expect(b.saves.done).toBe(1));
    await act(async () => {});
    expect(screen.queryByText("No such prompt.")).toBeNull();
    expect(screen.getByText(deletedNotice)).toBeTruthy();
  });

  // Types into the editor that has the focus, after the pasted text.
  const typeAtEnd = (text: string) => userEvent.setup().keyboard(text);

  test("text typed while the list is read again after a save is saved", async () => {
    const b = heldBackend();
    renderPrompts();
    await selectCustom(userEvent.setup());
    await setText(textArea(), "T1");
    b.reads.holdNext = true;
    await waitFor(() => expect(textCalls(b.tauri)).toEqual([{ id: "c1", text: "T1" }]), { timeout: 1500 });
    await waitFor(() => expect(b.saves.done).toBe(1));
    await typeAtEnd("x");
    await b.releaseRead();
    await waitFor(() => expect(b.reads.done).toBeGreaterThanOrEqual(2));
    expect(screen.queryByText(/changed on another computer/)).toBeNull();
    act(() => void fireEvent.blur(textArea()));
    await waitFor(() =>
      expect(textCalls(b.tauri)).toEqual([
        { id: "c1", text: "T1" },
        { id: "c1", text: "T1x" },
      ]),
    );
  });

  test("a change on another computer back to a text that this window saved before shows the text", async () => {
    const b = heldBackend();
    renderPrompts();
    await selectCustom(userEvent.setup());
    await setText(textArea(), "A");
    await waitFor(() => expect(textCalls(b.tauri)).toHaveLength(1), { timeout: 1500 });
    await setText(textArea(), "AB");
    await waitFor(() => expect(textCalls(b.tauri)).toHaveLength(2), { timeout: 1500 });
    // The read after the second save is done.
    await waitFor(() => expect(b.reads.done).toBe(3));
    await b.remote("A");
    await waitFor(() => expect(textArea().textContent).toBe("A"));
  });

  test("a list read that ends after a later read does not replace the later list", async () => {
    const b = heldBackend();
    renderPrompts();
    await selectCustom(userEvent.setup());
    b.reads.holdNext = true;
    await b.remote("Sync text");
    await b.remote("New text");
    await waitFor(() => expect(textArea().textContent).toBe("New text"));
    await b.releaseRead();
    await waitFor(() => expect(b.reads.done).toBe(3));
    await act(async () => {});
    expect(textArea().textContent).toBe("New text");
  });

  test("a window close after a pane change waits for a save that was queued behind a running save", async () => {
    const b = heldBackend();
    mockWindows("settings");
    const { unmount } = render(<SummaryPrompts onSettings={() => {}} />);
    await selectCustom(userEvent.setup());
    const release = b.holdSaves();
    await setText(textArea(), "A");
    await waitFor(() => expect(textCalls(b.tauri)).toHaveLength(1), { timeout: 1500 });
    await typeAtEnd("B");
    act(() => void fireEvent.blur(textArea()));
    unmount();
    await b.tauri.emit("tauri://close-requested");
    await release();
    await waitFor(() => expect(called(b.tauri, "plugin:window|destroy")).toBe(1));
    expect(textCalls(b.tauri)).toEqual([
      { id: "c1", text: "A" },
      { id: "c1", text: "AB" },
    ]);
  });

  test("a change of prompt saves the name typed for the prompt before", async () => {
    const b = heldBackend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await user.type(screen.getByRole("textbox", { name: "Name" }), " notes");
    // A click in WKWebView does not move the focus, so no blur comes.
    act(() => void fireEvent.click(row("Topic summary")));
    await waitFor(() =>
      expect(b.tauri.calls.filter((c) => c.cmd === "rename_summary_prompt").map((c) => c.args)).toEqual([
        { id: "c1", name: "Team sync notes" },
      ]),
    );
  });

  test("after a failed save, a change on another computer and Reload, Duplicate and New Prompt run", async () => {
    const b = heldBackend();
    renderPrompts();
    await selectCustom(userEvent.setup());
    b.state.fail = true;
    await setText(textArea(), "Mine");
    expect(await screen.findByText("The prompt file could not be written.", {}, { timeout: 1500 })).toBeTruthy();
    await b.remote("Theirs");
    expect(await screen.findByText("The prompt text changed on another computer.")).toBeTruthy();
    await act(async () => void fireEvent.click(button("Reload")));
    await waitFor(() => expect(textArea().textContent).toBe("Theirs"));
    await act(async () => void fireEvent.click(button("Duplicate")));
    await waitFor(() => expect(called(b.tauri, "duplicate_summary_prompt")).toBe(1));
    await act(async () => void fireEvent.click(button("New Prompt")));
    await waitFor(() => expect(called(b.tauri, "create_summary_prompt")).toBe(1));
  });
});
