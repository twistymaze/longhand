// The library keeps one selected note and its preview through navigation.

import { useEffect, useMemo, useRef, useState } from "react";
import {
  commands,
  type Readiness,
  type SessionEntry,
} from "../bindings";
import {
  clockTime,
  isOpen,
  missingPreconditions,
  sentence,
} from "../processing/describe";
import { useDeleteSession } from "../session/DeleteSession";
import { commitTitle, type Notice } from "../session/TitleField";
import { AppIcon, CautionIcon, NoteIcon, PlusIcon } from "../settings/icons";
import { Button } from "../settings/ui";
import { groupByDay, rowTitle } from "./group";
import { PreviewPanel } from "./PreviewPanel";
import { type RecordingNow, RecordingRow } from "./RecordingRow";
import { SessionRow } from "./SessionRow";

export type { RecordingNow };

export function Home({
  entries,
  active = true,
  loaded = true,
  loadError = null,
  listRevision = 0,
  getListRevision,
  revisions = {},
  stages,
  recording,
  readiness,
  chord,
  onOpen,
  onChanged,
  now,
}: {
  entries: SessionEntry[];
  active?: boolean;
  loaded?: boolean;
  loadError?: string | null;
  listRevision?: number;
  getListRevision?: () => number;
  revisions?: Record<string, number>;
  stages: Record<string, string>;
  /// The open session: recording, or interrupted.
  recording: RecordingNow | null;
  /// Null until it is read; Home then offers no New Note button.
  readiness: Readiness | null;
  /// The recording shortcut as this platform writes it, or null when the
  /// user cleared it (R14).
  chord: string | null;
  onOpen: (id: string) => void;
  /// Read the list again after a rename or a delete.
  onChanged: () => void;
  now?: Date;
}) {
  const [deleted, setDeleted] = useState<Map<string, number>>(() => new Map());
  const latestListRevision = useRef(listRevision);
  latestListRevision.current = listRevision;
  useEffect(() => {
    setDeleted((ids) => {
      // A successful read after deletion is authoritative, including a restored note.
      const remaining = new Map(
        [...ids].filter(
          ([id, revision]) =>
            revision === listRevision && entries.some((e) => e.id === id),
        ),
      );
      return remaining.size === ids.size ? ids : remaining;
    });
  }, [entries, listRevision]);
  const listed = useMemo(
    () => entries.filter((e) => !deleted.has(e.id)),
    [entries, deleted],
  );
  const groups = useMemo(() => groupByDay(listed, now), [listed, now]);
  const order = useMemo(() => groups.flatMap((g) => g.entries), [groups]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [starting, setStarting] = useState(false);
  const list = useRef<HTMLDivElement>(null);
  const newNote = useRef<HTMLButtonElement>(null);
  const restoreFocus = useRef(false);
  const selected = order.find((e) => e.id === selectedId) ?? order[0] ?? null;
  const pendingCapture =
    recording !== null && !entries.some((e) => e.id === recording.sessionId);
  useEffect(() => {
    setSelectedId(selected?.id ?? null);
  }, [selected?.id]);

  const focusRow = (id: string) =>
    requestAnimationFrame(() =>
      list.current
        ?.querySelector<HTMLElement>(`[data-session="${CSS.escape(id)}"]`)
        ?.focus(),
    );

  const moveSelection = (step: number) => {
    if (order.length === 0) return;
    const at = order.findIndex((e) => e.id === selected?.id);
    const next =
      order[at < 0 ? 0 : Math.min(order.length - 1, Math.max(0, at + step))];
    setSelectedId(next.id);
    // The row takes the focus after it renders as selected.
    focusRow(next.id);
  };

  const rename = async (entry: SessionEntry, title: string) => {
    setRenamingId(null);
    focusRow(entry.id);
    if (title === (entry.title ?? "")) return;
    setNotice(await commitTitle(entry.id, title));
    onChanged();
  };

  const deletion = useDeleteSession({
    titleOf: (entry) => rowTitle(entry, now),
    onDeleted: (entry) => {
      setNotice(null);
      // Delete can finish after this callback's render or before a queued render.
      const revision = getListRevision?.() ?? latestListRevision.current;
      setDeleted((ids) => new Map(ids).set(entry.id, revision));
      onChanged();
    },
    onFailed: (text) => {
      setNotice({ tone: "negative", text });
      onChanged();
    },
  });

  useEffect(() => {
    if (!restoreFocus.current || deletion.dialog !== null) return;
    restoreFocus.current = false;
    if (selected) focusRow(selected.id);
    else requestAnimationFrame(() => newNote.current?.focus());
  }, [deletion.dialog, selected?.id]);

  useEffect(() => {
    const navigateFromPage = (event: KeyboardEvent) => {
      if (
        !active ||
        renamingId ||
        deletion.dialog ||
        list.current?.querySelector('[role="menu"]')
      )
        return;
      if (
        event.defaultPrevented ||
        event.isComposing ||
        event.altKey ||
        event.ctrlKey ||
        event.metaKey ||
        event.shiftKey
      )
        return;
      if (
        event.target !== document.body &&
        event.target !== document.documentElement
      )
        return;
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
      if (order.length === 0) return;
      event.preventDefault();
      moveSelection(event.key === "ArrowDown" ? 1 : -1);
    };
    window.addEventListener("keydown", navigateFromPage);
    return () => window.removeEventListener("keydown", navigateFromPage);
  }, [active, order, selected?.id, renamingId, deletion.dialog]);

  const record = async () => {
    setStarting(true);
    try {
      const result = await commands.startRecording(null);
      setNotice(
        result.status === "error"
          ? { tone: "negative", text: sentence(result.error) }
          : null,
      );
      return result.status === "ok";
    } catch {
      setNotice({ tone: "negative", text: "Could not start recording." });
      return false;
    } finally {
      setStarting(false);
    }
  };

  const missing = readiness ? missingPreconditions(readiness) : "";
  const count = listed.length;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header
        data-tauri-drag-region
        className="flex shrink-0 flex-wrap items-center gap-2 border-b border-separator px-[24px] pt-1 pb-3.5 select-none"
      >
        <AppIcon className="pointer-events-none size-10" />
        <span
          data-tauri-drag-region
          className="min-w-0 flex-1 text-[2rem] leading-[1.1] font-semibold tracking-[-0.02em]"
        >
          Longhand
        </span>
        {readiness && !recording && (
          <>
            <Button
              ref={newNote}
              prominent
              disabled={starting || !readiness.armed}
              onClick={() => void record()}
            >
              <span className="inline-flex items-center gap-1.5">
                <PlusIcon className="-ml-0.5 size-3.5" />
                New Note
              </span>
            </Button>
          </>
        )}
      </header>
      <div className="flex min-h-0 flex-1 overflow-hidden bg-pane">
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex shrink-0 items-baseline gap-2.5 px-[24px] pt-6 pb-2 select-none">
            <h1 className="text-[1.75rem] leading-none font-bold tracking-[-0.025em]">
              Notes
            </h1>
            {count > 0 && (
              <span className="text-[0.85rem] text-label-tertiary tabular-nums">
                {count === 1 ? "1 note" : `${count} notes`}
              </span>
            )}
          </div>

          {readiness && !readiness.armed && !recording && (
            <div className="mx-[24px] mt-1 mb-2 flex items-center gap-2.5 rounded-[10px] bg-caution/10 px-3 py-2 text-[0.85rem] shadow-[inset_0_0_0_0.5px_color-mix(in_srgb,var(--caution)_35%,transparent)]">
              <CautionIcon className="size-4 text-caution" />
              <span className="min-w-0 flex-1">
                {missing
                  ? `To record, ${missing} in Settings.`
                  : "Not ready to record."}
              </span>
              <Button onClick={() => void commands.openSettings()}>
                Open Settings…
              </Button>
            </div>
          )}
          {notice && (
            <p
              role="status"
              className={`mx-[24px] mt-1 mb-2 rounded-[10px] px-3 py-2 text-[0.85rem] ${
                notice.tone === "caution"
                  ? "bg-caution/10 text-caution"
                  : "bg-negative/10 text-negative"
              }`}
            >
              {notice.text}
            </p>
          )}

          <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto px-[16px] pt-1 pb-6 select-none">
            {loadError && (
              <div className="flex items-center gap-3 px-2 py-3">
                <p role="status" className="text-negative">
                  {loadError}
                </p>
                <Button onClick={onChanged}>Retry</Button>
              </div>
            )}
            {order.length === 0 &&
              (!loaded || pendingCapture) &&
              !loadError && (
                <p role="status" className="px-2 py-3 text-label-secondary">
                  Loading notes…
                </p>
              )}
            {loaded && order.length === 0 && !pendingCapture && (
              <EmptyState chord={chord} />
            )}
            {order.length > 0 && (
              <div
                ref={list}
                role="listbox"
                aria-label="Notes"
                onKeyDown={(e) => {
                  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                    e.preventDefault();
                    moveSelection(e.key === "ArrowDown" ? 1 : -1);
                  }
                }}
              >
                {groups.map((group) => (
                  <div
                    key={group.key}
                    role="group"
                    aria-label={group.label}
                    className="pb-3"
                  >
                    <h2 className="px-2.5 pt-2 pb-1.5 text-[0.72rem] font-semibold tracking-[0.06em] text-label-tertiary uppercase">
                      {group.label}
                    </h2>
                    <div className="grid gap-px">
                      {group.entries.map((entry) =>
                        isOpen(entry.state) ? (
                          <RecordingRow
                            key={entry.id}
                            recording={{
                              sessionId: entry.id,
                              elapsedMs:
                                recording?.sessionId === entry.id
                                  ? recording.elapsedMs
                                  : entry.duration_ms,
                              interrupted: entry.state === "interrupted",
                            }}
                            selected={entry.id === selected?.id}
                            onSelect={() => setSelectedId(entry.id)}
                            onOpen={onOpen}
                          />
                        ) : (
                          <SessionRow
                            key={entry.id}
                            entry={entry}
                            title={rowTitle(entry, now)}
                            time={clockTime(entry.started)}
                            stage={stages[entry.id] ?? null}
                            selected={entry.id === selected?.id}
                            renaming={entry.id === renamingId}
                            onSelect={() => setSelectedId(entry.id)}
                            onOpen={() => onOpen(entry.id)}
                            onRename={() => setRenamingId(entry.id)}
                            onRenameCommit={(title) =>
                              void rename(entry, title)
                            }
                            onRenameCancel={() => {
                              setRenamingId(null);
                              focusRow(entry.id);
                            }}
                            onDelete={() => {
                              restoreFocus.current =
                                list.current?.contains(
                                  document.activeElement,
                                ) ?? false;
                              deletion.ask(entry);
                            }}
                          />
                        ),
                      )}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
        {selected && (
          <PreviewPanel
            entry={selected}
            stage={stages[selected.id] ?? null}
            revision={revisions[selected.id] ?? 0}
            now={now}
          />
        )}
      </div>
      {deletion.dialog}
    </div>
  );
}

function EmptyState({ chord }: { chord: string | null }) {
  return (
    <div className="flex h-full min-h-64 flex-col items-center justify-center px-6 pb-10 text-center">
      <div className="mb-4 flex size-14 items-center justify-center rounded-[14px] bg-group text-label-tertiary shadow-[0_0_0_0.5px_var(--separator),0_2px_8px_rgb(0_0_0/0.06)]">
        <NoteIcon className="size-7" />
      </div>
      <p className="text-[1rem] font-semibold tracking-[-0.01em]">
        No notes yet
      </p>
      <p className="mt-1 max-w-72 leading-[1.5] text-label-secondary">
        {chord ? (
          <>
            Press <kbd className="kbd">{chord}</kbd> in any app to start a new
            note.
          </>
        ) : (
          "Select New Note to start a new note."
        )}
      </p>
    </div>
  );
}
