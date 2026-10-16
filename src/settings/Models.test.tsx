import { describe, expect, test } from "bun:test";
import type { ComponentProps } from "react";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type {
  ExecutionSpec,
  DownloadUpdate,
  ModelStatus,
  RecognitionChoice,
  RecognitionConfig,
  RecognitionState,
  Settings,
} from "../bindings";
import { mockTauri, shortcutStatuses } from "../test/tauri";
import { Models } from "./Models";
import { recognitionDefaults } from "./RecognitionOptions";
import { SettingsWindow } from "./SettingsWindow";
import { useRecognitionDefaultsSave } from "./useAppSettings";
import { describeLanguages } from "./languages";
import catalog from "../../crates/models/catalog.json";

const stream = "parakeet-unified-en-0.6b-q80";
const offline = "whisper-large-v3-turbo-q5km";
const catalogModels = [
  offline,
  "parakeet-tdt-0.6b-v3-q80",
  "nemotron-3.5-asr-streaming-0.6b-q80",
];
const model = (id: string, name: string): ModelStatus => ({
  id,
  name,
  role: "recognition",
  streaming: id === stream,
  languages: ["en"],
  description: "",
  assessment: null,
  speaker_preferences: {
    after_stop: ["nemotron", "pyannote"],
    during_recording: ["nemotron"],
  },
  recommended: false,
  size_bytes: 100,
  bundle: [id],
  bundle_size_bytes: 100,
  bundle_complete: true,
  required: false,
  state: { state: "present" },
  repository: "example",
  revision: "rev",
  licence: { name: "MIT", url: "https://example.com", attribution: "Example" },
});
const models = [model(stream, "Parakeet"), model(offline, "Whisper")];
const inUse: RecognitionChoice = {
  asr_model: stream,
  speaker_source: "nemotron",
  live_transcript: true,
  speaker_processing_mode: "after_stop",
  default_speaker_count: null,
  recognition_language: null,
};
const settled: RecognitionState = { differs: false, chosen_not_known: false, waiting_for: [], failure: null };
const settings = {
  ...inUse,
  chosen_recognition: inUse,
  recognition: settled,
  shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [], files_writable: true, damaged_file: null },
} as unknown as Settings;

/// The settings with `config` chosen and the configuration in use kept.
function choosing(config: RecognitionConfig, state: Partial<RecognitionState> = {}): Settings {
  return {
    ...settings,
    chosen_recognition: {
      asr_model: config.asr_model,
      speaker_source: config.speaker_source,
      live_transcript: config.processing_mode === "during_recording",
      speaker_processing_mode: config.speaker_processing_mode ?? "after_stop",
      default_speaker_count: config.speaker_count,
      recognition_language: config.language,
    },
    recognition: { ...settled, differs: true, ...state },
  };
}

function TestModels(props: Omit<ComponentProps<typeof Models>, "defaults">) {
  const defaults = useRecognitionDefaultsSave(async () => {});
  return <Models {...props} defaults={defaults} />;
}
function resolve(config: RecognitionConfig): ExecutionSpec {
  if (
    (config.asr_model === offline || config.asr_model === catalogModels[1]) &&
    config.processing_mode === "during_recording"
  )
    throw "Whisper requires After Stop";
  if (
    (config.speaker_source === "pyannote" || config.speaker_source === "off") &&
    config.speaker_processing_mode === "during_recording"
  )
    throw "No live speaker analysis";
  if (
    config.speaker_source === "sortformer" &&
    config.speaker_count?.kind === "exact" &&
    config.speaker_count.count > 4
  )
    throw "Four speakers maximum";
  if (
    config.speaker_source === "nemotron" &&
    config.speaker_count?.kind === "exact" &&
    config.speaker_count.count > 8
  )
    throw "Eight speakers maximum";
  return {
    config,
    schema_version: 1,
    catalog_revision: "test",
    artifacts: [
      {
        id: "a",
        filename: "exact-model.gguf",
        format: "gguf",
        revision: "rev",
        sha256: "hash",
        url: "https://example.com/a",
        size_bytes: 100,
      },
    ],
    runtimes: [],
    capabilities: {
      live_transcription: config.asr_model === stream,
      live_speaker_analysis:
        config.speaker_source === "sortformer" ||
        config.speaker_source === "nemotron",
      live_attributed_text: false,
      maximum_speakers: config.speaker_source === "sortformer" ? 4 : null,
      locally_verified: false,
    },
    enabled_live_transcription: config.processing_mode === "during_recording",
    enabled_live_speaker_analysis:
      config.speaker_processing_mode === "during_recording",
    preprocessing_revision: "test",
  };
}
function open(
  handlers: Record<string, (args: Record<string, unknown>) => unknown> = {},
  initial = settings,
  catalog = models,
  live: Record<string, DownloadUpdate> = {},
) {
  // A test that changes the values in use and keeps the base chosen
  // configuration saved those values: they are chosen too.
  if (initial.chosen_recognition === settings.chosen_recognition) {
    const { asr_model, speaker_source, live_transcript, speaker_processing_mode, default_speaker_count, recognition_language } = initial;
    initial = {
      ...initial,
      chosen_recognition: { asr_model, speaker_source, live_transcript, speaker_processing_mode, default_speaker_count, recognition_language },
    };
  }
  const tauri = mockTauri({
    resolve_recognition: ({ config }) => resolve(config as RecognitionConfig),
    set_recognition_defaults: () => initial,
    download_model_bundle: () => null,
    ...handlers,
  });
  const refreshes = { count: 0 };
  const view = render(
    <TestModels
      models={catalog}
      live={live}
      settings={initial}
      actions={{
        cancel: () => {},
        refresh: async () => {
          refreshes.count++;
        },
      }}
    />,
  );
  return { tauri, user: userEvent.setup(), rerender: view.rerender, refreshes };
}
/// The group that compares the chosen model with the model in use.
const status = () => screen.getByRole("heading", { name: "Model change" }).closest("section") as HTMLElement;
const inUseRow = () => within(status()).getByText("In use").closest(".settings-row") as HTMLElement;
async function ready() {
  await waitFor(() =>
    expect(
      (screen.getByLabelText("Transcription timing") as HTMLSelectElement)
        .disabled,
    ).toBe(false),
  );
}

