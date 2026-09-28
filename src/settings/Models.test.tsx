import { describe, expect, test } from "bun:test";
import type { ComponentProps } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type {
  ExecutionSpec,
  DownloadUpdate,
  ModelDeletionSummary,
  ModelStatus,
  RecognitionConfig,
  Settings,
} from "../bindings";
import { mockTauri, shortcutStatuses } from "../test/tauri";
import { Models } from "./Models";
import { SettingsWindow } from "./SettingsWindow";
import { useRecognitionDefaultsSave } from "./useAppSettings";
import { describeLanguages } from "./languages";
import catalog from "../../crates/models/catalog.json";

const stream = "parakeet-unified-en-0.6b-q80";
const offline = "moss-transcribe-diarize-q80";
const integratedModels = [
  offline,
  "granite-speech-4.1-2b-plus-q5km",
  "multitalker-parakeet-streaming-0.6b-v1-q80",
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
    after_stop:
      id === offline
        ? ["integrated", "nemotron", "pyannote"]
        : ["nemotron", "pyannote"],
    during_recording: ["nemotron"],
  },
  recommended: false,
  size_bytes: 100,
  required: false,
  state: { state: "present" },
  repository: "example",
  revision: "rev",
  licence: { name: "MIT", url: "https://example.com", attribution: "Example" },
});
const models = [model(stream, "Parakeet"), model(offline, "MOSS")];
const settings = {
  asr_model: stream,
  speaker_source: "nemotron",
  live_transcript: true,
  speaker_processing_mode: "after_stop",
  default_speaker_count: null,
  recognition_language: null,
} as Settings;

