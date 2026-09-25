// Home (R5–R8, R33): every session by day, newest first. The UI calls a
// session a "note". The open session shows as a recording row above the
// list. A click or Return opens a session's page. The preview panel shows
// only while the pointer is on a row or a row is selected with the keys;
// otherwise the list uses the full width.

import { useEffect, useMemo, useRef, useState } from "react";
import { commands, type Readiness, type SessionEntry } from "../bindings";
import { clockTime, isOpen, missingPreconditions, sentence } from "../processing/describe";
import { useDeleteSession } from "../session/DeleteSession";
import { commitTitle, type Notice } from "../session/TitleField";
import { AppIcon, CautionIcon, NoteIcon, PlusIcon } from "../settings/icons";
import { Button } from "../settings/ui";
import { groupByDay, rowTitle } from "./group";
import { type Loaded, PreviewPanel } from "./PreviewPanel";
import { type RecordingNow, RecordingRow } from "./RecordingRow";
import { SessionRow } from "./SessionRow";

export type { RecordingNow };

/// The pointer must rest this long on a row before the panel first shows,
/// so a pointer that crosses the list on its way to a button shows nothing.
const SHOW_DELAY_MS = 90;
/// The panel stays this long after the pointer leaves a row, so a move
/// across a day heading to the next row keeps the panel.
const HIDE_DELAY_MS = 160;

