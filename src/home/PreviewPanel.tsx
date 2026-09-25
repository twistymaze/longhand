// The selected session beside Home's list (R6): the title, the date, the
// summary overview, the duration, the speaker count and the folder name.
// The overview and the folder are read when the selection changes, not for
// each row (KTD11). With no selection the panel is empty.

import { useEffect, useState } from "react";
import { commands, type SessionEntry } from "../bindings";
import { entryStatus, formatDuration, startedAt } from "../processing/describe";
import { folderName } from "../settings/ui";
import { firstParagraph } from "../session/markdown";
import { rowTitle } from "./group";

type Loaded = { id: string; overview: string | null; folder: string | null };

function Detail({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className="flex gap-3 py-1.5">
      <dt className="w-20 shrink-0 text-label-secondary">{label}</dt>
      <dd className="min-w-0 flex-1 break-words" title={title}>
        {value}
      </dd>
    </div>
  );
}

export function PreviewPanel({
  entry,
  stage,
  now,
}: {
  entry: SessionEntry | null;
  stage: string | null;
  now?: Date;
}) {
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const id = entry?.id ?? null;
  const saved = entry?.state === "saved";
  // A rename can move the folder, so a new title reads it again.
  const title = entry?.title ?? null;

  useEffect(() => {
    if (id === null) return;
    let current = true;
    void Promise.all([
      saved ? commands.sessionDocument(id) : Promise.resolve(null),
      commands.sessionDetail(id),
    ]).then(([document, detail]) => {
      if (!current) return;
      const body = document?.status === "ok" ? document.data.summary?.body : undefined;
      setLoaded({
        id,
        overview: body ? firstParagraph(body) : null,
        folder: detail.status === "ok" ? detail.data.home_path : null,
      });
    });
    return () => {
      current = false;
    };
  }, [id, saved, title]);

  const shown = entry && loaded?.id === entry.id ? loaded : null;

  return (
    <section
      aria-label="Preview"
      className="w-72 shrink-0 overflow-y-auto border-l border-separator px-5 pt-5 pb-6 select-text"
    >
      {entry && (
        <>
          <h2 className="text-[1.1rem] leading-tight font-semibold tracking-[-0.01em] break-words">
            {rowTitle(entry, now)}
          </h2>
          <p className="mt-0.5 text-[0.85rem] text-label-secondary">{startedAt(entry.started)}</p>
          {!saved && <p className="mt-3 text-label-secondary">{entryStatus(entry, stage)}</p>}
          {shown?.overview && <p className="mt-3 leading-[1.45]">{shown.overview}</p>}
          <dl className="mt-4 divide-y divide-separator border-t border-separator text-[0.85rem]">
            <Detail label="Duration" value={formatDuration(entry.duration_ms)} />
            {entry.found_speakers !== null && <Detail label="Speakers" value={String(entry.found_speakers)} />}
            {shown?.folder && <Detail label="Folder" value={folderName(shown.folder)} title={shown.folder} />}
          </dl>
        </>
      )}
    </section>
  );
}
