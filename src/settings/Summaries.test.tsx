import { describe, expect, test } from "bun:test";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockWindows } from "@tauri-apps/api/mocks";
import type { ProviderEntry, Settings } from "../bindings";
import { mockTauri } from "../test/tauri";
import { SummariesPane } from "./Summaries";

const empty: ProviderEntry = { address: null, model: "", context_limit_tokens: null };

const baseSettings: Settings = {
  settings_schema_version: 7,
  interface_zoom_percent: 100,
  shortcut: "Alt+Shift+L",
  show_window_shortcut: null,
  pause_shortcut: null,
  widget_shortcut: null,
  storage_folder: "/notes",
  active_provider: null,
  providers: {
    openai: empty,
    anthropic: empty,
    gemini: empty,
    openrouter: empty,
    ollama: empty,
    lm_studio: empty,
    custom: empty,
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
  system_audio_default: false,
  new_note_records: true,
  alert_sound: true,
  show_recording_widget: true,
  hide_from_dock: false,
  check_updates_automatically: true,
  audio_removal: { kind: "after_days", days: 30 },
  summary_prompts: [],
  active_summary_prompt: "topic-summary",
  shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [], files_writable: true, damaged_file: null },
  background: "",
  background_limit: 4000,
  background_read_only: null,
};

function renderPane(settings: Partial<Settings>) {
  mockTauri({
    summary_prompts: () => ({
      prompts: [{ id: "topic-summary", name: "Topic summary", text: "Topic text", built_in: true, over_limit: false, too_large: false }],
      active: "topic-summary",
      text_limit: 4000,
    }),
  });
  mockWindows("settings");
  const opened: string[] = [];
  render(
    <SummariesPane
      settings={{ ...baseSettings, ...settings }}
      onSettings={() => {}}
      onOpenProvider={() => opened.push("provider")}
    />,
  );
  return { opened };
}

describe("the Summaries pane", () => {
  test("with no provider, the line says No provider set and Set Up… opens the AI Provider pane", async () => {
    const { opened } = renderPane({});
    await screen.findByRole("radiogroup", { name: "Summary prompts" });
    expect(screen.getByText("No provider set")).toBeTruthy();
    await userEvent.setup().click(screen.getByRole("button", { name: "Set Up…" }));
    expect(opened).toEqual(["provider"]);
  });

  test("the line names the provider and the model", async () => {
    const { opened } = renderPane({
      active_provider: "anthropic",
      providers: { ...baseSettings.providers, anthropic: { ...empty, model: "claude-haiku-5-5" } },
    });
    await screen.findByRole("radiogroup", { name: "Summary prompts" });
    expect(screen.getByText("Anthropic · claude-haiku-5-5")).toBeTruthy();
    await userEvent.setup().click(screen.getByRole("button", { name: "Change…" }));
    expect(opened).toEqual(["provider"]);
  });

  test("a provider with no model says so", async () => {
    renderPane({ active_provider: "lm_studio" });
    await screen.findByRole("radiogroup", { name: "Summary prompts" });
    expect(screen.getByText("LM Studio · No model chosen")).toBeTruthy();
  });

  test("the pane has no connection rows, and the Background comes before the summary prompts", async () => {
    renderPane({ background: "I am Erv." });
    const prompts = await screen.findByRole("radiogroup", { name: "Summary prompts" });
    const background = screen.getByRole("textbox", { name: "Background" });
    expect(background.textContent).toBe("I am Erv.");
    expect(background.compareDocumentPosition(prompts) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.queryByLabelText("API key")).toBeNull();
    expect(screen.queryByLabelText("Model")).toBeNull();
    expect(screen.queryByLabelText("Context limit")).toBeNull();
  });
});
