import { describe, expect, test } from "bun:test";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockWindows } from "@tauri-apps/api/mocks";
import type { Readiness, Settings, SummaryPrompt } from "../bindings";
import { type CommandHandler, type TauriMock, mockTauri, shortcutStatuses } from "../test/tauri";
import { SettingsWindow } from "./SettingsWindow";
import { SummaryPrompts } from "./SummaryPrompts";

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
  hide_from_dock: false,
  check_updates_automatically: true,
  audio_removal: { kind: "after_days", days: 30 },
  learned_context_limits: {},
  summary_prompts: [],
  active_summary_prompt: "topic-summary",
  shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [] },
};

const readiness: Readiness = {
  microphone_authorized: true,
  system_audio: "allowed",
  destination: { kind: "ready", path: "/x", free_bytes: null },
  armed: true,
  shortcut: { chord: "Alt+Shift+L", registered: true, error: null },
  missing_models: [],
  endpoint_configured: true,
};

/// A fake of the prompt commands that keeps its own list, as the backend does.
/// The backend lists the custom prompts by name.
function backend(over: Record<string, CommandHandler> = {}) {
  const builtIns = [
    { id: "topic-summary", name: "Topic summary", text: "Topic text", title_text: "Built-in title text", built_in: true },
    { id: "play-by-play", name: "Play-by-play summary", text: "Play text", title_text: "Built-in title text", built_in: true },
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
      prompts: [...builtIns, ...byName().map((p) => ({ ...p, built_in: false }))],
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
const textArea = () => screen.getByRole("textbox", { name: "Prompt text" }) as HTMLTextAreaElement;
const button = (name: string | RegExp) => screen.getByRole("button", { name });
const textCalls = (tauri: TauriMock) =>
  tauri.calls.filter((c) => c.cmd === "set_summary_prompt_text").map((c) => c.args);
const titleArea = () => screen.getByRole("textbox", { name: "Title instructions" }) as HTMLTextAreaElement;
const titleCalls = (tauri: TauriMock) =>
  tauri.calls.filter((c) => c.cmd === "set_summary_prompt_title_text").map((c) => c.args);

async function selectCustom(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("radio", { name: /^Team sync/ }));
  await waitFor(() => expect(textArea().value).toBe("Sync text"));
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
    expect(within(group).getAllByText("In use")).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Use Topic summary" })).toBeNull();
    expect(button("Use Play-by-play summary")).toBeTruthy();
    expect(button("Use Team sync")).toBeTruthy();
  });

  test("a built-in prompt shows read-only text and Duplicate, with no Delete or count", async () => {
    backend();
    renderPrompts();
    await waitFor(() => expect(textArea().value).toBe("Topic text"));
    expect(row("Topic summary").getAttribute("aria-checked")).toBe("true");
    expect(textArea().readOnly).toBe(true);
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
    expect(textArea().readOnly).toBe(false);
    expect(onSettings).toHaveLength(1);
  });

  test("Duplicate calls the duplicate command and selects the copy", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await waitFor(() => expect(textArea().value).toBe("Topic text"));
    await user.click(button("Duplicate"));
    await waitFor(() => expect(row("Topic summary copy").getAttribute("aria-checked")).toBe("true"));
    expect(tauri.calls.find((c) => c.cmd === "duplicate_summary_prompt")?.args).toEqual({ id: "topic-summary" });
  });

  test("typing saves the text one second after the last change, in one call", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "Short" } });
    fireEvent.change(textArea(), { target: { value: "Short re" } });
    fireEvent.change(textArea(), { target: { value: "Short note" } });
    await act(() => new Promise((r) => setTimeout(r, 500)));
    expect(textCalls(tauri)).toEqual([]);
    await waitFor(() => expect(textCalls(tauri)).toHaveLength(1), { timeout: 3000 });
    expect(textCalls(tauri)[0]).toEqual({ id: "c1", text: "Short note" });
    expect(textArea().value).toBe("Short note");
  });

  test("blur saves the text at once", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "Short note" } });
    fireEvent.blur(textArea());
    await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "Short note" }]), { timeout: 500 });
  });

  test("selecting another prompt saves the text first and asks nothing", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "Changed" } });
    await user.click(row("Topic summary"));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    await waitFor(() => expect(textArea().value).toBe("Topic text"));
    await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "Changed" }]), { timeout: 500 });

    await user.click(row("Team sync"));
    await waitFor(() => expect(textArea().value).toBe("Changed"));
  });

  test("empty text is not saved and the editor says why", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    expect(screen.queryByText("A prompt can’t be empty.")).toBeNull();
    fireEvent.change(textArea(), { target: { value: "   " } });
    expect(screen.getByText("A prompt can’t be empty.")).toBeTruthy();
    fireEvent.blur(textArea());
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(textCalls(tauri)).toEqual([]);
    expect(textArea().value).toBe("   ");

    // A change of prompt drops the text that is not valid.
    await user.click(row("Topic summary"));
    await user.click(row("Team sync"));
    await waitFor(() => expect(textArea().value).toBe("Sync text"));
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
    fireEvent.change(textArea(), { target: { value: "Short note" } });
    fireEvent.blur(textArea());
    expect(await screen.findByText("The prompt text cannot be saved.")).toBeTruthy();
    expect(textArea().value).toBe("Short note");
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
    fireEvent.change(textArea(), { target: { value: "Changed" } });
    await user.click(row("Topic summary"));
    await waitFor(() => expect(textCalls(tauri)).toHaveLength(1));
    await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(textArea().value).toBe("Topic text");
    expect(screen.queryByText("The prompt text cannot be saved.")).toBeNull();

    await user.click(row("Team sync"));
    expect(textArea().value).toBe("Changed");
    expect(screen.getByText("The prompt text cannot be saved.")).toBeTruthy();

    // A later edit saves again.
    fail = false;
    fireEvent.change(textArea(), { target: { value: "Changed again" } });
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
    fireEvent.change(textArea(), { target: { value: "Changed" } });
    await user.click(screen.getByRole("button", { name: "New Prompt" }));
    expect(await screen.findByText("The prompt text cannot be saved.")).toBeTruthy();
    await user.click(button("Duplicate"));
    await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(tauri.calls.some((c) => c.cmd === "create_summary_prompt")).toBe(false);
    expect(tauri.calls.some((c) => c.cmd === "duplicate_summary_prompt")).toBe(false);
    expect(row("Team sync").getAttribute("aria-checked")).toBe("true");
    expect(textArea().value).toBe("Changed");
  });

  test("pagehide saves the text before the timer", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "Short note" } });
    act(() => void window.dispatchEvent(new Event("pagehide")));
    await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "Short note" }]), { timeout: 500 });
  });

  test("beforeunload saves the text before the timer", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "Short note" } });
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
    fireEvent.change(textArea(), { target: { value: "Changed" } });
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
    fireEvent.change(textArea(), { target: { value: "Changed" } });
    await act(() => new Promise((r) => setTimeout(r, 10)));

    await tauri.emit("tauri://close-requested");
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "plugin:window|destroy")).toBe(true));
    expect(textCalls(tauri)).toHaveLength(1);
  });

  test("a window close closes the window after the time limit when the save does not complete", async () => {
    const tauri = backend({
      // The save never completes.
      set_summary_prompt_text: () => new Promise(() => {}),
      "plugin:window|destroy": () => null,
      log_from_webview: () => null,
    });
    renderPrompts(50);
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "Changed" } });
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
    expect(lines[3]).toStartWith("error: destroy failed: Error: destroy refused");
    expect(lines).toHaveLength(4);
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
    fireEvent.change(textArea(), { target: { value: "Changed" } });
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
          { id: "topic-summary", name: "Topic summary", text: "Topic text", title_text: "Built-in title text", built_in: true },
          { id: "play-by-play", name: "Play-by-play summary", text: "Play text", title_text: "Built-in title text", built_in: true },
          { id: "c1", name: "Team sync", text, title_text: "Sync title", built_in: false },
        ],
        active: "topic-summary",
        text_limit: 4000,
        title_text_limit: 1000,
      }),
    });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "First" } });
    fireEvent.blur(textArea());
    fireEvent.change(textArea(), { target: { value: "First, then second" } });
    fireEvent.blur(textArea());
    await act(() => new Promise((r) => setTimeout(r, 50)));
    // The second save waits for the first.
    expect(textCalls(tauri)).toHaveLength(1);

    await act(async () => release());
    await waitFor(() => expect(done).toEqual(["First", "First, then second"]));
    expect(textCalls(tauri)[1]).toEqual({ id: "c1", text: "First, then second" });
    await waitFor(() => expect(text).toBe("First, then second"));
    expect(textArea().value).toBe("First, then second");
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
          { id: "topic-summary", name: "Topic summary", text: "Topic text", title_text: "Built-in title text", built_in: true },
          { id: "play-by-play", name: "Play-by-play summary", text: "Play text", title_text: "Built-in title text", built_in: true },
          { id: "c1", name: "Team sync", text, title_text: "Sync title", built_in: false },
        ],
        active: "topic-summary",
        text_limit: 4000,
        title_text_limit: 1000,
      }),
    });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "Short note" } });
    fireEvent.blur(textArea());
    fireEvent.change(textArea(), { target: { value: "Short note, longer" } });
    await act(async () => finish());
    await waitFor(() => expect(text).toBe("Short note"));
    await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(textArea().value).toBe("Short note, longer");

    fireEvent.blur(textArea());
    await waitFor(() => expect(text).toBe("Short note, longer"));
    expect(textArea().value).toBe("Short note, longer");
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
    await waitFor(() => expect(textArea().value).toBe("Topic text"));
    row("Play-by-play summary").focus();
    await user.keyboard("{Enter}");
    await waitFor(() => expect(textArea().value).toBe("Play text"));
    row("Team sync").focus();
    await user.keyboard(" ");
    await waitFor(() => expect(textArea().value).toBe("Sync text"));
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

  test("clicking a row only selects it, and its Use button makes it the prompt in use", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    expect(tauri.calls.some((c) => c.cmd === "activate_summary_prompt")).toBe(false);
    expect(within(row("Topic summary").parentElement!).getByText("In use")).toBeTruthy();

    await user.click(button("Use Team sync"));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Use Team sync" })).toBeNull());
    expect(tauri.calls.find((c) => c.cmd === "activate_summary_prompt")?.args).toEqual({ id: "c1" });
    expect(button("Use Topic summary")).toBeTruthy();
  });

  test("an activate error shows inline in the list", async () => {
    backend({
      activate_summary_prompt: () => {
        throw "The settings could not be saved.";
      },
    });
    renderPrompts();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Use Team sync" }));
    expect(await screen.findByText("The settings could not be saved.")).toBeTruthy();
  });

  test("text of 4,001 characters shows the count in the error colour and is not saved", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "a".repeat(4001) } });
    expect(screen.getByText("4,001 / 4,000").className).toContain("text-negative");
    fireEvent.blur(textArea());
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(textCalls(tauri)).toEqual([]);
    expect(textArea().value).toHaveLength(4001);

    fireEvent.change(textArea(), { target: { value: "a".repeat(4000) } });
    expect(screen.getByText("4,000 / 4,000").className).not.toContain("text-negative");
    fireEvent.blur(textArea());
    await waitFor(() => expect(textCalls(tauri)).toHaveLength(1));
  });

  test("the count counts characters, not UTF-16 units", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "😀".repeat(4000) } });
    expect(screen.getByText("4,000 / 4,000")).toBeTruthy();
    fireEvent.blur(textArea());
    await waitFor(() => expect(textCalls(tauri)).toHaveLength(1));
  });

  test("a built-in prompt shows its title text read-only, with no count", async () => {
    backend();
    renderPrompts();
    await waitFor(() => expect(titleArea().value).toBe("Built-in title text"));
    expect(titleArea().readOnly).toBe(true);
    expect(screen.queryByText(/ \/ 1,000$/)).toBeNull();
  });

  test("typing in the title field saves the title text with the title command only", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    expect(titleArea().value).toBe("Sync title");
    expect(screen.getByText("10 / 1,000")).toBeTruthy();
    fireEvent.change(titleArea(), { target: { value: "Type: topics" } });
    await waitFor(() => expect(titleCalls(tauri)).toEqual([{ id: "c1", text: "Type: topics" }]), { timeout: 3000 });
    expect(textCalls(tauri)).toEqual([]);
    expect(titleArea().value).toBe("Type: topics");
    expect(textArea().value).toBe("Sync text");
  });

  test("an empty title field saves and shows no error", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(titleArea(), { target: { value: "" } });
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
    fireEvent.change(titleArea(), { target: { value: "a".repeat(1001) } });
    expect(screen.getByText("1,001 / 1,000").className).toContain("text-negative");
    fireEvent.blur(titleArea());
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(titleCalls(tauri)).toEqual([]);

    fireEvent.change(titleArea(), { target: { value: "a".repeat(1000) } });
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
    fireEvent.change(titleArea(), { target: { value: "New title text" } });
    fireEvent.blur(titleArea());
    expect(
      await screen.findByText("The title instructions could not be saved. The prompt file cannot be written: disk full"),
    ).toBeTruthy();
    expect(titleArea().value).toBe("New title text");

    fireEvent.change(textArea(), { target: { value: "New summary text" } });
    fireEvent.blur(textArea());
    await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "New summary text" }]));
    expect(screen.getByText(/^The title instructions could not be saved/)).toBeTruthy();
  });

  test("an edit in one field does not drop the waiting save of the other field", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(titleArea(), { target: { value: "New title text" } });
    fireEvent.change(textArea(), { target: { value: "New summary text" } });
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
    fireEvent.change(titleArea(), { target: { value: "a".repeat(1001) } });
    await user.click(row("Topic summary"));
    await user.click(row("Team sync"));
    await waitFor(() => expect(titleArea().value).toBe("Sync title"));
    expect(titleCalls(tauri)).toEqual([]);

    fireEvent.change(titleArea(), { target: { value: "Changed title" } });
    await user.click(row("Topic summary"));
    await waitFor(() => expect(titleCalls(tauri)).toEqual([{ id: "c1", text: "Changed title" }]), { timeout: 500 });
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
  fireEvent.change(textArea(), { target: { value: "Changed" } });
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
        { id: "topic-summary", name: "Topic summary", text: "Topic text", title_text: "Built-in title text", built_in: true },
        { id: "c1", name: "Team sync", text: state.text, title_text: state.title_text, built_in: false },
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
    within(screen.getByRole("radiogroup", { name: "Summary prompts" })).getByText("In use").closest(".settings-row")
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
    await waitFor(() => expect(textArea().value).toBe("Text from the other computer"));
    expect(screen.queryByText(/changed on another computer/)).toBeNull();
  });

  test("a change from another computer to a prompt with unsaved text keeps the text and offers Reload", async () => {
    const { tauri, remote } = sharedBackend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "My unsaved text" } });
    await remote({ text: "Text from the other computer" });
    expect(await screen.findByText("The prompt text changed on another computer.")).toBeTruthy();
    expect(textArea().value).toBe("My unsaved text");
    // The unsaved text does not replace the text of the other computer.
    await new Promise((r) => setTimeout(r, 1200));
    fireEvent.blur(textArea());
    await new Promise((r) => setTimeout(r, 20));
    expect(textCalls(tauri)).toEqual([]);
    await user.click(button("Reload"));
    expect(textArea().value).toBe("Text from the other computer");
    expect(screen.queryByText("The prompt text changed on another computer.")).toBeNull();
    expect(textCalls(tauri)).toEqual([]);
  });

  test("text typed after the change from another computer is not saved until Keep My Text", async () => {
    const { tauri, remote } = sharedBackend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "My unsaved text" } });
    await remote({ text: "Text from the other computer" });
    expect(await screen.findByText("The prompt text changed on another computer.")).toBeTruthy();
    fireEvent.change(textArea(), { target: { value: "My unsaved text and more" } });
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
    fireEvent.change(textArea(), { target: { value: "My unsaved text" } });
    await remote({ text: "Text from the other computer" });
    await user.click(await screen.findByRole("button", { name: "Keep My Text" }));
    await waitFor(() => expect(textCalls(tauri)).toEqual([{ id: "c1", text: "My unsaved text" }]));
    await waitFor(() => expect(screen.queryByText("The prompt text changed on another computer.")).toBeNull());
    expect(textArea().value).toBe("My unsaved text");
  });

  test("text typed after this computer's own save shows no notice", async () => {
    const { tauri } = sharedBackend();
    renderPrompts();
    await selectCustom(userEvent.setup());
    fireEvent.change(textArea(), { target: { value: "First" } });
    fireEvent.blur(textArea());
    await waitFor(() => expect(textCalls(tauri)).toHaveLength(1));
    fireEvent.change(textArea(), { target: { value: "First and more" } });
    await act(() => tauri.emit("settings-changed", { settings: baseSettings }));
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByText(/changed on another computer/)).toBeNull();
    expect(textArea().value).toBe("First and more");
  });

  test("a change from another computer to the title text shows the notice for the title field only", async () => {
    const { tauri, remote } = sharedBackend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(titleArea(), { target: { value: "My title text" } });
    await remote({ title_text: "Title from the other computer" });
    expect(await screen.findByText("The title instructions changed on another computer.")).toBeTruthy();
    expect(screen.queryByText("The prompt text changed on another computer.")).toBeNull();
    expect(titleArea().value).toBe("My title text");

    // An edit in the summary field still saves.
    fireEvent.change(textArea(), { target: { value: "New summary text" } });
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
    fireEvent.change(titleArea(), { target: { value: "My title text" } });
    await remote({ title_text: "Title from the other computer" });
    await user.click(await screen.findByRole("button", { name: "Reload title instructions" }));
    expect(titleArea().value).toBe("Title from the other computer");
    expect(screen.queryByText(/changed on another computer/)).toBeNull();
    expect(titleCalls(tauri)).toEqual([]);
  });

  test("a read-only state disables the prompt editor and the prompt in use", async () => {
    sharedBackend();
    mockWindows("settings");
    render(<SummaryPrompts onSettings={() => {}} readOnly="The storage folder is not available." />);
    await selectCustom(userEvent.setup());
    expect(screen.getByText("The storage folder is not available. Prompts can’t be changed now.")).toBeTruthy();
    expect(textArea().readOnly).toBe(true);
    expect(titleArea().readOnly).toBe(true);
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).disabled).toBe(true);
    expect((button("New Prompt") as HTMLButtonElement).disabled).toBe(true);
    expect((button("Use Team sync") as HTMLButtonElement).disabled).toBe(true);
    expect((button("Duplicate") as HTMLButtonElement).disabled).toBe(true);
    expect((button("Delete") as HTMLButtonElement).disabled).toBe(true);
  });
});
