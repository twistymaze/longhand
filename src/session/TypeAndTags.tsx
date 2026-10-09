// The row under the note's title and subtitle (R16): the note type as a
// menu, each tag with a control that removes it, and "+ Tag". A type
// change shows its result as a notice under the title, as a rename does.
//
// During a recording, the row follows the attention budget of the
// recording page (R17). It changes only after an action of the user. A
// type or tag change from another computer shows when the recording
// stops. The row has a minimum height, so it does not move the page when
// it loads. Nothing in the row moves or flashes.

import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type NoteTypeEntry } from "../bindings";
import { GENERAL, TagLabel, TypeLabel, withGeneral } from "../lib/Labels";
import { NewTypeDialog } from "../lib/NewTypeDialog";
import { TagPicker } from "../lib/TagPicker";
import { useReloadOn } from "../lib/useReloadOn";
import { sentence } from "../processing/describe";
import { Menu, type MenuEntry, separator } from "./SessionMenu";
import type { Notice } from "./TitleField";

type View = { noteType: string | null; tags: string[] };

/// The notice for the result of a type change: only an error. A folder
/// move that waits or must try again gets no notice, because the app
/// moves the folder later without help (CLAUDE.md, UI design).
function typeNotice(result: Awaited<ReturnType<typeof commands.setNoteType>>): Notice | null {
  if (result.status === "error") return { tone: "negative", text: sentence(result.error) };
  return null;
}

/// WebKit puts typed text into the text selection, also when the focus is
/// on a button or a menu item. The selection leaves the page text before a
/// control in the row takes the keys, so that no text goes into My Notes
/// (docs/solutions/ui-bugs/typed-text-went-to-hidden-notes-after-tab-click.md).
function clearSelection() {
  window.getSelection()?.removeAllRanges();
}

