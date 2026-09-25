// A question before a change that cannot be undone, such as a delete. Cancel
// has the focus, so Return does not confirm by accident.

import { type ReactNode, useEffect, useId, useRef } from "react";

export function ConfirmDialog({
  title,
  message,
  action,
  busy = false,
  destructive = true,
  onConfirm,
  onCancel,
}: {
  title: string;
  message: ReactNode;
  action: string;
  /// The confirmed change is in progress. The confirm button is disabled.
  busy?: boolean;
  /// The action removes something, so its button is red. An action that
  /// replaces something uses the accent colour.
  destructive?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const cancel = useRef<HTMLButtonElement>(null);
  const id = useId();
  useEffect(() => cancel.current?.focus(), []);
  return (
    <div className="fixed inset-0 z-20 flex items-start justify-center bg-black/20 pt-[18vh]">
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={`${id}-title`}
        aria-describedby={`${id}-message`}
        onKeyDown={(e) => {
          if (e.key === "Escape") onCancel();
        }}
        className="fade-in w-[19rem] rounded-xl bg-group p-4 shadow-[0_0_0_0.5px_var(--separator),0_12px_40px_rgb(0_0_0/0.25)]"
      >
        <h2 id={`${id}-title`} className="font-semibold break-words">
          {title}
        </h2>
        <div id={`${id}-message`} className="mt-1 grid gap-1.5 text-[0.85rem] text-label-secondary">
          {typeof message === "string" ? <p>{message}</p> : message}
        </div>
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
            className={`rounded-md px-2.5 py-[3px] text-white active:opacity-80 outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40 ${
              destructive ? "bg-negative" : "bg-accent"
            }`}
          >
            {action}
          </button>
        </div>
      </div>
    </div>
  );
}
