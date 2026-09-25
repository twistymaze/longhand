// Home (R5–R8, R33): every session by day, newest first, with a preview of
// the selected one. The open session shows as a recording row above the
// list. Double-click or Return opens a session's page.

import { useMemo, useRef, useState } from "react";
import { commands, type Readiness, type SessionEntry } from "../bindings";
import { isOpen, missingPreconditions, sentence } from "../processing/describe";
import { useDeleteSession } from "../session/DeleteSession";
import { commitTitle, type Notice } from "../session/TitleField";
import { Button } from "../settings/ui";
import { groupByDay, rowTitle } from "./group";
import { PreviewPanel } from "./PreviewPanel";
import { type RecordingNow, RecordingRow } from "./RecordingRow";
import { SessionRow } from "./SessionRow";

export type { RecordingNow };

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
  /// Null until it is read; Home then offers no Record button.
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
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [starting, setStarting] = useState(false);
  const list = useRef<HTMLDivElement>(null);

  const selected = order.find((e) => e.id === selectedId) ?? null;

  const focusRow = (id: string) =>
    requestAnimationFrame(() =>
      list.current?.querySelector<HTMLElement>(`[data-session="${CSS.escape(id)}"]`)?.focus(),
    );

  const moveSelection = (step: number) => {
    if (order.length === 0) return;
    const at = order.findIndex((e) => e.id === selectedId);
    const next = order[at < 0 ? 0 : Math.min(order.length - 1, Math.max(0, at + step))];
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
      if (selectedId === entry.id) setSelectedId(null);
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

  const time = (iso: string) =>
    new Date(iso).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });

  const missing = readiness ? missingPreconditions(readiness) : "";

  return (
    <div className="flex h-full min-h-0">
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex shrink-0 items-center gap-3 px-5 pt-5 pb-3 select-none">
          <h1 className="min-w-0 flex-1 text-[1.35rem] leading-tight font-semibold tracking-[-0.01em]">Sessions</h1>
          {readiness && !recording && (
            <Button prominent disabled={starting || !readiness.armed} onClick={() => void record()}>
              Record
            </Button>
          )}
        </header>
        {readiness && !readiness.armed && !recording && (
          <p className="flex items-center gap-2 px-5 pb-3 text-[0.85rem] text-caution">
            <span className="min-w-0 flex-1">
              {missing ? `Not ready to record: ${missing}.` : "Not ready to record."}
            </span>
            <Button onClick={() => void commands.openSettings()}>Open Settings…</Button>
          </p>
        )}
        {notice && (
          <p
            role="status"
            className={`px-5 pb-3 text-[0.85rem] ${notice.tone === "caution" ? "text-caution" : "text-negative"}`}
          >
            {notice.text}
          </p>
        )}
        <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-4 select-none">
          {recording && (
            <div className="px-2 pb-3">
              <RecordingRow recording={recording} onOpen={onOpen} />
            </div>
          )}
          {groups.length === 0 && !recording && (
            <p className="px-2 pt-8 text-center text-label-secondary">
              No sessions yet. Press {chord} to record.
            </p>
          )}
          {groups.length > 0 && (
            <div
              ref={list}
              role="listbox"
              aria-label="Sessions"
              onKeyDown={(e) => {
                if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                  e.preventDefault();
                  moveSelection(e.key === "ArrowDown" ? 1 : -1);
                }
              }}
            >
              {groups.map((group) => (
                <div key={group.key} role="group" aria-label={group.label} className="pb-2">
                  <h2 className="px-2.5 pt-2 pb-1 text-[0.8rem] font-semibold text-label-tertiary">
                    {group.label}
                  </h2>
                  {group.entries.map((entry) => (
                    <SessionRow
                      key={entry.id}
                      entry={entry}
                      title={rowTitle(entry, now)}
                      time={time(entry.started)}
                      stage={stages[entry.id] ?? null}
                      selected={entry.id === selectedId}
                      focusable={selected ? entry.id === selected.id : entry.id === order[0]?.id}
                      renaming={entry.id === renamingId}
                      onSelect={() => setSelectedId(entry.id)}
                      onOpen={() => onOpen(entry.id)}
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
              ))}
            </div>
          )}
        </div>
      </div>
      <PreviewPanel entry={selected} stage={selected ? (stages[selected.id] ?? null) : null} now={now} />
      {deletion.dialog}
    </div>
  );
}
