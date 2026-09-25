// The summary prompts in the Summaries pane (R1–R4, KTD11): the list of
// prompts with the prompt in use, and an editor for the selected prompt
// under it. Each change is one backend command (KTD7). The prompt text has
// Save and Revert buttons, and unsaved text asks before it is lost.

import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useState } from "react";
import { commands, type Result, type Settings, type SummaryPromptEntry, type SummaryPromptList } from "../bindings";
import { Button, CommitField, ErrorText, Group, Row, fieldClass, useConfirm } from "./ui";

/// Resolves to true when the pane can change: no text is unsaved, or the
/// user discards it.
export type Guard = () => Promise<boolean>;

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

export function SummaryPrompts({
  onSettings,
  onGuard,
}: {
  onSettings: (s: Settings) => void;
  /// Receives the question to ask before the pane changes, or null when no
  /// text is unsaved.
  onGuard?: (guard: Guard | null) => void;
}) {
  const [list, setList] = useState<SummaryPromptList | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // The unsaved text of one prompt, or null when the editor shows the saved text.
  const [edit, setEdit] = useState<{ id: string; text: string } | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [editorError, setEditorError] = useState<string | null>(null);
  const { ask, dialog } = useConfirm();

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

  // A selection that is gone (a deleted prompt) goes back to the prompt in use.
  const selected =
    list?.prompts.find((p) => p.id === selectedId) ?? list?.prompts.find((p) => p.id === list.active) ?? null;
  const draft = selected && edit?.id === selected.id ? edit.text : (selected?.text ?? "");
  const dirty = !!selected && !selected.built_in && draft !== selected.text;
  const count = [...draft].length;
  const limit = list?.text_limit ?? 0;
  const canSave = dirty && draft.trim() !== "" && count <= limit;
  const name = selected?.name ?? "";

  const guard = useCallback<Guard>(
    async () =>
      !dirty ||
      (await ask({
        title: `Discard your changes to “${name}”?`,
        message: "The text you changed isn’t saved.",
        action: "Discard",
      })),
    [dirty, name, ask],
  );

  useEffect(() => {
    if (!onGuard) return;
    onGuard(dirty ? guard : null);
    return () => onGuard(null);
  }, [onGuard, dirty, guard]);

  // The close-requested listener exists only while text is unsaved: with a
  // listener, the window closes only when the page destroys it.
  useEffect(() => {
    if (!dirty) return;
    const win = getCurrentWindow();
    let stop: (() => void) | undefined;
    let gone = false;
    void win
      .onCloseRequested(async (event) => {
        event.preventDefault();
        if (!(await guard())) return;
        // With no unsaved text the listener goes, so a second close works
        // even if the destroy below is refused.
        setEdit(null);
        try {
          await win.destroy();
        } catch (e) {
          console.error("The Settings window could not close:", e);
        }
      })
      .then((f) => {
        if (gone) f();
        else stop = f;
      });
    return () => {
      gone = true;
      stop?.();
    };
  }, [dirty, guard]);

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

  const select = async (id: string) => {
    if (id === selected?.id || !(await guard())) return;
    show(id);
  };

  const create = async () => {
    if (await guard()) await run(() => commands.createSummaryPrompt(), setListError, showNewest);
  };

  if (!list || !selected) {
    return listError ? (
      <Group header="Prompts">
        <ErrorText>{listError}</ErrorText>
      </Group>
    ) : null;
  }

  const id = selected.id;
  const duplicate = async () => {
    if (await guard()) await run(() => commands.duplicateSummaryPrompt(id), setEditorError, showNewest);
  };
  const remove = async () => {
    const yes = await ask({ title: `Delete “${name}”?`, message: "This can’t be undone.", action: "Delete" });
    if (yes) await run(() => commands.deleteSummaryPrompt(id), setEditorError, () => show(null));
  };
  // The text stays editable while the save runs. Clear the edit only if it
  // still holds the saved text, so that text typed after Save stays unsaved.
  const save = () => {
    const sent = draft;
    return run(
      () => commands.setSummaryPromptText(id, sent),
      setEditorError,
      () => setEdit((e) => (e && e.id === id && e.text === sent ? null : e)),
    );
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
              onSelect={() => void select(p.id)}
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
              onChange={(e) => setEdit({ id, text: e.target.value })}
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
                <span aria-hidden className="w-2" />
                <Button disabled={!dirty} onClick={() => setEdit(null)}>
                  Revert
                </Button>
                <Button prominent disabled={!canSave} onClick={() => void save()}>
                  Save
                </Button>
              </>
            )}
          </div>
        </div>
        {editorError && <ErrorText>{editorError}</ErrorText>}
      </Group>
      {dialog}
    </>
  );
}
