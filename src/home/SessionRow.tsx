// One session in Home's list (R5, R7): the title, the start time, the
// duration, the speaker count, and a status while the session is not
// ready, then the labels of the note type and tags. A click selects the note. A double-click opens it. The row menu holds Rename and Delete.
// Hover, selection and the menu change at once, with no motion: a list is
// moved through many times a day.

import { useRef, useState } from "react";
import type { SessionEntry } from "../bindings";
import { entryStatus, entryTone, formatDuration, sessionTitle } from "../processing/describe";
import { canDelete, Menu, remoteLabels, remoteReasons } from "../session/SessionMenu";
import { MoreIcon } from "../settings/icons";
import { Status } from "../settings/ui";
import { speakerCount } from "./group";
import { NoteLabels } from "./NoteLabels";

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
  noteType,
  title,
  time,
  stage,
  processingWaits = false,
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
  /// The type that the row's label shows: the note's type, General when
  /// types are in use (R4), or null for no type label.
  noteType: string | null;
  title: string;
  /// The start time of day, "14:05".
  time: string;
  /// The stage an event named since the list was read.
  stage: string | null;
  /// Processing waits for a recording to finish (KTD20).
  processingWaits?: boolean;
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
  // A note that another computer works on says so (R20, R23).
  const status = entry.remote
    ? remoteLabels[entry.remote]
    : entry.state === "saved"
      ? null
      : entryStatus(entry, stage, processingWaits);
  const tone = entryTone(entry.state) ?? "neutral";
  // An untitled row shows the start time as its title already.
  // A note whose storage folder is unavailable says so in place of its
  // details (R16).
  const details = entry.storage_unavailable
    ? "Storage folder unavailable"
    : [
        entry.title === null ? null : time,
        // A note with no audio has no length (R15). A saved note with no
        // audio says so, so the row always has details.
        entry.duration_ms > 0
          ? formatDuration(entry.duration_ms)
          : entry.state === "saved"
            ? "Notes only"
            : null,
        speakerCount(entry.found_speakers),
      ]
        .filter(Boolean)
        // An empty line keeps the row as tall as the others.
        .join(" · ") || " ";
  // The selected row has a neutral fill and an accent bar to its left, so
  // the colours of the labels stay the strongest colour in the row.
  const surface = selected ? "bg-row-selected" : menuOpen ? "bg-row-hover" : "";

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
      className={`group relative flex cursor-default items-center gap-3 rounded-[9px] px-2.5 py-2 text-label ${surface} outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent`}
    >
      {/* The bar is in the list's left padding, outside the row's corners. */}
      {selected && <span aria-hidden className="absolute inset-y-1.5 -left-2 w-[3px] rounded-full bg-accent" />}
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
        {/* The type and tag labels follow the details (R22). */}
        <div className="flex min-w-0 items-center gap-2">
          <div className="min-w-0 shrink-0 truncate text-[0.8rem] text-label-secondary">{details}</div>
          <NoteLabels noteType={noteType} tags={entry.tags} />
        </div>
      </div>
      {status && (
        <span className="shrink-0 rounded-full bg-fill px-2 py-0.5 text-[0.75rem] font-medium">
          <Status tone={tone}>{status}</Status>
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
          className={`flex h-6 w-7 items-center justify-center rounded-md text-label-secondary hover:bg-fill active:bg-fill-pressed ${menuOpen || selected ? "" : "opacity-0 group-hover:opacity-100"}`}
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
                disabled: !canDelete(entry.state) || entry.storage_unavailable || entry.remote === "in_use",
                reason: entry.storage_unavailable
                  ? "The storage folder for this note is unavailable."
                  : entry.remote === "in_use"
                    ? remoteReasons.in_use
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
