// One session in Home's list (R5, R7): the title, the start time, the
// duration, the speaker count, and a status while the session is not
// ready. A click selects the note. A double-click opens it. The row menu holds Rename and Delete.
// Hover, selection and the menu change at once, with no motion: a list is
// moved through many times a day.

import { useRef, useState } from "react";
import type { SessionEntry } from "../bindings";
import { entryStatus, entryTone, formatDuration, sessionTitle } from "../processing/describe";
import { canDelete, Menu } from "../session/SessionMenu";
import { MoreIcon, NoteIcon } from "../settings/icons";
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
  // A note whose storage folder is unavailable says so in place of its
  // details (R16).
  const details = entry.storage_unavailable
    ? "Storage folder unavailable"
    : [entry.title === null ? null : time, formatDuration(entry.duration_ms), speakerCount(entry.found_speakers)]
        .filter(Boolean)
        .join(" · ");
  const secondary = selected ? "text-white/75" : "text-label-secondary";
  const surface = selected ? "bg-accent text-white" : menuOpen ? "bg-row-hover text-label" : "text-label";

  const closeMenu = () => {
    setMenuOpen(false);
    row.current?.focus();
  };

  return (
    <div
      ref={row}
      role="option"
      aria-selected={selected}
      tabIndex={selected ? 0 : -1}
      data-session={entry.id}
      onClick={renaming ? undefined : () => { row.current?.focus(); onSelect(); }}
      onDoubleClick={renaming ? undefined : onOpen}
      onContextMenu={(e) => {
        e.preventDefault();
        setMenuOpen(true);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          onOpen();
        }
      }}
      className={`group relative flex cursor-default items-center gap-3 rounded-[9px] px-2.5 py-2 ${surface} outline-offset-1 ${selected ? "outline-none" : "focus-visible:outline-2 focus-visible:outline-accent"}`}
    >
      <span
        aria-hidden
        className={`flex size-8 shrink-0 items-center justify-center rounded-[8px] ${
          selected ? "bg-white/20 text-white" : "bg-accent/12 text-accent"
        }`}
      >
        <NoteIcon className="size-[17px]" />
      </span>
      <div className="min-w-0 flex-1">
        {renaming ? (
          <TitleField
            initial={entry.title ?? ""}
            placeholder={sessionTitle(entry.started)}
            onCommit={onRenameCommit}
            onCancel={onRenameCancel}
          />
        ) : (
          <div className="truncate font-medium tracking-[-0.005em]">{title}</div>
        )}
        <div className={`truncate text-[0.8rem] ${secondary}`}>{details}</div>
      </div>
      {status && (
        <span
          className={`shrink-0 rounded-full px-2 py-0.5 text-[0.75rem] font-medium ${
            selected ? "bg-white/20 text-white" : "bg-fill"
          }`}
        >
          {selected ? status : <Status tone={tone}>{status}</Status>}
        </span>
      )}
      <div className="relative shrink-0" onDoubleClick={(e) => e.stopPropagation()}>
        <button
          type="button"
          aria-label="Actions"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          tabIndex={-1}
          onClick={(e) => {
            e.stopPropagation();
            setMenuOpen((open) => !open);
          }}
          className={`flex h-6 w-7 items-center justify-center rounded-md ${
            selected ? "text-white hover:bg-white/15 active:bg-white/25" : "text-label-secondary hover:bg-fill active:bg-fill-pressed"
          } ${menuOpen || selected ? "" : "opacity-0 group-hover:opacity-100"}`}
        >
          <MoreIcon />
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
                disabled: !canDelete(entry.state) || entry.storage_unavailable,
                reason: entry.storage_unavailable
                  ? "The storage folder for this note is unavailable."
                  : "You can delete this note after processing finishes.",
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
