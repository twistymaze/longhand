import { expect, test } from "bun:test";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ExecutionSpec, RecognitionConfig } from "../bindings";
import { mockTauri } from "../test/tauri";
import { Home } from "../home/Home";
import { useTranscribeAgain } from "./TranscribeAgain";

const config: RecognitionConfig = {
  asr_model: "model",
  speaker_source: "off",
  processing_mode: "after_stop",
  speaker_processing_mode: "after_stop",
  speaker_count: null,
  language: null,
};
const execution = (value: RecognitionConfig): ExecutionSpec => ({
  schema_version: 1,
  catalog_revision: "test",
  config: value,
  artifacts: [],
  runtimes: [],
  capabilities: {
    live_transcription: false,
    live_speaker_analysis: false,
    live_attributed_text: false,
    maximum_speakers: null,
    locally_verified: false,
  },
  enabled_live_transcription: false,
  enabled_live_speaker_analysis: false,
  preprocessing_revision: "test",
});
const handlers = {
  get_settings: () => ({
    asr_model: "model",
    speaker_source: "off",
    live_transcript: false,
    speaker_processing_mode: "after_stop",
    recognition_language: null,
    default_speaker_count: null,
  }),
  model_statuses: () => [
    {
      id: "model",
      name: "Test model",
      role: "recognition",
      languages: ["en"],
      size_bytes: 1,
      assessment: null,
      speaker_preferences: { after_stop: ["pyannote"], during_recording: [] },
    },
  ],
  resolve_recognition: ({ config }: Record<string, unknown>) =>
    execution(config as RecognitionConfig),
};
function Replacement() {
  const { ask, dialog } = useTranscribeAgain({
    sessionId: "note",
    noteModel: "model",
    onQueued: () => {},
    onFailed: () => {},
  });
  return (
    <>
      <button onClick={ask}>Ask</button>
      {dialog}
    </>
  );
}

test("replacement previews retranscription and needs explicit consent before queuing", async () => {
  const preview = {
    execution: execution(config),
    requires_transcription: true,
    explanation:
      "This note needs transcription again because timing is missing.",
    manifest: "original-manifest",
  };
  const tauri = mockTauri({
    ...handlers,
    preview_reprocess: () => preview,
    reprocess_session: () => null,
  });
  render(<Replacement />);
  const user = userEvent.setup();
  await user.click(screen.getByText("Ask"));
  const button = await screen.findByRole("button", { name: "Preview changes" });
  await waitFor(() =>
    expect((button as HTMLButtonElement).disabled).toBe(false),
  );
  await user.click(button);
  expect(await screen.findByText(preview.explanation)).toBeTruthy();
  const reprocess = screen.getByRole("button", { name: "Reprocess note" });
  expect((reprocess as HTMLButtonElement).disabled).toBe(true);
  expect(tauri.calls.some((c) => c.cmd === "reprocess_session")).toBe(false);
  await user.click(screen.getByRole("checkbox"));
  await user.click(reprocess);
  expect(tauri.calls.find((c) => c.cmd === "reprocess_session")?.args).toEqual({
    sessionId: "note",
    preview,
    allowTranscription: true,
  });
});

test("failed speaker-only replacement keeps the previous result and reports the error", async () => {
  const preview = {
    execution: execution(config),
    requires_transcription: false,
    explanation: "Reuse timed transcription and update speakers.",
    manifest: "original",
  };
  mockTauri({
    ...handlers,
    preview_reprocess: () => preview,
    reprocess_session: () => {
      throw "The note changed. Preview again.";
    },
  });
  render(<Replacement />);
  const user = userEvent.setup();
  await user.click(screen.getByText("Ask"));
  const button = await screen.findByRole("button", { name: "Preview changes" });
  await waitFor(() =>
    expect((button as HTMLButtonElement).disabled).toBe(false),
  );
  await user.click(button);
  await user.click(
    await screen.findByRole("button", { name: "Reprocess note" }),
  );
  const dialog = screen.getByRole("alertdialog");
  expect(
    await within(dialog).findByText("The note changed. Preview again."),
  ).toBeTruthy();
  expect(dialog.textContent).toContain("previous result stays");
  expect(screen.queryByRole("checkbox")).toBeNull();
});

for (const reject of [false, true]) {
  test(`changed choices discard a pending preview ${reject ? "error" : "result"}`, async () => {
    let finish!: (value: unknown) => void;
    let fail!: (reason: unknown) => void;
    const tauri = mockTauri({
      ...handlers,
      preview_reprocess: () =>
        new Promise((resolve, reject) => {
          finish = resolve;
          fail = reject;
        }),
    });
    render(<Replacement />);
    const user = userEvent.setup();
    await user.click(screen.getByText("Ask"));
    const button = await screen.findByRole("button", {
      name: "Preview changes",
    });
    await waitFor(() =>
      expect((button as HTMLButtonElement).disabled).toBe(false),
    );
    await user.click(button);
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Speaker recognition" }),
      "after_stop",
    );
    await act(async () => {
      if (reject) fail("Old preview failed");
      else
        finish({
          execution: execution(config),
          requires_transcription: false,
          explanation: "Stale preview",
          manifest: "old",
        });
    });
    await waitFor(() =>
      expect((button as HTMLButtonElement).disabled).toBe(false),
    );
    expect(screen.queryByText("Stale preview")).toBeNull();
    expect(screen.queryByText(/Old preview failed/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Reprocess note" })).toBeNull();
    expect(tauri.calls.some((call) => call.cmd === "reprocess_session")).toBe(
      false,
    );
  });
}

test("speaker-only preview uses a reprocessing dialog title", async () => {
  mockTauri({
    ...handlers,
    preview_reprocess: () => ({
      execution: execution(config),
      requires_transcription: false,
      explanation: "Reuse timed transcription and update speakers.",
      manifest: "original",
    }),
  });
  render(<Replacement />);
  const user = userEvent.setup();
  await user.click(screen.getByText("Ask"));
  const button = await screen.findByRole("button", { name: "Preview changes" });
  await waitFor(() =>
    expect((button as HTMLButtonElement).disabled).toBe(false),
  );
  await user.click(button);
  expect(
    await screen.findByRole("alertdialog", { name: "Reprocess this note?" }),
  ).toBeTruthy();
  expect(screen.queryByRole("checkbox")).toBeNull();
});

test("Home starts immediately with the saved default configuration", async () => {
  const tauri = mockTauri({ ...handlers, start_recording: () => "new-note" });
  render(
    <Home
      entries={[]}
      stages={{}}
      recording={null}
      readiness={{
        armed: true,
        microphone_authorized: true,
        destination: { kind: "ready", path: "/notes", free_bytes: null },
        shortcut: { chord: null, registered: false, error: null },
        system_audio: "allowed",
        missing_models: [],
        endpoint_configured: true,
      }}
      chord={null}
      onOpen={() => {}}
      onChanged={() => {}}
    />,
  );
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "New Note" }));
  expect(
    tauri.calls.find((call) => call.cmd === "start_recording")?.args,
  ).toEqual({ config: null });
  expect(
    tauri.calls.some((call) => call.cmd === "set_recognition_override"),
  ).toBe(false);
});
