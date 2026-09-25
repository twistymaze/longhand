// One session in Home's list (R5, R7): the title, the start time, the
// duration, the speaker count, and a status while the session is not
// ready. The row menu holds Rename and Delete. Selection and the menu
// change at once, with no motion: a list is moved through many times a day.

import { useRef, useState } from "react";
import type { SessionEntry } from "../bindings";
import { entryStatus, entryTone, formatDuration, sessionTitle } from "../processing/describe";
import { canDelete, Menu } from "../session/SessionMenu";
import { Status } from "../settings/ui";
import { speakerCount } from "./group";

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
          <Menu
            onClose={closeMenu}
            items={[
              {
                label: "Rename",
                onSelect: () => {
                  setMenuOpen(false);
                  onRename();
                },
              },
              {
                label: "Delete…",
                disabled: !canDelete(entry.state),
                reason: "You can delete this session after processing finishes.",
                onSelect: () => {
                  setMenuOpen(false);
                  onDelete();
                },
              },
            ]}
          />
        )}
      </div>
    </div>
  );
}