describe("catalog speaker defaults", () => {
  for (const id of catalogModels) {
    const entry = catalog.models.find((item) => item.id === id)!;
    const selected: ModelStatus = {
      ...model(id, entry.name),
      speaker_preferences: entry.speaker_preferences as ModelStatus["speaker_preferences"],
    };
    test(`${id} selects Nemotron and offers no transcription model labels`, async () => {
      const { user, tauri } = open({}, settings, [models[0], selected]);
      await ready();
      await user.click(screen.getByRole("radio", { name: entry.name }));
      await ready();
      const saved = () => tauri.calls.filter((call) => call.cmd === "set_recognition_defaults");
      expect((saved().slice(-1)[0]?.args.config as RecognitionConfig).speaker_source).toBe("nemotron");
      await user.click(screen.getByText("Advanced options"));
      const override = screen.getByLabelText("Speaker model override") as HTMLSelectElement;
      expect([...override.options].map((option) => option.textContent)).not.toContain(
        "Transcription model labels",
      );
    });
    for (const source of ["pyannote", "sortformer", "nemotron", "off"] as const) {
      test(`${id} preserves saved ${source} choices`, async () => {
        const { user, tauri } = open({}, {
          ...settings,
          asr_model: id,
          speaker_source: source,
          live_transcript: false,
        }, [models[0], selected]);
        await ready();
        expect(tauri.calls.filter((call) => call.cmd === "set_recognition_defaults")).toHaveLength(0);
        await user.click(screen.getByText("Advanced options"));
        const override = screen.getByLabelText("Speaker model override") as HTMLSelectElement;
        expect(override.value).toBe(source === "off" || source === entry.speaker_preferences?.after_stop[0] ? "automatic" : source);
        if (source !== "off") {
          await user.selectOptions(screen.getByLabelText("Transcription timing"), "after_stop");
          const saved = tauri.calls.filter((call) => call.cmd === "set_recognition_defaults");
          expect((saved[saved.length - 1]?.args.config as RecognitionConfig).speaker_source).toBe(source);
        } else {
          expect((screen.getByLabelText("Speaker recognition") as HTMLSelectElement).value).toBe("off");
        }
      });
    }
  }
});

