// The import sheet (R2, R2a of the audio import plan): before the import
// starts, the user can set the note type and the tags. For one file, the
// sheet also has a title field with the file name, and a small Notes
// field: the My Notes editor, whose text becomes the first manual notes
// of the note (the import-sheet notes plan). For several files, it lists
// the file names: the same type and tags apply to each note, and each note
// gets its title from its own file. Import with no change accepts the
// default type and no tags. Cancel imports nothing. The sheet fades in as
// the other dialogs do, and closes at once.
//
// Keys: Cmd+Return (Ctrl+Return on Windows) imports and Escape cancels,
// from each field. In the Notes field, Return makes a new line and Tab
// stays in the field, as in My Notes.

import { type CSSProperties, type FormEvent, useEffect, useId, useRef, useState } from "react";
import { commands, type ImportResult, type NewNoteType, type NoteTypeList, type Result } from "../bindings";
import { NoteEditor } from "../editor/NoteEditor";
import { GENERAL, TagLabel, tagKey, withGeneral } from "../lib/Labels";
import { TagPicker } from "../lib/TagPicker";
// The last part of a path, on macOS and on Windows. A file path does not
// end with a separator, so the last part is the file name.
import { fieldClass, folderName as fileName } from "../settings/ui";

/// The value of General in the type field. A type name is never empty.
const GENERAL_VALUE = "";

/// The / menu of the editor while it is open. The format toolbar has the
/// same attribute, but Escape does not close it.
const SLASH_MENU = "[data-editor-escape-consumer]:not([role=toolbar])";

/// The minimum and the maximum height of the Notes field, in lines. Above
/// the maximum, the text scrolls inside the field.
const NOTES_ROWS = 4;
const NOTES_MAX_ROWS = 7;

const inEditor = (target: EventTarget | null) =>
  target instanceof Element && target.closest(".prosemirror-editor") !== null;

/// Cmd+Return, or Ctrl+Return on Windows: import.
const isSubmitKey = (e: KeyboardEvent) => e.key === "Enter" && (e.metaKey || e.ctrlKey);

/// Whether the cursor of the editor is in a task, where Cmd+Return checks
/// the task.
function selectionInTask(): boolean {
  const node = window.getSelection()?.anchorNode;
  const element = node instanceof Element ? node : node?.parentElement;
  return element?.closest('li[data-type="taskItem"]') != null;
}

