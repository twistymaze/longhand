// The filter bar under the "Notes" heading (R19). A segmented control
// chooses "All", "No type" or one note type. "No type" shows only when a
// listed note has no type. When the segments do not fit,
// one menu replaces the control. The Tags menu adds a tag to the filter, and
// each chosen tag shows as a label with a control that removes it. The bar
// changes at once, with no motion: it is used many times a day.

import { type ReactNode, type Ref, type RefObject, useLayoutEffect, useRef, useState } from "react";
import type { NoteTypeEntry, TagEntry } from "../bindings";
import { TagLabel } from "../lib/Labels";
import { type Segment, SegmentedControl } from "../lib/SegmentedControl";
import { Menu, type MenuEntry } from "../session/SessionMenu";
import { ChevronIcon } from "../settings/icons";
import type { NoteFilter, TypeChoice } from "./filter";

/// With more types than this, the type choice is always one menu.
export const MAX_SEGMENT_TYPES = 6;

const ALL = "all";
const NO_TYPE = "no_type";
const NAMED = "type:";

const valueOf = (choice: TypeChoice) =>
  choice.kind === "named" ? `${NAMED}${choice.name}` : choice.kind === "all" ? ALL : NO_TYPE;

const choiceOf = (value: string): TypeChoice =>
  value === ALL ? { kind: "all" } : value === NO_TYPE ? { kind: "no_type" } : { kind: "named", name: value.slice(NAMED.length) };

export function FilterBar({
  filter,
  types,
  noType,
  tags,
  onChange,
}: {
  filter: NoteFilter;
  /// The effective type list. The filter shows every entry (KTD9).
  types: NoteTypeEntry[];
  /// At least one listed note has no type, so "No type" is a choice.
  noType: boolean;
  /// The tag list (`list_tags`).
  tags: TagEntry[];
  onChange: (filter: NoteFilter) => void;
}) {
  const bar = useRef<HTMLDivElement>(null);
  const tagsButton = useRef<HTMLButtonElement>(null);
  const natural = useRef<HTMLDivElement>(null);
  const [overflows, setOverflows] = useState(false);

  const segments: Segment<string>[] = [
    { value: ALL, label: "All" },
    ...(noType ? [{ value: NO_TYPE, label: "No type" }] : []),
    ...types.map((t) => ({ value: `${NAMED}${t.name}`, label: t.name })),
  ];
  const value = valueOf(filter.type);
  const asMenu = types.length > MAX_SEGMENT_TYPES || overflows;

  // A copy of the control that is not shown gives its full width. The
  // control shows when that width fits beside the Tags button.
  useLayoutEffect(() => {
    const el = bar.current;
    if (!el || types.length > MAX_SEGMENT_TYPES) return;
    const measure = () => {
      const full = natural.current?.offsetWidth ?? 0;
      const room = el.clientWidth - (tagsButton.current?.offsetWidth ?? 0) - 8;
      setOverflows(full > 0 && room > 0 && full > room);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [noType, types.map((t) => t.name).join("\n")]);

  const setType = (type: TypeChoice) => onChange({ ...filter, type });
  const toggleTag = (key: string) =>
    onChange({
      ...filter,
      tags: filter.tags.includes(key) ? filter.tags.filter((k) => k !== key) : [...filter.tags, key],
    });
  const textOf = (key: string) => tags.find((t) => t.key === key)?.text ?? key;

  const typeLabel =
    filter.type.kind === "all" ? "All Types" : filter.type.kind === "no_type" ? "No type" : filter.type.name;

  return (
    <div className="shrink-0 px-[24px] pb-2 select-none">
      <div ref={bar} className="relative flex min-w-0 items-center gap-2">
        {asMenu ? (
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
        )}
        <PopUpMenu
          ref={tagsButton}
          label="Tags"
          content="Tags"
          items={
            tags.length === 0
              ? [{ label: "No tags", disabled: true, reason: "No note has a tag.", onSelect: () => {} }]
              : tags.map((t) => ({
                  key: t.key,
                  label: t.text,
                  checked: filter.tags.includes(t.key),
                  onSelect: () => toggleTag(t.key),
                }))
          }
        />
        {types.length <= MAX_SEGMENT_TYPES && (
          // The measured copy: not shown, not in the tab order, not read.
          <div ref={natural} aria-hidden inert className="pointer-events-none invisible absolute top-0 left-0 w-max">
            <SegmentedControl kind="radio" label="" items={segments} value={value} onChange={() => {}} />
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

/// A button that opens a menu under its left edge. The menu items close the
/// menu before they act. Escape gives the focus back to the button.
function PopUpMenu({
  label,
  content,
  items,
  ref,
}: {
  label: string;
  content: ReactNode;
  items: MenuEntry[];
  ref?: Ref<HTMLButtonElement>;
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
        ref={(el) => {
          button.current = el;
          if (typeof ref === "function") ref(el);
          else if (ref) (ref as RefObject<HTMLButtonElement | null>).current = el;
        }}
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