describe("recognition defaults", () => {
  test("recognition choices expose separate timing and keep speaker accuracy unknown", async () => {
    open();
    await ready();
    expect(screen.getByLabelText("Speaker recognition")).toBeTruthy();
    expect(screen.getByLabelText("Transcription timing")).toBeTruthy();
    expect(screen.queryByLabelText("Speaker timing")).toBeNull();
    expect(
      screen.getAllByLabelText("Transcription accuracy: Not rated"),
    ).toHaveLength(2);
    expect(screen.queryByLabelText("Runtime")).toBeNull();
    expect(describeLanguages(["en", "zh"])).toBe("English and Chinese");
  });
  test("model changes reset incompatible timing visibly and apply the complete configuration", async () => {
    const { user, tauri } = open();
    await ready();
    await user.click(screen.getByRole("radio", { name: "Whisper" }));
    await ready();
    expect(
      (screen.getByLabelText("Transcription timing") as HTMLSelectElement)
        .value,
    ).toBe("after_stop");
    expect(screen.getByText(/processing choices changed/)).toBeTruthy();

    const calls = tauri.calls.filter(
      (c) => c.cmd === "set_recognition_defaults",
    );
    expect(calls).toHaveLength(1);
    expect(calls[0].args.config).toEqual({
      asr_model: offline,
      speaker_source: "nemotron",
      processing_mode: "after_stop",
      speaker_processing_mode: "after_stop",
      speaker_count: null,
      language: null,
    });
    expect(
      tauri.calls.some(
        (c) => c.cmd === "select_asr_model" || c.cmd === "set_live_transcript",
      ),
    ).toBe(false);
  });
  test("offline text can have independent live speaker analysis", async () => {
    const { user } = open();
    await ready();
    await user.click(screen.getByRole("radio", { name: "Whisper" }));
    await ready();

    await user.selectOptions(
      screen.getByLabelText("Speaker recognition"),
      "during_recording",
    );
    expect(
      (screen.getByLabelText("Transcription timing") as HTMLSelectElement)
        .value,
    ).toBe("after_stop");
    expect(
      (screen.getByLabelText("Speaker recognition") as HTMLSelectElement).value,
    ).toBe("during_recording");
  });
  test("five expected speakers cannot select Sortformer", async () => {
    const { user } = open(
      {},
      { ...settings, default_speaker_count: { kind: "exact", count: 5 } },
    );
    await ready();
    await user.click(screen.getByText("Advanced options"));
    expect(
      (screen.getByRole("option", { name: "Sortformer" }) as HTMLOptionElement)
        .disabled,
    ).toBe(true);
  });
  test("an unavailable resolver blocks changes and explains recovery", async () => {
    open({
      resolve_recognition: () => {
        throw "catalog unavailable";
      },
    });
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Apply defaults" })).toBeNull();
  });
  test("a selected model with missing files downloads its bundle, and a failure shows (AE10)", async () => {
    const incomplete = [{ ...models[0], bundle_complete: false }, models[1]];
    const { user, tauri } = open(
      {
        download_model_bundle: () => {
          throw "Download unavailable";
        },
      },
      settings,
      incomplete,
    );
    await ready();
    const row = screen.getByRole("radio", { name: "Parakeet" }).parentElement!;
    expect(row.textContent).toContain("Some files missing");
    const download = Array.from(row.querySelectorAll("button")).find((b) => b.textContent === "Download")!;
    await user.click(download);
    expect(await screen.findByText(/Download unavailable/)).toBeTruthy();
    expect(tauri.calls.find((c) => c.cmd === "download_model_bundle")?.args.asrModel).toBe(stream);
  });
  test("a complete bundle has no Download button, and the tab lists no supporting models", async () => {
    open();
    await ready();
    expect(screen.queryByRole("button", { name: "Download" })).toBeNull();
    expect(screen.queryByText(/Supporting models|Delete models|Download required models/)).toBeNull();
  });
  test("late resolver responses cannot replace the current model choice", async () => {
    let release: () => void = () => {};
    const delayed = new Promise<void>((done) => {
      release = done;
    });
    const { user } = open({
      resolve_recognition: async ({ config }) => {
        const c = config as RecognitionConfig;
        if (c.asr_model === stream) await delayed;
        return resolve(c);
      },
    });
    await user.click(screen.getByRole("radio", { name: "Whisper" }));
    await ready();
    await act(async () => release());
    expect(
      screen.getByRole("radio", { name: "Whisper" }).getAttribute("aria-checked"),
    ).toBe("true");
  });
});

test("model comparison shows capabilities before selection and keeps engines advanced", async () => {
  open();
  await ready();
  expect(screen.getByRole("radio", { name: /Parakeet/ })).toBeTruthy();
  expect(screen.getByRole("radio", { name: /Whisper/ })).toBeTruthy();
  expect(screen.getByText("Live transcription")).toBeTruthy();
  expect(screen.getByText("Transcription afterwards")).toBeTruthy();
  expect(screen.getByLabelText("Speaker recognition").tagName).toBe("SELECT");
  expect(screen.getByRole("option", { name: "Off" })).toBeTruthy();
  expect(screen.queryByRole("option", { name: "Nemotron 3" })).toBeNull();
});

test("speaker mode uses catalog preferences and Off adds no speaker model", async () => {
  const { user, tauri } = open();
  await ready();
  await user.selectOptions(
    screen.getByLabelText("Transcription timing"),
    "after_stop",
  );
  expect(
    (
      tauri.calls.find((c) => c.cmd === "set_recognition_defaults")?.args
        .config as RecognitionConfig
    ).speaker_source,
  ).toBe("nemotron");
  await user.selectOptions(screen.getByLabelText("Speaker recognition"), "off");
  expect(
    (
      tauri.calls
        .filter((c) => c.cmd === "set_recognition_defaults")
        .slice(-1)[0]?.args.config as RecognitionConfig
    ).speaker_source,
  ).toBe("off");
});

