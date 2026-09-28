// A question before a change that cannot be undone, such as a delete. Cancel
// has the focus, so Return does not confirm by accident.
//
// The dialog can offer a second action between Cancel and the primary action,
// for a choice between two changes (for example "Delete unused" and "Delete
// all"). Each action can be off with a reason line that tells why.

import { type ReactNode, useEffect, useId, useRef } from "react";

export type SecondaryAction = {
  label: string;
  onClick: () => void;
  /// The action is off. `reason` tells why.
  disabled?: boolean;
  /// The line that shows while the action is off.
  reason?: string;
};

export function ConfirmDialog({
  title,
  message,
  action,
  busy = false,
  destructive = true,
  actionDisabled = false,
  actionReason,
  secondary,
  onConfirm,
  onCancel,
}: {
  title: string;
  message: ReactNode;
  action: string;
  /// The confirmed change is in progress. The primary action and the second
  /// action are disabled.
  busy?: boolean;
  /// The action removes something, so its button is red. An action that
  /// replaces something uses the accent colour.
  destructive?: boolean;
  /// The primary action is off. `actionReason` tells why.
  actionDisabled?: boolean;
  /// The line that shows while the primary action is off.
  actionReason?: string;
  /// A second action, between Cancel and the primary action.
  secondary?: SecondaryAction;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const cancel = useRef<HTMLButtonElement>(null);
  const id = useId();
  // Focus Cancel in code: WKWebView does not focus a clicked button, so the
  // focus must not depend on a click.
  useEffect(() => cancel.current?.focus(), []);

  const secondaryReason = secondary?.disabled && secondary.reason ? secondary.reason : undefined;
  const primaryReason = actionDisabled && actionReason ? actionReason : undefined;
  const secondaryReasonId = `${id}-secondary-reason`;
  const primaryReasonId = `${id}-action-reason`;

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
        {(secondaryReason || primaryReason) && (
          <div className="mt-2 grid gap-0.5 text-[0.8rem] text-label-secondary">
            {secondaryReason && <p id={secondaryReasonId}>{secondaryReason}</p>}
            {primaryReason && <p id={primaryReasonId}>{primaryReason}</p>}
          </div>
        )}
        <div className="mt-4 flex flex-wrap justify-end gap-2">
          <button
            ref={cancel}
            type="button"
            onClick={onCancel}
            className="rounded-md bg-fill px-2.5 py-[3px] text-label active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent"
          >
            Cancel
          </button>
          {secondary && (
            <button
              type="button"
              disabled={busy || secondary.disabled}
              aria-describedby={secondaryReason ? secondaryReasonId : undefined}
              onClick={secondary.onClick}
              className={`rounded-md bg-fill px-2.5 py-[3px] active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40 ${
                destructive ? "text-negative" : "text-label"
              }`}
            >
              {secondary.label}
            </button>
          )}
          <button
            type="button"
            disabled={busy || actionDisabled}
            aria-describedby={primaryReason ? primaryReasonId : undefined}
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
