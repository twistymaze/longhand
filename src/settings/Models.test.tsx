import { describe, expect, test } from "bun:test";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type {
  ExecutionSpec,
  ModelStatus,
  RecognitionConfig,
  Settings,
} from "../bindings";
import { mockTauri } from "../test/tauri";
import { Models } from "./Models";
import { describeLanguages } from "./languages";

const stream = "parakeet-unified-en-0.6b-q80";
const offline = "moss-transcribe-diarize-q80";
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
function resolve(config: RecognitionConfig): ExecutionSpec {
  if (
    config.asr_model === offline &&
    config.processing_mode === "during_recording"
  )
    throw "MOSS requires After Stop";
  if (config.speaker_source === "integrated" && config.asr_model !== offline)
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
) {
  const tauri = mockTauri({
    resolve_recognition: ({ config }) => resolve(config as RecognitionConfig),
    set_recognition_defaults: () => initial,
    download_recognition_artifacts: () => null,
    ...handlers,
  });
  render(
    <Models
      models={catalog}
      live={{}}
      settings={initial}
      onSettings={() => {}}
      actions={{
        cancel: () => {},
        refresh: async () => {},
      }}
    />,
  );
  return { tauri, user: userEvent.setup() };
}
async function ready() {
  await waitFor(() =>
    expect(
      (screen.getByLabelText("Transcription timing") as HTMLSelectElement)
        .disabled,
    ).toBe(false),
  );
}

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
