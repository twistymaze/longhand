// "New Type…": a small dialog under the control that chooses a type (R3).
// It has one name field, Create and Cancel. Create adds the type to the
// shared list. A refused name shows its reason in the dialog, and the
// dialog stays open. Escape, Cancel or a click outside closes it. The
// dialog opens and closes at once, with no motion.

import { useEffect, useId, useRef, useState } from "react";
import { commands, type NoteTypeList } from "../bindings";
import { sentence } from "../processing/describe";
import { fieldClass } from "../settings/ui";

export function NewTypeDialog({
  onCreated,
  onCancel,
  className = "top-full left-0 mt-1",
}: {
  /// The type is in the list. `name` is the name of the new type as the
  /// list has it.
  onCreated: (name: string, list: NoteTypeList) => void;
  onCancel: () => void;
  /// The position classes of the dialog in its `relative` parent.
  className?: string;
}) {
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const panel = useRef<HTMLDivElement>(null);
  const field = useRef<HTMLInputElement>(null);
  const cancelled = useRef(onCancel);
  cancelled.current = onCancel;
  const errorId = useId();

  // The field takes the focus. WKWebView does not move the focus on the
  // click that opened the dialog.
  useEffect(() => {
    field.current?.focus();
    const outside = (event: PointerEvent) => {
      if (!panel.current?.contains(event.target as Node)) cancelled.current();
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, []);

  const create = async () => {
    const text = name.trim();
    if (!text || busy) return;
    setBusy(true);
    try {
      const result = await commands.addNoteType(text);
      if (result.status === "error") {
        setError(sentence(result.error));
        field.current?.focus();
        return;
      }
      const key = text.toLowerCase();
      const made = result.data.types.find((t) => t.name.toLowerCase() === key)?.name ?? text;
      onCreated(made, result.data);
    } finally {
      setBusy(false);
    }
  };

  const button =
    "rounded-md px-2.5 py-[3px] whitespace-nowrap outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent";
  return (
    <div
      ref={panel}
      role="dialog"
      aria-label="New Type"
      className={`absolute z-20 w-64 rounded-lg bg-group p-3 shadow-[0_0_0_0.5px_var(--separator),0_6px_20px_rgb(0_0_0/0.18)] ${className}`}
      onKeyDown={(e) => {
        // Escape closes the dialog only, not the page around it.
        e.stopPropagation();
        if (e.key === "Escape") {
          e.preventDefault();
          onCancel();
        }
      }}
    >
      <input
        ref={field}
        aria-label="Type name"
        aria-invalid={error !== null}
        aria-describedby={error ? errorId : undefined}
        value={name}
        placeholder="Type name"
        spellCheck={false}
        autoComplete="off"
        className={`w-full text-[0.85rem] ${fieldClass}`}
        onChange={(e) => {
          setName(e.target.value);
          setError(null);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            void create();
          }
        }}
      />
      {error && (
        <p id={errorId} role="alert" className="mt-1.5 text-[0.8rem] text-negative">
          {error}
        </p>
      )}
      <div className="mt-3 flex justify-end gap-2">
        <button type="button" onClick={onCancel} className={`${button} bg-fill text-label active:bg-fill-pressed`}>
          Cancel
        </button>
        <button
          type="button"
          disabled={!name.trim() || busy}
          onClick={() => void create()}
          className={`${button} bg-accent text-white active:opacity-80 disabled:opacity-40`}
        >
          Create
        </button>
      </div>
    </div>
  );
}