test("nine speakers keeps recognition enabled with a compatible after-Stop choice", async () => {
  const { user, tauri } = open(
    {},
    {
      ...settings,
      default_speaker_count: { kind: "exact", count: 9 },
      speaker_processing_mode: "during_recording",
    },
  );
  await ready();
  const mode = screen.getByLabelText(
    "Speaker recognition",
  ) as HTMLSelectElement;
  expect(mode.value).toBe("after_stop");
  expect(
    Array.from(mode.options).find((o) => o.value === "during_recording")
      ?.disabled,
  ).toBe(true);
  await user.selectOptions(
    screen.getByLabelText("Transcription timing"),
    "after_stop",
  );
  expect(
    (
      tauri.calls.find((c) => c.cmd === "set_recognition_defaults")?.args
        .config as RecognitionConfig
    ).speaker_source,
  ).toBe("pyannote");
});

test("opening settings does not save and changes save without an Apply step", async () => {
  const { user, tauri } = open();
  await ready();
  expect(
    tauri.calls.filter((c) => c.cmd === "set_recognition_defaults"),
  ).toHaveLength(0);
  expect(screen.queryByRole("button", { name: "Apply defaults" })).toBeNull();
  await user.selectOptions(screen.getByLabelText("Speaker recognition"), "off");
  await waitFor(() =>
    expect(
      tauri.calls.filter((c) => c.cmd === "set_recognition_defaults"),
    ).toHaveLength(1),
  );
});

test("rapid setting changes save in order and keep the latest choice", async () => {
  let release!: (value: Settings) => void;
  let calls = 0;
  const { user, tauri } = open({
    set_recognition_defaults: () =>
      ++calls === 1
        ? new Promise<Settings>((done) => {
            release = done;
          })
        : settings,
  });
  await ready();
  await user.selectOptions(screen.getByLabelText("Speaker recognition"), "off");
  await user.selectOptions(
    screen.getByLabelText("Speaker recognition"),
    "during_recording",
  );
  expect(calls).toBe(1);
  await act(async () => release(settings));
  await waitFor(() => expect(calls).toBe(2));
  const saved = tauri.calls.filter((c) => c.cmd === "set_recognition_defaults");
  expect(
    (saved[1].args.config as RecognitionConfig).speaker_processing_mode,
  ).toBe("during_recording");
});

test("installed artifacts show Models ready instead of a download size", async () => {
  open({
    resolve_recognition: ({ config }) => {
      const spec = resolve(config as RecognitionConfig);
      return { ...spec, artifacts: [{ ...spec.artifacts[0], id: stream }] };
    },
  });
  await ready();
  expect(screen.getByText("Models ready")).toBeTruthy();
  expect(screen.queryByText(/Required download size/)).toBeNull();
});

test("a failed download of the chosen model shows its cause and Retry", async () => {
  const failed = choosing(
    { ...recognitionDefaults(settings), asr_model: offline, processing_mode: "after_stop" },
    { waiting_for: [offline], failure: { model_id: offline, message: "the checksum does not match" } },
  );
  const { user, tauri, refreshes } = open({ retry_recognition_downloads: () => settings }, failed);
  await ready();
  const group = status();
  expect(group.textContent).toContain("Whisper could not download");
  expect(group.textContent).toContain("The checksum does not match.");
  expect(inUseRow().textContent).toContain("Parakeet");
  expect(group.querySelector('[role="progressbar"]')).toBeNull();
  await user.click(within(group).getByRole("button", { name: "Retry" }));
  expect(tauri.calls.some((c) => c.cmd === "retry_recognition_downloads")).toBe(true);
  await waitFor(() => expect(refreshes.count).toBeGreaterThan(0));
});

test("a chosen configuration from a newer version keeps the model in use", async () => {
  open({}, { ...settings, recognition: { ...settled, differs: true, chosen_not_known: true } });
  await ready();
  expect(status().textContent).toContain("Chosen in a newer version of Longhand");
  expect(inUseRow().textContent).toContain("Parakeet");
  expect(status().querySelector('[role="progressbar"]')).toBeNull();
});

test("a chosen model that differs from the model in use shows both and the progress", async () => {
  const waiting = choosing(
    { ...recognitionDefaults(settings), asr_model: offline, processing_mode: "after_stop" },
    { waiting_for: [offline, "segmentation"] },
  );
  open({}, waiting, models, {
    [offline]: { kind: "progress", downloaded: 30_000_000, total: 100_000_000 },
    segmentation: { kind: "progress", downloaded: 10_000_000, total: 100_000_000 },
  });
  await ready();
  const chosenRow = within(status()).getByText("Whisper").closest(".settings-row")!;
  expect(chosenRow.textContent).toContain("Downloading… 40 MB of 200 MB");
  expect(chosenRow.querySelector('[role="progressbar"]')?.getAttribute("aria-valuenow")).toBe("20");
  expect(inUseRow().textContent).toContain("Parakeet");
  expect(inUseRow().textContent).toContain("New notes use this model until the download is done.");
});