export function Home({
  entries,
  stages,
  recording,
  readiness,
  chord,
  onOpen,
  onChanged,
  now,
}: {
  entries: SessionEntry[];
  stages: Record<string, string>;
  /// The open session: recording, or interrupted.
  recording: RecordingNow | null;
  /// Null until it is read; Home then offers no New Note button.
  readiness: Readiness | null;
  /// The recording shortcut as this platform writes it.
  chord: string;
  onOpen: (id: string) => void;
  /// Read the list again after a rename or a delete.
  onChanged: () => void;
  now?: Date;
}) {
  const listed = useMemo(() => entries.filter((e) => !isOpen(e.state)), [entries]);
  const groups = useMemo(() => groupByDay(listed, now), [listed, now]);
  const order = useMemo(() => groups.flatMap((g) => g.entries), [groups]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [starting, setStarting] = useState(false);
  const list = useRef<HTMLDivElement>(null);
  const hoverTimer = useRef<number | undefined>(undefined);
  // The overview and folder of each previewed session, so a move back to a
  // row shows its preview at once.
  const previews = useRef(new Map<string, Loaded>());

  const selected = order.find((e) => e.id === selectedId) ?? null;
  const previewed = order.find((e) => e.id === (hoveredId ?? selectedId)) ?? null;

  useEffect(() => () => window.clearTimeout(hoverTimer.current), []);

  const hover = (id: string | null) => {
    window.clearTimeout(hoverTimer.current);
    if (id === null) {
      hoverTimer.current = window.setTimeout(() => setHoveredId(null), HIDE_DELAY_MS);
    } else if (previewed !== null) {
      setHoveredId(id);
    } else {
      hoverTimer.current = window.setTimeout(() => setHoveredId(id), SHOW_DELAY_MS);
    }
  };
  // The pointer on the panel keeps the panel.
  const keepPanel = () => window.clearTimeout(hoverTimer.current);

  const open = (id: string) => {
    window.clearTimeout(hoverTimer.current);
    setHoveredId(null);
    onOpen(id);
  };

  const focusRow = (id: string) =>
    requestAnimationFrame(() =>
      list.current?.querySelector<HTMLElement>(`[data-session="${CSS.escape(id)}"]`)?.focus(),
    );

  const moveSelection = (step: number) => {
    if (order.length === 0) return;
    const at = order.findIndex((e) => e.id === selectedId);
    const next = order[at < 0 ? 0 : Math.min(order.length - 1, Math.max(0, at + step))];
    setSelectedId(next.id);
    // The keys move the preview, not the pointer.
    setHoveredId(null);
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
      if (selectedId === entry.id) setSelectedId(null);
      if (hoveredId === entry.id) setHoveredId(null);
      onChanged();
    },
    onFailed: (text) => {
      setNotice({ tone: "negative", text });
      onChanged();
    },
  });

  const record = async () => {
    setStarting(true);
    const result = await commands.startRecording();
    setStarting(false);
    setNotice(result.status === "error" ? { tone: "negative", text: sentence(result.error) } : null);
  };

  const missing = readiness ? missingPreconditions(readiness) : "";
  const count = listed.length;

  return (
    <div className="flex h-full min-h-0 flex-col">
        <header className="flex shrink-0 items-center gap-2.5 border-b border-separator px-5 pt-3 pb-3 select-none">
          <AppIcon className="size-8 drop-shadow-[0_1px_1.5px_rgb(0_0_0/0.18)] dark:drop-shadow-[0_0_0.5px_rgb(255_255_255/0.55)]" />
          <span className="min-w-0 flex-1 text-[0.95rem] font-semibold tracking-[-0.01em]">Longhand</span>
          {readiness && !recording && (
            <Button prominent disabled={starting || !readiness.armed} onClick={() => void record()}>
              <span className="inline-flex items-center gap-1.5">
                <PlusIcon className="-ml-0.5 size-3.5" />
                New Note
              </span>
            </Button>
          )}
        </header>
      <div className="flex min-h-0 flex-1">
      <div className="flex min-w-0 flex-1 flex-col">

        <div className="flex shrink-0 items-baseline gap-2.5 px-5 pt-5 pb-2 select-none">
          <h1 className="text-[1.75rem] leading-none font-bold tracking-[-0.025em]">Notes</h1>
          {count > 0 && (
            <span className="text-[0.85rem] text-label-tertiary tabular-nums">
              {count === 1 ? "1 note" : `${count} notes`}
            </span>
          )}
        </div>

        {readiness && !readiness.armed && !recording && (
          <div className="mx-5 mt-1 mb-2 flex items-center gap-2.5 rounded-[10px] bg-caution/10 px-3 py-2 text-[0.85rem] shadow-[inset_0_0_0_0.5px_color-mix(in_srgb,var(--caution)_35%,transparent)]">
            <CautionIcon className="size-4 text-caution" />
            <span className="min-w-0 flex-1">
              {missing ? `To record, ${missing} in Settings.` : "Not ready to record."}
            </span>
            <Button onClick={() => void commands.openSettings()}>Open Settings…</Button>
          </div>
        )}
        {notice && (
          <p
            role="status"
            className={`mx-5 mt-1 mb-2 rounded-[10px] px-3 py-2 text-[0.85rem] ${
              notice.tone === "caution" ? "bg-caution/10 text-caution" : "bg-negative/10 text-negative"
            }`}
          >
            {notice.text}
          </p>
        )}

        <div className="min-h-0 flex-1 overflow-y-auto px-3 pt-1 pb-4 select-none">
          {recording && (
            <div className="px-2 pt-1 pb-3">
              <RecordingRow recording={recording} onOpen={open} />
            </div>
          )}
          {groups.length === 0 && !recording && <EmptyState chord={chord} />}
          {groups.length > 0 && (
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
                <div key={group.key} role="group" aria-label={group.label} className="pb-3">
                  <h2 className="px-2.5 pt-2 pb-1.5 text-[0.72rem] font-semibold tracking-[0.06em] text-label-tertiary uppercase">
                    {group.label}
                  </h2>
                  <div className="grid gap-px">
                    {group.entries.map((entry) => (
                      <SessionRow
                        key={entry.id}
                        entry={entry}
                        title={rowTitle(entry, now)}
                        time={clockTime(entry.started)}
                        stage={stages[entry.id] ?? null}
                        selected={entry.id === selectedId}
                        previewed={entry.id === previewed?.id}
                        focusable={selected ? entry.id === selected.id : entry.id === order[0]?.id}
                        renaming={entry.id === renamingId}
                        onHover={(on) => hover(on ? entry.id : null)}
                        onOpen={() => open(entry.id)}
                        onRename={() => setRenamingId(entry.id)}
                        onRenameCommit={(title) => void rename(entry, title)}
                        onRenameCancel={() => {
                          setRenamingId(null);
                          focusRow(entry.id);
                        }}
                        onDelete={() => deletion.ask(entry)}
                      />
                    ))}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
      {previewed && (
        <PreviewPanel
          entry={previewed}
          stage={stages[previewed.id] ?? null}
          cache={previews.current}
          now={now}
          onPointerEnter={keepPanel}
          onPointerLeave={() => hover(null)}
        />
      )}
      </div>
      {deletion.dialog}
    </div>
  );
}

function EmptyState({ chord }: { chord: string }) {
  return (
    <div className="flex h-full min-h-64 flex-col items-center justify-center px-6 pb-10 text-center">
      <div className="mb-4 flex size-14 items-center justify-center rounded-[14px] bg-group text-label-tertiary shadow-[0_0_0_0.5px_var(--separator),0_2px_8px_rgb(0_0_0/0.06)]">
        <NoteIcon className="size-7" />
      </div>
      <p className="text-[1rem] font-semibold tracking-[-0.01em]">No notes yet</p>
      <p className="mt-1 max-w-72 leading-[1.5] text-label-secondary">
        Press <kbd className="kbd">{chord}</kbd> in any app to start a new note.
      </p>
    </div>
  );
}
