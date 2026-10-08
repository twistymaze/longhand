// A session's actions: the menu under a Home row's "…" button and the
// session page's "…" menu (R7, R19). The menu and its submenus open and
// close at once, with no motion: they are used many times a day.

import { type KeyboardEvent, useEffect, useRef, useState } from "react";
import type { RemoteView } from "../bindings";
import { CheckIcon, ChevronIcon } from "../settings/icons";

export type MenuItem = {
  label: string;
  onSelect: () => void;
  /// The React key, when two items can have the same label (two
  /// microphones with one name).
  key?: string;
  disabled?: boolean;
  /// Why the item is disabled, as a tooltip.
  reason?: string;
  /// A check mark. `true` or `false` makes the item a check box, as
  /// "Record Computer Audio" is.
  checked?: boolean;
  /// With `checked`, the item is one choice of a group: a radio item.
  radio?: boolean;
  /// Items that open to the side of this item. Then `onSelect` is not used.
  submenu?: MenuEntry[];
};

/// A line between two groups of items. It is not focusable.
export type MenuSeparator = { separator: true; key: string };

export type MenuEntry = MenuItem | MenuSeparator;

/// A separator with a React key that is unique in its menu.
export const separator = (key: string): MenuSeparator => ({ separator: true, key });

const isSeparator = (entry: MenuEntry): entry is MenuSeparator => "separator" in entry;

/// States in which the store refuses a delete (`delete_refusal`, R8).
export function canDelete(state: string): boolean {
  return !["capturing", "interrupted", "queued", "processing"].includes(state);
}

/// The short status of a note that another computer works on, for a Home
/// row and the preview (R20, R23).
export const remoteLabels: Record<RemoteView, string> = {
  in_use: "In use on another computer",
  not_finished: "Not finished on another computer",
};

/// The full status for the status line of the note page. The marker of
/// another computer cannot tell a recording from processing, so one
/// sentence says both.
export const remoteSentences: Record<RemoteView, string> = {
  in_use: "Recording or processing on another computer",
  not_finished: "Not finished on another computer",
};

/// Why an action that needs the audio waits, for a note that another
/// computer works on (R21, R23).
export const remoteReasons: Record<RemoteView, string> = {
  in_use: "Wait until the other computer is done with this note.",
  not_finished: "Another computer did not finish this note.",
};

const itemClass =
  "flex w-full items-center gap-1 rounded-[4px] px-2.5 py-[3px] text-left text-label enabled:hover:bg-accent enabled:hover:text-white focus-visible:bg-accent focus-visible:text-white focus-visible:outline-none disabled:text-label-tertiary";

const roleOf = (entry: MenuItem) =>
  entry.checked === undefined ? "menuitem" : entry.radio ? "menuitemradio" : "menuitemcheckbox";

const keyOf = (entry: MenuEntry) => (isSeparator(entry) ? `separator:${entry.key}` : (entry.key ?? entry.label));

