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
  secondary,
  focus = "cancel",
  width = "w-[19rem]",
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
  /// A second action, with a button between Cancel and the action.
  secondary?: { label: string; onClick: () => void };
  /// The button that has the focus when the dialog opens.
  focus?: "cancel" | "confirm";
  /// The width class of the panel.
  width?: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const cancel = useRef<HTMLButtonElement>(null);
  const second = useRef<HTMLButtonElement>(null);
  const confirm = useRef<HTMLButtonElement>(null);
  const first = focus === "confirm" ? confirm : cancel;
  const cancelled = useRef(onCancel);
  cancelled.current = onCancel;
  const id = useId();
  useEffect(() => first.current?.focus(), []);
  // The keys go to the window. In WKWebView a click does not give a button
  // the focus, so the focus can be on the page body after a click in the
  // dialog.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        cancelled.current();
      }
      // Tab and Shift+Tab move between the buttons that can be used only.
      if (e.key === "Tab") {
        e.preventDefault();
        const list = [cancel, second, confirm]
          .map((b) => b.current)
          .filter((b): b is HTMLButtonElement => b !== null && !b.disabled);
        const at = list.indexOf(document.activeElement as HTMLButtonElement);
        const start = first.current && list.includes(first.current) ? first.current : list[0];
        const next = at < 0 ? start : list[(at + (e.shiftKey ? list.length - 1 : 1)) % list.length];
        next?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [first]);
  const plain =
    "rounded-md bg-fill px-2.5 py-[3px] text-label active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent";
  // With three buttons, the row wraps before a label does.
  const nowrap = secondary ? " whitespace-nowrap" : "";
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
        className={`fade-in flex max-h-[calc(82vh-1rem)] ${width} max-w-[calc(100vw-2rem)] flex-col rounded-xl bg-group p-4 shadow-[0_0_0_0.5px_var(--separator),0_12px_40px_rgb(0_0_0/0.25)]`}
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
        <div className={`mt-4 flex shrink-0 justify-end gap-2${secondary ? " flex-wrap" : ""}`}>
          <button ref={cancel} type="button" onClick={onCancel} className={plain + nowrap}>
            Cancel
          </button>
          {secondary && (
            <button ref={second} type="button" onClick={secondary.onClick} className={plain + nowrap}>
              {secondary.label}
            </button>
          )}
          <button
            ref={confirm}
            type="button"
            disabled={busy}
            onClick={onConfirm}
            className={`rounded-md px-2.5 py-[3px] text-white active:opacity-80 outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40 ${
              destructive ? "bg-negative" : "bg-accent"
            }${nowrap}`}
          >
            {action}
          </button>
        </div>
      </div>
    </div>
  );
}
