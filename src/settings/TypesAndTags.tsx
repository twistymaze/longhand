// The Types and Tags pane (R12-R20, KTD7): the default type for new notes,
// the note types with their summary prompts, and the tags with their
// counts. The rows follow the macOS System Settings lists: each name is
// plain text, and only the popups and the ⋯ buttons are controls. Rename…
// and Delete… are in the ⋯ menu of a row. A rename applies on Return, and
// Escape or a click elsewhere cancels it. A delete asks for one
// confirmation that says how many notes change. A rename onto another type
// or tag merges the two, and the pane says so. A popup choice applies at
// once. Each change is one backend command.

import { type ReactNode, useCallback, useEffect, useId, useRef, useState } from "react";
import {
  commands,
  type NoteTypeEntry,
  type NoteTypeList,
  type Result,
  type SummaryPromptEntry,
  type TagEntry,
} from "../bindings";
import { GENERAL, sameType, tagKey } from "../lib/Labels";
import { useReloadOn } from "../lib/useReloadOn";
import { SessionMenu } from "../session/SessionMenu";
import { Button, ErrorText, fieldClass, Group, NameField, Pane, Row, notes, useConfirm } from "./ui";

/// Runs a command. A command that does not run gives `fallback` as its
/// error.
async function attempt<T>(op: () => Promise<Result<T, string>>, fallback: string): Promise<Result<T, string>> {
  try {
    return await op();
  } catch (e) {
    console.error(fallback, e);
    return { status: "error", error: fallback };
  }
}

/// The width of the prompt popup column. A row with no popup keeps the
/// same width empty, so the columns of all rows align (R20).
const promptColumn = "w-[14rem] min-w-0 shrink";
/// The width of the ⋯ button column.
const menuColumn = "w-7 shrink-0";

/// One row of a list: the name and the controls on one line, and an
/// optional line of detail under them. `nameId` names the row for
/// assistive technology.
function ListRow({ nameId, children, detail }: { nameId: string; children: ReactNode; detail?: ReactNode }) {
  return (
    <div role="listitem" aria-labelledby={nameId} className="settings-row px-4">
      <div className="py-1.5">
        <div className="flex min-h-[2.25rem] items-center gap-3">{children}</div>
        {detail}
      </div>
    </div>
  );
}

/// The name of a row as plain text, or the name field while the row is in
/// a rename.
function RowName({ id, text, field }: { id: string; text: string; field: ReactNode }) {
  return (
    <div className="flex min-w-[6rem] flex-1 items-center">
      {/* The field moves left by its padding and border, so its text stays where the name was. */}
      {field ? <div className="-ml-[9px] max-w-[20rem] min-w-0 flex-1">{field}</div> : (
        <span id={id} title={text} className="min-w-0 truncate">
          {text}
        </span>
      )}
    </div>
  );
}

/// The count of notes in a row, right-aligned in its column.
function Count({ n }: { n: number }) {
  return (
    <span className="w-[4.75rem] shrink-0 text-right text-[0.85rem] whitespace-nowrap text-label-secondary tabular-nums">
      {notes(n)}
    </span>
  );
}

/// What a type delete does to the notes of the type, for the confirmation.
function typeDeleteMessage(t: NoteTypeEntry): string {
  if (t.notes === 0) return "No notes have this type.";
  const verb = t.notes === 1 ? "moves" : "move";
  return `${notes(t.notes)} ${verb} from the ${t.name} folder to ${GENERAL}.`;
}

/// What a tag delete does, for the confirmation.
function tagDeleteMessage(t: TagEntry): string {
  return `${notes(t.count)} ${t.count === 1 ? "loses" : "lose"} this tag.`;
}

/// A row of a list, or the Add Type row.
type RowRef = { list: "types" | "tags"; name: string } | { list: "add" };

/// The option value of "Default" in a prompt popup and of General in the
/// default type popup. No prompt id and no type name is empty.
const NONE = "";

