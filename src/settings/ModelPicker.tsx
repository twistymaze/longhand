// A searchable model field after the ARIA combobox pattern: an input with
// role "combobox", a popup with role "listbox", and aria-activedescendant
// for the active row. The picker only shows and filters the ids that it
// gets. The caller loads the list and saves the chosen model.

import { autoUpdate, computePosition, flip, offset, shift } from "@floating-ui/dom";
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { fieldClass } from "./ui";

type Row = { kind: "model" | "typed"; model: string };

export function ModelPicker({
  value,
  models,
  onCommit,
  label = "Model",
  width = "w-64",
}: {
  /// The saved model, or "" when no model is saved.
  value: string;
  /// The ids that the server lists, in the order to show.
  models: string[];
  onCommit: (model: string) => void;
  label?: string;
  width?: string;
}) {
  const [open, setOpen] = useState(false);
  // The typed text, or null when the user did not type since the popup
  // opened. With null, the input shows the saved model and the popup shows
  // every id.
  const [query, setQuery] = useState<string | null>(null);
  const [active, setActive] = useState(-1);
  const inputRef = useRef<HTMLInputElement>(null);
  const popupRef = useRef<HTMLDivElement>(null);
  const listId = useId();

  const text = query?.trim() ?? "";
  const needle = text.toLowerCase();
  const matches = query === null ? models : models.filter((m) => m.toLowerCase().includes(needle));
  const exact = models.includes(text);
  const rows: Row[] = matches.map((model) => ({ kind: "model", model }));
  if (text && !exact) rows.push({ kind: "typed", model: text });

  const close = () => {
    setOpen(false);
    setQuery(null);
    setActive(-1);
  };

  const commit = (model: string) => {
    close();
    if (model !== value) onCommit(model);
  };

  const onEnter = () => {
    if (!open) return;
    const row = rows[active];
    if (row) return commit(row.model);
    // With no active row, Enter saves an exact match or a name that matches
    // no id. A part of a name that matches some ids does nothing.
    if (text && (exact || matches.length === 0)) return commit(text);
    if (!text) close();
  };

  const move = (step: number) => {
    if (!open) {
      setOpen(true);
      return;
    }
    if (rows.length === 0) return;
    setActive((at) => (at < 0 ? (step > 0 ? 0 : rows.length - 1) : Math.min(rows.length - 1, Math.max(0, at + step))));
  };

  // Keep the active row visible when the keys move it past the edge.
  useEffect(() => {
    if (active < 0) return;
    const row = popupRef.current?.querySelector(`[data-index="${active}"]`);
    (row as HTMLElement | null)?.scrollIntoView?.({ block: "nearest" });
  }, [active]);

  const visible = open && rows.length > 0;

  // The popup is outside the Settings group, whose overflow clips. It sits
  // under the input and keeps at least the input's width.
  useLayoutEffect(() => {
    const reference = inputRef.current;
    const popup = popupRef.current;
    if (!visible || !reference || !popup) return;
    const update = () => {
      void computePosition(reference, popup, {
        placement: "bottom-start",
        strategy: "fixed",
        middleware: [offset(4), flip(), shift({ padding: 8 })],
      }).then(({ x, y }) => {
        Object.assign(popup.style, {
          left: `${x}px`,
          top: `${y}px`,
          minWidth: `${reference.offsetWidth}px`,
        });
      });
    };
    return autoUpdate(reference, popup, update);
  }, [visible]);

  const optionId = (index: number) => `${listId}-${index}`;
  const unlisted = value !== "" && models.length > 0 && !models.includes(value);

  return (
    <div className="flex flex-col items-end gap-1">
      <div className={`relative ${width}`}>
        <input
          ref={inputRef}
          role="combobox"
          aria-label={label}
          aria-expanded={visible}
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={visible && active >= 0 ? optionId(active) : undefined}
          className={`${fieldClass} w-full pr-6`}
          value={query ?? value}
          placeholder="Choose a model"
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          autoComplete="off"
          onChange={(e) => {
            setQuery(e.target.value);
            setActive(-1);
            setOpen(true);
          }}
          onMouseDown={() => {
            if (document.activeElement === inputRef.current) setOpen((o) => !o);
          }}
          onFocus={() => setOpen(true)}
          onBlur={close}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown" || e.key === "ArrowUp") {
              e.preventDefault();
              move(e.key === "ArrowDown" ? 1 : -1);
            } else if (e.key === "Enter") {
              e.preventDefault();
              onEnter();
            } else if (e.key === "Escape" && (open || query !== null)) {
              // The Escape closes the popup only, not the window around it.
              e.preventDefault();
              e.stopPropagation();
              close();
            }
          }}
        />
        {/* The chevron shows that the field opens a list, as a macOS combo
            box does. A press keeps the focus in the input and opens or closes
            the list. */}
        <button
          type="button"
          tabIndex={-1}
          aria-label="Show models"
          className="absolute inset-y-0 right-0 flex w-6 items-center justify-center text-label-secondary"
          onMouseDown={(e) => {
            e.preventDefault();
            if (document.activeElement === inputRef.current) setOpen((o) => !o);
            else inputRef.current?.focus();
          }}
        >
          <svg viewBox="0 0 10 6" className="h-1.5 w-2.5" fill="none" stroke="currentColor" strokeWidth="1.5">
            <path d="M1 1l4 4 4-4" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </button>
      </div>
      {unlisted && (
        <p className="text-[0.8rem] text-label-secondary">This server doesn't list this model</p>
      )}
      {visible &&
        createPortal(
          <div
            ref={popupRef}
            id={listId}
            role="listbox"
            aria-label={label}
            className="fixed z-50 max-h-64 max-w-[24rem] overflow-y-auto rounded-lg bg-group p-1 text-label shadow-[0_0_0_0.5px_var(--separator),0_4px_16px_rgb(0_0_0/0.14)]"
            style={{ top: 0, left: 0 }}
            // A press on the popup keeps the focus in the input, so the input
            // does not blur and discard the choice before the click.
            onMouseDown={(e) => e.preventDefault()}
          >
            {rows.map((row, index) => (
              <div
                key={`${row.kind}:${row.model}`}
                id={optionId(index)}
                data-index={index}
                role="option"
                aria-selected={row.kind === "model" && row.model === value}
                className={`flex h-7 cursor-default items-center gap-2 rounded-md px-2 select-none ${
                  index === active ? "bg-fill" : ""
                }`}
                onMouseMove={() => setActive(index)}
                onClick={() => commit(row.model)}
              >
                <span aria-hidden className="flex w-3 shrink-0 justify-center text-accent">
                  {row.kind === "model" && row.model === value && (
                    <svg viewBox="0 0 12 12" className="size-3" fill="none" stroke="currentColor" strokeWidth="1.75">
                      <path d="M2.5 6.5 5 9l4.5-6" strokeLinecap="round" strokeLinejoin="round" />
                    </svg>
                  )}
                </span>
                <span className={`min-w-0 flex-1 truncate ${row.kind === "typed" ? "text-label-secondary" : ""}`}>
                  {row.kind === "typed" ? `Use "${row.model}"` : row.model}
                </span>
              </div>
            ))}
          </div>,
          document.body,
        )}
    </div>
  );
}
