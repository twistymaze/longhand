// The summary prompts in the Summaries pane (R1–R4, KTD11): the list of
// prompts with the prompt in use, and an editor for the selected prompt
// under it. Each change is one backend command (KTD7). The editor saves the
// prompt text and the title text as the user types: one second after the
// last change, and at once on blur or a change of prompt or pane. The two
// texts are separate fields: each has its own draft, base, save error and
// notice of a change on another computer (KTD10).

import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useRef, useState } from "react";
import { commands, events, type Result, type Settings, type SummaryPromptEntry, type SummaryPromptList } from "../bindings";
import { log, logText } from "../lib/log";
import { Button, CommitField, ErrorText, Group, Row, fieldClass, useConfirm } from "./ui";

/// The time from the last change to the save, in milliseconds.
const saveDelay = 1000;

/// The longest time that a window close waits for the save, in
/// milliseconds. A save that does not complete must not keep the window
/// open.
const defaultCloseSaveLimit = 2000;

/// The longest time that a window close waits for each later step (the
/// listener removal, the destroy), in milliseconds.
const stepLimit = 1000;

type Outcome<T> = { kind: "done"; value: T } | { kind: "error"; error: unknown } | { kind: "timeout" };

/// The result of `promise`, or a timeout when it does not settle in `ms`.
function within<T>(promise: Promise<T>, ms: number): Promise<Outcome<T>> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve({ kind: "timeout" }), ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve({ kind: "done", value });
      },
      (error: unknown) => {
        clearTimeout(timer);
        resolve({ kind: "error", error });
      },
    );
  });
}

const format = (n: number) => n.toLocaleString("en-US");

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