test("a chosen model that waits with no progress yet says so", async () => {
  const waiting = choosing(
    { ...recognitionDefaults(settings), asr_model: offline, processing_mode: "after_stop" },
    { waiting_for: [offline] },
  );
  open({}, waiting);
  await ready();
  expect(within(status()).getByText("Whisper").closest(".settings-row")!.textContent).toContain(
    "Waiting to download…",
  );
});

test("a choice from another computer shows in the model picker", async () => {
  const { rerender } = open();
  await ready();
  expect(screen.getByRole("radio", { name: "Parakeet" }).getAttribute("aria-checked")).toBe("true");
  const remote = choosing(
    { ...recognitionDefaults(settings), asr_model: offline, processing_mode: "after_stop" },
    { waiting_for: [offline] },
  );
  rerender(<TestModels models={models} live={{}} settings={remote} actions={{ cancel: () => {}, refresh: async () => {} }} />);
  await waitFor(() => expect(screen.getByRole("radio", { name: "Whisper" }).getAttribute("aria-checked")).toBe("true"));
});

test("this computer's own choice does not read the choices again", async () => {
  const saved = choosing({ ...recognitionDefaults(settings), asr_model: offline, processing_mode: "after_stop" });
  const { user, rerender, tauri } = open({ set_recognition_defaults: () => saved });
  await ready();
  await user.click(screen.getByRole("radio", { name: "Whisper" }));
  await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "set_recognition_defaults")).toBe(true));
  await ready();
  const sent = tauri.calls.find((c) => c.cmd === "set_recognition_defaults")!.args.config as RecognitionConfig;
  const resolves = tauri.calls.filter((c) => c.cmd === "resolve_recognition").length;
  rerender(
    <TestModels models={models} live={{}} settings={choosing(sent)} actions={{ cancel: () => {}, refresh: async () => {} }} />,
  );
  await new Promise((r) => setTimeout(r, 20));
  expect(tauri.calls.filter((c) => c.cmd === "resolve_recognition").length).toBe(resolves);
  expect(screen.getByRole("radio", { name: "Whisper" }).getAttribute("aria-checked")).toBe("true");
});

test("a choice from another computer of a configuration that this pane chose before shows in the picker", async () => {
  const { user, rerender, tauri } = open();
  await ready();
  const sent = () =>
    tauri.calls.filter((c) => c.cmd === "set_recognition_defaults").map((c) => c.args.config as RecognitionConfig);
  const show = (s: Settings) =>
    rerender(<TestModels models={models} live={{}} settings={s} actions={{ cancel: () => {}, refresh: async () => {} }} />);
  // This pane chooses Whisper, then Parakeet again, and the settings show each choice.
  await user.click(screen.getByRole("radio", { name: "Whisper" }));
  await waitFor(() => expect(sent()).toHaveLength(1));
  show(choosing(sent()[0]));
  await ready();
  await user.click(screen.getByRole("radio", { name: "Parakeet" }));
  await waitFor(() => expect(sent()).toHaveLength(2));
  show(choosing(sent()[1]));
  await ready();
  // Another computer then chooses the Whisper configuration.
  show(choosing(sent()[0]));
  await waitFor(() => expect(screen.getByRole("radio", { name: "Whisper" }).getAttribute("aria-checked")).toBe("true"));
});

test("a settled configuration shows no download status", async () => {
  open();
  await ready();
  expect(screen.queryByRole("heading", { name: "Model change" })).toBeNull();
});

