// New Note as a split button (R9, KTD10). The main part does the default
// that Settings > Recording sets, and its label says which default that is.
// The main part starts a note of the default type (R23). The menu part
// lists General and the other note types, the other choice, and "New
// Type…" (R24). Each type item starts a note of that type in the same way
// as the main part. Both parts are unavailable, with the reason, while a note
// cannot start (R7). The menu opens and closes at once, with no motion: the
// button is used many times a day.

import { type Ref, useRef, useState } from "react";
import type { NewNoteType, NoteTypeEntry, NoteTypeList, Readiness } from "../bindings";
import { withGeneral } from "../lib/Labels";
import { NewTypeDialog } from "../lib/NewTypeDialog";
import { missingPreconditions } from "../processing/describe";
import { Menu, type MenuEntry, separator } from "../session/SessionMenu";
import { ChevronIcon, PlusIcon } from "../settings/icons";
import { Button } from "../settings/ui";

/// The label of each choice, in the main part and in the menu.
const labels = {
  record: { main: "Record New Note", menu: "Record New Note" },
  note: { main: "New Note", menu: "New Note Without Recording" },
};

/// Why no new note can start now, or null when one can.
export function noteStartRefusal(readiness: Readiness): string | null {
  if (readiness.can_start_note) return null;
  return readiness.note_start_refusal ?? "A new note can’t start now.";
}

/// Why the choice `record` cannot start now, or null when it can.
function refusal(readiness: Readiness, record: boolean): string | null {
  const noteRefusal = noteStartRefusal(readiness);
  if (noteRefusal) return noteRefusal;
  if (record && !readiness.armed) {
    const missing = missingPreconditions(readiness);
    return missing ? `To record, ${missing} in Settings.` : "Not ready to record.";
  }
  return null;
}

export function NewNoteButton({
  records,
  readiness,
  types = [],
  busy = false,
  onStart,
  onTypesChanged,
  ref,
}: {
  /// The default of Settings > Recording: true when New Note records.
  records: boolean;
  readiness: Readiness;
  /// The type list. The menu offers General and the valid types.
  types?: NoteTypeEntry[];
  /// A start is in progress: both parts wait.
  busy?: boolean;
  /// Starts a note. With no `noteType`, the note gets the default type.
  onStart: (record: boolean, noteType?: NewNoteType) => void;
  /// "New Type…" added a type. Gives the list after the change.
  onTypesChanged?: (list: NoteTypeList) => void;
  /// The main part, which Home focuses after the last note goes.
  ref?: Ref<HTMLButtonElement>;
}) {
  const [open, setOpen] = useState(false);
  const [naming, setNaming] = useState(false);
  const menuPart = useRef<HTMLButtonElement>(null);
  const mainLabel = records ? labels.record.main : labels.note.main;
  const mainRefusal = refusal(readiness, records);
  const otherRefusal = refusal(readiness, !records);
  const close = () => {
    setOpen(false);
    menuPart.current?.focus();
  };
  const startAs = (noteType: NewNoteType) => () => {
    setOpen(false);
    onStart(records, noteType);
  };

  const items: MenuEntry[] = [
    ...withGeneral(types)
      .filter((t) => t.valid)
      .map((t) => ({
        key: t.general ? "general" : `type:${t.name}`,
        label: t.name,
        disabled: mainRefusal !== null,
        reason: mainRefusal ?? undefined,
        onSelect: startAs(t.general ? { kind: "general" } : { kind: "named", name: t.name }),
      })),
    separator("other"),
    {
      label: records ? labels.note.menu : labels.record.menu,
      disabled: otherRefusal !== null,
      reason: otherRefusal ?? undefined,
      onSelect: () => {
        setOpen(false);
        onStart(!records);
      },
    },
    separator("new-type"),
    {
      label: "New Type…",
      disabled: mainRefusal !== null,
      reason: mainRefusal ?? undefined,
      onSelect: () => {
        setOpen(false);
        setNaming(true);
      },
    },
  ];

  return (
    <div className="relative inline-flex gap-px">
      <span title={mainRefusal ?? undefined} className="inline-flex">
        <Button ref={ref} prominent joined="start" disabled={busy || mainRefusal !== null} onClick={() => onStart(records)}>
          <span className="inline-flex items-center gap-1.5">
            <PlusIcon className="-ml-0.5 size-3.5" />
            {mainLabel}
          </span>
        </Button>
      </span>
      <span title={readiness.can_start_note ? undefined : (mainRefusal ?? undefined)} className="inline-flex">
        <Button
          ref={menuPart}
          prominent
          joined="end"
          label="New Note Options"
          expanded={open}
          disabled={busy || !readiness.can_start_note}
          onClick={(e) => {
            // WKWebView does not focus a clicked button. The focus must be
            // here, so that Escape in the menu gives it back to this part.
            e.currentTarget.focus();
            setNaming(false);
            setOpen((o) => !o);
          }}
        >
          <ChevronIcon className="size-3 rotate-90" />
        </Button>
      </span>
      {open && (
        <Menu
          onClose={close}
          // The menu is as wide as its label, so the label shows in full.
          width="w-max min-w-52 max-w-80"
          items={items}
        />
      )}
      {naming && (
        <NewTypeDialog
          // The button is at the right edge of the window.
          className="top-full right-0 mt-1"
          onCancel={() => {
            setNaming(false);
            menuPart.current?.focus();
          }}
          onCreated={(name, list) => {
            setNaming(false);
            onTypesChanged?.(list);
            onStart(records, { kind: "named", name });
          }}
        />
      )}
    </div>
  );
}