/// The open menu, below its button and aligned to its right edge, or a
/// submenu to the left of its item. The first enabled item has the focus,
/// the arrow keys move it, and Escape or a click outside closes the menu.
/// The right arrow opens a submenu, and the left arrow closes it.
/// A menu with no submenu has a height limit and scrolls, so a long list
/// of note types stays in the window. A menu with a submenu does not
/// scroll, because the scroll area would cut off the submenu.
export function Menu({
  items,
  onClose,
  submenu = false,
  focusFirst = true,
  width = "w-52",
}: {
  items: MenuEntry[];
  onClose: () => void;
  submenu?: boolean;
  /// The width classes of a top menu. A menu with long labels sets a
  /// width that fits them.
  width?: string;
  /// A submenu that the pointer opens keeps the focus on its item.
  focusFirst?: boolean;
}) {
  const menu = useRef<HTMLDivElement>(null);
  const [openKey, setOpenKey] = useState<string | null>(null);
  const [openFocus, setOpenFocus] = useState(true);
  // The page renders again at each capture tick. The focus moves and the
  // outside listener starts only when the menu opens, not at each render.
  const close = useRef(onClose);
  close.current = onClose;

  useEffect(() => {
    if (focusFirst) menu.current?.querySelector<HTMLButtonElement>(":scope > div > button:not(:disabled)")?.focus();
    const outside = (event: PointerEvent) => {
      if (!menu.current?.parentElement?.contains(event.target as Node)) close.current();
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [focusFirst]);

  const buttonOf = (key: string) =>
    menu.current?.querySelector<HTMLButtonElement>(`:scope > div[data-key="${CSS.escape(key)}"] > button`) ?? null;

  const openSubmenu = (key: string, focus: boolean) => {
    setOpenFocus(focus);
    setOpenKey(key);
  };
  const closeSubmenu = () => {
    const key = openKey;
    setOpenKey(null);
    if (key) buttonOf(key)?.focus();
  };

  const onKeyDown = (event: KeyboardEvent) => {
    event.stopPropagation();
    if (event.key === "Escape" || (submenu && event.key === "ArrowLeft")) {
      event.preventDefault();
      onClose();
      return;
    }
    const at = document.activeElement;
    const entry = items.find((e): e is MenuItem => !isSeparator(e) && buttonOf(keyOf(e)) === at);
    if (event.key === "ArrowRight" && entry?.submenu && !entry.disabled) {
      event.preventDefault();
      openSubmenu(keyOf(entry), true);
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    setOpenKey(null);
    const buttons = [...(menu.current?.querySelectorAll<HTMLButtonElement>(":scope > div > button:not(:disabled)") ?? [])];
    const index = buttons.indexOf(at as HTMLButtonElement);
    const next = event.key === "ArrowDown" ? index + 1 : index - 1;
    buttons[(next + buttons.length) % buttons.length]?.focus();
  };

  // A menu with a check mark keeps a column for it, so the labels align.
  const checks = items.some((e) => !isSeparator(e) && e.checked !== undefined);
  const scrolls = !items.some((e) => !isSeparator(e) && e.submenu !== undefined);
  return (
    <div
      ref={menu}
      role="menu"
      onKeyDown={onKeyDown}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      className={`absolute z-10 rounded-md bg-group p-1 shadow-[0_0_0_0.5px_var(--separator),0_6px_20px_rgb(0_0_0/0.18)] ${
        submenu ? "top-[-4px] right-full mr-1 w-60" : `top-full right-0 mt-1 ${width}`
      } ${scrolls ? "max-h-[min(24rem,calc(100vh-8rem))] overflow-y-auto overscroll-contain" : ""}`}
    >
      {items.map((entry) => {
        const key = keyOf(entry);
        if (isSeparator(entry)) {
          return <div key={key} role="separator" className="mx-2.5 my-1 h-px bg-separator" />;
        }
        const open = openKey === key;
        const opens = entry.submenu !== undefined && !entry.disabled;
        return (
          <div
            key={key}
            data-key={key}
            className="relative"
            // As in a macOS menu, the pointer on an item opens its submenu
            // and closes any other.
            onPointerEnter={() => {
              if (open) return;
              if (opens) openSubmenu(key, false);
              else setOpenKey(null);
            }}
          >
            <button
              type="button"
              role={roleOf(entry)}
              aria-checked={entry.checked}
              aria-haspopup={entry.submenu ? "menu" : undefined}
              aria-expanded={entry.submenu ? open : undefined}
              className={`${itemClass} ${open ? "bg-fill" : ""}`}
              disabled={entry.disabled}
              title={entry.disabled ? entry.reason : undefined}
              onClick={(e) => {
                if (!entry.submenu) {
                  entry.onSelect();
                  return;
                }
                // WKWebView does not focus a clicked button, and the arrow
                // keys go to the focused element.
                e.currentTarget.focus();
                openSubmenu(key, true);
              }}
            >
              {checks && (
                <span aria-hidden className="flex w-4 shrink-0 justify-center">
                  {entry.checked && <CheckIcon className="size-[12px]" />}
                </span>
              )}
              <span className="min-w-0 flex-1 truncate">{entry.label}</span>
              {entry.submenu && <ChevronIcon className="size-3 opacity-70" />}
            </button>
            {open && entry.submenu && (
              <Menu items={entry.submenu} onClose={closeSubmenu} submenu focusFirst={openFocus} />
            )}
          </div>
        );
      })}
    </div>
  );
}

/// The session page's "…" button and its menu. Each item closes the menu
/// before it acts, so a dialog or the title field can take the focus.
export function SessionMenu({ items, onOpen }: { items: MenuEntry[]; onOpen?: () => void }) {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const close = () => {
    setOpen(false);
    button.current?.focus();
  };
  const closing = (entries: MenuEntry[]): MenuEntry[] =>
    entries.map((entry) =>
      isSeparator(entry)
        ? entry
        : {
            ...entry,
            submenu: entry.submenu && closing(entry.submenu),
            onSelect: () => {
              setOpen(false);
              entry.onSelect();
            },
          },
    );
  return (
    <div className="relative">
      <button
        ref={button}
        type="button"
        aria-label="Actions"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => {
          if (!open) onOpen?.();
          setOpen((o) => !o);
        }}
        className={`flex h-[22px] w-7 items-center justify-center rounded-md text-label-secondary hover:bg-fill active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent ${open ? "bg-fill" : ""}`}
      >
        <svg aria-hidden viewBox="0 0 16 16" className="size-4">
          <circle cx="3.5" cy="8" r="1.25" fill="currentColor" />
          <circle cx="8" cy="8" r="1.25" fill="currentColor" />
          <circle cx="12.5" cy="8" r="1.25" fill="currentColor" />
        </svg>
      </button>
      {open && <Menu onClose={close} items={closing(items)} />}
    </div>
  );
}
