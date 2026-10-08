// "+ Tag": the control that adds a tag to a note (R16). A click shows a
// text field in place of the button. The field is a combobox after the
// ARIA pattern of `ModelPicker`: the list under it suggests the tags of the
// tag list that are not on the note, and a last row adds the typed text as
// a new tag. The picker only shows and filters the tags that it gets. The
// caller loads the tag list and saves the tag. The picker opens and closes
// at once, with no motion, so it keeps the attention budget of the
// recording page (R17).

import { autoUpdate, computePosition, flip, offset, shift } from "@floating-ui/dom";
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { TagLabel, tagKey, tagName } from "./Labels";

type Row = { kind: "tag" | "typed"; text: string };

export function TagPicker({
  tags,
  applied,
  onAdd,
  onOpen,
}: {
  /// The tag list (`list_tags`), in the order to show.
  tags: string[];
  /// The tags on the note. The list leaves them out, and the picker does
  /// not add one of them again.
  applied: string[];
  /// Adds a tag to the note: a listed spelling, or the typed text.
  onAdd: (tag: string) => void;
  /// Called when the picker opens, so the caller can load the tag list.
  onOpen?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(-1);
  const openerRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const popupRef = useRef<HTMLDivElement>(null);
  // Set when Escape closes the picker: the opener then takes the focus back.
  const returnFocus = useRef(false);
  const listId = useId();

  const onNote = new Set(applied.map(tagKey));
  const needle = tagKey(query);
  const available = tags.filter((t) => !onNote.has(tagKey(t)));
  const matches = available.filter((t) => tagKey(t).includes(needle));
  const listed = tags.find((t) => tagKey(t) === needle);
  const rows: Row[] = matches.map((text) => ({ kind: "tag", text }));
  if (needle && !listed && !onNote.has(needle)) rows.push({ kind: "typed", text: query.trim() });

  // WKWebView does not focus a clicked button, so the field and the opener
  // take the focus here and not from the click.
  useLayoutEffect(() => {
    if (open) inputRef.current?.focus();
    else if (returnFocus.current) {
      returnFocus.current = false;
      openerRef.current?.focus();
    }
  }, [open]);

  const close = (toOpener: boolean) => {
    returnFocus.current = toOpener;
    setOpen(false);
    setQuery("");
    setActive(-1);
  };

  const add = (text: string) => {
    if (!text || onNote.has(tagKey(text))) return;
    onAdd(text);
    setQuery("");
    setActive(-1);
  };

  const onEnter = () => {
    const row = rows[active];
    if (row) return add(row.text);
    // With no active row, Enter adds the listed spelling of an exact match,
    // or the typed text as a new tag.
    if (needle) add(listed ?? query.trim());
  };

  const move = (step: number) => {
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

  // The popup is in a portal, so a parent with overflow does not clip it.
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
        Object.assign(popup.style, { left: `${x}px`, top: `${y}px`, minWidth: `${reference.offsetWidth}px` });
      });
    };
    return autoUpdate(reference, popup, update);
  }, [visible]);

  const optionId = (index: number) => `${listId}-${index}`;

  if (!open) {
    return (
      <button
        ref={openerRef}
        type="button"
        aria-label="Add Tag"
        onClick={() => {
          onOpen?.();
          setOpen(true);
        }}
        className="rounded-[5px] px-[6px] py-[2px] text-[0.875rem] leading-[1.4] font-medium whitespace-nowrap text-label-secondary hover:bg-fill hover:text-label active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent"
      >
        + Tag
      </button>
    );
  }

  return (
    <>
      <input
        ref={inputRef}
        role="combobox"
        aria-label="Tag"
        aria-expanded={visible}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={visible && active >= 0 ? optionId(active) : undefined}
        className="h-[20px] w-44 rounded-[5px] bg-control px-[6px] text-[0.875rem] text-label shadow-[inset_0_0_0_1px_var(--control-border)] outline-offset-0 placeholder:text-label-tertiary focus:outline-2 focus:outline-accent"
        value={query}
        placeholder="Tag name"
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        autoComplete="off"
        onChange={(e) => {
          setQuery(e.target.value);
          setActive(-1);
        }}
        onBlur={() => close(false)}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            move(e.key === "ArrowDown" ? 1 : -1);
          } else if (e.key === "Enter") {
            e.preventDefault();
            onEnter();
          } else if (e.key === "Escape") {
            // The Escape closes the picker only, not the page around it.
            e.preventDefault();
            e.stopPropagation();
            close(true);
          }
        }}
      />
      {visible &&
        createPortal(
          <div
            ref={popupRef}
            id={listId}
            role="listbox"
            aria-label="Tags"
            className="fixed z-50 max-h-64 max-w-[20rem] overflow-y-auto rounded-lg bg-group p-1 text-label shadow-[0_0_0_0.5px_var(--separator),0_4px_16px_rgb(0_0_0/0.14)]"
            style={{ top: 0, left: 0 }}
            // A press on the popup keeps the focus in the field, so the field
            // does not blur and close the picker before the click.
            onMouseDown={(e) => e.preventDefault()}
          >
            {rows.map((row, index) => (
              <div
                key={`${row.kind}:${row.text}`}
                id={optionId(index)}
                data-index={index}
                role="option"
                aria-selected={index === active}
                aria-label={row.kind === "typed" ? `Add "${row.text}"` : tagName(row.text)}
                className={`flex h-7 cursor-default items-center rounded-md px-1.5 select-none ${
                  index === active ? "bg-fill" : ""
                }`}
                onMouseMove={() => setActive(index)}
                onClick={() => add(row.text)}
              >
                {row.kind === "typed" ? (
                  <span className="flex min-w-0 items-center gap-1.5 text-label-secondary">
                    <span className="shrink-0">Add</span>
                    <TagLabel text={row.text} />
                  </span>
                ) : (
                  <TagLabel text={row.text} />
                )}
              </div>
            ))}
          </div>,
          document.body,
        )}
    </>
  );
}
