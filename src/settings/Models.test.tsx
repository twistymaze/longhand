import { describe, expect, test } from "bun:test";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ModelState, ModelStatus, Settings } from "../bindings";
import { mockTauri, shortcutStatuses } from "../test/tauri";
import { SettingsWindow } from "./SettingsWindow";

const PARAKEET = "parakeet-unified-en-0.6b-q80";
const WHISPER = "whisper-large-v3-turbo-q5km";
const TDT = "parakeet-tdt-1.1b-q5km";
const SEGMENTATION = "pyannote-segmentation-3.0";
const EMBEDDING = "wespeaker-voxceleb-resnet34-lm";

type Entry = [id: string, name: string, streaming: boolean, languages: string[]];

const recognition: Entry[] = [
  [PARAKEET, "Parakeet Unified EN 0.6B", true, ["en"]],
  ["nemotron-speech-streaming-en-0.6b-q80", "Nemotron Speech Streaming EN 0.6B", true, ["en"]],
  ["nemotron-3.5-asr-streaming-0.6b-q80", "Nemotron 3.5 ASR Streaming 0.6B", true, Array(28).fill("xx")],
  ["multitalker-parakeet-streaming-0.6b-v1-q80", "Multitalker Parakeet Streaming 0.6B", true, ["en"]],
  ["parakeet-tdt-0.6b-v2-q80", "Parakeet TDT 0.6B v2", false, ["en"]],
  ["parakeet-tdt-0.6b-v3-q80", "Parakeet TDT 0.6B v3", false, Array(25).fill("xx")],
  [TDT, "Parakeet TDT 1.1B", false, ["en"]],
  [WHISPER, "Whisper Large v3 Turbo", false, Array(100).fill("xx")],
  ["moss-transcribe-diarize-q80", "MOSS Transcribe Diarize", false, ["en", "zh"]],
];

function status(
  id: string,
  name: string,
  role: ModelStatus["role"],
  state: ModelState,
  extra: Partial<ModelStatus> = {},
): ModelStatus {
  return {
    id,
    role,
    name,
    description: "",
    streaming: false,
    languages: [],
    recommended: false,
    size_bytes: 700_000_000,
    required: false,
    state,
    repository: `example/${id}`,
    revision: "0123456789abcdef",
    licence: { name: "MIT", url: "https://example.com", attribution: name },
    ...extra,
  };
}

const present: ModelState = { state: "present" };
const absent: ModelState = { state: "absent", partial_bytes: 0 };

/// The nine recognition models and the two speaker weights. `states` sets
/// the state of a model by id; every other model is absent.
function catalog(selected: string, states: Record<string, ModelState>): ModelStatus[] {
  return [
    ...recognition.map(([id, name, streaming, languages]) =>
      status(id, name, "recognition", states[id] ?? absent, {
        streaming,
        languages,
        recommended: id === PARAKEET,
        required: id === selected,
      }),
    ),
    status(SEGMENTATION, "pyannote segmentation 3.0 (ONNX)", "segmentation", states[SEGMENTATION] ?? present, {
      size_bytes: 6_000_000,
      required: true,
    }),
    status(EMBEDDING, "WeSpeaker ResNet34-LM (ONNX)", "embedding", states[EMBEDDING] ?? present, {
      size_bytes: 26_000_000,
      required: true,
    }),
  ];
}

function settings(asr_model: string, extra: Partial<Settings> = {}): Settings {
  return {
    settings_schema_version: 1,
    shortcut: "CmdOrCtrl+Shift+R",
    show_window_shortcut: null,
    destination_roots: [],
    current_destination: null,
    endpoint_base_url: "",
    endpoint_model: "",
    context_limit_tokens: null,
    default_speaker_count: null,
    asr_model,
    pending_asr_model: null,
    live_transcript: true,
    learned_context_limits: {},
    alert_tone_heard: true,
    summary_prompts: [],
    active_summary_prompt: "topic-summary",
    ...extra,
  };
}

