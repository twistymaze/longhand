// A question before a change that cannot be undone, such as a delete or a
// replacement of the transcript. Cancel
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
  const confirm = useRef<HTMLButtonElement>(null);
  const id = useId();
  useEffect(() => cancel.current?.focus(), []);
  // The panel is never wider or taller than the window, also at the minimum
  // window size with 200% zoom. The message scrolls, and the buttons stay
  // visible.
  return (
    <div className="fixed inset-0 z-20 flex items-start justify-center bg-black/20 px-4 pt-[18vh] pb-4">
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={`${id}-title`}
        aria-describedby={`${id}-message`}
        onKeyDown={(e) => {
          if (e.key === "Escape") onCancel();
          // Tab and Shift+Tab move between the two buttons only.
          if (e.key === "Tab") {
            e.preventDefault();
            (document.activeElement === cancel.current && !confirm.current?.disabled ? confirm : cancel).current?.focus();
          }
        }}
        className="fade-in flex max-h-[calc(82vh-1rem)] w-[19rem] max-w-[calc(100vw-2rem)] flex-col rounded-xl bg-group p-4 shadow-[0_0_0_0.5px_var(--separator),0_12px_40px_rgb(0_0_0/0.25)]"
      >
        <h2 id={`${id}-title`} className="shrink-0 font-semibold [overflow-wrap:anywhere]">
          {title}
        </h2>
        <div
          id={`${id}-message`}
          className="mt-1 grid min-h-0 grid-cols-[minmax(0,1fr)] gap-1.5 overflow-y-auto text-[0.85rem] text-label-secondary [overflow-wrap:anywhere]"
        >
          {typeof message === "string" ? <p>{message}</p> : message}
        </div>
        <div className="mt-4 flex shrink-0 justify-end gap-2">
          <button
            ref={cancel}
            type="button"
            onClick={onCancel}
            className="rounded-md bg-fill px-2.5 py-[3px] text-label active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent"
          >
            Cancel
          </button>
          <button
            ref={confirm}
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