export function TypesAndTags({
  readOnly = null,
}: {
  /// Why the shared settings are read-only now, or null (R19).
  readOnly?: string | null;
}) {
  const [types, setTypes] = useState<NoteTypeList | null>(null);
  const [tags, setTags] = useState<TagEntry[] | null>(null);
  const [prompts, setPrompts] = useState<SummaryPromptEntry[] | null>(null);
  const [typeError, setTypeError] = useState<string | null>(null);
  const [tagError, setTagError] = useState<string | null>(null);
  // The result of the last merge, for each list.
  const [typeNotice, setTypeNotice] = useState<string | null>(null);
  const [tagNotice, setTagNotice] = useState<string | null>(null);
  // The row that has the name field open, or null.
  const [editing, setEditing] = useState<RowRef | null>(null);
  // The control that takes the focus after the next render: the ⋯ button
  // of a row, or the Add Type button.
  const [focusTo, setFocusTo] = useState<RowRef | null>(null);
  const { ask, dialog } = useConfirm();
  const ids = useId();
  const typeList = useRef<HTMLDivElement>(null);
  const tagList = useRef<HTMLDivElement>(null);
  const addButton = useRef<HTMLButtonElement>(null);

  // Reads can end out of order. Each read and each command result takes a
  // number, and only the newest number changes a list.
  const typeRequest = useRef(0);
  const tagRequest = useRef(0);
  const promptRequest = useRef(0);

  const load = useCallback(async () => {
    const typeAt = ++typeRequest.current;
    const tagAt = ++tagRequest.current;
    const promptAt = ++promptRequest.current;
    const [t, g, p] = await Promise.all([
      attempt(() => commands.noteTypes(), "The types could not be read."),
      attempt(() => commands.listTags(), "The tags could not be read."),
      commands.summaryPrompts().then(
        (list) => list.prompts,
        (e: unknown) => {
          console.error("The summary prompts could not be read.", e);
          return null;
        },
      ),
    ]);
    if (typeAt === typeRequest.current) {
      if (t.status === "ok") {
        setTypes(t.data);
        setTypeError(null);
      } else setTypeError(t.error);
    }
    if (tagAt === tagRequest.current) {
      if (g.status === "ok") {
        setTags(g.data);
        setTagError(null);
      } else setTagError(g.error);
    }
    if (promptAt === promptRequest.current && p) setPrompts(p);
  }, []);

  // A change on another computer, in the Summaries pane, on a note page or
  // in the library reads the lists again.
  useEffect(() => {
    void load();
  }, [load]);
  useReloadOn(load);

  // WKWebView does not focus a clicked button, so the focus goes back to
  // the ⋯ button of a row with an explicit focus() (KTD7). A rename can
  // change the case of the name or merge it into another row, so the row
  // is found by its name as the list compares names.
  useEffect(() => {
    if (!focusTo) return;
    setFocusTo(null);
    if (focusTo.list === "add") {
      addButton.current?.focus();
      return;
    }
    const match =
      focusTo.list === "types"
        ? types?.types.find((t) => sameType(t.name, focusTo.name))?.name
        : tags?.find((t) => t.key === tagKey(focusTo.name))?.text;
    const list = focusTo.list === "types" ? typeList.current : tagList.current;
    if (match === undefined || !list) return;
    list.querySelector<HTMLButtonElement>(`button[aria-label="${CSS.escape(`Actions for ${match}`)}"]`)?.focus();
  }, [focusTo, types, tags]);

  // Runs a type command. On success the pane shows the new list.
  const runType = async <T,>(op: () => Promise<Result<T, string>>, listOf: (data: T) => NoteTypeList) => {
    setTypeNotice(null);
    const result = await attempt(op, "The type could not be changed.");
    if (result.status === "error") {
      setTypeError(result.error);
      return false;
    }
    setTypeError(null);
    // The command result is newer than each read that started before it.
    typeRequest.current++;
    setTypes(listOf(result.data));
    return true;
  };

  const runTag = async (op: () => Promise<Result<{ tags: TagEntry[] }, string>>) => {
    setTagNotice(null);
    const result = await attempt(op, "The tag could not be changed.");
    if (result.status === "error") {
      setTagError(result.error);
      return false;
    }
    setTagError(null);
    tagRequest.current++;
    setTags(result.data.tags);
    return true;
  };

  /// Closes the name field. Escape gives the focus back to the control
  /// that opened the field; a click elsewhere leaves the focus there.
  const cancelEdit = (byKey: boolean) => {
    const was = editing;
    setEditing(null);
    if (byKey && was) setFocusTo(was);
  };

  const addType = async (name: string) => {
    if (name.trim() === "") {
      cancelEdit(true);
      return true;
    }
    const done = await runType(() => commands.addNoteType(name), (l) => l);
    if (done) {
      setEditing(null);
      setFocusTo({ list: "add" });
    }
    return done;
  };

  const renameType = async (t: NoteTypeEntry, to: string) => {
    if (to === t.name) {
      cancelEdit(true);
      return true;
    }
    // A case-only change is not a merge.
    const into = types?.types.find((o) => o.name !== t.name && sameType(o.name, to));
    const done = await runType(() => commands.renameNoteType(t.name, to), (e) => e.list);
    if (done) {
      setEditing(null);
      setFocusTo({ list: "types", name: to });
      if (into) setTypeNotice(`Merged “${t.name}” into “${into.name}”.`);
    }
    return done;
  };

  const deleteType = async (t: NoteTypeEntry) => {
    const yes = await ask({ title: `Delete “${t.name}”?`, message: typeDeleteMessage(t), action: "Delete" });
    if (yes) await runType(() => commands.deleteNoteType(t.name), (e) => e.list);
    else setFocusTo({ list: "types", name: t.name });
  };

  const renameTag = async (t: TagEntry, to: string) => {
    if (to === t.text) {
      cancelEdit(true);
      return true;
    }
    const into = tags?.find((o) => o.key !== t.key && o.key === tagKey(to));
    const done = await runTag(() => commands.renameTag(t.text, to));
    if (done) {
      setEditing(null);
      setFocusTo({ list: "tags", name: to });
      if (into) setTagNotice(`Merged “${t.text}” into “${into.text}”.`);
    }
    return done;
  };

  const deleteTag = async (t: TagEntry) => {
    const yes = await ask({ title: `Delete the tag “${t.text}”?`, message: tagDeleteMessage(t), action: "Delete" });
    if (yes) await runTag(() => commands.deleteTag(t.text));
    else setFocusTo({ list: "tags", name: t.text });
  };

  const setDefault = (name: string | null) => void runType(() => commands.setDefaultNoteType(name), (l) => l);
  const setPrompt = (t: NoteTypeEntry, id: string | null) =>
    void runType(() => commands.setNoteTypePrompt(t.name, id), (l) => l);

  const locked = readOnly !== null;
  const valid = types?.types.filter((t) => !t.general && t.valid) ?? [];
  const defaultValue = valid.find((t) => sameType(t.name, types?.default_type ?? ""))?.name ?? NONE;
  const isEditing = (list: "types" | "tags", name: string) =>
    editing !== null && editing.list === list && editing.name === name;

  const menuItems = (onRename: () => void, onDelete: () => void) => [
    { label: "Rename…", onSelect: onRename },
    { label: "Delete…", onSelect: onDelete },
  ];

  return (
    <Pane title="Types and Tags">
      {types && (
        <Group>
          <Row label="New notes get the type">
            <select
              aria-label="New notes get the type"
              className={`${fieldClass} w-[14rem]`}
              value={defaultValue}
              disabled={locked}
              onChange={(e) => setDefault(e.target.value === NONE ? null : e.target.value)}
            >
              <option value={NONE}>{GENERAL}</option>
              {valid.map((t) => (
                <option key={t.name} value={t.name}>
                  {t.name}
                </option>
              ))}
            </select>
          </Row>
        </Group>
      )}

      <Group
        header="Types"
        footer={
          locked
            ? `${readOnly} Types can’t be changed now.`
            : "Each type is a folder in the storage folder. A type set to “Default” uses the default prompt in Summaries."
        }
      >
        {types && (
          <div ref={typeList} role="list" aria-label="Types">
            {types.types.map((t, index) => {
              const nameId = `${ids}-type-${index}`;
              const warning = !t.valid && (
                <p className="mt-0.5 text-[0.85rem] leading-[1.4] text-caution">
                  {sameType(t.name, GENERAL)
                    ? "General is a built-in type. Rename this type."
                    : "This name can’t be a folder name. Rename the type."}
                </p>
              );
              const note = t.general && (
                <p className="mt-0.5 text-[0.85rem] leading-[1.4] text-label-secondary">
                  Notes at the top of the storage folder.
                </p>
              );
              return (
                <ListRow key={t.name} nameId={nameId} detail={warning || note}>
                  <RowName
                    id={nameId}
                    text={t.name}
                    field={
                      isEditing("types", t.name) && (
                        <NameField
                          label={`Name of type ${t.name}`}
                          value={t.name}
                          onApply={(v) => renameType(t, v)}
                          onCancel={cancelEdit}
                        />
                      )
                    }
                  />
                  <Count n={t.notes} />
                  {t.valid ? (
                    <select
                      aria-label={`Summary prompt for ${t.name}`}
                      className={`${fieldClass} ${promptColumn}`}
                      value={t.prompt_choice ?? NONE}
                      disabled={locked || prompts === null}
                      onChange={(e) => setPrompt(t, e.target.value === NONE ? null : e.target.value)}
                    >
                      <option value={NONE}>Default</option>
                      {prompts?.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.name}
                        </option>
                      ))}
                      {/* R7: the file of the chosen prompt is missing, and summaries use the last copy read. */}
                      {prompts && t.prompt_choice !== null && !prompts.some((p) => p.id === t.prompt_choice) && (
                        <option value={t.prompt_choice}>{t.prompt_name}</option>
                      )}
                    </select>
                  ) : (
                    <span aria-hidden className={promptColumn} />
                  )}
                  <div className={menuColumn}>
                    {!t.general && (
                      <SessionMenu
                        floating
                        label={`Actions for ${t.name}`}
                        disabled={locked}
                        items={menuItems(
                          () => setEditing({ list: "types", name: t.name }),
                          () => void deleteType(t),
                        )}
                      />
                    )}
                  </div>
                </ListRow>
              );
            })}
          </div>
        )}
        <div className="px-4">
          {/* The list is between this row and the row before it, so the row draws its own separator. */}
          <div className={`flex min-h-[2.75rem] items-center py-1.5 ${types && types.types.length > 0 ? "border-t border-separator" : ""}`}>
            {editing?.list === "add" ? (
              <div className="max-w-[20rem] min-w-0 flex-1">
                <NameField
                  label="New type"
                  placeholder="New type"
                  value=""
                  onApply={addType}
                  onCancel={cancelEdit}
                />
              </div>
            ) : (
              <Button ref={addButton} disabled={locked} onClick={() => setEditing({ list: "add" })}>
                Add Type…
              </Button>
            )}
          </div>
        </div>
        {typeNotice && (
          <p role="status" className="px-4 pb-2 text-[0.85rem] text-label-secondary">
            {typeNotice}
          </p>
        )}
        {typeError && <ErrorText>{typeError}</ErrorText>}
      </Group>

      <Group header="Tags" footer="A change applies to every note with the tag. A rename to an existing tag joins the two tags.">
        {tags && tags.length === 0 && (
          <div className="settings-row px-4">
            <p className="flex min-h-[2.25rem] items-center py-1.5 text-label-secondary">
              No tags yet. Add tags on the page of a note.
            </p>
          </div>
        )}
        {tags && tags.length > 0 && (
          <div ref={tagList} role="list" aria-label="Tags">
            {tags.map((t, index) => {
              const nameId = `${ids}-tag-${index}`;
              return (
                <ListRow key={t.key} nameId={nameId}>
                  <RowName
                    id={nameId}
                    text={t.text}
                    field={
                      isEditing("tags", t.text) && (
                        <NameField
                          label={`Name of tag ${t.text}`}
                          value={t.text}
                          onApply={(v) => renameTag(t, v)}
                          onCancel={cancelEdit}
                        />
                      )
                    }
                  />
                  <Count n={t.count} />
                  <div className={menuColumn}>
                    <SessionMenu
                      floating
                      label={`Actions for ${t.text}`}
                      items={menuItems(
                        () => setEditing({ list: "tags", name: t.text }),
                        () => void deleteTag(t),
                      )}
                    />
                  </div>
                </ListRow>
              );
            })}
          </div>
        )}
        {tagNotice && (
          <p role="status" className="px-4 pb-2 text-[0.85rem] text-label-secondary">
            {tagNotice}
          </p>
        )}
        {tagError && <ErrorText>{tagError}</ErrorText>}
      </Group>
      {dialog}
    </Pane>
  );
}
