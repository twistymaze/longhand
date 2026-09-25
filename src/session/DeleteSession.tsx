// Delete a session (R8, R33), from Home or from the session page. Delete
// asks once and moves the folder to the Trash. When the Trash refuses the
// folder, a second question offers a permanent delete, and Cancel changes
// nothing.

import { useState } from "react";
import { commands, type SessionEntry } from "../bindings";
import { ConfirmDialog } from "../lib/ConfirmDialog";
import { sentence } from "../processing/describe";

type Confirm =
  | { kind: "trash"; entry: SessionEntry }
  | { kind: "permanent"; entry: SessionEntry; reason: string };

/// The delete questions for one surface. `ask` shows the first question;
/// `dialog` is the question to render, or null.
export function useDeleteSession({
  titleOf,
  onDeleted,
  onFailed,
}: {
  titleOf: (entry: SessionEntry) => string;
  /// The session and its folder are gone.
  onDeleted: (entry: SessionEntry) => void;
  /// The delete did not complete, with a sentence that says why.
  onFailed: (text: string) => void;
}) {
  const [confirm, setConfirm] = useState<Confirm | null>(null);
  const [busy, setBusy] = useState(false);

  const remove = async (current: Confirm) => {
    setBusy(true);
    const result =
      current.kind === "trash"
        ? await commands.deleteSession(current.entry.id)
        : await commands.deleteSessionPermanently(current.entry.id);
    setBusy(false);
    if (result.status === "ok") {
      setConfirm(null);
      onDeleted(current.entry);
      return;
    }
    const error = result.error;
    if (error.kind === "cannot_move_to_trash" && current.kind === "trash") {
      setConfirm({ kind: "permanent", entry: current.entry, reason: error.reason });
      return;
    }
    setConfirm(null);
    onFailed(sentence(error.kind === "cannot_move_to_trash" ? error.reason : error.message));
  };

  const title = confirm ? titleOf(confirm.entry) : "";
  const dialog =
    confirm === null ? null : confirm.kind === "trash" ? (
      <ConfirmDialog
        title="Delete note?"
        message={`“${title}” moves to the Trash.`}
        action="Delete"
        busy={busy}
        onConfirm={() => void remove(confirm)}
        onCancel={() => setConfirm(null)}
      />
    ) : (
      <ConfirmDialog
        title="This note can’t be moved to the Trash"
        message={`${sentence(confirm.reason)} Delete “${title}” permanently? You can’t undo this.`}
        action="Delete Permanently"
        busy={busy}
        onConfirm={() => void remove(confirm)}
        onCancel={() => setConfirm(null)}
      />
    );

  return { ask: (entry: SessionEntry) => setConfirm({ kind: "trash", entry }), dialog };
}
