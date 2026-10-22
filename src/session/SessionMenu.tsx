// A session's actions: the menu under a Home row's "…" button and the
// session page's "…" menu (R7, R19). The menu and its submenus open and
// close at once, with no motion: they are used many times a day.

import { autoUpdate, computePosition, flip, offset, shift } from "@floating-ui/dom";
import { type KeyboardEvent, type Ref, type RefObject, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
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
  /// A check mark. `true` or `false` makes the item a check box, as each
  /// app in the App Audio submenu is.
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
/// With `floating`, the menu is in a portal with a fixed position under
/// that button, so a parent with overflow hidden does not cut it off
/// (KTD7). The button counts as part of the menu for a press outside, and
/// Tab closes the menu, because the portal is outside the Tab order.
export function Menu({
  items,
  onClose,
  submenu = false,
  focusFirst = true,
  width = "w-52",
  align = "right",
  floating,
}: {
  items: MenuEntry[];
  onClose: () => void;
  /// The button of a floating top menu.
  floating?: RefObject<HTMLElement | null>;
  submenu?: boolean;
  /// The width classes of a top menu. A menu with long labels sets a
  /// width that fits them.
  width?: string;
  /// The edge of its button that a top menu aligns to. A button at the
  /// left of the page opens its menu to the right.
  align?: "left" | "right";
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
      const target = event.target as Node;
      const inside = floating
        ? menu.current?.contains(target) || floating.current?.contains(target)
        : menu.current?.parentElement?.contains(target);
      if (!inside) close.current();
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [focusFirst, floating]);

  // A floating menu is under its button, and stays in the window.
  useLayoutEffect(() => {
    const reference = floating?.current;
    const popup = menu.current;
    if (!reference || !popup) return;
    const update = () => {
      void computePosition(reference, popup, {
        placement: align === "left" ? "bottom-start" : "bottom-end",
        strategy: "fixed",
        middleware: [offset(4), flip(), shift({ padding: 8 })],
      }).then(({ x, y }) => {
        Object.assign(popup.style, { left: `${x}px`, top: `${y}px` });
      });
    };
    return autoUpdate(reference, popup, update);
  }, [floating, align]);

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
    if (event.key === "Escape" || (submenu && event.key === "ArrowLeft") || (floating && event.key === "Tab")) {
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
  const place = floating
    ? `fixed z-50 ${width}`
    : `absolute z-10 ${
        submenu ? "top-[-4px] right-full mr-1 w-60" : `top-full ${align === "left" ? "left-0" : "right-0"} mt-1 ${width}`
      }`;
  const body = (
    <div
      ref={menu}
      role="menu"
      onKeyDown={onKeyDown}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      style={floating ? { top: 0, left: 0 } : undefined}
      className={`${place} rounded-md bg-group p-1 shadow-[0_0_0_0.5px_var(--separator),0_6px_20px_rgb(0_0_0/0.18)] ${
        scrolls ? "max-h-[min(24rem,calc(100vh-8rem))] overflow-y-auto overscroll-contain" : ""
      }`}
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
  return floating ? createPortal(body, document.body) : body;
}

/// The session page's "…" button and its menu. Each item closes the menu
/// before it acts, so a dialog or the title field can take the focus.
/// Settings rows use the same button with the row's name in `label`, and
/// `floating`, because a Settings group has overflow hidden (KTD7).
export function SessionMenu({
  items,
  onOpen,
  label = "Actions",
  disabled,
  floating = false,
  ref,
}: {
  items: MenuEntry[];
  onOpen?: () => void;
  /// The accessible name of the button.
  label?: string;
  disabled?: boolean;
  /// The menu is in a floating layer (`Menu`'s `floating`).
  floating?: boolean;
  /// The button, so a page can give it the focus.
  ref?: Ref<HTMLButtonElement>;
}) {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement | null>(null);
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
        ref={(el) => {
          button.current = el;
          if (typeof ref === "function") ref(el);
          else if (ref) (ref as RefObject<HTMLButtonElement | null>).current = el;
        }}
        type="button"
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={disabled}
        onClick={() => {
          if (!open) onOpen?.();
          setOpen((o) => !o);
        }}
        className={`flex h-[22px] w-7 items-center justify-center rounded-md text-label-secondary enabled:hover:bg-fill enabled:active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40 ${open ? "bg-fill" : ""}`}
      >
        <svg aria-hidden viewBox="0 0 16 16" className="size-4">
          <circle cx="3.5" cy="8" r="1.25" fill="currentColor" />
          <circle cx="8" cy="8" r="1.25" fill="currentColor" />
          <circle cx="12.5" cy="8" r="1.25" fill="currentColor" />
        </svg>
      </button>
      {open && <Menu onClose={close} items={closing(items)} floating={floating ? button : undefined} />}
    </div>
  );
}