function TestModels(props: Omit<ComponentProps<typeof Models>, "defaults">) {
  const defaults = useRecognitionDefaultsSave(async () => {});
  return <Models {...props} defaults={defaults} />;
}
function resolve(config: RecognitionConfig): ExecutionSpec {
  if (
    (config.asr_model === offline || config.asr_model === integratedModels[1]) &&
    config.processing_mode === "during_recording"
  )
    throw "MOSS requires After Stop";
  if (config.speaker_source === "integrated" && !integratedModels.includes(config.asr_model))
    throw "No integrated labels";
  if (
    (config.speaker_source === "pyannote" ||
      config.speaker_source === "off" ||
      config.speaker_source === "integrated") &&
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
  const tauri = mockTauri({
    resolve_recognition: ({ config }) => resolve(config as RecognitionConfig),
    set_recognition_defaults: () => initial,
    download_recognition_artifacts: () => null,
    recording_status: () => ({ recording: false }),
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
async function ready() {
  await waitFor(() =>
    expect(
      (screen.getByLabelText("Transcription timing") as HTMLSelectElement)
        .disabled,
    ).toBe(false),
  );
}

describe("catalog speaker defaults", () => {
  for (const id of integratedModels) {
    const entry = catalog.models.find((item) => item.id === id)!;
    const selected: ModelStatus = {
      ...model(id, entry.name),
      speaker_preferences: entry.speaker_preferences as ModelStatus["speaker_preferences"],
    };
    test(`${id} selects Nemotron and permits an integrated override`, async () => {
      const { user, tauri } = open({}, settings, [models[0], selected]);
      await ready();
      await user.click(screen.getByRole("radio", { name: entry.name }));
      await ready();
      const saved = () => tauri.calls.filter((call) => call.cmd === "set_recognition_defaults");
      expect((saved().slice(-1)[0]?.args.config as RecognitionConfig).speaker_source).toBe("nemotron");
      await user.click(screen.getByText("Advanced options"));
      await user.selectOptions(screen.getByLabelText("Speaker model override"), "integrated");
      await ready();
      expect((saved().slice(-1)[0]?.args.config as RecognitionConfig).speaker_source).toBe("integrated");
    });
    for (const source of ["integrated", "pyannote", "sortformer", "nemotron", "off"] as const) {
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
    await user.click(screen.getByRole("radio", { name: "MOSS" }));
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
      speaker_source: "integrated",
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
    await user.click(screen.getByRole("radio", { name: "MOSS" }));
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
    expect(
      (
        screen.getByRole("button", {
          name: "Download required models",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    expect(screen.queryByRole("button", { name: "Apply defaults" })).toBeNull();
  });
  test("download passes the resolved artifact specification and failures are visible", async () => {
    const { user, tauri } = open({
      download_recognition_artifacts: () => {
        throw "Download unavailable";
      },
    });
    await ready();
    await user.click(
      screen.getByRole("button", { name: "Download required models" }),
    );
    expect(await screen.findByText(/Download unavailable/)).toBeTruthy();
    const spec = tauri.calls.find(
      (c) => c.cmd === "download_recognition_artifacts",
    )?.args.spec as ExecutionSpec;
    expect(spec.artifacts[0].filename).toBe("exact-model.gguf");
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
    await user.click(screen.getByRole("radio", { name: "MOSS" }));
    await ready();
    await act(async () => release());
    expect(
      screen.getByRole("radio", { name: "MOSS" }).getAttribute("aria-checked"),
    ).toBe("true");
  });
});

test("model comparison shows capabilities before selection and keeps engines advanced", async () => {
  open();
  await ready();
  expect(screen.getByRole("radio", { name: /Parakeet/ })).toBeTruthy();
  expect(screen.getByRole("radio", { name: /MOSS/ })).toBeTruthy();
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

test("completed model downloads clear the pending defaults notice", async () => {
  const pending: RecognitionConfig = {
    asr_model: offline,
    speaker_source: "integrated",
    processing_mode: "after_stop",
    speaker_processing_mode: "after_stop",
    speaker_count: null,
    language: null,
  };
  const pendingSettings = { ...settings, pending_recognition: pending };
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
  await user.click(screen.getByRole("radio", { name: "MOSS" }));
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
  expect(screen.getByText(/Models are downloading for MOSS/)).toBeTruthy();
  expect(screen.getByText("Downloading required models…")).toBeTruthy();

  update({
    ...settings,
    asr_model: offline,
    speaker_source: "integrated",
    live_transcript: false,
    pending_recognition: null,
  }, models);
  expect(screen.getByText("Models ready")).toBeTruthy();
  expect(screen.queryByText(/Models are downloading for/)).toBeNull();
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
    "MOSS",
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

test("chosen recommendations precede the alphabetical model list", async () => {
  open({}, settings, [
    model("z", "Zebra"),
    {
      ...model("granite-speech-4.1-2b-plus-q5km", "Granite"),
      recommended: true,
    },
    {
      ...model("parakeet-unified-en-0.6b-q80", "Parakeet Unified"),
      recommended: true,
    },
    model("a", "Alpha"),
    {
      ...model("nemotron-speech-streaming-en-0.6b-q80", "Nemotron"),
      recommended: true,
    },
  ]);
  await waitFor(() => expect(screen.getAllByRole("radio")).toHaveLength(5));
  expect(
    screen.getAllByRole("radio").map((row) => row.getAttribute("aria-label")),
  ).toEqual(["Nemotron", "Parakeet Unified", "Granite", "Alpha", "Zebra"]);
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
  const row = screen.getByRole("radio", { name: "MOSS" }).parentElement!;
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
  await user.click(
    screen.getByRole("button", { name: "Download required models" }),
  );
  const spec = tauri.calls.find(
    (c) => c.cmd === "download_recognition_artifacts",
  )?.args.spec as ExecutionSpec;
  expect(spec.config.speaker_source).toBe("pyannote");
  expect(tauri.calls.some((c) => c.cmd === "set_recognition_defaults")).toBe(
    false,
  );
});


test("model download progress appears beside that model", async () => {
  open({}, settings, models, { [offline]: { kind: "progress", downloaded: 25_000_000, total: 100_000_000 } });
  await ready();
  const row = screen.getByRole("radio", { name: "MOSS" }).parentElement!;
  expect(row.textContent).toContain("Downloading… 25 MB of 100 MB");
  expect(row.querySelector('[role="progressbar"]')?.getAttribute("aria-valuenow")).toBe("25");
});

test("choosing a model gives immediate feedback while its request is pending", async () => {
  let release: (s: Settings) => void = () => {};
  const { user } = open({set_recognition_defaults: () => new Promise<Settings>(resolve => { release = resolve; })});
  await ready();
  await user.click(screen.getByRole("radio", { name: "MOSS" }));
  await screen.findByText("Preparing model…");
  expect(screen.getByRole("radio", { name: "MOSS" }).parentElement!.textContent).toContain("Preparing model…");
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
      destination: { kind: "unset" },
      armed: false,
      shortcut: shortcutStatuses.recording,
      missing_models: [],
      endpoint_configured: false,
    }),
    shortcut_status: () => shortcutStatuses,
    login_item_status: () => "disabled",
    model_statuses: () => models,
    resolve_recognition: ({ config }) => resolve(config as RecognitionConfig),
    set_recognition_defaults: ({ config }) => {
      const choice = config as RecognitionConfig;
      saved = {
        ...saved,
        asr_model: choice.asr_model,
        speaker_source: choice.speaker_source,
        live_transcript: choice.processing_mode === "during_recording",
        speaker_processing_mode: choice.speaker_processing_mode ?? "after_stop",
        default_speaker_count: choice.speaker_count,
        recognition_language: choice.language,
      };
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
    ["ArrowDown", "Zebra", "zebra"],
    ["ArrowDown", "MOSS", offline],
    ["ArrowUp", "Zebra", "zebra"],
    ["Home", "MOSS", offline],
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

describe("delete models", () => {
  const segmentation: ModelStatus = {
    ...model("segmentation", "Speaker segmentation weights with a very long name for the row"),
    role: "segmentation",
    size_bytes: 200_000_000,
  };
  const catalogWithSupport = [...models, segmentation];
  const summary = (overrides: Partial<ModelDeletionSummary> = {}): ModelDeletionSummary => ({
    models: [
      { id: stream, name: "Parakeet", size_bytes: 1_000_000_000, unused: false },
      { id: "segmentation", name: "Speaker segmentation", size_bytes: 200_000_000, unused: false },
      { id: offline, name: "MOSS", size_bytes: 2_500_000_000, unused: true },
    ],
    unfinished_notes: 0,
    delete_all_block: null,
    ...overrides,
  });
  const allUsed = (overrides: Partial<ModelDeletionSummary> = {}) =>
    summary({
      models: [
        { id: stream, name: "Parakeet", size_bytes: 1_000_000_000, unused: false },
        { id: "segmentation", name: "Speaker segmentation", size_bytes: 200_000_000, unused: false },
      ],
      ...overrides,
    });
  const deleteButton = () =>
    screen.getByRole("button", { name: "Delete models…" }) as HTMLButtonElement;
  const dialog = () => screen.getByRole("alertdialog");

  test("unused models give two choices with the space each one recovers (AE5)", async () => {
    const { user } = open({ model_deletion_summary: () => summary() }, settings, catalogWithSupport);
    await ready();
    await user.click(deleteButton());
    await screen.findByRole("alertdialog");
    expect(screen.getByRole("button", { name: "Delete unused (2.5 GB)" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Delete all (3.7 GB)" })).toBeTruthy();
    expect(dialog().textContent).toContain("cannot transcribe new notes");
  });

  test("no unused model gives only Delete all, with the warning (AE6)", async () => {
    const { user } = open({ model_deletion_summary: () => allUsed() }, settings, catalogWithSupport);
    await ready();
    await user.click(deleteButton());
    await screen.findByRole("alertdialog");
    expect(screen.queryByRole("button", { name: /Delete unused/ })).toBeNull();
    expect(screen.getByRole("button", { name: "Delete all (1.2 GB)" })).toBeTruthy();
    expect(dialog().textContent).toContain(
      "Longhand cannot transcribe new notes until you download the models again.",
    );
    expect(dialog().textContent).not.toMatch(/unfinished/);
  });

  test("Delete all warns about waiting notes, deletes every model and refreshes (AE7)", async () => {
    const { user, tauri, refreshes } = open(
      { model_deletion_summary: () => allUsed({ unfinished_notes: 2 }), delete_models: () => [] },
      settings,
      catalogWithSupport,
    );
    await ready();
    await user.click(deleteButton());
    await screen.findByRole("alertdialog");
    expect(dialog().textContent).toContain("2 unfinished notes will wait for the models.");
    await user.click(screen.getByRole("button", { name: /^Delete all/ }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(tauri.calls.find((c) => c.cmd === "delete_models")?.args.scope).toBe("all");
    expect(refreshes.count).toBe(1);
  });

  test("one unfinished note is singular", async () => {
    const { user } = open(
      { model_deletion_summary: () => allUsed({ unfinished_notes: 1 }) },
      settings,
      catalogWithSupport,
    );
    await ready();
    await user.click(deleteButton());
    await screen.findByRole("alertdialog");
    expect(dialog().textContent).toContain("1 unfinished note will wait for the models.");
  });

  test("a recording turns the button off (AE8)", async () => {
    open({ recording_status: () => ({ recording: true }) }, settings, catalogWithSupport);
    await ready();
    await waitFor(() => expect(deleteButton().disabled).toBe(true));
  });

  test("the button follows recording changes", async () => {
    let recording = false;
    const { tauri } = open({ recording_status: () => ({ recording }) }, settings, catalogWithSupport);
    await ready();
    expect(deleteButton().disabled).toBe(false);
    recording = true;
    await act(async () => tauri.emit("recording-changed", { session_id: "s" }));
    await waitFor(() => expect(deleteButton().disabled).toBe(true));
  });

  test("an active download turns the button off", async () => {
    open({}, settings, catalogWithSupport, {
      [offline]: { kind: "progress", downloaded: 1, total: 2 },
    });
    await ready();
    expect(deleteButton().disabled).toBe(true);
  });

  test("a model in the downloading state turns the button off", async () => {
    open({}, settings, [...models, { ...segmentation, state: { state: "downloading" } }]);
    await ready();
    expect(deleteButton().disabled).toBe(true);
  });

  test("no downloaded model turns the button off", async () => {
    open(
      {},
      settings,
      models.map((m) => ({ ...m, state: { state: "absent" as const, partial_bytes: 0 } })),
    );
    await ready();
    expect(deleteButton().disabled).toBe(true);
  });

  test("a transcribing note turns Delete all off, and Delete unused still works (AE9)", async () => {
    const { user, tauri } = open(
      {
        model_deletion_summary: () => summary({ delete_all_block: "transcription" }),
        delete_models: () => [],
      },
      settings,
      catalogWithSupport,
    );
    await ready();
    await user.click(deleteButton());
    await screen.findByRole("alertdialog");
    const all = screen.getByRole("button", { name: /^Delete all/ }) as HTMLButtonElement;
    expect(all.disabled).toBe(true);
    expect(dialog().textContent).toContain("Delete all is off while a note is transcribing.");
    await user.click(screen.getByRole("button", { name: /^Delete unused/ }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(tauri.calls.find((c) => c.cmd === "delete_models")?.args.scope).toBe("unused");
  });

  for (const [block, reason] of [
    ["recording", "Delete all is off while a recording is in progress."],
    ["download", "Delete all is off while a download is in progress."],
  ] as const) {
    test(`the ${block} block gives its reason`, async () => {
      const { user } = open(
        { model_deletion_summary: () => summary({ delete_all_block: block }) },
        settings,
        catalogWithSupport,
      );
      await ready();
      await user.click(deleteButton());
      await screen.findByRole("alertdialog");
      expect(dialog().textContent).toContain(reason);
    });
  }

  test("a partial failure names each model that was not deleted", async () => {
    const { user, refreshes } = open(
      {
        model_deletion_summary: () => summary(),
        delete_models: () => [
          { id: offline, name: "MOSS", message: "The file is in use." },
          { id: "segmentation", name: "Speaker segmentation", message: "Permission denied." },
        ],
      },
      settings,
      catalogWithSupport,
    );
    await ready();
    await user.click(deleteButton());
    await screen.findByRole("alertdialog");
    await user.click(screen.getByRole("button", { name: /^Delete all/ }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(screen.getByText(/Could not delete MOSS and Speaker segmentation/)).toBeTruthy();
    expect(refreshes.count).toBe(1);
  });

  test("a summary failure shows the error and no dialog", async () => {
    const { user } = open(
      {
        model_deletion_summary: () => {
          throw "The model folder could not be read";
        },
      },
      settings,
      catalogWithSupport,
    );
    await ready();
    await user.click(deleteButton());
    expect(await screen.findByText(/The model folder could not be read/)).toBeTruthy();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(deleteButton().disabled).toBe(false);
  });

  test("the supporting models list opens from Show models", async () => {
    const { user } = open({}, settings, catalogWithSupport);
    await ready();
    const toggle = screen.getByRole("button", { name: "Show models" });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText(segmentation.name)).toBeNull();
    await user.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    const list = document.getElementById(toggle.getAttribute("aria-controls")!)!;
    expect(list.textContent).toContain(segmentation.name);
    expect(list.textContent).toContain("200 MB");
    expect(
      Array.from(list.querySelectorAll("button")).some((b) => b.textContent === "Delete…"),
    ).toBe(true);
  });
});
