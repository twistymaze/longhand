// The filter bar under the "Notes" heading (R19). Each control shows only
// when it gives information. A segmented control chooses "All", "General"
// or one other note type. It shows only when the type list has a type other
// than General (R1, R2). When the segments do not fit, one menu replaces the
// control. The Tags menu adds a tag to the filter. It shows only when a note
// has a tag (R5). Each chosen tag shows as a label with a control that
// removes it. While a filter is set, "Clear Filter" shows at the end of the
// bar (R12). The bar changes at once, with no motion: it is used many times
// a day.

import { type ReactNode, useLayoutEffect, useRef, useState } from "react";
import type { NoteTypeEntry, TagEntry } from "../bindings";
import { GENERAL, TagLabel } from "../lib/Labels";
import { type Segment, SegmentedControl } from "../lib/SegmentedControl";
import { Menu, type MenuEntry } from "../session/SessionMenu";
import { ChevronIcon } from "../settings/icons";
import { isFiltered, type NoteFilter, type TypeChoice, typesInUse } from "./filter";
import { TagFilterMenu } from "./TagFilterMenu";

/// With more types than this, the type choice is always one menu.
export const MAX_SEGMENT_TYPES = 6;

const ALL = "all";
const GENERAL_VALUE = "general";
const NAMED = "type:";

const valueOf = (choice: TypeChoice) =>
  choice.kind === "named" ? `${NAMED}${choice.name}` : choice.kind === "all" ? ALL : GENERAL_VALUE;

const choiceOf = (value: string): TypeChoice =>
  value === ALL
    ? { kind: "all" }
    : value === GENERAL_VALUE
      ? { kind: "general" }
      : { kind: "named", name: value.slice(NAMED.length) };

