// The Tags menu of the Home filter bar (R6 to R11). A press on the button
// opens a popup with a text field and the tag list. The field is a
// combobox after the ARIA pattern of `TagPicker`. It keeps the focus while
// the menu is open. The list is a listbox with more than one choice: a
// chosen tag shows a check mark. A choice does not close the menu, so the
// user can choose several tags in turn. The tags with no scope come first,
// then each scope (KTD6). Each row shows the tag as the label that Home rows
// show. The popup is in a portal, because the Home pane has overflow hidden
// and would cut off a wide menu (KTD3). It opens and closes at once, with no
// motion: it is used many times a day.

import { autoUpdate, computePosition, flip, offset, shift } from "@floating-ui/dom";
import { type Ref, type RefObject, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { TagEntry } from "../bindings";
import { TagLabel, tagKey, tagName } from "../lib/Labels";
import { ChevronIcon } from "../settings/icons";
import { menuOrder } from "./filter";

export function TagFilterMenu({
  tags,
  chosen,
  onToggle,
  ref,
}: {
  /// The tag list (`list_tags`).
  tags: TagEntry[];
  /// The keys of the chosen tags.
  chosen: string[];
  /// Chooses or clears the tag with this key.
  onToggle: (key: string) => void;
  /// The Tags button, so the bar can give it the focus.
  ref?: Ref<HTMLButtonElement>;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(-1);
  const button = useRef<HTMLButtonElement | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const popupRef = useRef<HTMLDivElement>(null);
  // Set when Escape closes the menu: the button then takes the focus back.
  const returnFocus = useRef(false);
  // The menu closes on a press outside it, and the button is outside it.
  // A press on the button of an open menu only closes the menu.
  const wasOpen = useRef(false);
  const listId = useId();

  const needle = tagKey(query);
  const rows = menuOrder(tags).filter((t) => t.key.includes(needle));
  const selected = new Set(chosen);

  // WKWebView does not focus a clicked button, so the field and the button
  // take the focus here and not from the click.
  useLayoutEffect(() => {
    if (open) inputRef.current?.focus();
    else if (returnFocus.current) {
      returnFocus.current = false;
      button.current?.focus();
    }
  }, [open]);

  // A sync can change the tag list while the menu is open. The rows follow
  // the list, and no row stays active.
  const tagKeys = tags.map((t) => t.key).join("\n");
  useEffect(() => setActive(-1), [tagKeys]);

  const close = (toButton: boolean) => {
    returnFocus.current = toButton;
    setOpen(false);
    setQuery("");
    setActive(-1);
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

  // The popup is under the left edge of the button, and stays in the window.
  useLayoutEffect(() => {
    const reference = button.current;
    const popup = popupRef.current;
    if (!open || !reference || !popup) return;
    const update = () => {
      void computePosition(reference, popup, {
        placement: "bottom-start",
        strategy: "fixed",
        middleware: [offset(4), flip(), shift({ padding: 8 })],
      }).then(({ x, y }) => {
        Object.assign(popup.style, { left: `${x}px`, top: `${y}px` });
      });
    };
    return autoUpdate(reference, popup, update);
  }, [open]);

  const optionId = (index: number) => `${listId}-${index}`;

  return (
    <div className="relative inline-flex min-w-0">
      <button
        ref={(el) => {
          button.current = el;
          if (typeof ref === "function") ref(el);
          else if (ref) (ref as RefObject<HTMLButtonElement | null>).current = el;
        }}
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        onPointerDown={() => {
          wasOpen.current = open;
        }}
        onClick={(e) => {
          // WKWebView does not focus a clicked button.
          e.currentTarget.focus();
          if (wasOpen.current) {
            wasOpen.current = false;
            if (open) close(false);
            return;
          }
          setOpen((o) => !o);
        }}
        className={`flex min-w-0 items-center gap-1 rounded-[6px] px-2.5 py-[3px] text-label hover:bg-fill-pressed active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent ${
          open ? "bg-fill-pressed" : "bg-fill"
        }`}
      >
        <span className="min-w-0 truncate">Tags</span>
        <ChevronIcon className="size-3 shrink-0 rotate-90 opacity-70" />
      </button>
      {open &&
        createPortal(
          <div
            ref={popupRef}
            className="fixed z-50 flex max-h-[min(24rem,calc(100vh-8rem))] w-max min-w-[240px] max-w-[360px] flex-col rounded-md bg-group p-1 text-label shadow-[0_0_0_0.5px_var(--separator),0_6px_20px_rgb(0_0_0/0.18)]"
            style={{ top: 0, left: 0 }}
            // A press on the popup keeps the focus in the field, so the field
            // does not blur and close the menu before the click.
            onMouseDown={(e) => e.preventDefault()}
          >
            <input
              ref={inputRef}
              role="combobox"
              aria-label="Filter Tags"
              aria-expanded
              aria-controls={listId}
              aria-autocomplete="list"
              aria-activedescendant={active >= 0 ? optionId(active) : undefined}
              className="mb-1 h-[22px] w-full shrink-0 rounded-[5px] bg-control px-[6px] text-label shadow-[inset_0_0_0_1px_var(--control-border)] outline-offset-0 placeholder:text-label-tertiary focus:outline-2 focus:outline-accent"
              value={query}
              placeholder="Filter"
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
                  const row = rows[active];
                  if (row) onToggle(row.key);
                } else if (e.key === "Escape") {
                  // The Escape closes the menu only, not the page around it.
                  e.preventDefault();
                  e.stopPropagation();
                  close(true);
                }
              }}
            />
            {rows.length === 0 && (
              <p role="status" className="px-2 py-1 text-label-secondary">
                No matching tags
              </p>
            )}
            <div
              id={listId}
              role="listbox"
              aria-label="Tags"
              aria-multiselectable="true"
              className="min-h-0 overflow-y-auto overscroll-contain"
            >
              {rows.map((tag, index) => (
                <div
                  key={tag.key}
                  id={optionId(index)}
                  data-index={index}
                  role="option"
                  aria-selected={selected.has(tag.key)}
                  aria-label={tagName(tag.text)}
                  title={tag.text}
                  className={`flex h-7 cursor-default items-center gap-1.5 rounded-[4px] px-1.5 select-none ${
                    index === active ? "bg-fill" : ""
                  }`}
                  onMouseMove={() => setActive(index)}
                  onClick={() => onToggle(tag.key)}
                >
                  <span aria-hidden className="flex w-3 shrink-0 justify-center text-accent">
                    {selected.has(tag.key) && (
                      <svg viewBox="0 0 12 12" className="size-3" fill="none" stroke="currentColor" strokeWidth="1.75">
                        <path d="M2.5 6.5 5 9l4.5-6" strokeLinecap="round" strokeLinejoin="round" />
                      </svg>
                    )}
                  </span>
                  <span className="flex min-w-0">
                    <TagLabel text={tag.text} />
                  </span>
                </div>
              ))}
            </div>
          </div>,
          document.body,
        )}
    </div>
  );
}
