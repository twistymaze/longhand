import { describe, expect, test } from "bun:test";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockWindows } from "@tauri-apps/api/mocks";
import type { Readiness, Settings, SummaryPrompt } from "../bindings";
import { type CommandHandler, mockTauri } from "../test/tauri";
import { SettingsWindow } from "./SettingsWindow";
import { SummaryPrompts } from "./SummaryPrompts";

const baseSettings: Settings = {
  settings_schema_version: 3,
  shortcut: "Alt+Shift+L",
  destination_roots: [],
  current_destination: null,
  endpoint_base_url: "",
  endpoint_model: "",
  context_limit_tokens: null,
  default_speaker_count: null,
  asr_model: "a",
  learned_context_limits: {},
  alert_tone_heard: true,
  summary_prompts: [],
  active_summary_prompt: "topic-summary",
};

const readiness: Readiness = {
  microphone_authorized: true,
  destination: { kind: "ready", path: "/x", free_bytes: null },
  armed: true,
  shortcut: { chord: "Alt+Shift+L", registered: true, error: null },
  missing_models: [],
  endpoint_configured: true,
};

/// A fake of the prompt commands that keeps its own list, as the backend does.
function backend(over: Record<string, CommandHandler> = {}) {
  const builtIns = [
    { id: "topic-summary", name: "Topic summary", text: "Topic text", built_in: true },
    { id: "play-by-play", name: "Play-by-play summary", text: "Play text", built_in: true },
  ];
  let custom: SummaryPrompt[] = [{ id: "c1", name: "Game recap", text: "Recap text" }];
  let active = "topic-summary";
  let next = 2;
  const settings = (): Settings => ({
    ...baseSettings,
    summary_prompts: custom.map((p) => ({ ...p })),
    active_summary_prompt: active,
  });
  const find = (id: unknown) => [...builtIns, ...custom].find((p) => p.id === id)!;
  return mockTauri({
    summary_prompts: () => ({
      prompts: [...builtIns, ...custom.map((p) => ({ ...p, built_in: false }))],
      active,
      text_limit: 4000,
    }),
    create_summary_prompt: () => {
      custom.push({ id: `c${next++}`, name: "New prompt", text: "Topic text" });
      return settings();
    },
    duplicate_summary_prompt: ({ id }) => {
      const from = find(id);
      custom.push({ id: `c${next++}`, name: `${from.name} copy`, text: from.text });
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

function renderPrompts() {
  mockWindows("settings");
  const onSettings: Settings[] = [];
  render(<SummaryPrompts onSettings={(s) => onSettings.push(s)} />);
  return onSettings;
}

const row = (name: string) => screen.getByRole("radio", { name: new RegExp(`^${name}`) });
const textArea = () => screen.getByRole("textbox", { name: "Prompt text" }) as HTMLTextAreaElement;
const button = (name: string | RegExp) => screen.getByRole("button", { name });

async function selectCustom(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("radio", { name: /^Game recap/ }));
  await waitFor(() => expect(textArea().value).toBe("Recap text"));
}

describe("SummaryPrompts", () => {
  test("the list shows the built-in prompts first and marks the one in use", async () => {
    backend();
    renderPrompts();
    const radios = await screen.findAllByRole("radio");
    expect(radios.map((r) => r.textContent)).toEqual([
      expect.stringContaining("Topic summary"),
      expect.stringContaining("Play-by-play summary"),
      expect.stringContaining("Game recap"),
    ]);
    const group = screen.getByRole("radiogroup", { name: "Summary prompts" });
    expect(within(group).getAllByText("In use")).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Use Topic summary" })).toBeNull();
    expect(button("Use Play-by-play summary")).toBeTruthy();
    expect(button("Use Game recap")).toBeTruthy();
  });

  test("a built-in prompt shows read-only text and Duplicate, with no Delete or Save", async () => {
    backend();
    renderPrompts();
    await waitFor(() => expect(textArea().value).toBe("Topic text"));
    expect(row("Topic summary").getAttribute("aria-checked")).toBe("true");
    expect(textArea().readOnly).toBe(true);
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).disabled).toBe(true);
    expect(screen.getByText(/Built-in prompts can’t be changed/)).toBeTruthy();
    expect(button("Duplicate")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
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

  test("Save is disabled while the text is empty, and Save calls set text", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    expect((button("Save") as HTMLButtonElement).disabled).toBe(true);
    expect((button("Revert") as HTMLButtonElement).disabled).toBe(true);

    fireEvent.change(textArea(), { target: { value: "   " } });
    expect((button("Save") as HTMLButtonElement).disabled).toBe(true);
    expect((button("Revert") as HTMLButtonElement).disabled).toBe(false);

    fireEvent.change(textArea(), { target: { value: "Short recap" } });
    await user.click(button("Save"));
    await waitFor(() => expect((button("Save") as HTMLButtonElement).disabled).toBe(true));
    expect(tauri.calls.find((c) => c.cmd === "set_summary_prompt_text")?.args).toEqual({
      id: "c1",
      text: "Short recap",
    });
    expect(textArea().value).toBe("Short recap");
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
    fireEvent.change(textArea(), { target: { value: "Short recap" } });
    await user.click(button("Save"));
    expect(await screen.findByText("The prompt text cannot be saved.")).toBeTruthy();
    expect(textArea().value).toBe("Short recap");
  });

  test("text typed while Save runs stays in the editor and stays unsaved", async () => {
    // The save command waits until the test calls `finish`.
    let finish: () => void = () => {};
    const saved = new Promise<void>((r) => (finish = r));
    let text = "Recap text";
    backend({
      set_summary_prompt_text: async (args) => {
        await saved;
        text = args.text as string;
        return { ...baseSettings, summary_prompts: [{ id: "c1", name: "Game recap", text }] };
      },
      summary_prompts: () => ({
        prompts: [
          { id: "topic-summary", name: "Topic summary", text: "Topic text", built_in: true },
          { id: "play-by-play", name: "Play-by-play summary", text: "Play text", built_in: true },
          { id: "c1", name: "Game recap", text, built_in: false },
        ],
        active: "topic-summary",
        text_limit: 4000,
      }),
    });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "Short recap" } });
    await user.click(button("Save"));
    fireEvent.change(textArea(), { target: { value: "Short recap, longer" } });
    await act(async () => finish());
    await waitFor(() => expect(text).toBe("Short recap"));
    await act(() => new Promise((r) => setTimeout(r, 10)));
    expect(textArea().value).toBe("Short recap, longer");
    expect((button("Save") as HTMLButtonElement).disabled).toBe(false);
    expect((button("Revert") as HTMLButtonElement).disabled).toBe(false);
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

  test("Revert puts back the saved text", async () => {
    backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "Changed" } });
    await user.click(button("Revert"));
    expect(textArea().value).toBe("Recap text");
  });

  test("selecting another prompt with unsaved text asks first, and Cancel keeps the edit", async () => {
    backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "Changed" } });

    await user.click(row("Topic summary"));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText("Discard your changes to “Game recap”?")).toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(row("Game recap").getAttribute("aria-checked")).toBe("true");
    expect(textArea().value).toBe("Changed");

    await user.click(row("Topic summary"));
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(textArea().value).toBe("Topic text"));
  });

  test("rows select with Enter and Space from the keyboard", async () => {
    backend();
    renderPrompts();
    const user = userEvent.setup();
    await waitFor(() => expect(textArea().value).toBe("Topic text"));
    row("Play-by-play summary").focus();
    await user.keyboard("{Enter}");
    await waitFor(() => expect(textArea().value).toBe("Play text"));
    row("Game recap").focus();
    await user.keyboard(" ");
    await waitFor(() => expect(textArea().value).toBe("Recap text"));
  });

  test("Delete asks for confirmation, then calls delete", async () => {
    const tauri = backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    await user.click(button("Delete"));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText("Delete “Game recap”?")).toBeTruthy();
    expect(within(dialog).getByText("This can’t be undone.")).toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(tauri.calls.some((c) => c.cmd === "delete_summary_prompt")).toBe(false);

    await user.click(button("Delete"));
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(screen.queryByRole("radio", { name: /^Game recap/ })).toBeNull());
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

    await user.click(button("Use Game recap"));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Use Game recap" })).toBeNull());
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
    await user.click(await screen.findByRole("button", { name: "Use Game recap" }));
    expect(await screen.findByText("The settings could not be saved.")).toBeTruthy();
  });

  test("text of 4,001 characters shows the count in the error colour and disables Save", async () => {
    backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "a".repeat(4000) } });
    expect(screen.getByText("4,000 / 4,000").className).not.toContain("text-negative");
    expect((button("Save") as HTMLButtonElement).disabled).toBe(false);
    fireEvent.change(textArea(), { target: { value: "a".repeat(4001) } });
    expect(screen.getByText("4,001 / 4,000").className).toContain("text-negative");
    expect((button("Save") as HTMLButtonElement).disabled).toBe(true);
  });

  test("the count counts characters, not UTF-16 units", async () => {
    backend();
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "😀".repeat(4000) } });
    expect(screen.getByText("4,000 / 4,000")).toBeTruthy();
    expect((button("Save") as HTMLButtonElement).disabled).toBe(false);
  });

  test("closing the Settings window with unsaved text asks first", async () => {
    const tauri = backend({ "plugin:window|destroy": () => null });
    renderPrompts();
    const user = userEvent.setup();
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "Changed" } });
    // The mock handles `plugin:event|listen` itself, so the call is not in
    // `calls`. Let the listener register.
    await act(() => new Promise((r) => setTimeout(r, 10)));

    await tauri.emit("tauri://close-requested");
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText("Discard your changes to “Game recap”?")).toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(tauri.calls.some((c) => c.cmd === "plugin:window|destroy")).toBe(false);
    expect(textArea().value).toBe("Changed");

    await tauri.emit("tauri://close-requested");
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "plugin:window|destroy")).toBe(true));
  });
});

