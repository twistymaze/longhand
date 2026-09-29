// The question after the user picks a new storage folder (R4): Move the
// notes of the old folder, leave them there, or cancel the change. Move
// has the focus, so Return moves the notes and no note is left behind by
// accident.

import { useCallback, useState } from "react";
import { commands, type OldNotes, type Result, type Settings } from "../bindings";
import { ConfirmDialog } from "../lib/ConfirmDialog";
import { folderName } from "./ui";

type Answer = OldNotes | "cancel";

export function OldNotesDialog({ folder, onAnswer }: { folder: string; onAnswer: (answer: Answer) => void }) {
  // Order in the row: Cancel, Leave Them, Move Notes.
  return (
    <ConfirmDialog
      title={`Move your notes to “${folderName(folder)}”?`}
      message={
        <>
          <p>Move Notes puts every note in the new folder. You can keep working while they move.</p>
          <p>Leave Them keeps the notes in the old folder. Longhand no longer lists them.</p>
        </>
      }
      action="Move Notes"
      destructive={false}
      secondary={{ label: "Leave Them", onClick: () => onAnswer("leave") }}
      focus="confirm"
      width="w-[22rem]"
      onConfirm={() => onAnswer("move")}
      onCancel={() => onAnswer("cancel")}
    />
  );
}

/// Change the storage folder: pick a folder, ask what to do with the
/// notes of the old folder, then change. `change` resolves to `null` data
/// when the user closes the folder dialog or selects Cancel. `dialog` is
/// the question to render, or null. `moved` is true when the change
/// started a Move.
export function useFolderChange() {
  const [asking, setAsking] = useState<{ folder: string; answer: (a: Answer) => void } | null>(null);
  const change = useCallback(async (): Promise<Result<{ settings: Settings; moved: boolean } | null, string>> => {
    const picked = await commands.pickStorageFolder();
    if (picked.status === "error") return picked;
    const folder = picked.data;
    if (folder === null) return { status: "ok", data: null };
    const answer = await new Promise<Answer>((resolve) => setAsking({ folder, answer: resolve }));
    setAsking(null);
    if (answer === "cancel") return { status: "ok", data: null };
    const changed = await commands.changeStorageFolder(folder, answer);
    if (changed.status === "error") return changed;
    return { status: "ok", data: { settings: changed.data, moved: answer === "move" } };
  }, []);
  const dialog = asking && <OldNotesDialog folder={asking.folder} onAnswer={asking.answer} />;
  return { change, dialog };
}
