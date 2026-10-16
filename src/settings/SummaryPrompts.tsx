// The summary prompts in the Summaries pane (R1–R4, KTD11): the list of
// prompts with the default prompt, and an editor for the selected prompt
// under it. Each change is one backend command (KTD7). The editor saves the
// prompt text and the title text as the user types: one second after the
// last change, and at once on blur or a change of prompt or pane. The two
// texts are separate fields: each has its own draft, base, save error and
// notice of a change on another computer (KTD10). Each text shows in the
// Markdown editor of My Notes (KTD7).

import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import {
  commands,
  events,
  type NoteTypeEntry,
  type RestoredPrompt,
  type Result,
  type Settings,
  type SummaryPromptEntry,
  type SummaryPromptList,
} from "../bindings";
import { defaultCloseSaveLimit, registerCloseSave } from "./closeSaves";
import { MarkdownField } from "./MarkdownField";
import { OwnSaves } from "./ownSaves";
import {
  Button,
  ChangedElsewhereNotice,
  CommitField,
  DeletedPromptNotice,
  ErrorText,
  Group,
  Row,
  useConfirm,
} from "./ui";

/// The time from the last change to the save, in milliseconds.
const saveDelay = 1000;

function CheckIcon() {
  return (
    <svg
      aria-hidden
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
      className="size-3.5 shrink-0"
    >
      <path d="M3.5 8.5l3 3 6-7" />
    </svg>
  );
}

const format = (n: number) => n.toLocaleString("en-US");

/// The text of a prompt that is over a limit (R21): the current and the
/// maximum character count of the text that is over, or null.
function overLimitText(prompt: SummaryPromptEntry, limits: Record<Field, number>): string | null {
  if (!prompt.over_limit) return null;
  if (prompt.too_large) return "File too large";
  const text = [...prompt.text].length;
  if (text > limits.text) return `${format(text)} / ${format(limits.text)} characters`;
  const title = [...(prompt.title_text ?? "")].length;
  return `Title ${format(title)} / ${format(limits.title)} characters`;
}

function PromptRow({
  prompt,
  selected,
  active,
  disabled,
  overLimit,
  onSelect,
  onUse,
}: {
  prompt: SummaryPromptEntry;
  selected: boolean;
  active: boolean;
  disabled: boolean;
  /// The counts of a prompt that is over a limit, or null.
  overLimit: string | null;
  onSelect: () => void;
  onUse: () => void;
}) {
  return (
    <div className={`settings-row px-4 ${selected ? "bg-sidebar-selected" : ""}`}>
      <div className="flex min-h-[2.75rem] items-center gap-4">
        <button
          type="button"
          role="radio"
          aria-checked={selected}
          onClick={onSelect}
          className="flex min-w-0 flex-1 items-baseline gap-2 self-stretch py-2 text-left outline-offset-2 focus-visible:outline-2 focus-visible:outline-accent"
        >
          <span className="truncate font-medium">{prompt.name}</span>
          {prompt.built_in && <span className="shrink-0 text-[0.85rem] text-label-secondary">Built-in</span>}
          {overLimit && <span className="shrink-0 text-[0.85rem] text-negative tabular-nums">{overLimit}</span>}
        </button>
        {active ? (
          <span className="flex shrink-0 items-center gap-1 text-label-secondary">
            <span className="text-accent">
              <CheckIcon />
            </span>
            Default
          </span>
        ) : (
          <Button disabled={disabled} onClick={onUse}>
            Make default<span className="sr-only"> {prompt.name}</span>
          </Button>
        )}
      </div>
    </div>
  );
}

/// The two texts of a prompt: the summary text and the title text.
type Field = "text" | "title";

/// The name of each field in the text that the editor shows.
const fieldName: Record<Field, string> = { text: "prompt text", title: "title instructions" };

/// The key of the editor state of one field of one prompt. A prompt id
/// contains no line break.
const keyOf = (id: string, field: Field) => `${field}\n${id}`;
const keysOf = (id: string) => [keyOf(id, "text"), keyOf(id, "title")];

/// The saved value of `field` of prompt `p`.
const savedOf = (p: SummaryPromptEntry, field: Field) => (field === "text" ? p.text : (p.title_text ?? ""));

type Edit = { id: string; field: Field; text: string };