/// Mock the backend with mutable state, render the Settings window and open
/// the Transcription pane.
async function open({
  selected = PARAKEET,
  states = { [PARAKEET]: present },
  extra = {},
  handlers = {},
}: {
  selected?: string;
  states?: Record<string, ModelState>;
  extra?: Partial<Settings>;
  handlers?: Record<string, (args: Record<string, unknown>) => unknown>;
} = {}) {
  const state = { settings: settings(selected, extra), states: { ...states } };
  const mock = mockTauri({
    get_settings: () => state.settings,
    readiness: () => ({
      microphone_authorized: true,
      destination: { kind: "unset" },
      armed: false,
      shortcut: { chord: "CmdOrCtrl+Shift+R", registered: true, error: null },
      missing_models: [],
      endpoint_configured: false,
    }),
    shortcut_status: () => shortcutStatuses,
    model_statuses: () => catalog(state.settings.asr_model, state.states),
    speaker_models_status: () => {
      const weights = [SEGMENTATION, EMBEDDING];
      const all = weights.every((id) => (state.states[id] ?? present).state === "present");
      return { ids: weights, size_bytes: 32_000_000, state: all ? present : absent };
    },
    select_asr_model: ({ modelId }) => {
      const id = modelId as string;
      const current = state.settings.asr_model;
      const picked = (state.states[id] ?? absent).state === "present";
      const kept = (state.states[current] ?? absent).state === "present";
      if (!picked) state.states[id] = { state: "downloading" };
      state.settings =
        picked || !kept || id === current
          ? { ...state.settings, asr_model: id, pending_asr_model: null }
          : { ...state.settings, pending_asr_model: id };
      return state.settings;
    },
    set_live_transcript: ({ enabled }) => {
      state.settings = { ...state.settings, live_transcript: enabled as boolean };
      return state.settings;
    },
    download_models: () => null,
    cancel_model_downloads: () => null,
    delete_model: ({ modelId }) => {
      state.states[modelId as string] = absent;
      return null;
    },
    ...handlers,
  });
  const user = userEvent.setup();
  render(<SettingsWindow />);
  await user.click(await screen.findByRole("tab", { name: /Transcription/ }));
  await screen.findByRole("radiogroup", { name: "Recognition model" });
  return { mock, user, state };
}

const row = (name: string) => screen.getByRole("radio", { name: new RegExp(name) }).closest("[data-model]") as HTMLElement;
const calls = (mock: { calls: { cmd: string; args: Record<string, unknown> }[] }, cmd: string) =>
  mock.calls.filter((c) => c.cmd === cmd).map((c) => c.args);

