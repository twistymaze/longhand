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
import { Models, describeLanguages } from "./Models";

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
  speaker_source: "pyannote",
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
) {
  const tauri = mockTauri({
    resolve_recognition: ({ config }) => resolve(config as RecognitionConfig),
    set_recognition_defaults: () => initial,
    download_recognition_artifacts: () => null,
    ...handlers,
  });
  render(
    <Models
      models={models}
      speakers={null}
      live={{}}
      settings={initial}
      onSettings={() => {}}
      actions={{
        download: () => {},
        cancel: () => {},
        select: async () => null,
        refresh: async () => {},
      }}
    />,
  );
  return { tauri, user: userEvent.setup() };
}
async function ready() {
  await waitFor(() =>
    expect(
      (
        screen.getByRole("button", {
          name: "Apply defaults",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false),
  );
}

describe("recognition defaults", () => {
  test("recognition choices expose separate timing and keep speaker accuracy unknown", async () => {
    open();
    await ready();
    expect(screen.getByLabelText("Speaker recognition")).toBeTruthy();
    expect(screen.getByLabelText("Transcription timing")).toBeTruthy();
    expect(screen.getByLabelText("Speaker timing")).toBeTruthy();
    expect(screen.getByText("Speaker accuracy: Unknown")).toBeTruthy();
    expect(screen.queryByLabelText("Runtime")).toBeNull();
    expect(describeLanguages(["en", "zh"])).toBe("English and Chinese");
  });
  test("model changes reset incompatible timing visibly and apply the complete configuration", async () => {
    const { user, tauri } = open();
    await ready();
    await user.selectOptions(
      screen.getByLabelText("Transcription model"),
      offline,
    );
    await ready();
    expect(
      (screen.getByLabelText("Transcription timing") as HTMLSelectElement)
        .value,
    ).toBe("after_stop");
    expect(screen.getByText(/processing choices changed/)).toBeTruthy();
    await user.selectOptions(
      screen.getByLabelText("Speaker recognition"),
      "integrated",
    );
    await user.click(screen.getByRole("button", { name: "Apply defaults" }));
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
    await user.selectOptions(
      screen.getByLabelText("Transcription model"),
      offline,
    );
    await ready();
    await user.selectOptions(
      screen.getByLabelText("Speaker recognition"),
      "nemotron",
    );
    await user.selectOptions(
      screen.getByLabelText("Speaker timing"),
      "during_recording",
    );
    expect(
      (screen.getByLabelText("Transcription timing") as HTMLSelectElement)
        .value,
    ).toBe("after_stop");
    expect(
      (screen.getByLabelText("Speaker timing") as HTMLSelectElement).value,
    ).toBe("during_recording");
  });
  test("five expected speakers cannot select Sortformer", async () => {
    open(
      {},
      { ...settings, default_speaker_count: { kind: "exact", count: 5 } },
    );
    await ready();
    expect(
      (screen.getByRole("option", { name: "Sortformer" }) as HTMLOptionElement)
        .disabled,
    ).toBe(true);
  });
  test("an unavailable resolver blocks Apply and explains recovery", async () => {
    open({
      resolve_recognition: () => {
        throw "catalog unavailable";
      },
    });
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(
      (
        screen.getByRole("button", {
          name: "Apply defaults",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
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
    await user.selectOptions(
      screen.getByLabelText("Transcription model"),
      offline,
    );
    await ready();
    await act(async () => release());
    expect(
      (screen.getByLabelText("Transcription model") as HTMLSelectElement).value,
    ).toBe(offline);
  });
});