/// A prompt with unsaved text that another computer deleted: the texts that
/// the editor showed, and the saved texts of the prompt.
type DeletedPrompt = {
  id: string;
  name: string;
  text: string;
  title: string;
  savedText: string;
  savedTitle: string;
  busy: boolean;
  error: string | null;
};

/// A copy of `map` with no entry for `id`.
function without<T>(map: Record<string, T>, id: string): Record<string, T> {
  if (!(id in map)) return map;
  const { [id]: _, ...rest } = map;
  return rest;
}

/// A copy of `map` with no entry for any of `ids`.
const withoutAll = <T,>(map: Record<string, T>, ids: string[]) => ids.reduce((acc, id) => without(acc, id), map);

export function SummaryPrompts({
  onSettings,
  readOnly = null,
  settingsReadOnly = readOnly,
  closeSaveLimit = defaultCloseSaveLimit,
}: {
  onSettings: (s: Settings) => void;
  /// Why the prompt files are read-only now, or null (R16).
  readOnly?: string | null;
  /// Why the settings file is read-only now, or null. The default prompt
  /// is in that file, so Make Default needs it (R22).
  settingsReadOnly?: string | null;
  /// Tests set a shorter limit.
  closeSaveLimit?: number;
}) {
  const [list, setList] = useState<SummaryPromptList | null>(null);
  // The note types, for the count in the delete confirmation. Null when
  // the list could not be read.
  const [types, setTypes] = useState<NoteTypeEntry[] | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // The state below is kept by field key (`keyOf`): one entry for each
  // field of each prompt.
  // The unsaved text of each field. A field with no entry shows its saved
  // text.
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  // The saved text that each draft started from. When the saved text
  // changes to a text that this window did not send, another computer
  // changed the field.
  const [bases, setBases] = useState<Record<string, string>>({});
  // The texts of each field that this window sent and that the saved text
  // did not show yet.
  const ownSaves = useRef(new Map<string, OwnSaves<string>>());
  const ownOf = (key: string) => {
    let own = ownSaves.current.get(key);
    if (!own) ownSaves.current.set(key, (own = new OwnSaves<string>()));
    return own;
  };
  // The number of the last list read that started.
  const lastRead = useRef(0);
  const [listError, setListError] = useState<string | null>(null);
  const [editorError, setEditorError] = useState<string | null>(null);
  // The error of the last failed save of each field. The editor shows it
  // only for that prompt, under that field.
  const [saveErrors, setSaveErrors] = useState<Record<string, string>>({});
  // The typed text of the Name field of each prompt that differs from its
  // name.
  const [nameDrafts, setNameDrafts] = useState<Record<string, string>>({});
  // The prompts with unsaved text that another computer deleted (R6). The
  // ids of the prompts that another computer deleted: a late save error of
  // such a prompt does not show. The ids of the prompts that this window
  // deleted.
  const [deleted, setDeleted] = useState<DeletedPrompt[]>([]);
  const gone = useRef(new Set<string>());
  const ownDeletes = useRef(new Set<string>());
  const { ask, dialog } = useConfirm();
  // The valid text that waits for the timer, the timer, and the last save
  // that started. Each save waits for the save before it, so the saves apply
  // in the order typed. `failed` holds the keys whose last save failed.
  const pending = useRef<Edit | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const saving = useRef<Promise<void>>(Promise.resolve());
  const failed = useRef(new Set<string>());
  const onSettingsRef = useRef(onSettings);
  onSettingsRef.current = onSettings;

  // Only the last read that started shows its list: an earlier read that
  // ends later has older files.
  const load = useCallback(async () => {
    const read = ++lastRead.current;
    try {
      const next = await commands.summaryPrompts();
      if (read !== lastRead.current) return;
      setList(next);
      try {
        const result = await commands.noteTypes();
        setTypes(result.status === "ok" ? result.data.types : null);
      } catch {
        setTypes(null);
      }
    } catch (e) {
      setListError(typeof e === "string" ? e : "The summary prompts could not be read.");
    }
  }, []);
  // A change from another computer, or from the note page, reads the
  // prompts again (R20).
  useEffect(() => {
    void load();
    const unlisten = events.settingsChanged.listen(() => void load());
    return () => {
      void unlisten.then((f) => f());
    };
  }, [load]);

  const cancel = useCallback(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    pending.current = null;
  }, []);

  // Starts the save of the pending text at once. Resolves when all saves
  // that started are complete: to false if the last save of a field of
  // prompt `id` failed, or with no `id`, if the last save of a field failed.
  const flush = useCallback(
    (id?: string) => {
      const sent = pending.current;
      cancel();
      if (sent) {
        const key = keyOf(sent.id, sent.field);
        // The error of the summary text comes from the backend as it is.
        // The error of the title text names the field, because the backend
        // error does not always name it.
        const fail = (reason: string | null) => {
          // A prompt that another computer deleted shows its notice instead.
          if (gone.current.has(sent.id)) return;
          const error =
            sent.field === "text"
              ? (reason ?? "The prompt text could not be saved.")
              : `The title instructions could not be saved.${reason ? ` ${reason}` : ""}`;
          failed.current.add(key);
          setSaveErrors((m) => ({ ...m, [key]: error }));
        };
        const own = ownOf(key);
        own.sent(sent.text);
        const save = sent.field === "text" ? commands.setSummaryPromptText : commands.setSummaryPromptTitleText;
        saving.current = saving.current
          .then(async () => {
            let result: Awaited<ReturnType<typeof save>>;
            try {
              result = await save(sent.id, sent.text);
            } finally {
              own.settled();
            }
            if (result.status === "error") {
              fail(result.error);
              return;
            }
            failed.current.delete(key);
            setSaveErrors((m) => without(m, key));
            // The base moves when the list shows this save, not here: until
            // the list is read again, it has the old text.
            onSettingsRef.current(result.data);
            await load();
            // Clear the draft only if it still holds the sent text, so that
            // text typed during the save stays for the next save.
            setDrafts((m) => (m[key] === sent.text ? without(m, key) : m));
          })
          .catch((e: unknown) => {
            // A failed save must not stop the saves after it.
            console.error(`The ${fieldName[sent.field]} could not be saved:`, e);
            fail(null);
          });
      }
      return saving.current.then(() =>
        id === undefined ? failed.current.size === 0 : !keysOf(id).some((k) => failed.current.has(k)),
      );
    },
    [cancel, load],
  );

  // A pane change unmounts the pane, and the save of the pending text then
  // starts. The registration stays until all saves that started are
  // complete, also a save that waits behind a running save, so that a window
  // close during the saves waits for them. A window close and the page
  // events go through the close coordinator (KTD9).
  useEffect(() => {
    const remove = registerCloseSave(() => flush(), closeSaveLimit);
    return () => {
      void flush().finally(remove);
    };
  }, [flush, closeSaveLimit]);

  // A field whose unsaved text started from a saved text that another
  // computer then changed. The editor keeps the unsaved text.
  const changedElsewhere = (p: SummaryPromptEntry, field: Field) => {
    const key = keyOf(p.id, field);
    const saved = savedOf(p, field);
    return (
      key in drafts &&
      drafts[key] !== saved &&
      key in bases &&
      saved !== bases[key] &&
      !ownSaves.current.get(key)?.has(saved)
    );
  };
  const held =
    list?.prompts.flatMap((p) =>
      (["text", "title"] as const).filter((f) => changedElsewhere(p, f)).map((f) => keyOf(p.id, f)),
    ) ?? [];
  const heldKey = held.join("\0");
  // The unsaved text of such a field waits for Reload or Keep My Text, and
  // does not replace the text of the other computer.
  useEffect(() => {
    const sent = pending.current;
    if (sent && held.includes(keyOf(sent.id, sent.field))) cancel();
  }, [heldKey, cancel]);
  // A base goes with its draft.
  useEffect(() => {
    setBases((m) => {
      const kept = Object.fromEntries(Object.entries(m).filter(([key]) => key in drafts));
      return Object.keys(kept).length === Object.keys(m).length ? m : kept;
    });
  }, [drafts]);

  // The editor of a field gets a new key only on a new document (KTD10): a
  // change of prompt or field, Reload, and a change from another computer
  // while the field has no draft. An own save, the notice of a change on
  // another computer and a read-only change keep the editor. An own save
  // moves the base of a draft to the saved text.
  const [generations, setGenerations] = useState<Record<string, number>>({});
  const bump = (key: string) => setGenerations((m) => ({ ...m, [key]: (m[key] ?? 0) + 1 }));
  // The saved text of each field at the last check.
  const shownSaved = useRef(new Map<string, string>());
  // The list at the last check.
  const shownList = useRef<SummaryPromptList | null>(null);
  useEffect(() => {
    // A prompt with unsaved text that is not in the new list was deleted on
    // another computer. Its text stays in a deleted prompt entry, so that it
    // is not lost with no notice (R6).
    const ids = new Set(list?.prompts.map((p) => p.id));
    // A prompt that a sync brings back shows its save errors again.
    for (const id of ids) gone.current.delete(id);
    const lost = (shownList.current?.prompts ?? []).filter(
      (p) =>
        !p.built_in &&
        !ids.has(p.id) &&
        !ownDeletes.current.has(p.id) &&
        (p.id in nameDrafts || keysOf(p.id).some((k) => k in drafts || failed.current.has(k))),
    );
    shownList.current = list;
    if (lost.length > 0) {
      const entries = lost.map((p) => ({
        id: p.id,
        // A prompt can't have an empty name, so a cleared Name field keeps
        // the saved name.
        name: nameDrafts[p.id]?.trim() ? nameDrafts[p.id] : p.name,
        text: drafts[keyOf(p.id, "text")] ?? p.text,
        title: drafts[keyOf(p.id, "title")] ?? savedOf(p, "title"),
        savedText: p.text,
        savedTitle: savedOf(p, "title"),
        busy: false,
        error: null,
      }));
      const keys = lost.flatMap((p) => keysOf(p.id));
      for (const p of lost) gone.current.add(p.id);
      if (pending.current && gone.current.has(pending.current.id)) cancel();
      for (const key of keys) failed.current.delete(key);
      setDrafts((m) => withoutAll(m, keys));
      setSaveErrors((m) => withoutAll(m, keys));
      setNameDrafts((m) => lost.reduce((acc, p) => without(acc, p.id), m));
      setDeleted((d) => [...d, ...entries]);
    }
    for (const p of list?.prompts ?? []) {
      for (const field of ["text", "title"] as const) {
        const key = keyOf(p.id, field);
        const saved = savedOf(p, field);
        const before = shownSaved.current.get(key);
        shownSaved.current.set(key, saved);
        if (before === undefined || before === saved) continue;
        if (ownOf(key).confirm(saved)) {
          if (key in drafts) setBases((m) => ({ ...m, [key]: saved }));
        } else if (!(key in drafts)) bump(key);
      }
    }
    // Only a new list is a change of the saved texts.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [list]);

  // A selection that is gone (a deleted prompt) goes back to the default prompt.
  const selected =
    list?.prompts.find((p) => p.id === selectedId) ?? list?.prompts.find((p) => p.id === list.active) ?? null;
  // The list and the selection of the last render, for commits that end
  // after it or run at an unmount.
  const listRef = useRef(list);
  listRef.current = list;
  const selectedRef = useRef<string | null>(null);
  selectedRef.current = selected?.id ?? null;
  const limits: Record<Field, number> = { text: list?.text_limit ?? 0, title: list?.title_text_limit ?? 0 };
  const draftOf = (p: SummaryPromptEntry, field: Field) => {
    const key = keyOf(p.id, field);
    return key in drafts ? drafts[key] : savedOf(p, field);
  };
  const name = selected?.name ?? "";
  // The summary text can't be empty. The title text can be empty (R4).
  const valid = (field: Field, text: string) =>
    (field === "title" || text.trim() !== "") && [...text].length <= limits[field];

  // Resolves to true when the command saved.
  const run = async (
    op: () => Promise<Result<Settings, string>>,
    setError: (e: string | null) => void,
    after?: (s: Settings) => void,
  ): Promise<boolean> => {
    const result = await op();
    if (result.status === "error") {
      setError(result.error);
      return false;
    }
    setError(null);
    onSettings(result.data);
    await load();
    after?.(result.data);
    return true;
  };

  const show = (id: string | null) => {
    setEditorError(null);
    setSelectedId(id);
  };

  // A new prompt is the one that the list did not have before. The settings
  // list the prompts by name, so a new prompt is not always the last one.
  const showNewest = (s: Settings) => {
    const before = new Set(list?.prompts.map((p) => p.id));
    const newest = s.summary_prompts.find((p) => !before.has(p.id));
    if (newest) show(newest.id);
  };

  // Text that is not valid is not saved, and a change of prompt drops it.
  // Valid text stays until its save is complete. If the save fails, the text
  // and the error stay with their prompt and field.
  const select = (id: string) => {
    if (!selected || id === selected.id) return;
    const from = selected.id;
    const dropped = (["text", "title"] as const)
      .map((f) => [f, keyOf(from, f)] as const)
      .filter(([f, key]) => key in drafts && !valid(f, drafts[key]))
      .map(([, key]) => key);
    for (const key of dropped) failed.current.delete(key);
    if (dropped.length > 0) setDrafts((m) => withoutAll(m, dropped));
    void flush();
    show(id);
  };

  // New and Duplicate stop if the save of the selected prompt fails.
  const create = async () => {
    if (selected && !(await flush(selected.id))) return;
    await run(() => commands.createSummaryPrompt(), setListError, showNewest);
  };

  if (!list || !selected) {
    return listError ? (
      <Group header="Prompts">
        <ErrorText>{listError}</ErrorText>
      </Group>
    ) : null;
  }

  const id = selected.id;
  // A file over the size limit has no text in the list (R21).
  const tooLarge = selected.too_large;
  const empty = !selected.built_in && !tooLarge && draftOf(selected, "text").trim() === "";
  // The copy gets the texts that the editor shows.
  const duplicate = async () => {
    if (!(await flush(id))) return;
    await run(() => commands.duplicateSummaryPrompt(id), setEditorError, showNewest);
  };
  // A save of the same prompt that already started completes first.
  const remove = async () => {
    // The types that choose this prompt. A type set to "Default" is not
    // counted, even when the prompt is the default prompt (R8).
    const users = types?.filter((t) => t.prompt_choice === id).length ?? 0;
    const message =
      users > 0
        ? `${users === 1 ? "1 type uses" : `${users} types use`} this prompt. ${users === 1 ? "It changes" : "They change"} to Default. This can’t be undone.`
        : "This can’t be undone.";
    const yes = await ask({ title: `Delete “${name}”?`, message, action: "Delete" });
    if (!yes) return;
    if (pending.current?.id === id) cancel();
    await saving.current;
    // The list without the prompt is not a delete on another computer.
    ownDeletes.current.add(id);
    const done = await run(() => commands.deleteSummaryPrompt(id), setEditorError, () => {
      const keys = keysOf(id);
      for (const key of keys) failed.current.delete(key);
      setDrafts((m) => withoutAll(m, keys));
      setSaveErrors((m) => withoutAll(m, keys));
      setNameDrafts((m) => without(m, id));
      show(null);
    });
    if (!done) ownDeletes.current.delete(id);
  };
  const updateDeleted = (gid: string, change: Partial<DeletedPrompt> | null) =>
    setDeleted((d) =>
      change === null ? d.filter((e) => e.id !== gid) : d.map((e) => (e.id === gid ? { ...e, ...change } : e)),
    );
  // Save as New Prompt makes a prompt with the texts of a deleted prompt
  // entry, in one command. A text that a prompt can't have (empty, over the
  // limit) is made with the saved text, and the kept text becomes the
  // unsaved text of the new prompt, so that the user can correct it.
  const restore = async (entry: DeletedPrompt) => {
    updateDeleted(entry.id, { busy: true, error: null });
    const textOk = valid("text", entry.text);
    const titleOk = valid("title", entry.title);
    let result: Result<RestoredPrompt, string>;
    try {
      result = await commands.restoreSummaryPrompt(
        entry.name,
        textOk ? entry.text : entry.savedText,
        titleOk ? entry.title : entry.savedTitle,
      );
    } catch (e) {
      console.error("The prompt could not be saved:", e);
      result = { status: "error", error: "The prompt could not be saved." };
    }
    if (result.status === "error") {
      updateDeleted(entry.id, { busy: false, error: result.error });
      return;
    }
    const newId = result.data.id;
    const kept: Edit[] = [
      ...(textOk ? [] : [{ id: newId, field: "text" as const, text: entry.text }]),
      ...(titleOk ? [] : [{ id: newId, field: "title" as const, text: entry.title }]),
    ];
    const savedOfField = (field: Field) => (field === "text" ? entry.savedText : entry.savedTitle);
    setBases((m) => kept.reduce((acc, e) => ({ ...acc, [keyOf(e.id, e.field)]: savedOfField(e.field) }), m));
    setDrafts((m) => kept.reduce((acc, e) => ({ ...acc, [keyOf(e.id, e.field)]: e.text }), m));
    updateDeleted(entry.id, null);
    onSettings(result.data.settings);
    await load();
    show(newId);
  };
  const discard = async (entry: DeletedPrompt) => {
    const yes = await ask({
      title: `Discard your changes to “${entry.name}”?`,
      message: "This can’t be undone.",
      action: "Discard",
    });
    if (yes) updateDeleted(entry.id, null);
  };
  // The key of the text that waits for the timer, or null.
  const waitingKey = () => (pending.current ? keyOf(pending.current.id, pending.current.field) : null);
  // The text stays editable while a save runs. Only valid text waits for
  // the timer; the last valid text stays saved. Text of a field that
  // changed on another computer does not wait for the timer: only Keep My
  // Text saves it. A waiting save of the other field starts at once, so an
  // edit in one field never drops the save of the other.
  const change = (field: Field, text: string) => {
    const key = keyOf(id, field);
    const saved = savedOf(selected, field);
    setBases((m) => (key in m ? m : { ...m, [key]: saved }));
    setDrafts((m) => ({ ...m, [key]: text }));
    if (waitingKey() !== null && waitingKey() !== key) void flush();
    cancel();
    if (!valid(field, text) || changedElsewhere(selected, field)) return;
    pending.current = { id, field, text };
    timer.current = setTimeout(() => void flush(), saveDelay);
  };
  // Reload shows the text of the other computer and drops the unsaved text.
  const reload = (field: Field) => {
    const key = keyOf(id, field);
    if (waitingKey() === key) cancel();
    failed.current.delete(key);
    setDrafts((m) => without(m, key));
    setSaveErrors((m) => without(m, key));
    bump(key);
  };
  // Keep My Text saves the unsaved text over the text of the other computer.
  const keep = (field: Field) => {
    const key = keyOf(id, field);
    const saved = savedOf(selected, field);
    setBases((m) => ({ ...m, [key]: saved }));
    const draft = draftOf(selected, field);
    if (valid(field, draft)) {
      if (waitingKey() !== null && waitingKey() !== key) void flush();
      pending.current = { id, field, text: draft };
      void flush();
    }
  };
  const locked = readOnly !== null;
  const defaultLocked = settingsReadOnly !== null;
  const fieldProps = (field: Field) => ({
    field,
    inputId: `prompt-${field}-${id}`,
    // The key holds the prompt id and the field, so a change of prompt
    // mounts a new editor. The editor mounts from the draft when one exists.
    editorKey: `${keyOf(id, field)}\n${generations[keyOf(id, field)] ?? 0}`,
    value: draftOf(selected, field),
    limit: limits[field],
    readOnly: selected.built_in || locked || tooLarge,
    showCount: !selected.built_in && !tooLarge,
    changed: changedElsewhere(selected, field),
    saveError: saveErrors[keyOf(id, field)] ?? null,
    onChange: (text: string) => change(field, text),
    onBlur: () => void flush(),
    onKeep: () => keep(field),
    onReload: () => reload(field),
  });

  return (
    <>
      <Group
        header="Prompts"
        footer={
          locked
            ? `${readOnly} Prompts can’t be changed now.`
            : defaultLocked
              ? `${settingsReadOnly} The default prompt can’t be changed now.`
              : "The default prompt writes the summary of each type set to “Default”. Summaries you already have don’t change."
        }
      >
        <div role="radiogroup" aria-label="Summary prompts">
          {list.prompts.map((p) => (
            <PromptRow
              key={p.id}
              prompt={p}
              selected={p.id === id}
              active={p.id === list.active}
              disabled={defaultLocked}
              overLimit={overLimitText(p, limits)}
              onSelect={() => select(p.id)}
              onUse={() => void run(() => commands.activateSummaryPrompt(p.id), setListError)}
            />
          ))}
        </div>
        <div className="settings-row px-4">
          <div className="py-2.5">
            <Button disabled={locked} onClick={() => void create()}>
              New Prompt
            </Button>
          </div>
        </div>
        {listError && <ErrorText>{listError}</ErrorText>}
      </Group>

      <Group>
        {deleted.map((entry) => (
          <Fragment key={entry.id}>
            <DeletedPromptNotice
              name={entry.name}
              busy={entry.busy}
              saveDisabled={entry.busy || locked}
              onDiscard={() => void discard(entry)}
              onSave={() => void restore(entry)}
            />
            {entry.error && <ErrorText>{entry.error}</ErrorText>}
          </Fragment>
        ))}
        <Row label="Name">
          <CommitField
            key={id}
            label="Name"
            value={name}
            disabled={selected.built_in || locked}
            onDraft={(d) => setNameDrafts((m) => (d === null ? without(m, id) : { ...m, [id]: d }))}
            onCommit={async (v) => {
              // A prompt that another computer deleted gets no rename: its
              // deleted prompt entry keeps the name.
              if (!listRef.current?.prompts.some((p) => p.id === id)) return false;
              const done = await run(
                () => commands.renameSummaryPrompt(id, v),
                // The error of a rename that ends after a change of prompt
                // does not show under the other prompt.
                (e) => {
                  if (selectedRef.current === id) setEditorError(e);
                },
              );
              if (done) setNameDrafts((m) => without(m, id));
              return done;
            }}
          />
        </Row>
        {tooLarge && (
          <ErrorText>
            The file of this prompt is too large to show. Summaries use the default prompt instead. Make the file
            shorter, or delete the prompt.
          </ErrorText>
        )}
        <PromptField {...fieldProps("text")} label="Prompt text" detail="How to write the summary." rows={14} />
        {empty && <ErrorText>A prompt can’t be empty.</ErrorText>}
        {!tooLarge && <OverLimitText field="text" text={draftOf(selected, "text")} limit={limits.text} />}
        <PromptField
          {...fieldProps("title")}
          label="Title instructions"
          detail="How to write the note title. This can be empty."
          rows={5}
        />
        {!tooLarge && <OverLimitText field="title" text={draftOf(selected, "title")} limit={limits.title} />}
        <div className="settings-row px-4">
          <div className="flex min-h-[3.25rem] items-center gap-2 py-2.5">
            {selected.built_in ? (
              <p className="min-w-0 flex-1 text-[0.85rem] leading-[1.4] text-label-secondary">
                Built-in prompts can’t be changed. Duplicate one to make your own.
              </p>
            ) : (
              <span className="flex-1" />
            )}
            <Button disabled={locked || tooLarge} onClick={() => void duplicate()}>
              Duplicate
            </Button>
            {!selected.built_in && (
              <Button disabled={locked} onClick={() => void remove()}>
                Delete
              </Button>
            )}
          </div>
        </div>
        {editorError && <ErrorText>{editorError}</ErrorText>}
      </Group>
      {dialog}
    </>
  );
}

/// What to do with a text over its limit (R21). A save is possible only at or
/// under the limit, and summaries use the whole text until then.
function OverLimitText({ field, text, limit }: { field: Field; text: string; limit: number }) {
  if ([...text].length <= limit) return null;
  const what = field === "text" ? "The prompt text is" : "The title instructions are";
  return (
    <ErrorText>
      {`${what} over the limit. Summaries use the whole text. To save a change, use ${format(limit)} characters or fewer.`}
    </ErrorText>
  );
}

/// One text of the selected prompt: its label, the Markdown editor, the count,
/// the notice of a change on another computer and the save error. The
/// notice and the error name the field.
function PromptField({
  field,
  inputId,
  editorKey,
  label,
  detail,
  rows,
  value,
  limit,
  readOnly,
  showCount,
  changed,
  saveError,
  onChange,
  onBlur,
  onKeep,
  onReload,
}: {
  field: Field;
  inputId: string;
  /// The key of the editor (KTD10).
  editorKey: string;
  label: string;
  detail: string;
  rows: number;
  value: string;
  limit: number;
  readOnly: boolean;
  showCount: boolean;
  changed: boolean;
  saveError: string | null;
  onChange: (text: string) => void;
  onBlur: () => void;
  onKeep: () => void;
  onReload: () => void;
}) {
  // The summary buttons keep their short names. The title buttons add the
  // field name for screen readers, because the two notices can show at the
  // same time.
  const suffix = field === "title" ? <span className="sr-only"> title instructions</span> : null;
  return (
    <>
      <div className="settings-row px-4">
        <div className="py-3">
          <MarkdownField
            key={editorKey}
            id={inputId}
            label={label}
            hint={detail}
            value={value}
            count={[...value].length}
            limit={limit}
            showCount={showCount}
            readOnly={readOnly}
            rows={rows}
            onChange={onChange}
            onBlur={onBlur}
          />
        </div>
      </div>
      {changed && <ChangedElsewhereNotice name={fieldName[field]} suffix={suffix} onKeep={onKeep} onReload={onReload} />}
      {saveError && <ErrorText>{saveError}</ErrorText>}
    </>
  );
}
