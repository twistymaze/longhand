// The previewed session beside Home's list (R6): the title, the date, the
// summary overview, the duration, the speaker count and the folder name.
// Home shows the panel only for a hovered or selected row. The overview and
// the folder are read when the previewed row changes, not for each row
// (KTD11). Home keeps each result, so a second look shows it at once.

import { useEffect, useState } from "react";
import { commands, type SessionEntry } from "../bindings";
import { entryStatus, entryTone, formatDuration, startedAt } from "../processing/describe";
import { firstParagraph } from "../session/markdown";
import { folderName, Status } from "../settings/ui";
import { rowTitle } from "./group";

export type Loaded = { overview: string | null; folder: string | null };

function Detail({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className="flex items-baseline gap-3 px-3 py-2">
      <dt className="shrink-0 text-label-secondary">{label}</dt>
      <dd className="min-w-0 flex-1 truncate text-right" title={title}>
        {value}
      </dd>
    </div>
  );
}

export function PreviewPanel({
  entry,
  stage,
  cache,
  now,
  onPointerEnter,
  onPointerLeave,
}: {
  entry: SessionEntry;
  stage: string | null;
  /// Earlier results, by `cacheKey`.
  cache: Map<string, Loaded>;
  now?: Date;
  onPointerEnter: () => void;
  onPointerLeave: () => void;
}) {
  const id = entry.id;
  const saved = entry.state === "saved";
  // A rename can move the folder, and a save gives the summary, so a new
  // title or a new state reads the session again.
  const key = `${id}\n${entry.title ?? ""}\n${entry.state}`;
  const [loaded, setLoaded] = useState<{ key: string; value: Loaded } | null>(null);

  useEffect(() => {
    if (cache.has(key)) return;
    let current = true;
    void Promise.all([
      saved ? commands.sessionDocument(id) : Promise.resolve(null),
      commands.sessionDetail(id),
    ]).then(([doc, detail]) => {
      const body = doc?.status === "ok" ? doc.data.summary?.body : undefined;
      const value = {
        overview: body ? firstParagraph(body) : null,
        folder: detail.status === "ok" ? detail.data.home_path : null,
      };
      cache.set(key, value);
      if (current) setLoaded({ key, value });
    });
    return () => {
      current = false;
    };
  }, [cache, key, id, saved]);

  const shown = cache.get(key) ?? (loaded?.key === key ? loaded.value : null);
  const tone = entryTone(entry.state) ?? "neutral";

  return (
    <section
      aria-label="Preview"
      onPointerEnter={onPointerEnter}
      onPointerLeave={onPointerLeave}
      className="preview-in flex w-[22rem] shrink-0 flex-col overflow-y-auto border-l border-separator bg-sidebar/60 px-5 pt-5 pb-6 select-text"
    >
      <h2 className="text-[1.15rem] leading-[1.25] font-semibold tracking-[-0.015em] break-words">
        {rowTitle(entry, now)}
      </h2>
      <p className="mt-1 text-[0.85rem] text-label-secondary">{startedAt(entry.started)}</p>
      {!saved && (
        <p className="mt-3 text-[0.85rem]">
          <Status tone={tone}>{entryStatus(entry, stage)}</Status>
        </p>
      )}

      {shown?.overview && (
        <div className="mt-5">
          <h3 className="mb-1.5 text-[0.72rem] font-semibold tracking-[0.06em] text-label-tertiary uppercase">
            Summary
          </h3>
          <p className="line-clamp-[12] leading-[1.5]">{shown.overview}</p>
        </div>
      )}

      <dl className="mt-5 divide-y divide-separator overflow-hidden rounded-[10px] bg-group text-[0.85rem] shadow-[0_0_0_0.5px_var(--separator)]">
        <Detail label="Duration" value={formatDuration(entry.duration_ms)} />
        {entry.found_speakers !== null && <Detail label="Speakers" value={String(entry.found_speakers)} />}
        {shown?.folder && <Detail label="Folder" value={folderName(shown.folder)} title={shown.folder} />}
      </dl>
    </section>
  );
}
