// The summary prompts in the Summaries pane (R1–R4, KTD11): the list of
// prompts with the prompt in use, and an editor for the selected prompt
// under it. Each change is one backend command (KTD7). The editor saves the
// prompt text as the user types: one second after the last change, and at
// once on blur or a change of prompt or pane.

import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type Result, type Settings, type SummaryPromptEntry, type SummaryPromptList } from "../bindings";
import { Button, CommitField, ErrorText, Group, Row, fieldClass, useConfirm } from "./ui";

/// The time from the last change to the save, in milliseconds.
const saveDelay = 1000;

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

export function SummaryPrompts({ onSettings }: { onSettings: (s: Settings) => void }) {
  const [list, setList] = useState<SummaryPromptList | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // The unsaved text of one prompt, or null when the editor shows the saved text.
  const [edit, setEdit] = useState<Edit | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [editorError, setEditorError] = useState<string | null>(null);
  const { ask, dialog } = useConfirm();
  // The valid text that waits for the timer, the timer, and the last save
  // that started. Each save waits for the save before it, so the saves apply
  // in the order typed.
  const pending = useRef<Edit | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const saving = useRef<Promise<void>>(Promise.resolve());
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
  // that started are complete.
  const flush = useCallback(() => {
    const sent = pending.current;
    cancel();
    if (sent) {
      saving.current = saving.current
        .then(async () => {
          const result = await commands.setSummaryPromptText(sent.id, sent.text);
          if (result.status === "error") {
            setEditorError(result.error);
            return;
          }
          setEditorError(null);
          onSettingsRef.current(result.data);
          await load();
          // Clear the edit only if it still holds the sent text, so that text
          // typed during the save stays for the next save.
          setEdit((e) => (e && e.id === sent.id && e.text === sent.text ? null : e));
        })
        .catch((e: unknown) => {
          // A failed save must not stop the saves after it.
          console.error("The prompt text could not be saved:", e);
          setEditorError("The prompt text could not be saved.");
        });
    }
    return saving.current;
  }, [cancel, load]);

  // A pane change or a window close unmounts the pane. The save then runs
  // with no wait. The page events also start it when the page closes.
  useEffect(() => {
    const onHide = () => void flush();
    window.addEventListener("pagehide", onHide);
    window.addEventListener("beforeunload", onHide);
    return () => {
      window.removeEventListener("pagehide", onHide);
      window.removeEventListener("beforeunload", onHide);
      void flush();
    };
  }, [flush]);

  // A selection that is gone (a deleted prompt) goes back to the prompt in use.
  const selected =
    list?.prompts.find((p) => p.id === selectedId) ?? list?.prompts.find((p) => p.id === list.active) ?? null;
  const draft = selected && edit?.id === selected.id ? edit.text : (selected?.text ?? "");
  const count = [...draft].length;
  const limit = list?.text_limit ?? 0;
  const name = selected?.name ?? "";

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
    setEdit(null);
    setEditorError(null);
    setSelectedId(id);
  };

  // A new prompt is the last one in the settings.
  const showNewest = (s: Settings) => {
    const newest = s.summary_prompts[s.summary_prompts.length - 1];
    if (newest) show(newest.id);
  };

  // Text that is not valid is not saved, and a change of prompt drops it.
  const select = (id: string) => {
    if (id === selected?.id) return;
    void flush();
    show(id);
  };

  const create = async () => {
    await flush();
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
  // The copy gets the text that the editor shows.
  const duplicate = async () => {
    await flush();
    await run(() => commands.duplicateSummaryPrompt(id), setEditorError, showNewest);
  };
  const remove = async () => {
    const yes = await ask({ title: `Delete “${name}”?`, message: "This can’t be undone.", action: "Delete" });
    if (!yes) return;
    if (pending.current?.id === id) cancel();
    await run(() => commands.deleteSummaryPrompt(id), setEditorError, () => show(null));
  };
  // The text stays editable while a save runs. Only valid text waits for
  // the timer; the last valid text stays saved.
  const change = (text: string) => {
    setEdit({ id, text });
    cancel();
    if (text.trim() === "" || [...text].length > limit) return;
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
        {editorError && <ErrorText>{editorError}</ErrorText>}
      </Group>
      {dialog}
    </>
  );
}