test("with no unsaved text the page does not hold the close", async () => {
  const tauri = backend({ "plugin:window|destroy": () => null });
  renderPrompts();
  await waitFor(() => expect(textArea().value).toBe("Topic text"));
  await act(() => new Promise((r) => setTimeout(r, 10)));
  await tauri.emit("tauri://close-requested");
  await act(() => new Promise((r) => setTimeout(r, 10)));
  expect(screen.queryByRole("alertdialog")).toBeNull();
  expect(tauri.calls.some((c) => c.cmd === "plugin:window|destroy")).toBe(false);
});

describe("SettingsWindow with unsaved prompt text", () => {
  async function openSummaries() {
    mockWindows("settings");
    backend({
      get_settings: () => baseSettings,
      readiness: () => readiness,
      model_statuses: () => [],
    });
    render(<SettingsWindow />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("tab", { name: "Summaries" }));
    await selectCustom(user);
    fireEvent.change(textArea(), { target: { value: "Changed" } });
    return user;
  }

  test("a pane change by click asks first", async () => {
    const user = await openSummaries();
    await user.click(screen.getByRole("tab", { name: "Transcription" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText("Discard your changes to “Game recap”?")).toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.getByRole("tab", { name: "Summaries" }).getAttribute("aria-selected")).toBe("true");
    expect(textArea().value).toBe("Changed");

    await user.click(screen.getByRole("tab", { name: "Transcription" }));
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Discard" }));
    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Transcription" }).getAttribute("aria-selected")).toBe("true"),
    );
  });

  test("a pane change by arrow key asks first", async () => {
    const user = await openSummaries();
    screen.getByRole("tab", { name: "Summaries" }).focus();
    await user.keyboard("{ArrowDown}");
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Cancel" }));
    expect(screen.getByRole("tab", { name: "Summaries" }).getAttribute("aria-selected")).toBe("true");

    screen.getByRole("tab", { name: "Summaries" }).focus();
    await user.keyboard("{ArrowUp}");
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Discard" }));
    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Storage" }).getAttribute("aria-selected")).toBe("true"),
    );
  });

  test("with no unsaved text a pane change does not ask", async () => {
    mockWindows("settings");
    backend({ get_settings: () => baseSettings, readiness: () => readiness, model_statuses: () => [] });
    render(<SettingsWindow />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("tab", { name: "Summaries" }));
    await waitFor(() => expect(textArea().value).toBe("Topic text"));
    await user.click(screen.getByRole("tab", { name: "Transcription" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(screen.getByRole("tab", { name: "Transcription" }).getAttribute("aria-selected")).toBe("true");
  });
});