describe("Transcription pane", () => {
  test("lists the nine models and marks the selected and the recommended model", async () => {
    await open({ selected: WHISPER, states: { [WHISPER]: present } });
    const radios = within(screen.getByRole("radiogroup", { name: "Recognition model" })).getAllByRole("radio");
    expect(radios).toHaveLength(9);
    expect(radios.filter((r) => r.getAttribute("aria-checked") === "true").map((r) => r.textContent)).toEqual([
      expect.stringContaining("Whisper Large v3 Turbo"),
    ]);
    expect(within(row("Parakeet Unified EN")).getByText("Recommended")).toBeTruthy();
    expect(within(row("Parakeet Unified EN")).getByText("Live")).toBeTruthy();
    expect(within(row("Whisper Large v3 Turbo")).queryByText("Live")).toBeNull();
    expect(within(row("Parakeet Unified EN")).getByText(/English · 700 MB/)).toBeTruthy();
    expect(within(row("Whisper Large v3 Turbo")).getByText(/100 languages/)).toBeTruthy();
    expect(within(row("MOSS")).getByText(/English and Chinese/)).toBeTruthy();
  });

  test("a click on a model that is not downloaded selects it and shows its progress", async () => {
    const { mock, user } = await open({ selected: WHISPER, states: { [WHISPER]: present } });
    await user.click(screen.getByRole("radio", { name: /Parakeet TDT 1.1B/ }));
    expect(calls(mock, "select_asr_model")).toEqual([{ modelId: TDT }]);

    await act(() => mock.emit("model-download-event", { model_id: TDT, update: { kind: "progress", downloaded: 350, total: 700 } }));
    const bar = await within(row("Parakeet TDT 1.1B")).findByRole("progressbar");
    expect(bar.getAttribute("aria-valuenow")).toBe("50");
    // The selected model stays in use until the download is verified.
    expect(screen.getByRole("radio", { name: /Whisper/ }).getAttribute("aria-checked")).toBe("true");
  });

  test("a selected model that is not downloaded has Download", async () => {
    const { mock, user } = await open({ states: {} });
    await user.click(within(row("Parakeet Unified EN")).getByRole("button", { name: "Download" }));
    expect(calls(mock, "select_asr_model")).toEqual([{ modelId: PARAKEET }]);
    expect(await within(row("Parakeet Unified EN")).findByRole("progressbar")).toBeTruthy();
  });

  test("a model becomes selected when the backend says the settings changed", async () => {
    const { mock, user, state } = await open({ selected: WHISPER, states: { [WHISPER]: present } });
    await user.click(screen.getByRole("radio", { name: /Parakeet TDT 1.1B/ }));
    state.states[TDT] = present;
    state.settings = { ...state.settings, asr_model: TDT, pending_asr_model: null };
    await act(() => mock.emit("settings-changed", { settings: state.settings }));
    await waitFor(() =>
      expect(screen.getByRole("radio", { name: /Parakeet TDT 1.1B/ }).getAttribute("aria-checked")).toBe("true"),
    );
  });

  test("a downloaded model that is not selected has Delete, with a confirmation", async () => {
    const { mock, user } = await open({ states: { [PARAKEET]: present, [WHISPER]: present } });
    expect(within(row("Parakeet Unified EN")).queryByRole("button", { name: "Delete…" })).toBeNull();
    expect(within(row("Parakeet TDT 1.1B")).queryByRole("button", { name: "Delete…" })).toBeNull();

    await user.click(within(row("Whisper")).getByRole("button", { name: "Delete…" }));
    // Delete asks first, and a click on it does not pick the model.
    expect(calls(mock, "select_asr_model")).toEqual([]);
    expect(calls(mock, "delete_model")).toEqual([]);
    await user.click(within(row("Whisper")).getByRole("button", { name: "Cancel" }));
    expect(calls(mock, "delete_model")).toEqual([]);

    await user.click(within(row("Whisper")).getByRole("button", { name: "Delete…" }));
    await user.click(within(row("Whisper")).getByRole("button", { name: "Delete" }));
    expect(calls(mock, "delete_model")).toEqual([{ modelId: WHISPER }]);
    await waitFor(() => expect(within(row("Whisper")).queryByRole("button", { name: "Delete…" })).toBeNull());
  });

  test("a refused delete shows the reason", async () => {
    const { user } = await open({
      states: { [PARAKEET]: present, [WHISPER]: present },
      handlers: {
        delete_model: () => {
          throw "A note that isn't finished needs this model.";
        },
      },
    });
    await user.click(within(row("Whisper")).getByRole("button", { name: "Delete…" }));
    await user.click(within(row("Whisper")).getByRole("button", { name: "Delete" }));
    expect(await within(row("Whisper")).findByText("A note that isn't finished needs this model.")).toBeTruthy();
  });

  test("the Live transcript switch shows only for a streaming model", async () => {
    const { mock, user } = await open();
    const toggle = screen.getByRole("switch", { name: "Live transcript" });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    await user.click(toggle);
    expect(calls(mock, "set_live_transcript")).toEqual([{ enabled: false }]);
    await waitFor(() => expect(screen.getByRole("switch", { name: "Live transcript" }).getAttribute("aria-checked")).toBe("false"));
  });

  test("no Live transcript switch for a model that does not stream", async () => {
    await open({ selected: WHISPER, states: { [WHISPER]: present } });
    expect(screen.queryByRole("switch")).toBeNull();
  });

  test("a failed download of the pending pick says so, and a click tries again", async () => {
    const { mock, user, state } = await open({ selected: WHISPER, states: { [WHISPER]: present } });
    await user.click(screen.getByRole("radio", { name: /Parakeet TDT 1.1B/ }));
    await act(() => mock.emit("model-download-event", { model_id: TDT, update: { kind: "progress", downloaded: 100, total: 700 } }));
    // The backend clears the pending pick before it sends the event.
    state.states[TDT] = absent;
    state.settings = { ...state.settings, pending_asr_model: null };
    await act(() => mock.emit("model-download-event", { model_id: TDT, update: { kind: "failed", message: "network down" } }));

    expect(await within(row("Parakeet TDT 1.1B")).findByText("Couldn't download. Try again.")).toBeTruthy();
    await waitFor(() => expect(within(row("Parakeet TDT 1.1B")).queryByRole("progressbar")).toBeNull());
    expect(screen.getByRole("radio", { name: /Whisper/ }).getAttribute("aria-checked")).toBe("true");

    await user.click(screen.getByRole("radio", { name: /Parakeet TDT 1.1B/ }));
    expect(calls(mock, "select_asr_model")).toEqual([{ modelId: TDT }, { modelId: TDT }]);
    await waitFor(() => expect(within(row("Parakeet TDT 1.1B")).queryByText("Couldn't download. Try again.")).toBeNull());
  });

  test("one Other required models row downloads both speaker weights", async () => {
    const { mock, user } = await open({ states: { [PARAKEET]: present, [EMBEDDING]: absent } });
    const other = screen.getByText("Other required models").closest(".settings-row") as HTMLElement;
    await user.click(within(other).getByRole("button", { name: "Download" }));
    expect(calls(mock, "download_models")).toEqual([{ ids: [SEGMENTATION, EMBEDDING] }]);

    await act(() => mock.emit("model-download-event", { model_id: EMBEDDING, update: { kind: "progress", downloaded: 13_000_000, total: 26_000_000 } }));
    // The present weight counts in full: (6 + 13) of 32 MB.
    const bar = await within(other).findByRole("progressbar");
    expect(bar.getAttribute("aria-valuenow")).toBe("59");
  });

  test("the pane never names the individual speaker weights", async () => {
    await open({ states: { [PARAKEET]: present, [SEGMENTATION]: absent, [EMBEDDING]: absent } });
    const text = document.body.textContent ?? "";
    for (const word of ["Speaker segmentation", "Speaker embedding", "pyannote", "WeSpeaker", "session"]) {
      expect(text).not.toContain(word);
    }
  });
});
