import { describe, expect, test } from "bun:test";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { SessionDetail, Settings } from "../bindings";
import { mockTauri } from "../test/tauri";
import { ProcessingStatus } from "./ProcessingStatus";

const waiting: SessionDetail = {
  entry: {
    id: "s1",
    started: new Date(2026, 8, 24, 14, 5).toISOString(),
    duration_ms: 60_000,
    state: "awaiting_destination",
    stage: null,
    title: null,
    found_speakers: null,
    storage_unavailable: false,
    problem: null,
    remote: null,
    note_type: null,
    tags: [],
    waits_for: null,
  },
  done: ["transcribe", "summarize"],
  current: null,
  speakers: null,
  windows_done: null,
  windows_total: null,
  failed_stage: null,
  message: "The storage folder is not available.",
  summary_problem: null,
  resumable: false,
  record: null,
  home_path: null,
  endpoint: null,
  audio_kept: true,
  transcript_kept: true,
  audio: "present",
  asr_model: null,
  live_transcript: false,
  transcribe_again: false,
  regenerating: false,
  transcribe_again_error: null,
  retry_starts_over: false,
  summary_not_updated: null,
  waiting: [],
  conflict_files: [],
  not_saved_yet: null,
};

function open() {
  const tauri = mockTauri({
    pick_storage_folder: () => "/Volumes/Disk/Notes",
    change_storage_folder: () => ({ storage_folder: "/Volumes/Disk/Notes" }) as Settings,
  });
  let changed = 0;
  render(<ProcessingStatus detail={waiting} live={undefined} onChanged={() => changed++} />);
  return { tauri, user: userEvent.setup(), changed: () => changed };
}

describe("Choose Folder on a note that waits for a folder", () => {
  test("asks Move or Leave Them, and Move moves the notes", async () => {
    const { tauri, user, changed } = open();
    await user.click(screen.getByRole("button", { name: "Choose Folder…" }));
    await user.click(await screen.findByRole("button", { name: "Move Notes" }));
    await waitFor(() => expect(changed()).toBe(1));
    expect(tauri.calls.find((c) => c.cmd === "change_storage_folder")?.args).toEqual({
      folder: "/Volumes/Disk/Notes",
      oldNotes: "move",
    });
  });

  test("Leave Them keeps the notes in the old folder", async () => {
    const { tauri, user } = open();
    await user.click(screen.getByRole("button", { name: "Choose Folder…" }));
    await user.click(await screen.findByRole("button", { name: "Leave Them" }));
    await waitFor(() =>
      expect(tauri.calls.find((c) => c.cmd === "change_storage_folder")?.args.oldNotes).toBe("leave"),
    );
  });

  test("Cancel changes nothing", async () => {
    const { tauri, user } = open();
    await user.click(screen.getByRole("button", { name: "Choose Folder…" }));
    await user.click(await screen.findByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(tauri.calls.map((c) => c.cmd)).toEqual(["pick_storage_folder"]);
  });

  test("a refused folder shows its cause", async () => {
    mockTauri({
      pick_storage_folder: () => "/notes",
      change_storage_folder: () => {
        throw "This folder is already the storage folder.";
      },
    });
    render(<ProcessingStatus detail={waiting} live={undefined} onChanged={() => {}} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Choose Folder…" }));
    await user.click(await screen.findByRole("button", { name: "Move Notes" }));
    expect(await screen.findByText("This folder is already the storage folder.")).toBeTruthy();
  });
});

describe("the summarize stage", () => {
  const processing = (over: Partial<SessionDetail>): SessionDetail => ({
    ...waiting,
    entry: { ...waiting.entry, state: "processing", stage: "summarize" },
    done: ["transcribe"],
    current: "summarize",
    message: null,
    ...over,
  });
  const show = (detail: SessionDetail) => {
    const tauri = mockTauri({ open_settings: () => null });
    render(<ProcessingStatus detail={detail} live={undefined} onChanged={() => {}} />);
    return tauri;
  };

  test("Covers AE2. With no provider, it says the note is saved without a summary and links to the AI Provider pane", async () => {
    const tauri = show(
      processing({ summary_problem: "no AI provider is set, so the note is saved without a summary", endpoint: null }),
    );
    expect(screen.getByText(/No AI provider is set, so the note is saved without a summary\./)).toBeTruthy();
    await userEvent.setup().click(screen.getByRole("button", { name: "Set Up AI Provider…" }));
    expect(tauri.calls.filter((c) => c.cmd === "open_settings").map((c) => c.args)).toEqual([{ pane: "provider" }]);
  });

  test("before the stage runs with no provider, it says so and links to the AI Provider pane", () => {
    show(processing({ endpoint: null }));
    expect(screen.getByText("No AI provider is set, so the note is saved without a summary.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Set Up AI Provider…" })).toBeTruthy();
  });

  test("for Anthropic, it names Anthropic and the model, with no host", () => {
    show(
      processing({
        endpoint: { provider: "anthropic", host: "api.anthropic.com", model: "claude-sonnet", encrypted: true },
      }),
    );
    const line = screen.getByText(/Sends the transcript and notes to/);
    expect(line.textContent).toBe("Sends the transcript and notes to Anthropic (claude-sonnet).");
    expect(screen.queryByRole("button", { name: "Set Up AI Provider…" })).toBeNull();
  });

  test("for Custom, it names the host, and plain HTTP to another computer is unencrypted", () => {
    show(processing({ endpoint: { provider: "custom", host: "llm.example.com", model: "m", encrypted: false } }));
    const line = screen.getByText(/Sends the transcript and notes to/);
    expect(line.textContent).toBe("Sends the transcript and notes to llm.example.com (m), unencrypted.");
  });

  test("for a local provider, it names the provider and the host", () => {
    show(processing({ endpoint: { provider: "ollama", host: "localhost:11434", model: "llama3", encrypted: true } }));
    const line = screen.getByText(/Sends the transcript and notes to/);
    expect(line.textContent).toBe("Sends the transcript and notes to Ollama at localhost:11434 (llama3).");
  });
});
