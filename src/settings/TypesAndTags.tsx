// The Types and Tags pane (R3, R6, R7, R13, R28, R31): the note types with
// the default type, and the tags with their counts. Each change is one
// backend command. A rename applies at once and asks nothing. A delete asks
// for one confirmation that says how many notes change. A rename onto
// another type or tag merges the two, and the pane says so.

import { type KeyboardEvent, type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { commands, type NoteTypeEntry, type NoteTypeList, type Result, type TagEntry } from "../bindings";
import { GENERAL, sameType, tagKey } from "../lib/Labels";
import { useReloadOn } from "../lib/useReloadOn";
import { Button, CommitField, ErrorText, fieldClass, Group, Pane, notes, useConfirm } from "./ui";

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

/// The radio mark of one choice of the default type.
function RadioMark({ checked }: { checked: boolean }) {
  return (
    <span
      aria-hidden
      className={`flex size-[15px] shrink-0 items-center justify-center rounded-full ${
        checked ? "bg-accent" : "bg-control shadow-[inset_0_0_0_1px_var(--control-border)]"
      }`}
    >
      {checked && <span className="size-[5px] rounded-full bg-white" />}
    </span>
  );
}

/// One choice of the default type. In WKWebView a click does not give a
/// button the focus, so the click gives it.
function Radio({
  label,
  checked,
  disabled,
  onChoose,
}: {
  label: string;
  checked: boolean;
  disabled: boolean;
  onChoose: () => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-label={label}
      aria-checked={checked}
      tabIndex={checked ? 0 : -1}
      disabled={disabled}
      onClick={(e) => {
        e.currentTarget.focus();
        if (!checked) onChoose();
      }}
      className="-m-1.5 shrink-0 rounded-full p-1.5 outline-offset-0 focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40"
    >
      <RadioMark checked={checked} />
    </button>
  );
}

/// One row of a list: the controls on one line, and an optional line of
/// detail under them.
function ListRow({ children, detail }: { children: ReactNode; detail?: ReactNode }) {
  return (
    <div className="settings-row px-4">
      <div className="py-2">
        <div className="flex min-h-[2.25rem] items-center gap-3">{children}</div>
        {detail}
      </div>
    </div>
  );
}

/// The count of notes in a row.
function Count({ n }: { n: number }) {
  return <span className="shrink-0 text-[0.85rem] whitespace-nowrap text-label-secondary tabular-nums">{notes(n)}</span>;
}

/// The name field of a row. It takes the free width, to a limit.
function NameField(props: { label: string; value: string; disabled?: boolean; onCommit: (v: string) => void }) {
  return (
    <div className="min-w-0 max-w-[20rem] flex-1">
      <CommitField {...props} width="w-full" />
    </div>
  );
}

function DeleteButton({ what, name, disabled, onClick }: { what: string; name: string; disabled?: boolean; onClick: () => void }) {
  return (
    <Button disabled={disabled} onClick={onClick}>
      Delete<span className="sr-only"> {what} {name}</span>
    </Button>
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

export function TypesAndTags({
  readOnly = null,
}: {
  /// Why the shared settings are read-only now, or null (R16).
  readOnly?: string | null;
}) {
  const [types, setTypes] = useState<NoteTypeList | null>(null);
  const [tags, setTags] = useState<TagEntry[] | null>(null);
  const [typeError, setTypeError] = useState<string | null>(null);
  const [tagError, setTagError] = useState<string | null>(null);
  // The result of the last merge, for each list.
  const [typeNotice, setTypeNotice] = useState<string | null>(null);
  const [tagNotice, setTagNotice] = useState<string | null>(null);
  const [newType, setNewType] = useState("");
  const { ask, dialog } = useConfirm();

  // Reads can end out of order. Each read and each command result takes a
  // number, and only the newest number changes a list.
  const typeRequest = useRef(0);
  const tagRequest = useRef(0);

  const load = useCallback(async () => {
    const typeAt = ++typeRequest.current;
    const tagAt = ++tagRequest.current;
    const [t, g] = await Promise.all([
      attempt(() => commands.noteTypes(), "The types could not be read."),
      attempt(() => commands.listTags(), "The tags could not be read."),
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
  }, []);

  // A change on another computer, on a note page or in the library reads
  // the lists again.
  useEffect(() => {
    void load();
  }, [load]);
  useReloadOn(load);

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

  const addType = async () => {
    const name = newType;
    if (name.trim() === "") return;
    if (await runType(() => commands.addNoteType(name), (l) => l)) setNewType("");
  };

  const renameType = async (t: NoteTypeEntry, to: string) => {
    // A case-only change is not a merge.
    const into = types?.types.find((o) => o.name !== t.name && sameType(o.name, to));
    const done = await runType(() => commands.renameNoteType(t.name, to), (e) => e.list);
    if (done && into) setTypeNotice(`Merged “${t.name}” into “${into.name}”.`);
  };

  const deleteType = async (t: NoteTypeEntry) => {
    const yes = await ask({ title: `Delete “${t.name}”?`, message: typeDeleteMessage(t), action: "Delete" });
    if (yes) await runType(() => commands.deleteNoteType(t.name), (e) => e.list);
  };

  const renameTag = async (t: TagEntry, to: string) => {
    const into = tags?.find((o) => o.key !== t.key && o.key === tagKey(to));
    const done = await runTag(() => commands.renameTag(t.text, to));
    if (done && into) setTagNotice(`Merged “${t.text}” into “${into.text}”.`);
  };

  const deleteTag = async (t: TagEntry) => {
    const yes = await ask({ title: `Delete the tag “${t.text}”?`, message: tagDeleteMessage(t), action: "Delete" });
    if (yes) await runTag(() => commands.deleteTag(t.text));
  };

  const locked = readOnly !== null;
  // General is the built-in type, first in the list. Its default is null.
  const general = types?.types.find((t) => t.general);
  const named = types?.types.filter((t) => !t.general) ?? [];
  const choices: (string | null)[] = [null, ...named.filter((t) => t.valid).map((t) => t.name)];
  const setDefault = (name: string | null) =>
    void runType(() => commands.setDefaultNoteType(name), (l) => l);

  // The up and down arrows move the default through the choices, as in a
  // radio group.
  const onRadioKey = (e: KeyboardEvent<HTMLDivElement>) => {
    // The arrows in a name field do not change the default.
    const onRadio = (e.target as HTMLElement).getAttribute("role") === "radio";
    if (locked || !types || !onRadio || (e.key !== "ArrowDown" && e.key !== "ArrowUp")) return;
    e.preventDefault();
    const at = choices.indexOf(types.default_type);
    const next = Math.min(choices.length - 1, Math.max(0, at + (e.key === "ArrowDown" ? 1 : -1)));
    if (next === at) return;
    const radios = e.currentTarget.querySelectorAll<HTMLElement>('[role="radio"]');
    radios[next]?.focus();
    setDefault(choices[next]);
  };

  return (
    <Pane title="Types and Tags">
      <Group
        header="Types"
        footer={
          locked
            ? `${readOnly} Types can’t be changed now.`
            : "Each type is a folder in the storage folder. A new note gets the checked type."
        }
      >
        {types && (
          <div role="radiogroup" aria-label="Default type" onKeyDown={onRadioKey}>
            <ListRow
              detail={
                <p className="mt-0.5 pl-[27px] text-[0.85rem] leading-[1.4] text-label-secondary">
                  Notes at the top of the storage folder.
                </p>
              }
            >
              <Radio
                label={GENERAL}
                checked={types.default_type === null}
                disabled={locked}
                onChoose={() => setDefault(null)}
              />
              <span className="min-w-0 flex-1 truncate">{GENERAL}</span>
              <Count n={general?.notes ?? 0} />
            </ListRow>
            {named.map((t) => (
              <ListRow
                key={t.name}
                detail={
                  !t.valid && (
                    <p className="mt-1 pl-[27px] text-[0.85rem] leading-[1.4] text-caution">
                      {sameType(t.name, GENERAL)
                        ? "General is a built-in type. Rename this type."
                        : "This name can’t be a folder name. Rename the type."}
                    </p>
                  )
                }
              >
                {t.valid ? (
                  <Radio
                    label={t.name}
                    checked={sameType(types.default_type ?? "", t.name)}
                    disabled={locked}
                    onChoose={() => setDefault(t.name)}
                  />
                ) : (
                  <span aria-hidden className="w-[15px] shrink-0" />
                )}
                <NameField
                  label={`Name of type ${t.name}`}
                  value={t.name}
                  disabled={locked}
                  onCommit={(v) => void renameType(t, v)}
                />
                <span className="flex-1" />
                <Count n={t.notes} />
                <DeleteButton what="type" name={t.name} disabled={locked} onClick={() => void deleteType(t)} />
              </ListRow>
            ))}
          </div>
        )}
        <ListRow>
          <span aria-hidden className="w-[15px] shrink-0" />
          <div className="min-w-0 max-w-[20rem] flex-1">
            <input
              aria-label="New type"
              placeholder="New type"
              className={`${fieldClass} w-full disabled:text-label-secondary`}
              value={newType}
              disabled={locked}
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              onChange={(e) => setNewType(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void addType();
              }}
            />
          </div>
          <span className="flex-1" />
          <Button disabled={locked || newType.trim() === ""} onClick={() => void addType()}>
            Add Type
          </Button>
        </ListRow>
        {typeNotice && (
          <p role="status" className="px-4 pb-2 text-[0.85rem] text-label-secondary">
            {typeNotice}
          </p>
        )}
        {typeError && <ErrorText>{typeError}</ErrorText>}
      </Group>

      <Group header="Tags" footer="A change applies to every note with the tag. A rename to an existing tag joins the two tags.">
        {tags && tags.length === 0 && (
          <ListRow>
            <span className="text-label-secondary">No tags yet. Add tags on the page of a note.</span>
          </ListRow>
        )}
        {tags?.map((t) => (
          <ListRow key={t.key}>
            <NameField label={`Name of tag ${t.text}`} value={t.text} onCommit={(v) => void renameTag(t, v)} />
            <span className="flex-1" />
            <Count n={t.count} />
            <DeleteButton what="tag" name={t.text} onClick={() => void deleteTag(t)} />
          </ListRow>
        ))}
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