function DownChevron() {
  return (
    <svg aria-hidden viewBox="0 0 10 6" className="h-[5px] w-[8px] shrink-0 text-label-secondary">
      <path d="M1 1l4 4 4-4" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function TypeAndTags({
  sessionId,
  present,
  noteType,
  tags,
  recording,
  onNotice,
  onChanged,
}: {
  sessionId: string;
  /// The list has the note. Until then the row is empty.
  present: boolean;
  /// The type that the app shows (`SessionEntry.note_type`).
  noteType: string | null;
  tags: string[];
  /// The note records now: changes from elsewhere wait (R17).
  recording: boolean;
  onNotice: (notice: Notice | null) => void;
  /// A change completed: the list must be read again.
  onChanged: () => void;
}) {
  const [shown, setShown] = useState<View | null>(present ? { noteType, tags } : null);
  // The values from the list replace the shown values, except during a
  // recording. When the recording stops, the row shows the list values.
  const tagsKey = tags.join("\n");
  useEffect(() => {
    if (!present) return;
    setShown((s) => (recording && s ? s : { noteType, tags }));
    // `tagsKey` stands for `tags`, which is a new array at each read.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [present, noteType, tagsKey, recording]);

  const [types, setTypes] = useState<NoteTypeEntry[]>([]);
  // Reads can end out of order. Each read and a new type from New Type…
  // take a number, and only the newest number changes the list.
  const typeRequest = useRef(0);
  const loadTypes = useCallback(async () => {
    const current = ++typeRequest.current;
    try {
      const result = await commands.noteTypes();
      if (current === typeRequest.current && result.status === "ok") setTypes(result.data.types);
    } catch (e) {
      console.error("The note types could not be read:", e);
    }
  }, []);
  // A type from Settings or a new type folder from a sync changes the list.
  useEffect(() => {
    void loadTypes();
  }, [loadTypes]);
  useReloadOn(loadTypes);

  const [tagList, setTagList] = useState<string[]>([]);
  const loadTags = async () => {
    clearSelection();
    try {
      const result = await commands.listTags();
      if (result.status === "ok") setTagList(result.data.map((t) => t.text));
    } catch (e) {
      console.error("The tag list could not be read:", e);
    }
  };

  const [menuOpen, setMenuOpen] = useState(false);
  const [creating, setCreating] = useState(false);
  const typeButton = useRef<HTMLButtonElement>(null);
  const rowRef = useRef<HTMLDivElement>(null);

  const chooseType = async (next: string | null) => {
    if (!shown || next === shown.noteType) return;
    const result = await commands.setNoteType(sessionId, next);
    onNotice(typeNotice(result));
    if (result.status === "ok") {
      setShown((s) => (s ? { ...s, noteType: next } : s));
      onChanged();
    }
  };

  const editTags = async (add: string[], remove: string[]) => {
    const result = await commands.setNoteTags(sessionId, add, remove);
    if (result.status === "error") {
      onNotice({ tone: "negative", text: sentence(result.error) });
      return;
    }
    onNotice(null);
    setShown((s) => (s ? { ...s, tags: result.data } : s));
    onChanged();
  };

  const removeTag = async (tag: string) => {
    await editTags([], [tag]);
    // The removed control had the focus. The focus goes to "+ Tag", so the
    // keys still go to the row.
    if (!rowRef.current?.contains(document.activeElement)) {
      rowRef.current?.querySelector<HTMLButtonElement>('button[aria-label="Add Tag"]')?.focus();
    }
  };

  const closeMenu = () => {
    setMenuOpen(false);
    typeButton.current?.focus();
  };

  if (!shown) return <div data-testid="type-and-tags" className="mt-2 min-h-5" />;

  // General is first. A note of General has the type null.
  const valid = withGeneral(types).filter((t) => t.valid);
  const items: MenuEntry[] = valid.map((t) => ({
    key: t.general ? "general" : `type:${t.name}`,
    label: t.name,
    checked: t.general ? shown.noteType === null : t.name === shown.noteType,
    radio: true,
    onSelect: () => {
      setMenuOpen(false);
      typeButton.current?.focus();
      void chooseType(t.general ? null : t.name);
    },
  }));
  // A type that is not valid can be the note's type, but not a choice.
  if (shown.noteType !== null && !valid.some((t) => !t.general && t.name === shown.noteType)) {
    items.push({
      key: "current",
      label: shown.noteType,
      checked: true,
      radio: true,
      disabled: true,
      reason: "Rename this type in Settings to choose it.",
      onSelect: () => {},
    });
  }
  items.push(
    separator("new"),
    {
      key: "new",
      label: "New Type…",
      onSelect: () => {
        setMenuOpen(false);
        setCreating(true);
      },
    },
  );

  return (
    <div
      ref={rowRef}
      role="group"
      aria-label="Type and tags"
      data-testid="type-and-tags"
      className="mt-2 flex min-h-5 flex-wrap items-center gap-x-1.5 gap-y-1"
    >
      <div className="relative inline-flex">
        <button
          ref={typeButton}
          type="button"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          onClick={(e) => {
            clearSelection();
            // WKWebView does not focus a clicked button.
            e.currentTarget.focus();
            if (!menuOpen) void loadTypes();
            setMenuOpen((o) => !o);
          }}
          className={`-mx-0.5 inline-flex items-center gap-1 rounded-[5px] px-0.5 py-px hover:bg-fill active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent ${
            menuOpen ? "bg-fill" : ""
          }`}
        >
          <TypeLabel name={shown.noteType ?? GENERAL} />
          <DownChevron />
        </button>
        {menuOpen && <Menu items={items} onClose={closeMenu} align="left" />}
        {creating && (
          <NewTypeDialog
            onCancel={() => {
              setCreating(false);
              typeButton.current?.focus();
            }}
            onCreated={(name, list) => {
              setCreating(false);
              typeRequest.current++;
              setTypes(list.types);
              typeButton.current?.focus();
              void chooseType(name);
            }}
          />
        )}
      </div>
      {shown.tags.map((tag) => (
        <TagLabel key={tag} text={tag} onRemove={() => void removeTag(tag)} />
      ))}
      <TagPicker tags={tagList} applied={shown.tags} onAdd={(tag) => void editTags([tag], [])} onOpen={() => void loadTags()} />
    </div>
  );
}