test("a read-only state makes the recognition choices read-only", async () => {
  const readOnly = {
    ...settings,
    shared: { ...settings.shared, availability: { kind: "read_only", cause: "folder_not_available" } },
  } as Settings;
  open({}, readOnly);
  await waitFor(() => expect(screen.queryByText("Checking compatible choices…")).toBeNull());
  expect(screen.getByText("The storage folder is not available. Transcription choices can’t be changed now.")).toBeTruthy();
  expect((screen.getByRole("radio", { name: "Whisper" }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByLabelText("Transcription timing") as HTMLSelectElement).disabled).toBe(true);
  expect((screen.getByLabelText("Speaker recognition") as HTMLSelectElement).disabled).toBe(true);
  await userEvent.setup().click(screen.getByText("Advanced options"));
  expect((screen.getByLabelText("Expected speakers") as HTMLSelectElement).disabled).toBe(true);
  expect((screen.getByLabelText("Language") as HTMLSelectElement).disabled).toBe(true);
});

test("completed model downloads clear the pending defaults notice", async () => {
  const pending: RecognitionConfig = {
    asr_model: offline,
    speaker_source: "nemotron",
    processing_mode: "after_stop",
    speaker_processing_mode: "after_stop",
    speaker_count: null,
    language: null,
  };
  const pendingSettings = choosing(pending, { waiting_for: [offline] });
  const downloading: ModelStatus[] = models.map((m) =>
    m.id === offline ? { ...m, state: { state: "downloading" } } : m,
  );
  const { user, rerender } = open(
    {
      set_recognition_defaults: () => pendingSettings,
      resolve_recognition: ({ config }) => {
        const spec = resolve(config as RecognitionConfig);
        return {
          ...spec,
          artifacts: [{ ...spec.artifacts[0], id: spec.config.asr_model }],
        };
      },
    },
    settings,
    downloading,
  );
  await ready();
  await user.click(screen.getByRole("radio", { name: "Whisper" }));
  await ready();
  await waitFor(() => expect(screen.queryByText("Saving…")).toBeNull());

  const update = (next: Settings, catalog: ModelStatus[]) => rerender(
    <TestModels
      models={catalog}
      live={{}}
      settings={next}
      actions={{ cancel: () => {}, refresh: async () => {} }}
    />,
  );
  update(pendingSettings, downloading);
  expect(status().textContent).toContain("Whisper");
  expect(screen.getByRole("radio", { name: "Whisper" }).parentElement!.textContent).toContain("Downloading…");

  update({
    ...pendingSettings,
    ...pendingSettings.chosen_recognition,
    recognition: settled,
  }, models);
  expect(screen.getByText("Models ready")).toBeTruthy();
  expect(screen.queryByRole("heading", { name: "Model change" })).toBeNull();
  expect(screen.queryByText("Downloading required models…")).toBeNull();
  expect(screen.queryByText(/current defaults stay/i)).toBeNull();
});

test("download size excludes present files and retained partial bytes", async () => {
  const partial = {
    ...model("extra", "Additional weights"),
    role: "recognition_artifact" as const,
    state: { state: "absent" as const, partial_bytes: 25_000_000 },
  };
  open(
    {
      resolve_recognition: ({ config }) => {
        const spec = resolve(config as RecognitionConfig);
        return {
          ...spec,
          artifacts: [
            { ...spec.artifacts[0], id: stream },
            { ...spec.artifacts[0], id: "extra", size_bytes: 100_000_000 },
          ],
        };
      },
    },
    settings,
    [...models, partial],
  );
  await ready();
  expect(screen.getByText("Download needed: 75 MB")).toBeTruthy();
});

test("recommended models appear first with one plain recommendation badge", async () => {
  open({}, settings, [models[0], { ...models[1], recommended: true }]);
  await ready();
  expect(screen.getAllByRole("radio")[0].getAttribute("aria-label")).toBe(
    "Whisper",
  );
  expect(screen.getByText("Recommended")).toBeTruthy();
  expect(
    screen.queryByText(/Recommended afterwards|Recommended for live/),
  ).toBeNull();
});

test("models sort alphabetically within the recommended and other groups", async () => {
  open({}, settings, [
    model("z", "Zebra"),
    { ...model("b", "Beta"), recommended: true },
    model("a", "Alpha"),
    { ...model("c", "Aster"), recommended: true },
  ]);
  await waitFor(() => expect(screen.getAllByRole("radio")).toHaveLength(4));
  expect(
    screen.getAllByRole("radio").map((row) => row.getAttribute("aria-label")),
  ).toEqual(["Aster", "Beta", "Alpha", "Zebra"]);
});

test("deletes a transcription model from its existing row without selecting it", async () => {
  let removed: unknown;
  let saves = 0;
  const { user } = open({
    delete_model: ({ modelId }) => {
      removed = modelId;
    },
    set_recognition_defaults: () => {
      saves++;
      return settings;
    },
  });
  await ready();
  const row = screen.getByRole("radio", { name: "Whisper" }).parentElement!;
  const button = Array.from(row.querySelectorAll("button")).find(
    (b) => b.textContent === "Delete…",
  )!;
  await user.click(button);
  await user.click(screen.getByRole("button", { name: "Delete" }));
  await waitFor(() => expect(removed).toBe(offline));
  expect(saves).toBe(0);
  expect(screen.queryByText("Downloaded models")).toBeNull();
});

test("opening settings preserves a saved speaker model override", async () => {
  const { user, tauri } = open({}, { ...settings, speaker_source: "pyannote" });
  await ready();
  await user.click(screen.getByText("Advanced options"));
  expect(
    (screen.getByLabelText("Speaker model override") as HTMLSelectElement)
      .value,
  ).toBe("pyannote");
  expect(tauri.calls.some((c) => c.cmd === "set_recognition_defaults")).toBe(
    false,
  );
});


test("model download progress appears beside that model", async () => {
  open({}, settings, models, { [offline]: { kind: "progress", downloaded: 25_000_000, total: 100_000_000 } });
  await ready();
  const row = screen.getByRole("radio", { name: "Whisper" }).parentElement!;
  expect(row.textContent).toContain("Downloading… 25 MB of 100 MB");
  expect(row.querySelector('[role="progressbar"]')?.getAttribute("aria-valuenow")).toBe("25");
});

describe("a shared supporting file download", () => {
  const shared = "sortformer-v2-q80";
  const bundled = [
    { ...models[0], bundle: [stream, shared], bundle_size_bytes: 300_000_000 },
    { ...models[1], bundle: [offline, shared], bundle_complete: false },
  ];
  const progress = { [shared]: { kind: "progress", downloaded: 50_000_000, total: 150_000_000 } } as Record<
    string,
    DownloadUpdate
  >;
  const rowOf = (name: string) => screen.getByRole("radio", { name }).parentElement!;

  test("shows only on the row of the pending model, with Cancel", async () => {
    let cancels = 0;
    mockTauri({
      resolve_recognition: ({ config }) => resolve(config as RecognitionConfig),
      set_recognition_defaults: () => settings,
    });
    const pending = choosing({ ...recognitionDefaults(settings), asr_model: stream });
    render(
      <TestModels
        models={bundled}
        live={progress}
        settings={pending}
        actions={{ cancel: () => cancels++, refresh: async () => {} }}
      />,
    );
    await ready();
    expect(rowOf("Parakeet").textContent).toContain("300 MB");
    expect(rowOf("Parakeet").textContent).toContain("Downloading… 50 MB of 150 MB");
    expect(rowOf("Whisper").textContent).not.toContain("Downloading");
    const cancel = Array.from(rowOf("Parakeet").querySelectorAll("button")).find((b) => b.textContent === "Cancel")!;
    await userEvent.setup().click(cancel);
    expect(cancels).toBe(1);
  });

  test("shows only on the row whose Download button started it", async () => {
    const { user, rerender } = open({ download_model_bundle: () => null }, settings, bundled);
    await ready();
    const download = Array.from(rowOf("Whisper").querySelectorAll("button")).find((b) => b.textContent === "Download")!;
    await user.click(download);
    rerender(
      <TestModels models={bundled} live={progress} settings={settings} actions={{ cancel: () => {}, refresh: async () => {} }} />,
    );
    expect(rowOf("Whisper").textContent).toContain("Downloading… 50 MB of 150 MB");
    expect(rowOf("Parakeet").textContent).not.toContain("Downloading");
  });

  test("a second Download click keeps the progress on the first row", async () => {
    const incomplete = bundled.map((m) => ({ ...m, bundle_complete: false }));
    const { user, rerender } = open({ download_model_bundle: () => null }, settings, incomplete);
    await ready();
    for (const name of ["Whisper", "Parakeet"]) {
      const download = Array.from(rowOf(name).querySelectorAll("button")).find((b) => b.textContent === "Download")!;
      await user.click(download);
    }
    rerender(
      <TestModels models={incomplete} live={progress} settings={settings} actions={{ cancel: () => {}, refresh: async () => {} }} />,
    );
    expect(rowOf("Whisper").textContent).toContain("Downloading… 50 MB of 150 MB");
    expect(rowOf("Parakeet").textContent).toContain("Downloading… 50 MB of 150 MB");
  });

  test("a selection owns the rest of its bundle download", async () => {
    const { user, rerender } = open({}, settings, bundled);
    await ready();
    await user.click(screen.getByRole("radio", { name: "Whisper" }));
    rerender(
      <TestModels models={bundled} live={progress} settings={settings} actions={{ cancel: () => {}, refresh: async () => {} }} />,
    );
    await waitFor(() => expect(rowOf("Whisper").textContent).toContain("Downloading… 50 MB of 150 MB"));
    expect(rowOf("Parakeet").textContent).not.toContain("Downloading");
  });
});

test("choosing a model gives immediate feedback while its request is pending", async () => {
  let release: (s: Settings) => void = () => {};
  const { user } = open({set_recognition_defaults: () => new Promise<Settings>(resolve => { release = resolve; })});
  await ready();
  await user.click(screen.getByRole("radio", { name: "Whisper" }));
  await screen.findByText("Preparing model…");
  expect(screen.getByRole("radio", { name: "Whisper" }).parentElement!.textContent).toContain("Preparing model…");
  await act(async () => release({...settings, asr_model: offline}));
  await waitFor(() => expect(screen.queryByText("Preparing model…")).toBeNull());
});

test("transcription saves remain ordered after switching panes", async () => {
  let release!: (value: Settings) => void;
  let saved = settings;
  let saves = 0;
  const tauri = mockTauri({
    get_settings: () => saved,
    readiness: () => ({
      microphone_authorized: true,
      system_audio: "allowed",
      destination: { kind: "ready", path: "/notes", free_bytes: null },
      armed: false,
      shortcut: shortcutStatuses.recording,
      missing_models: [],
      endpoint_configured: false, can_start_note: true, note_start_refusal: null,
    }),
    shortcut_status: () => shortcutStatuses,
    login_item_status: () => "disabled",
    model_statuses: () => models,
    resolve_recognition: ({ config }) => resolve(config as RecognitionConfig),
    set_recognition_defaults: ({ config }) => {
      // The models are on disk, so the choice is in use at once.
      const chosen = choosing(config as RecognitionConfig).chosen_recognition;
      saved = { ...saved, ...chosen, chosen_recognition: chosen, recognition: settled };
      return ++saves === 1
        ? new Promise<Settings>((done) => { release = done; })
        : saved;
    },
  });
  const user = userEvent.setup();
  render(<SettingsWindow />);
  await user.click(await screen.findByRole("tab", { name: "Transcription" }));
  await ready();
  await user.selectOptions(screen.getByLabelText("Speaker recognition"), "off");
  await waitFor(() => expect(saves).toBe(1));
  const first = saved;
  await user.click(screen.getByRole("tab", { name: /General/ }));
  await user.click(screen.getByRole("tab", { name: "Transcription" }));
  await ready();
  await user.selectOptions(screen.getByLabelText("Speaker recognition"), "during_recording");
  // A new pane must use the existing queue while its first command is pending.
  expect(saves).toBe(1);
  await act(async () => release(first));
  await waitFor(() => expect(saves).toBe(2));
  const calls = tauri.calls.filter((call) => call.cmd === "set_recognition_defaults");
  expect((calls[1].args.config as RecognitionConfig).speaker_processing_mode).toBe("during_recording");
  await waitFor(() => expect(screen.queryByText("Saving…")).toBeNull());
  expect(saved.speaker_source).toBe("nemotron");
  expect(saved.speaker_processing_mode).toBe("during_recording");
  await user.click(screen.getByRole("tab", { name: /General/ }));
  await user.click(screen.getByRole("tab", { name: "Transcription" }));
  await ready();
  expect((screen.getByLabelText("Speaker recognition") as HTMLSelectElement).value).toBe("during_recording");
  expect(screen.getByRole("radio", { name: "Parakeet" }).getAttribute("aria-checked")).toBe("true");
});

for (const failure of ["result", "rejected"] as const) {
  test(`a ${failure} save failure is visible and the next choice saves`, async () => {
    let saves = 0;
    const { user, tauri } = open({
      set_recognition_defaults: () => {
        if (++saves === 1) {
          if (failure === "result") throw "The defaults could not be saved";
          throw new Error("IPC unavailable");
        }
        return settings;
      },
    });
    await ready();
    await user.selectOptions(screen.getByLabelText("Speaker recognition"), "off");
    const message = failure === "result"
      ? "The defaults could not be saved"
      : "Could not save the transcription defaults";
    expect(await screen.findByText(new RegExp(message))).toBeTruthy();
    expect(screen.queryByText("Saving…")).toBeNull();
    await user.selectOptions(screen.getByLabelText("Speaker recognition"), "during_recording");
    await waitFor(() => expect(saves).toBe(2));
    await waitFor(() => expect(screen.queryByText(new RegExp(message))).toBeNull());
    const calls = tauri.calls.filter((call) => call.cmd === "set_recognition_defaults");
    expect((calls[1].args.config as RecognitionConfig).speaker_processing_mode).toBe("during_recording");
  });
}

test("model keys move focus, wrap, and save the selected model", async () => {
  const { user, tauri } = open({}, settings, [...models, model("zebra", "Zebra")]);
  await ready();
  screen.getByRole("radio", { name: "Parakeet" }).focus();
  for (const [key, name, id] of [
    ["ArrowDown", "Whisper", offline],
    ["ArrowDown", "Zebra", "zebra"],
    ["ArrowDown", "Parakeet", stream],
    ["ArrowUp", "Zebra", "zebra"],
    ["Home", "Parakeet", stream],
    ["End", "Zebra", "zebra"],
  ]) {
    const before = tauri.calls.filter((call) => call.cmd === "set_recognition_defaults").length;
    await user.keyboard(`{${key}}`);
    const selected = screen.getByRole("radio", { name });
    expect(document.activeElement).toBe(selected);
    expect(selected.getAttribute("aria-checked")).toBe("true");
    expect(selected.tabIndex).toBe(0);
    await waitFor(() => {
      const saves = tauri.calls.filter((call) => call.cmd === "set_recognition_defaults");
      expect(saves).toHaveLength(before + 1);
      expect((saves[saves.length - 1].args.config as RecognitionConfig).asr_model).toBe(id);
    });
  }
});
