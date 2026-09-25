// Delete a session (R8, R33), from Home or from the session page. Delete
// asks once and moves the folder to the Trash. When the Trash refuses the
// folder, a second question offers a permanent delete, and Cancel changes
// nothing.

import { useEffect, useRef, useState } from "react";
import { commands, type SessionEntry } from "../bindings";
import { sentence } from "../processing/describe";

type Confirm =
  | { kind: "trash"; entry: SessionEntry }
  | { kind: "permanent"; entry: SessionEntry; reason: string };

function ConfirmDialog({
  title,
  message,
  action,
  busy,
  onConfirm,
  onCancel,
}: {
  title: string;
  message: string;
  action: string;
  busy: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => cancel.current?.focus(), []);
  // Cancel has the focus: Return must not delete by accident.
  return (
    <div className="fixed inset-0 z-20 flex items-start justify-center bg-black/20 pt-[18vh]">
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        aria-describedby="confirm-message"
        onKeyDown={(e) => {
          if (e.key === "Escape") onCancel();
        }}
        className="fade-in w-[19rem] rounded-xl bg-group p-4 shadow-[0_0_0_0.5px_var(--separator),0_12px_40px_rgb(0_0_0/0.25)]"
      >
        <h2 id="confirm-title" className="font-semibold break-words">
          {title}
        </h2>
        <p id="confirm-message" className="mt-1 text-[0.85rem] text-label-secondary">
          {message}
        </p>
        <div className="mt-4 flex justify-end gap-2">
          <button
            ref={cancel}
            type="button"
            onClick={onCancel}
            className="rounded-md bg-fill px-2.5 py-[3px] text-label active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={onConfirm}
            className="rounded-md bg-negative px-2.5 py-[3px] text-white active:opacity-80 outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40"
          >
            {action}
          </button>
        </div>
      </div>
    </div>
  );
}

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
        title={`Delete “${title}”?`}
        message="Its folder moves to the Trash."
        action="Delete"
        busy={busy}
        onConfirm={() => void remove(confirm)}
        onCancel={() => setConfirm(null)}
      />
    ) : (
      <ConfirmDialog
        title={`“${title}” can’t be moved to the Trash`}
        message={`${sentence(confirm.reason)} Delete it permanently? You can’t undo this.`}
        action="Delete Permanently"
        busy={busy}
        onConfirm={() => void remove(confirm)}
        onCancel={() => setConfirm(null)}
      />
    );

  return { ask: (entry: SessionEntry) => setConfirm({ kind: "trash", entry }), dialog };
}