export function FilterBar({
  filter,
  types: allTypes,
  tags,
  onChange,
  onClear,
}: {
  filter: NoteFilter;
  /// The type list. The filter shows every entry (KTD9), also a type with
  /// no notes. An empty list is not read yet.
  types: NoteTypeEntry[];
  /// The tag list (`list_tags`). An empty list has no tags, or is not read
  /// yet.
  tags: TagEntry[];
  onChange: (filter: NoteFilter) => void;
  /// Sets the filter to no filter (R12).
  onClear: () => void;
}) {
  const bar = useRef<HTMLDivElement>(null);
  const tagsButton = useRef<HTMLButtonElement>(null);
  const natural = useRef<HTMLDivElement>(null);
  const clearCopy = useRef<HTMLButtonElement>(null);
  const [overflows, setOverflows] = useState(false);
  const showTypes = typesInUse(allTypes);
  const showTags = tags.length > 0;
  const filtered = isFiltered(filter);
  // General is a choice of its own, before the other types.
  const types = allTypes.filter((t) => !t.general);

  const segments: Segment<string>[] = [
    { value: ALL, label: "All" },
    { value: GENERAL_VALUE, label: GENERAL },
    ...types.map((t) => ({ value: `${NAMED}${t.name}`, label: t.name })),
  ];
  const value = valueOf(filter.type);
  const asMenu = types.length > MAX_SEGMENT_TYPES || overflows;

  // A copy of the control that is not shown gives its full width. The
  // control shows when that width fits beside the Tags button and the
  // Clear Filter button. The room for Clear Filter is always kept, so the
  // control does not change when a filter is set.
  useLayoutEffect(() => {
    const el = bar.current;
    if (!el || !showTypes || types.length > MAX_SEGMENT_TYPES) return;
    const measure = () => {
      const full = natural.current?.offsetWidth ?? 0;
      const others = (tagsButton.current?.offsetWidth ?? 0) + (clearCopy.current?.offsetWidth ?? 0);
      const room = el.clientWidth - others - 16;
      setOverflows(full > 0 && room > 0 && full > room);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [showTypes, showTags, types.map((t) => t.name).join("\n")]);

  if (!showTypes && !showTags && !filtered) return null;

  const setType = (type: TypeChoice) => onChange({ ...filter, type });
  const toggleTag = (key: string) =>
    onChange({
      ...filter,
      tags: filter.tags.includes(key) ? filter.tags.filter((k) => k !== key) : [...filter.tags, key],
    });
  const textOf = (key: string) => tags.find((t) => t.key === key)?.text ?? key;

  const typeLabel =
    filter.type.kind === "all" ? "All Types" : filter.type.kind === "general" ? GENERAL : filter.type.name;

  return (
    <div role="group" aria-label="Filter" className="shrink-0 px-[24px] pb-2 select-none">
      <div ref={bar} className="relative flex min-w-0 items-center gap-2">
        {showTypes &&
          (asMenu ? (
            <PopUpMenu
              label={`Note Type: ${typeLabel}`}
              content={typeLabel}
              items={segments.map((s) => ({
                key: s.value,
                label: s.value === ALL ? "All Types" : s.label,
                checked: s.value === value,
                radio: true,
                onSelect: () => setType(choiceOf(s.value)),
              }))}
            />
          ) : (
            <SegmentedControl
              kind="radio"
              label="Note Type"
              items={segments}
              value={value}
              onChange={(v) => setType(choiceOf(v))}
            />
          ))}
        {showTags && <TagFilterMenu ref={tagsButton} tags={tags} chosen={filter.tags} onToggle={toggleTag} />}
        {filtered && (
          <button type="button" onClick={onClear} className={`ml-auto ${clearClass}`}>
            Clear Filter
          </button>
        )}
        {showTypes && types.length <= MAX_SEGMENT_TYPES && (
          // The measured copies: not shown, not in the tab order, not read.
          <div aria-hidden inert className="pointer-events-none invisible absolute top-0 left-0 flex w-max">
            <div ref={natural}>
              <SegmentedControl kind="radio" label="" items={segments} value={value} onChange={() => {}} />
            </div>
            <button ref={clearCopy} type="button" tabIndex={-1} className={clearClass}>
              Clear Filter
            </button>
          </div>
        )}
      </div>
      {filter.tags.length > 0 && (
        <ul aria-label="Chosen tags" className="mt-2 flex flex-wrap items-center gap-1.5">
          {filter.tags.map((key) => (
            <li key={key} className="flex min-w-0 max-w-full">
              <TagLabel
                text={textOf(key)}
                onRemove={() => {
                  toggleTag(key);
                  // The removed label had the focus. WKWebView needs the
                  // focus set in code.
                  tagsButton.current?.focus();
                }}
              />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/// The look of Clear Filter: a plain text button with the accent colour,
/// like a link in a macOS filter bar.
const clearClass =
  "shrink-0 rounded-[6px] px-2 py-[3px] text-accent hover:bg-fill active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent";

/// A button that opens a menu under its left edge. The menu items close the
/// menu before they act. Escape gives the focus back to the button.
function PopUpMenu({
  label,
  content,
  items,
}: {
  label: string;
  content: ReactNode;
  items: MenuEntry[];
}) {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement | null>(null);
  // The menu closes on a press outside it, and the button is outside it.
  // A press on the button of an open menu only closes the menu.
  const wasOpen = useRef(false);
  const close = () => {
    setOpen(false);
    button.current?.focus();
  };
  return (
    <div className="relative inline-flex min-w-0">
      <button
        ref={button}
        type="button"
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        onPointerDown={() => {
          wasOpen.current = open;
        }}
        onClick={(e) => {
          // WKWebView does not focus a clicked button. Escape in the menu
          // gives the focus back here.
          e.currentTarget.focus();
          if (wasOpen.current) {
            wasOpen.current = false;
            return;
          }
          setOpen((o) => !o);
        }}
        className={`flex min-w-0 items-center gap-1 rounded-[6px] px-2.5 py-[3px] text-label hover:bg-fill-pressed active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent ${
          open ? "bg-fill-pressed" : "bg-fill"
        }`}
      >
        <span className="min-w-0 truncate">{content}</span>
        <ChevronIcon className="size-3 shrink-0 rotate-90 opacity-70" />
      </button>
      {open && (
        // The menu aligns to the right edge of its parent, so this box puts
        // that edge one menu width after the left edge of the button.
        <div className="pointer-events-none absolute top-0 left-0 h-full w-60">
          <Menu
            onClose={close}
            width="pointer-events-auto w-60"
            items={items.map((entry) =>
              "separator" in entry
                ? entry
                : {
                    ...entry,
                    onSelect: () => {
                      close();
                      entry.onSelect();
                    },
                  },
            )}
          />
        </div>
      )}
    </div>
  );
}
