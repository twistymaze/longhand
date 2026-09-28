import { type ReactNode, useEffect, useId, useRef } from "react";
import { Button } from "../settings/ui";

export function RecognitionDialog({
  title,
  children,
  action,
  busy,
  disabled,
  onConfirm,
  onCancel,
}: {
  title: string;
  children: ReactNode;
  action: string;
  busy: boolean;
  disabled?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const id = useId();
  const panel = useRef<HTMLDivElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    cancel.current?.focus();
    return () => previous?.focus();
  }, []);
  return (
    <div className="fixed inset-0 z-20 flex items-start justify-center overflow-y-auto bg-black/20 px-4 py-[8vh]">
      <div
        ref={panel}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={id}
        className="w-full max-w-[36rem] min-w-0 rounded-xl bg-group p-4 shadow-xl"
        onKeyDown={(event) => {
          if (event.key === "Escape" && !busy) {
            event.stopPropagation();
            onCancel();
          }
          if (event.key !== "Tab") return;
          const controls = [
            ...(panel.current?.querySelectorAll<HTMLElement>(
              'button:not([disabled]),select:not([disabled]),input:not([disabled]),summary,[tabindex="0"]',
            ) ?? []),
          ];
          const first = controls[0],
            last = controls[controls.length - 1];
          if (event.shiftKey && document.activeElement === first) {
            event.preventDefault();
            last?.focus();
          } else if (!event.shiftKey && document.activeElement === last) {
            event.preventDefault();
            first?.focus();
          }
        }}
      >
        <h2 id={id} className="font-semibold">
          {title}
        </h2>
        {children}
        <div className="mt-3 flex justify-end gap-2">
          <Button ref={cancel} disabled={busy} onClick={onCancel}>
            Cancel
          </Button>
          <Button prominent disabled={busy || disabled} onClick={onConfirm}>
            {action}
          </Button>
        </div>
      </div>
    </div>
  );
}
