// The selected note shows its summary or its current status. A note with
// no audio shows the start of its notes.

import { useEffect, useState } from "react";
import { commands, type SessionEntry } from "../bindings";
import { entryStatus, entryTone, formatDuration, startedAt } from "../processing/describe";
import { firstParagraph } from "../session/markdown";
import { remoteLabels } from "../session/SessionMenu";
import { folderName, Status } from "../settings/ui";
import { rowTitle } from "./group";

type Loaded = { overview: string | null; folder: string | null; error: boolean };

function Detail({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className="preview-detail flex flex-wrap items-baseline gap-x-3 gap-y-0.5 px-3 py-2">
      <dt className="shrink-0 text-label-secondary">{label}</dt>
      <dd className="min-w-0 flex-[1_1_4rem] truncate text-right" title={title}>
        {value}
      </dd>
    </div>
  );
}

export function PreviewPanel({
  entry,
  stage,
  revision,
  now,
}: {
  entry: SessionEntry;
  stage: string | null;
  revision: number;
  now?: Date;
}) {
  const id = entry.id;
  const saved = entry.state === "saved";
  // A saved note with no recorded time never recorded (R15).
  const hasAudio = !saved || entry.duration_ms > 0;
  // A rename can move the folder, and a save gives the summary, so a new
  // title or a new state reads the session again.
  const key = `${id}\n${entry.title ?? ""}\n${entry.state}\n${revision}`;
  const state = entry.state;
  const [loaded, setLoaded] = useState<{ id: string; state: string; value: Loaded } | null>(null);

  useEffect(() => {
    let current = true;
    // A note with no audio has no summary: the preview shows the start of
    // its notes in place of the summary.
    const overview = !saved
      ? Promise.resolve(null)
      : hasAudio
        ? commands.sessionDocument(id).then((doc) =>
            doc.status === "ok" ? { ok: true, text: doc.data.summary?.body } : { ok: false, text: undefined },
          )
        : commands.loadSessionNotes(id).then((notes) =>
            notes.status === "ok" ? { ok: true, text: notes.data.markdown } : { ok: false, text: undefined },
          );
    void Promise.all([overview, commands.sessionDetail(id)]).then(([doc, detail]) => {
      const body = doc?.text;
      const value = {
        overview: body ? firstParagraph(body) : null,
        error: doc?.ok === false || detail.status === "error",
        folder: detail.status === "ok" ? detail.data.home_path : null,
      };
      if (current) setLoaded({ id, state, value });
    }).catch(() => {
      if (current) setLoaded({ id, state, value: { overview: null, folder: null, error: true } });
    });
    return () => {
      current = false;
    };
  }, [key, id, saved, hasAudio, state]);

  // Keep the current preview during a refresh, but never across notes or states.
  const shown = loaded?.id === id && loaded.state === state ? loaded.value : null;
  const tone = entryTone(entry.state) ?? "neutral";

  return (
    <section
      aria-label="Preview"
      className="flex w-[45%] max-w-[22rem] min-w-0 shrink-0 flex-col overflow-x-hidden overflow-y-auto border-l border-separator bg-sidebar/60 px-[clamp(0.5rem,2vw,1.5rem)] pt-6 pb-6 select-text"
    >
      <h2 className="text-[1.15rem] leading-[1.25] font-semibold tracking-[-0.015em] break-words">
        {rowTitle(entry, now)}
      </h2>
      <p className="mt-1 text-[0.85rem] text-label-secondary">{startedAt(entry.started)}</p>
      {(!saved || entry.remote) && (
        <p className="mt-3 text-[0.85rem]">
          <Status tone={tone}>{entry.remote ? remoteLabels[entry.remote] : entryStatus(entry, stage)}</Status>
        </p>
      )}

      {shown?.error && <p role="status" className="mt-3 text-negative">Could not read this note.</p>}
      {saved && hasAudio && !shown && <p role="status" className="mt-3 text-label-secondary">Loading summary…</p>}
      {/* A note with no audio has no summary to wait for (R14). */}
      {saved && hasAudio && shown && !shown.error && !shown.overview && (
        <p className="mt-3 text-label-secondary">No summary yet.</p>
      )}
      {saved && !hasAudio && shown && !shown.error && !shown.overview && (
        <p className="mt-3 text-label-secondary">No notes yet.</p>
      )}
      {shown?.overview && (
        <div className="mt-5">
          <h3 className="mb-1.5 text-[0.72rem] font-semibold tracking-[0.06em] text-label-tertiary uppercase">
            {hasAudio ? "Summary" : "My Notes"}
          </h3>
          <p className="line-clamp-[12] leading-[1.5]">{shown.overview}</p>
        </div>
      )}

      <dl className="mt-5 divide-y divide-separator overflow-hidden rounded-[10px] bg-group text-[0.85rem] shadow-[0_0_0_0.5px_var(--separator)]">
        {hasAudio && <Detail label="Duration" value={formatDuration(entry.duration_ms)} />}
        {entry.found_speakers !== null && <Detail label="Speakers" value={String(entry.found_speakers)} />}
        {shown?.folder && <Detail label="Folder" value={folderName(shown.folder)} title={shown.folder} />}
      </dl>
    </section>
  );
}
