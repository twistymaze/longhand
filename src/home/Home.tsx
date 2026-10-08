// The library keeps one selected note and its preview through navigation.

import {
  type ReactNode,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  commands,
  type NewNoteType,
  type Readiness,
  type SessionEntry,
} from "../bindings";
import { GENERAL } from "../lib/Labels";
import {
  clockTime,
  isOpen,
  missingPreconditions,
  sentence,
} from "../processing/describe";
import { useDeleteSession } from "../session/DeleteSession";
import { commitTitle, type Notice } from "../session/TitleField";
import { AppIcon, CautionIcon, NoteIcon } from "../settings/icons";
import { Button } from "../settings/ui";
import { FilterBar } from "./FilterBar";
import { generalIsChoice, isFiltered, keepExisting, matches, type NoteFilter, noFilter, typesInUse } from "./filter";
import { groupByDay, rowTitle } from "./group";
import { NewNoteButton, noteStartRefusal } from "./NewNoteButton";
import { PreviewPanel } from "./PreviewPanel";
import { type RecordingNow, RecordingRow } from "./RecordingRow";
import { SessionRow } from "./SessionRow";
import { useTypesAndTags } from "./useTypesAndTags";

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
  processingWaits = false,
  readiness,
  chord,
  newNoteRecords = true,
  onOpen,
  onStarted,
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
  /// Processing waits for a recording to finish (KTD20).
  processingWaits?: boolean;
  /// Null until it is read; Home then offers no New Note button.
  readiness: Readiness | null;
  /// The recording shortcut as this platform writes it, or null when the
  /// user cleared it (R14).
  chord: string | null;
  /// The default of Settings > Recording: New Note starts to record (R8).
  newNoteRecords?: boolean;
  onOpen: (id: string) => void;
  /// A note that does not record started. A note that records opens from
  /// the recording event.
  onStarted?: (id: string) => void;
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
  // The filter stays while Home is mounted, so it stays when the user opens
  // a note and comes back (R33). A type or a tag that no longer exists drops
  // out of it. General drops out when no other type is left (R3).
  const lists = useTypesAndTags(active);
  const [chosenFilter, setFilter] = useState<NoteFilter>(noFilter);
  const types = lists.types?.types ?? null;
  const filter = keepExisting(chosenFilter, types, lists.tags, generalIsChoice(types));
  // When types are in use, a row of General shows a General label (R4).
  // A type list that is not read yet shows no General label, though it
  // keeps a chosen General in the filter.
  const showGeneral = typesInUse(types);
  const rowType = (noteType: string | null) => noteType ?? (showGeneral ? GENERAL : null);
  useEffect(() => {
    if (filter !== chosenFilter) setFilter(filter);
  }, [filter, chosenFilter]);
  const filtered = useMemo(
    () => (isFiltered(filter) ? listed.filter((e) => matches(e, filter)) : listed),
    [listed, filter],
  );
  const groups = useMemo(() => groupByDay(filtered, now), [filtered, now]);
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

  // Home shows no note, so no note is left here: the view change to Home
  // already left the note that the page showed (R18).
  const start = async (record: boolean, noteType?: NewNoteType) => {
    setStarting(true);
    try {
      const result = await commands.startRecording(record, null, noteType ?? null);
      setNotice(
        result.status === "error"
          ? { tone: "negative", text: sentence(result.error) }
          : null,
      );
      if (result.status === "ok" && !record) {
        onChanged();
        onStarted?.(result.data);
      }
      return result.status === "ok";
    } catch {
      setNotice({
        tone: "negative",
        text: record ? "Could not start recording." : "Could not start a new note.",
      });
      return false;
    } finally {
      setStarting(false);
    }
  };

  const missing = readiness ? missingPreconditions(readiness) : "";
  const count = filtered.length;
  const showFilter = listed.length > 0 || isFiltered(filter);
  const clearFilter = () => {
    setFilter(noFilter);
    // The Clear Filter button goes. The chosen row takes the focus.
    requestAnimationFrame(() =>
      list.current?.querySelector<HTMLElement>('[role="option"][tabindex="0"]')?.focus(),
    );
  };

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
          <NewNoteButton
            ref={newNote}
            records={newNoteRecords}
            readiness={readiness}
            busy={starting}
            types={lists.types?.types}
            onStart={(record, noteType) => void start(record, noteType)}
            onTypesChanged={lists.setTypes}
          />
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
          {showFilter && (
            <FilterBar
              filter={filter}
              types={types ?? []}
              tags={lists.tags ?? []}
              onChange={setFilter}
              onClear={clearFilter}
            />
          )}

          {readiness && !readiness.can_start_note && !recording && (
            <CautionBanner>
              <span className="min-w-0 flex-1">
                {noteStartRefusal(readiness)}
              </span>
              {/* The fix of an unavailable storage folder is in Settings. */}
              <Button onClick={() => void commands.openSettings()}>
                Open Settings…
              </Button>
            </CautionBanner>
          )}
          {/* While no note can start, the banner above is the only one. */}
          {readiness && readiness.can_start_note && !readiness.armed && !recording && (
            <CautionBanner>
              <span className="min-w-0 flex-1">
                {missing
                  ? `To record, ${missing} in Settings.`
                  : "Not ready to record."}
              </span>
              <Button onClick={() => void commands.openSettings()}>
                Open Settings…
              </Button>
            </CautionBanner>
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
            {loaded && listed.length === 0 && !pendingCapture && (
              <EmptyState chord={chord} />
            )}
            {loaded && listed.length > 0 && order.length === 0 && (
              // No note matches the filter (R32).
              <div className="flex flex-col items-center px-6 pt-16 pb-10 text-center">
                <p role="status" className="text-label-secondary">
                  No notes match the filter.
                </p>
                <div className="mt-3">
                  <Button onClick={clearFilter}>Clear Filter</Button>
                </div>
              </div>
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
                    {/* One column that is never wider than the list:
                        the labels of a row do not make the row wider. */}
                    <div className="grid grid-cols-[minmax(0,1fr)] gap-px">
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
                              noteType: rowType(entry.note_type),
                              tags: entry.tags,
                            }}
                            selected={entry.id === selected?.id}
                            onSelect={() => setSelectedId(entry.id)}
                            onOpen={onOpen}
                          />
                        ) : (
                          <SessionRow
                            key={entry.id}
                            entry={entry}
                            noteType={rowType(entry.note_type)}
                            title={rowTitle(entry, now)}
                            time={clockTime(entry.started)}
                            stage={stages[entry.id] ?? null}
                            processingWaits={processingWaits}
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
            processingWaits={processingWaits}
          />
        )}
      </div>
      {deletion.dialog}
    </div>
  );
}

function CautionBanner({ children }: { children: ReactNode }) {
  return (
    <div className="mx-[24px] mt-1 mb-2 flex items-center gap-2.5 rounded-[10px] bg-caution/10 px-3 py-2 text-[0.85rem] shadow-[inset_0_0_0_0.5px_color-mix(in_srgb,var(--caution)_35%,transparent)]">
      <CautionIcon className="size-4 text-caution" />
      {children}
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