/// The file name with no extension: the first title of the note (R15).
function stem(path: string): string {
  const name = fileName(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
}

export function ImportSheet({
  paths,
  types,
  tags,
  onDone,
  onCancel,
}: {
  /// The files to import, as the drop or the file dialog gave them.
  paths: string[];
  /// The type list, or null before it is read. The type field shows the
  /// default type.
  types: NoteTypeList | null;
  /// The tag list, which the tag field suggests.
  tags: string[];
  /// The import command ended. Home shows its refusals.
  onDone: (result: Result<ImportResult, string>) => void;
  onCancel: () => void;
}) {
  const one = paths.length === 1;
  const proposed = one ? stem(paths[0]) : "";
  const [title, setTitle] = useState(proposed);
  // Null until the user chooses a type: the import then uses the default
  // type, also when the list is not read yet.
  const [chosen, setChosen] = useState<string | null>(null);
  const [applied, setApplied] = useState<string[]>([]);
  // The Notes field as Markdown. Only one file has the field.
  const [notes, setNotes] = useState("");
  const form = useRef<HTMLFormElement>(null);
  const notesFrame = useRef<HTMLDivElement>(null);
  const [busy, setBusy] = useState(false);
  const titleField = useRef<HTMLInputElement>(null);
  const typeField = useRef<HTMLSelectElement>(null);
  const cancelled = useRef(onCancel);
  cancelled.current = onCancel;
  // Escape does nothing while the import runs: the import still ends.
  const running = useRef(false);
  const id = useId();

  // The first field takes the focus. WKWebView does not move the focus on
  // the click that opened the sheet.
  useEffect(() => {
    if (titleField.current) {
      titleField.current.focus();
      titleField.current.select();
    } else typeField.current?.focus();
  }, []);

  // Escape and Cmd+Return go to the window: after a click on a button in
  // the sheet, WKWebView can leave the focus on the page body. The tag
  // field stops its own Escape, and the link field of the editor's toolbar
  // prevents the default of its own Escape.
  //
  // In the Notes editor, ProseMirror prevents the default of every Return
  // and Escape, so the sheet records before the editor sees the key
  // whether the editor uses it: Escape closes an open / menu, Return runs
  // the command of an open / menu, and Cmd+Return checks a task. The /
  // menu can leave the page before the key gets to the window.
  //
  // A key of an input-method composition belongs to the composition.
  useEffect(() => {
    let editorUsesKey = false;
    const before = (e: KeyboardEvent) => {
      editorUsesKey = false;
      if (!inEditor(e.target)) return;
      const slashMenuOpen = document.querySelector(SLASH_MENU) !== null;
      if (e.key === "Escape") editorUsesKey = slashMenuOpen;
      else if (isSubmitKey(e)) editorUsesKey = slashMenuOpen || selectionInTask();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.isComposing) return;
      if (inEditor(e.target) ? editorUsesKey : e.defaultPrevented) return;
      if (e.key === "Escape") {
        e.preventDefault();
        if (running.current) return;
        cancelled.current();
      } else if (isSubmitKey(e)) {
        e.preventDefault();
        form.current?.requestSubmit();
      }
    };
    window.addEventListener("keydown", before, true);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", before, true);
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  const valid = withGeneral(types?.types ?? []).filter((t) => t.valid);
  const defaultValue = types?.default_type ?? GENERAL_VALUE;
  const noteType = (): NewNoteType =>
    chosen === null
      ? { kind: "default" }
      : chosen === GENERAL_VALUE
        ? { kind: "general" }
        : { kind: "named", name: chosen };

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (busy) return;
    running.current = true;
    setBusy(true);
    // An unchanged file name stays a proposed title, so the summary can
    // replace it (R15). An empty field also gives the file name.
    const edited = title.trim();
    const userTitle = one && edited && edited !== proposed ? edited : null;
    let result: Result<ImportResult, string>;
    try {
      // A field with no text sends no notes.
      const typed = notes.trim();
      result = await commands.importFiles(paths, noteType(), applied, userTitle, one && typed ? typed : null);
    } catch {
      result = { status: "error", error: "Could not import the files." };
    }
    onDone(result);
  };

  const label = "text-right text-[0.85rem] text-label-secondary";
  const button =
    "rounded-md px-2.5 py-[3px] whitespace-nowrap outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40";
  return (
    <div className="fixed inset-0 z-20 flex items-start justify-center bg-black/20 px-4 pt-[14vh] pb-4">
      <form
        ref={form}
        role="dialog"
        aria-modal="true"
        aria-labelledby={`${id}-title`}
        onSubmit={(e) => void submit(e)}
        className={`fade-in flex max-h-[calc(86vh-1rem)] ${one ? "w-[28rem]" : "w-[24rem]"} max-w-[calc(100vw-2rem)] flex-col rounded-xl bg-group p-4 shadow-[0_0_0_0.5px_var(--separator),0_12px_40px_rgb(0_0_0/0.25)]`}
      >
        <h2 id={`${id}-title`} className="shrink-0 font-semibold">
          {one ? "Import Audio" : `Import ${paths.length} Audio Files`}
        </h2>
        <p className="mt-0.5 shrink-0 truncate text-[0.85rem] text-label-secondary">
          {one ? fileName(paths[0]) : "Each file becomes its own note."}
        </p>
        {!one && (
          <ul
            aria-label="Files"
            className="mt-2.5 max-h-32 min-h-0 shrink overflow-y-auto rounded-md bg-fill px-2.5 py-1.5 text-[0.85rem]"
          >
            {paths.map((path) => (
              <li key={path} className="truncate leading-[1.6]">
                {fileName(path)}
              </li>
            ))}
          </ul>
        )}
        <div className="mt-3.5 grid shrink-0 grid-cols-[auto_minmax(0,1fr)] items-center gap-x-3 gap-y-2.5">
          {one && (
            <>
              <label htmlFor={`${id}-name`} className={label}>
                Title
              </label>
              <input
                ref={titleField}
                id={`${id}-name`}
                value={title}
                spellCheck={false}
                autoComplete="off"
                className={`w-full text-[0.85rem] ${fieldClass}`}
                onChange={(e) => setTitle(e.target.value)}
              />
            </>
          )}
          <label htmlFor={`${id}-type`} className={label}>
            Type
          </label>
          <select
            ref={typeField}
            id={`${id}-type`}
            className={`w-full text-[0.85rem] ${fieldClass}`}
            value={chosen ?? defaultValue}
            onChange={(e) => setChosen(e.target.value)}
          >
            {valid.map((t) => (
              <option key={t.general ? "general" : t.name} value={t.general ? GENERAL_VALUE : t.name}>
                {t.general ? GENERAL : t.name}
              </option>
            ))}
          </select>
          <span className={`${label} self-start pt-[3px]`}>Tags</span>
          <div className="flex min-w-0 flex-wrap items-center gap-1">
            {applied.map((tag) => (
              <TagLabel key={tagKey(tag)} text={tag} onRemove={() => setApplied((a) => a.filter((t) => t !== tag))} />
            ))}
            <TagPicker tags={tags} applied={applied} onAdd={(tag) => setApplied((a) => [...a, tag])} />
          </div>
          {one && (
            <>
              {/* Not a <label>: the editor is not a form control. A click
                  on it puts the focus in the editor, as a label does. */}
              <span
                id={`${id}-notes`}
                className={`${label} self-start pt-[5px]`}
                onClick={() => notesFrame.current?.querySelector<HTMLElement>('[role="textbox"]')?.focus()}
              >
                Notes
              </span>
              <div
                ref={notesFrame}
                data-markdown-field
                style={
                  {
                    "--markdown-field-rows": NOTES_ROWS,
                    maxHeight: `calc(${NOTES_MAX_ROWS} * 1.5em + 6px + 2px)`,
                  } as CSSProperties
                }
                className="overflow-y-auto rounded-md border border-control-border bg-control px-2 py-[3px] text-[0.85rem] text-label outline-offset-0 focus-within:outline-3 focus-within:outline-accent/50"
              >
                <NoteEditor markdown="" onChange={setNotes} labelledBy={`${id}-notes`} immediate className="markdown-field" />
              </div>
            </>
          )}
        </div>
        <div className="mt-4 flex shrink-0 justify-end gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={() => cancelled.current()}
            className={`${button} bg-fill text-label active:bg-fill-pressed`}
          >
            Cancel
          </button>
          <button type="submit" disabled={busy} className={`${button} bg-accent text-white active:opacity-80`}>
            Import
          </button>
        </div>
      </form>
    </div>
  );
}
