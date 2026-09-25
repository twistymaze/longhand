// A session's actions: the menu under a Home row's "…" button and the
// session page's "…" menu (R7, R19). The menu opens and closes at once,
// with no motion: it is used many times a day.

import { type KeyboardEvent, useEffect, useRef, useState } from "react";

export type MenuItem = {
  label: string;
  onSelect: () => void;
  disabled?: boolean;
  /// Why the item is disabled, as a tooltip.
  reason?: string;
};

/// States in which the store refuses a delete (`delete_refusal`, R8).
export function canDelete(state: string): boolean {
  return !["capturing", "interrupted", "queued", "processing"].includes(state);
}

/// The open menu, below its button and aligned to its right edge. The
/// first enabled item has the focus, the arrow keys move it, and Escape or
/// a click outside closes the menu.
export function Menu({ items, onClose }: { items: MenuItem[]; onClose: () => void }) {
  const menu = useRef<HTMLDivElement>(null);

  useEffect(() => {
    menu.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    const outside = (event: PointerEvent) => {
      if (!menu.current?.parentElement?.contains(event.target as Node)) onClose();
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [onClose]);

  const onKeyDown = (event: KeyboardEvent) => {
    event.stopPropagation();
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const buttons = [...(menu.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === "ArrowDown" ? at + 1 : at - 1;
    buttons[(next + buttons.length) % buttons.length]?.focus();
  };

  const item =
    "block w-full rounded-[4px] px-2.5 py-[3px] text-left text-label enabled:hover:bg-accent enabled:hover:text-white focus-visible:bg-accent focus-visible:text-white focus-visible:outline-none disabled:text-label-tertiary";
  return (
    <div
      ref={menu}
      role="menu"
      onKeyDown={onKeyDown}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      className="absolute top-full right-0 z-10 mt-1 w-48 rounded-md bg-group p-1 shadow-[0_0_0_0.5px_var(--separator),0_6px_20px_rgb(0_0_0/0.18)]"
    >
      {items.map((entry) => (
        <button
          key={entry.label}
          type="button"
          role="menuitem"
          className={item}
          disabled={entry.disabled}
          title={entry.disabled ? entry.reason : undefined}
          onClick={entry.onSelect}
        >
          {entry.label}
        </button>
      ))}
    </div>
  );
}

/// The session page's "…" button and its menu. Each item closes the menu
/// before it acts, so a dialog or the title field can take the focus.
export function SessionMenu({ items }: { items: MenuItem[] }) {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const close = () => {
    setOpen(false);
    button.current?.focus();
  };
  return (
    <div className="relative">
      <button
        ref={button}
        type="button"
        aria-label="Actions"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className={`flex h-[22px] w-7 items-center justify-center rounded-md text-label-secondary hover:bg-fill active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent ${open ? "bg-fill" : ""}`}
      >
        <svg aria-hidden viewBox="0 0 16 16" className="size-4">
          <circle cx="3.5" cy="8" r="1.25" fill="currentColor" />
          <circle cx="8" cy="8" r="1.25" fill="currentColor" />
          <circle cx="12.5" cy="8" r="1.25" fill="currentColor" />
        </svg>
      </button>
      {open && (
        <Menu
          onClose={close}
          items={items.map((entry) => ({
            ...entry,
            onSelect: () => {
              setOpen(false);
              entry.onSelect();
            },
          }))}
        />
      )}
    </div>
  );
}
