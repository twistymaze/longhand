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
  home_path: null,
  endpoint: null,
  audio_kept: true,
  transcript_kept: true,
  asr_model: null,
  live_transcript: false,
  transcribe_again: false,
  transcribe_again_error: null,
  summary_not_updated: null,
  waiting: [],
  conflict_files: [],
};

function open() {
  const tauri = mockTauri({
    pick_storage_folder: () => "/Volumes/Disk/Notes",
    change_storage_folder: () => ({ storage_folder: "/Volumes/Disk/Notes" }) as Settings,
  });
  let changed = 0;
  render(<ProcessingStatus detail={waiting} live={undefined} waitsForRecording={false} onChanged={() => changed++} />);
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
    render(<ProcessingStatus detail={waiting} live={undefined} waitsForRecording={false} onChanged={() => {}} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Choose Folder…" }));
    await user.click(await screen.findByRole("button", { name: "Move Notes" }));
    expect(await screen.findByText("This folder is already the storage folder.")).toBeTruthy();
  });
});
