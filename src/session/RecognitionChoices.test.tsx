import { expect, test } from "bun:test";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ExecutionSpec, RecognitionConfig } from "../bindings";
import { mockTauri } from "../test/tauri";
import { NewNoteChoices } from "./NewNoteChoices";
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

test("one-note choices set only the override before capture and clear it afterwards", async () => {
  const tauri = mockTauri({
    ...handlers,
    set_recognition_override: () => null,
  });
  let started = false;
  render(
    <NewNoteChoices
      disabled={false}
      onStart={async () => {
        started = true;
        expect(tauri.calls[tauri.calls.length - 1]?.cmd).toBe(
          "set_recognition_override",
        );
        return true;
      }}
    />,
  );
  const user = userEvent.setup();
  await user.click(
    screen.getByRole("button", { name: "Choices for next note…" }),
  );
  const record = await screen.findByRole("button", {
    name: "Record this note",
  });
  await waitFor(() =>
    expect((record as HTMLButtonElement).disabled).toBe(false),
  );
  await user.click(record);
  await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  expect(started).toBe(true);
  expect(
    tauri.calls
      .filter((c) => c.cmd === "set_recognition_override")
      .map((c) => c.args),
  ).toEqual([{ config }, { config: null }]);
  expect(tauri.calls.some((c) => c.cmd === "set_recognition_defaults")).toBe(
    false,
  );
});

test("cancel before recording changes neither defaults nor override", async () => {
  const tauri = mockTauri(handlers);
  render(
    <NewNoteChoices
      disabled={false}
      onStart={async () => {
        throw new Error("must not record");
      }}
    />,
  );
  const user = userEvent.setup();
  await user.click(
    screen.getByRole("button", { name: "Choices for next note…" }),
  );
  await user.click(await screen.findByRole("button", { name: "Cancel" }));
  expect(tauri.calls.some((c) => c.cmd.startsWith("set_"))).toBe(false);
});

test("one-note choices can download their resolved artifacts before recording", async () => {
  const tauri = mockTauri({
    ...handlers,
    download_recognition_artifacts: () => null,
  });
  render(<NewNoteChoices disabled={false} onStart={async () => false} />);
  const user = userEvent.setup();
  await user.click(
    screen.getByRole("button", { name: "Choices for next note…" }),
  );
  const download = await screen.findByRole("button", {
    name: "Download required models",
  });
  await waitFor(() =>
    expect((download as HTMLButtonElement).disabled).toBe(false),
  );
  await user.click(download);
  expect(
    tauri.calls.find((c) => c.cmd === "download_recognition_artifacts")?.args
      .spec,
  ).toEqual(execution(config));
  expect(tauri.calls.some((c) => c.cmd === "set_recognition_defaults")).toBe(
    false,
  );
});

test("recording stays disabled until the exact native artifact finishes downloading", async () => {
  let present = false;
  const artifact = {
    id: "native-artifact",
    filename: "native.gguf",
    sha256: "hash",
    revision: "revision",
    url: "https://example.com/native",
    size_bytes: 10,
    format: "gguf",
  };
  const tauri = mockTauri({
    ...handlers,
    resolve_recognition: ({ config }) => ({
      ...execution(config as RecognitionConfig),
      artifacts: [artifact],
    }),
    model_statuses: () => [
      ...handlers.model_statuses(),
      {
        id: artifact.id,
        name: "Native artifact",
        role: "recognition_artifact",
        state: present
          ? { state: "present" }
          : { state: "absent", partial_bytes: 0 },
      },
    ],
    download_recognition_artifacts: () => null,
  });
  render(
    <NewNoteChoices
      disabled={false}
      onStart={async () => {
        throw new Error("not requested");
      }}
    />,
  );
  const user = userEvent.setup();
  await user.click(
    screen.getByRole("button", { name: "Choices for next note…" }),
  );
  expect(
    await screen.findByText("1 required model file is not downloaded."),
  ).toBeTruthy();
  const record = screen.getByRole("button", { name: "Record this note" });
  expect((record as HTMLButtonElement).disabled).toBe(true);
  await user.click(
    screen.getByRole("button", { name: "Download required models" }),
  );
  expect((record as HTMLButtonElement).disabled).toBe(true);
  present = true;
  await act(() =>
    tauri.emit("model-download-event", {
      model_id: artifact.id,
      update: { kind: "completed" },
    }),
  );
  await waitFor(() =>
    expect((record as HTMLButtonElement).disabled).toBe(false),
  );
});

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