function PromptRow({
  prompt,
  selected,
  active,
  disabled,
  onSelect,
  onUse,
}: {
  prompt: SummaryPromptEntry;
  selected: boolean;
  active: boolean;
  disabled: boolean;
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
        </button>
        {active ? (
          <span className="flex shrink-0 items-center gap-1 text-label-secondary">
            <span className="text-accent">
              <CheckIcon />
            </span>
            In use
          </span>
        ) : (
          <Button disabled={disabled} onClick={onUse}>
            Use<span className="sr-only"> {prompt.name}</span>
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

/// A copy of `map` with no entry for `id`.
function without<T>(map: Record<string, T>, id: string): Record<string, T> {
  if (!(id in map)) return map;
  const { [id]: _, ...rest } = map;
  return rest;
}

export function SummaryPrompts({
  onSettings,
  readOnly = null,
  closeSaveLimit = defaultCloseSaveLimit,
}: {
  onSettings: (s: Settings) => void;
  /// Why the shared settings are read-only now, or null (R16).
  readOnly?: string | null;
  /// Tests set a shorter limit.
  closeSaveLimit?: number;
}) {
  const [list, setList] = useState<SummaryPromptList | null>(null);
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
  // The texts that this window sent.
  const sentTexts = useRef(new Map<string, Set<string>>());
  const [listError, setListError] = useState<string | null>(null);
  const [editorError, setEditorError] = useState<string | null>(null);
  // The error of the last failed save of each field. The editor shows it
  // only for that prompt, under that field.
  const [saveErrors, setSaveErrors] = useState<Record<string, string>>({});
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

  const load = useCallback(async () => {
    try {
      setList(await commands.summaryPrompts());
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
          const error =
            sent.field === "text"
              ? (reason ?? "The prompt text could not be saved.")
              : `The title instructions could not be saved.${reason ? ` ${reason}` : ""}`;
          failed.current.add(key);
          setSaveErrors((m) => ({ ...m, [key]: error }));
        };
        const texts = sentTexts.current.get(key) ?? new Set<string>();
        sentTexts.current.set(key, texts.add(sent.text));
        const save = sent.field === "text" ? commands.setSummaryPromptText : commands.setSummaryPromptTitleText;
        saving.current = saving.current
          .then(async () => {
            const result = await save(sent.id, sent.text);
            if (result.status === "error") {
              fail(result.error);
              return;
            }
            failed.current.delete(key);
            setSaveErrors((m) => without(m, key));
            setBases((m) => ({ ...m, [key]: sent.text }));
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

  // A pane change unmounts the pane, and the save then runs with no wait.
  // A window close destroys the page but does not unmount the pane, so the
  // close waits for the save, for at most `closeSaveLimit`. The window
  // closes also if the save fails or does not complete, and the close asks
  // nothing. When the app quits, the page events start the save, but
  // nothing waits for it. Each close step goes to the app log.
  useEffect(() => {
    const onHide = () => void flush();
    window.addEventListener("pagehide", onHide);
    window.addEventListener("beforeunload", onHide);
    const win = getCurrentWindow();
    let stop: (() => unknown) | undefined;
    let gone = false;
    let closing = false;
    const close = async () => {
      log("info", "close requested");
      const saved = await within(flush(), closeSaveLimit);
      if (saved.kind === "timeout") log("warn", `the save did not complete in ${closeSaveLimit} ms; the window closes`);
      else if (saved.kind === "error") log("error", `the save failed: ${logText(saved.error)}`);
      else if (saved.value) log("info", "the save is complete");
      else log("error", "the save failed; the window closes with prompt text that is not saved");
      // Remove the listener before the destroy. Tauri keeps the listener
      // record of a destroyed webview, and that record stops the close of
      // the next Settings window.
      const removed = await within(Promise.resolve(stop?.()), stepLimit);
      if (removed.kind !== "done") log("warn", `the close listener was not removed (${removed.kind})`);
      stop = undefined;
      log("info", "destroy called");
      const destroyed = await within(win.destroy(), stepLimit);
      if (destroyed.kind === "done") return;
      log(
        "error",
        destroyed.kind === "error"
          ? `destroy failed: ${logText(destroyed.error)}`
          : `destroy did not complete in ${stepLimit} ms`,
      );
      // The backend destroys the window with no window permission and no
      // close request.
      const closed = await within(commands.closeSettingsWindow(), stepLimit);
      if (closed.kind === "done" && closed.value.status === "ok") return;
      const why =
        closed.kind === "timeout"
          ? `no answer in ${stepLimit} ms`
          : closed.kind === "error"
            ? logText(closed.error)
            : closed.value.status === "error"
              ? closed.value.error
              : "";
      log("error", `the backend close failed: ${why}`);
    };
    win
      .onCloseRequested(async (event) => {
        event.preventDefault();
        // A second request while the window closes changes nothing.
        if (closing) return;
        closing = true;
        try {
          await close();
        } finally {
          closing = false;
        }
      })
      .then((f) => {
        // The pane can unmount before the listener is registered.
        if (gone) f();
        else stop = f;
      })
      .catch((e: unknown) => log("error", `the close listener could not be registered: ${logText(e)}`));
    return () => {
      gone = true;
      stop?.();
      window.removeEventListener("pagehide", onHide);
      window.removeEventListener("beforeunload", onHide);
      void flush();
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
      !sentTexts.current.get(key)?.has(saved)
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

  // A selection that is gone (a deleted prompt) goes back to the prompt in use.
  const selected =
    list?.prompts.find((p) => p.id === selectedId) ?? list?.prompts.find((p) => p.id === list.active) ?? null;
  const limits: Record<Field, number> = { text: list?.text_limit ?? 0, title: list?.title_text_limit ?? 0 };
  const draftOf = (p: SummaryPromptEntry, field: Field) => {
    const key = keyOf(p.id, field);
    return key in drafts ? drafts[key] : savedOf(p, field);
  };
  const name = selected?.name ?? "";
  // The summary text can't be empty. The title text can be empty (R4).
  const valid = (field: Field, text: string) =>
    (field === "title" || text.trim() !== "") && [...text].length <= limits[field];

  const run = async (
    op: () => Promise<Result<Settings, string>>,
    setError: (e: string | null) => void,
    after?: (s: Settings) => void,
  ) => {
    const result = await op();
    if (result.status === "error") {
      setError(result.error);
      return;
    }
    setError(null);
    onSettings(result.data);
    await load();
    after?.(result.data);
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
    if (dropped.length > 0) setDrafts((m) => dropped.reduce((acc, key) => without(acc, key), m));
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
  const empty = !selected.built_in && draftOf(selected, "text").trim() === "";
  // The copy gets the texts that the editor shows.
  const duplicate = async () => {
    if (!(await flush(id))) return;
    await run(() => commands.duplicateSummaryPrompt(id), setEditorError, showNewest);
  };
  // A save of the same prompt that already started completes first.
  const remove = async () => {
    const yes = await ask({ title: `Delete “${name}”?`, message: "This can’t be undone.", action: "Delete" });
    if (!yes) return;
    if (pending.current?.id === id) cancel();
    await saving.current;
    await run(() => commands.deleteSummaryPrompt(id), setEditorError, () => {
      const keys = keysOf(id);
      for (const key of keys) failed.current.delete(key);
      setDrafts((m) => keys.reduce((acc, key) => without(acc, key), m));
      setSaveErrors((m) => keys.reduce((acc, key) => without(acc, key), m));
      show(null);
    });
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
    setDrafts((m) => without(m, key));
    setSaveErrors((m) => without(m, key));
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
  const fieldProps = (field: Field) => ({
    field,
    inputId: `prompt-${field}-${id}`,
    value: draftOf(selected, field),
    limit: limits[field],
    readOnly: selected.built_in || locked,
    showCount: !selected.built_in,
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
            : "The prompt in use writes each new summary. Summaries you already have don’t change."
        }
      >
        <div role="radiogroup" aria-label="Summary prompts">
          {list.prompts.map((p) => (
            <PromptRow
              key={p.id}
              prompt={p}
              selected={p.id === id}
              active={p.id === list.active}
              disabled={locked}
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
        <Row label="Name">
          <CommitField
            key={id}
            label="Name"
            value={name}
            disabled={selected.built_in || locked}
            onCommit={(v) => void run(() => commands.renameSummaryPrompt(id, v), setEditorError)}
          />
        </Row>
        <PromptField {...fieldProps("text")} label="Prompt text" detail="How to write the summary." rows={14} />
        {empty && <ErrorText>A prompt can’t be empty.</ErrorText>}
        <PromptField
          {...fieldProps("title")}
          label="Title instructions"
          detail="How to write the note title. This can be empty."
          rows={5}
        />
        <div className="settings-row px-4">
          <div className="flex min-h-[3.25rem] items-center gap-2 py-2.5">
            {selected.built_in ? (
              <p className="min-w-0 flex-1 text-[0.85rem] leading-[1.4] text-label-secondary">
                Built-in prompts can’t be changed. Duplicate one to make your own.
              </p>
            ) : (
              <span className="flex-1" />
            )}
            <Button disabled={locked} onClick={() => void duplicate()}>
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

/// One text of the selected prompt: its label, the text area, the count,
/// the notice of a change on another computer and the save error. The
/// notice and the error name the field.
function PromptField({
  field,
  inputId,
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
  const count = [...value].length;
  const detailId = `${inputId}-detail`;
  // The summary buttons keep their short names. The title buttons add the
  // field name for screen readers, because the two notices can show at the
  // same time.
  const suffix = field === "title" ? <span className="sr-only"> title instructions</span> : null;
  return (
    <>
      <div className="settings-row px-4">
        <div className="py-3">
          <div className="mb-2 flex items-end gap-3">
            <div className="min-w-0 flex-1">
              <label htmlFor={inputId} className="font-medium">
                {label}
              </label>
              <p id={detailId} className="mt-0.5 text-[0.85rem] leading-[1.4] text-label-secondary">
                {detail}
              </p>
            </div>
            {showCount && (
              <span
                className={`shrink-0 text-[0.85rem] tabular-nums ${count > limit ? "text-negative" : "text-label-secondary"}`}
              >
                {`${format(count)} / ${format(limit)}`}
              </span>
            )}
          </div>
          <textarea
            id={inputId}
            aria-describedby={detailId}
            value={value}
            readOnly={readOnly}
            rows={rows}
            onChange={(e) => onChange(e.target.value)}
            onBlur={onBlur}
            className={`${fieldClass} block w-full resize-y py-1.5 leading-[1.45] read-only:text-label-secondary`}
          />
        </div>
      </div>
      {changed && (
        <div className="settings-row px-4">
          <div className="flex min-h-[3.25rem] flex-wrap items-center gap-2 py-2.5">
            <div role="status" className="min-w-0 flex-1 text-[0.85rem] leading-[1.4]">
              <p className="font-medium text-caution">{`The ${fieldName[field]} changed on another computer.`}</p>
              <p className="text-label-secondary">Reload shows the new text. Keep My Text saves your text.</p>
            </div>
            <Button onClick={onKeep}>
              Keep My Text{suffix}
            </Button>
            <Button prominent onClick={onReload}>
              Reload{suffix}
            </Button>
          </div>
        </div>
      )}
      {saveError && <ErrorText>{saveError}</ErrorText>}
    </>
  );
}
