// One session in Home's list (R5, R7): the title, the start time, the
// duration, the speaker count, and a status while the session is not
// ready. The row menu holds Rename and Delete. Selection and the menu
// change at once, with no motion: a list is moved through many times a day.

import { type KeyboardEvent, useEffect, useRef, useState } from "react";
import type { SessionEntry } from "../bindings";
import { entryStatus, entryTone, formatDuration, sessionTitle } from "../processing/describe";
import { Status } from "../settings/ui";
import { speakerCount } from "./group";

/// States in which the store refuses a delete (`delete_refusal`, R8).
export function canDelete(state: string): boolean {
  return !["capturing", "interrupted", "queued", "processing"].includes(state);
}

function RowMenu({
  canDelete,
  onRename,
  onDelete,
  onClose,
}: {
  canDelete: boolean;
  onRename: () => void;
  onDelete: () => void;
  onClose: () => void;
}) {
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
    const items = [...(menu.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === "ArrowDown" ? at + 1 : at - 1;
    items[(next + items.length) % items.length]?.focus();
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
      className="absolute top-full right-0 z-10 mt-1 w-40 rounded-md bg-group p-1 shadow-[0_0_0_0.5px_var(--separator),0_6px_20px_rgb(0_0_0/0.18)]"
    >
      <button type="button" role="menuitem" className={item} onClick={onRename}>
        Rename
      </button>
      <button
        type="button"
        role="menuitem"
        className={item}
        disabled={!canDelete}
        title={canDelete ? undefined : "You can delete this session after processing finishes."}
        onClick={onDelete}
      >
        Delete…
      </button>
    </div>
  );
}

function TitleField({
  initial,
  placeholder,
  onCommit,
  onCancel,
}: {
  initial: string;
  placeholder: string;
  onCommit: (title: string) => void;
  onCancel: () => void;
}) {
  const done = useRef(false);
  const finish = (commit: boolean, value: string) => {
    if (done.current) return;
    done.current = true;
    if (commit) onCommit(value.trim());
    else onCancel();
  };
  return (
    <input
      aria-label="Title"
      defaultValue={initial}
      placeholder={placeholder}
      autoFocus
      onFocus={(e) => e.currentTarget.select()}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") finish(true, e.currentTarget.value);
        if (e.key === "Escape") finish(false, "");
      }}
      onBlur={(e) => finish(true, e.currentTarget.value)}
      className="w-full rounded-[4px] bg-control px-1 text-label outline-2 outline-accent select-text"
    />
  );
}

export function SessionRow({
  entry,
  title,
  time,
  stage,
  selected,
  focusable,
  renaming,
  onSelect,
  onOpen,
  onRename,
  onRenameCommit,
  onRenameCancel,
  onDelete,
}: {
  entry: SessionEntry;
  title: string;
  /// The start time of day, "14:05".
  time: string;
  /// The stage an event named since the list was read.
  stage: string | null;
  selected: boolean;
  /// The one row that Tab moves to: the selected row, or else the first.
  focusable: boolean;
  renaming: boolean;
  onSelect: () => void;
  onOpen: () => void;
  onRename: () => void;
  onRenameCommit: (title: string) => void;
  onRenameCancel: () => void;
  onDelete: () => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const row = useRef<HTMLDivElement>(null);
  const status = entry.state === "saved" ? null : entryStatus(entry, stage);
  const tone = entryTone(entry.state) ?? "neutral";
  // An untitled row shows the start time as its title already.
  const details = [entry.title === null ? null : time, formatDuration(entry.duration_ms), speakerCount(entry.found_speakers)]
    .filter(Boolean)
    .join(" · ");
  const secondary = selected ? "text-white/80" : "text-label-secondary";

  const closeMenu = () => {
    setMenuOpen(false);
    row.current?.focus();
  };

  return (
    <div
      ref={row}
      role="option"
      aria-selected={selected}
      tabIndex={focusable ? 0 : -1}
      data-session={entry.id}
      onClick={onSelect}
      onDoubleClick={onOpen}
      onContextMenu={(e) => {
        e.preventDefault();
        onSelect();
        setMenuOpen(true);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          onOpen();
        }
      }}
      className={`group relative flex items-center gap-2 rounded-md px-2.5 py-1.5 ${
        selected ? "bg-accent text-white" : "text-label"
      } outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent`}
    >
      <div className="min-w-0 flex-1">
        {renaming ? (
          <TitleField
            initial={entry.title ?? ""}
            placeholder={sessionTitle(entry.started)}
            onCommit={onRenameCommit}
            onCancel={onRenameCancel}
          />
        ) : (
          <div className="truncate">{title}</div>
        )}
        <div className={`truncate text-[0.85rem] ${secondary}`}>{details}</div>
      </div>
      {status && (
        <span className={`shrink-0 text-[0.85rem] ${selected ? "text-white/80" : ""}`}>
          {selected ? status : <Status tone={tone}>{status}</Status>}
        </span>
      )}
      <div className="relative shrink-0">
        <button
          type="button"
          aria-label="Actions"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          tabIndex={-1}
          onClick={(e) => {
            e.stopPropagation();
            onSelect();
            setMenuOpen((open) => !open);
          }}
          onDoubleClick={(e) => e.stopPropagation()}
          className={`rounded-[4px] px-1.5 leading-5 ${
            selected ? "text-white active:bg-white/20" : "text-label-secondary active:bg-fill-pressed"
          } ${menuOpen || selected ? "" : "opacity-0 group-hover:opacity-100"}`}
        >
          …
        </button>
        {menuOpen && (
          <RowMenu
            canDelete={canDelete(entry.state)}
            onClose={closeMenu}
            onRename={() => {
              setMenuOpen(false);
              onRename();
            }}
            onDelete={() => {
              setMenuOpen(false);
              onDelete();
            }}
          />
        )}
      </div>
    </div>
  );
}
