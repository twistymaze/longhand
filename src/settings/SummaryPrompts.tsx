// The summary prompts in the Summaries pane (R1–R4, KTD11): the list of
// prompts with the prompt in use, and an editor for the selected prompt
// under it. Each change is one backend command (KTD7). The editor saves the
// prompt text as the user types: one second after the last change, and at
// once on blur or a change of prompt or pane.

import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type Result, type Settings, type SummaryPromptEntry, type SummaryPromptList } from "../bindings";
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
  onSelect,
  onUse,
}: {
  prompt: SummaryPromptEntry;
  selected: boolean;
  active: boolean;
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
          <Button onClick={onUse}>
            Use<span className="sr-only"> {prompt.name}</span>
          </Button>
        )}
      </div>
    </div>
  );
}

type Edit = { id: string; text: string };

/// A copy of `map` with no entry for `id`.
function without<T>(map: Record<string, T>, id: string): Record<string, T> {
  if (!(id in map)) return map;
  const { [id]: _, ...rest } = map;
  return rest;
}

export function SummaryPrompts({
  onSettings,
  closeSaveLimit = defaultCloseSaveLimit,
}: {
  onSettings: (s: Settings) => void;
  /// Tests set a shorter limit.
  closeSaveLimit?: number;
}) {
  const [list, setList] = useState<SummaryPromptList | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // The unsaved text of each prompt, by id. A prompt with no entry shows its
  // saved text.
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [listError, setListError] = useState<string | null>(null);
  const [editorError, setEditorError] = useState<string | null>(null);
  // The error of the last failed text save, by prompt id. The editor shows
  // it only for that prompt.
  const [saveErrors, setSaveErrors] = useState<Record<string, string>>({});
  const { ask, dialog } = useConfirm();
  // The valid text that waits for the timer, the timer, and the last save
  // that started. Each save waits for the save before it, so the saves apply
  // in the order typed. `failed` holds the ids whose last text save failed.
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
  useEffect(() => {
    void load();
  }, [load]);

  const cancel = useCallback(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    pending.current = null;
  }, []);

  // Starts the save of the pending text at once. Resolves when all saves
  // that started are complete: to false if the last text save of prompt
  // `id` failed, or with no `id`, if the last text save of a prompt failed.
  const flush = useCallback(
    (id?: string) => {
      const sent = pending.current;
      cancel();
      if (sent) {
        const fail = (error: string) => {
          failed.current.add(sent.id);
          setSaveErrors((m) => ({ ...m, [sent.id]: error }));
        };
        saving.current = saving.current
          .then(async () => {
            const result = await commands.setSummaryPromptText(sent.id, sent.text);
            if (result.status === "error") {
              fail(result.error);
              return;
            }
            failed.current.delete(sent.id);
            setSaveErrors((m) => without(m, sent.id));
            onSettingsRef.current(result.data);
            await load();
            // Clear the draft only if it still holds the sent text, so that
            // text typed during the save stays for the next save.
            setDrafts((m) => (m[sent.id] === sent.text ? without(m, sent.id) : m));
          })
          .catch((e: unknown) => {
            // A failed save must not stop the saves after it.
            console.error("The prompt text could not be saved:", e);
            fail("The prompt text could not be saved.");
          });
      }
      return saving.current.then(() => (id === undefined ? failed.current.size === 0 : !failed.current.has(id)));
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

  // A selection that is gone (a deleted prompt) goes back to the prompt in use.
  const selected =
    list?.prompts.find((p) => p.id === selectedId) ?? list?.prompts.find((p) => p.id === list.active) ?? null;
  const draft = selected && selected.id in drafts ? drafts[selected.id] : (selected?.text ?? "");
  const count = [...draft].length;
  const limit = list?.text_limit ?? 0;
  const name = selected?.name ?? "";
  const valid = (text: string) => text.trim() !== "" && [...text].length <= limit;

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
  // and the error stay with their prompt.
  const select = (id: string) => {
    if (!selected || id === selected.id) return;
    const from = selected.id;
    if (from in drafts && !valid(drafts[from])) setDrafts((m) => without(m, from));
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
  const empty = !selected.built_in && draft.trim() === "";
  const saveError = saveErrors[id] ?? null;
  // The copy gets the text that the editor shows.
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
      failed.current.delete(id);
      setDrafts((m) => without(m, id));
      setSaveErrors((m) => without(m, id));
      show(null);
    });
  };
  // The text stays editable while a save runs. Only valid text waits for
  // the timer; the last valid text stays saved.
  const change = (text: string) => {
    setDrafts((m) => ({ ...m, [id]: text }));
    cancel();
    if (!valid(text)) return;
    pending.current = { id, text };
    timer.current = setTimeout(() => void flush(), saveDelay);
  };

  return (
    <>
      <Group
        header="Prompts"
        footer="The prompt in use writes each new summary. Summaries you already have don’t change."
      >
        <div role="radiogroup" aria-label="Summary prompts">
          {list.prompts.map((p) => (
            <PromptRow
              key={p.id}
              prompt={p}
              selected={p.id === id}
              active={p.id === list.active}
              onSelect={() => select(p.id)}
              onUse={() => void run(() => commands.activateSummaryPrompt(p.id), setListError)}
            />
          ))}
        </div>
        <div className="settings-row px-4">
          <div className="py-2.5">
            <Button onClick={() => void create()}>New Prompt</Button>
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
            disabled={selected.built_in}
            onCommit={(v) => void run(() => commands.renameSummaryPrompt(id, v), setEditorError)}
          />
        </Row>
        <div className="settings-row px-4">
          <div className="py-3">
            <textarea
              aria-label="Prompt text"
              value={draft}
              readOnly={selected.built_in}
              rows={14}
              onChange={(e) => change(e.target.value)}
              onBlur={() => void flush()}
              className={`${fieldClass} block w-full resize-y py-1.5 leading-[1.45] read-only:text-label-secondary`}
            />
          </div>
        </div>
        <div className="settings-row px-4">
          <div className="flex min-h-[3.25rem] items-center gap-2 py-2.5">
            {selected.built_in ? (
              <>
                <p className="min-w-0 flex-1 text-[0.85rem] leading-[1.4] text-label-secondary">
                  Built-in prompts can’t be changed. Duplicate one to make your own.
                </p>
                <Button onClick={() => void duplicate()}>Duplicate</Button>
              </>
            ) : (
              <>
                <span
                  className={`min-w-0 flex-1 text-[0.85rem] tabular-nums ${count > limit ? "text-negative" : "text-label-secondary"}`}
                >
                  {`${format(count)} / ${format(limit)}`}
                </span>
                <Button onClick={() => void duplicate()}>Duplicate</Button>
                <Button onClick={() => void remove()}>Delete</Button>
              </>
            )}
          </div>
        </div>
        {empty && <ErrorText>A prompt can’t be empty.</ErrorText>}
        {saveError && <ErrorText>{saveError}</ErrorText>}
        {editorError && <ErrorText>{editorError}</ErrorText>}
      </Group>
      {dialog}
    </>
  );
}
