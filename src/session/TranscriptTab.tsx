// The saved transcript (R16, R18). Consecutive turns of one speaker show
// under one label, each paragraph as its own block. A paragraph's time
// shows while the pointer is on it. A resume shows as a marker with its
// date and time. The tab opens at the end, where a reader who returns to a
// session usually continues.

import { useLayoutEffect, useRef } from "react";
import type { DocLine } from "../bindings";
import { speakerCount } from "../home/group";
import { formatPosition } from "../lib/timeline";

type Turn = Extract<DocLine, { kind: "turn" }>;

type Part =
  | { kind: "speaker"; speaker: number | null; turns: Turn[] }
  | { kind: "marker"; label: string };

/// "35:48", or "4:02:03" with hours.
export function formatClock(ms: number): string {
  const secs = Math.floor(Math.max(0, ms) / 1000);
  const h = Math.floor(secs / 3600);
  const m = Math.floor(secs / 60) % 60;
  const s = (secs % 60).toString().padStart(2, "0");
  return h > 0 ? `${h}:${m.toString().padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/// "Resumed 30 Sep, 10:02", with the year when it is not this year.
export function resumedLabel(iso: string, now: Date = new Date()): string {
  const at = new Date(iso);
  const date = at.toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    year: at.getFullYear() === now.getFullYear() ? undefined : "numeric",
  });
  const time = at.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  return `Resumed ${date}, ${time}`;
}

/// The lines in groups: the turns of one speaker until another speaks or
/// a marker comes.
function parts(lines: DocLine[]): Part[] {
  const out: Part[] = [];
  for (const line of lines) {
    if (line.kind === "turn") {
      const last = out[out.length - 1];
      if (last?.kind === "speaker" && last.speaker === line.speaker) last.turns.push(line);
      else out.push({ kind: "speaker", speaker: line.speaker, turns: [line] });
    } else if (line.kind === "gap") {
      out.push({
        kind: "marker",
        label: line.wall_start ? resumedLabel(line.wall_start) : `Not recorded for ${formatClock(line.duration_ms)}`,
      });
    } else {
      out.push({ kind: "marker", label: `Silence for ${formatClock(line.duration_ms)}` });
    }
  }
  return out;
}

function Marker({ label }: { label: string }) {
  return (
    <div className="my-5 flex items-center gap-3 text-[0.85rem] text-label-tertiary">
      <span className="h-px flex-1 bg-separator" />
      {label}
      <span className="h-px flex-1 bg-separator" />
    </div>
  );
}

export function TranscriptTab({
  lines,
  hasSpeakers,
  foundSpeakers,
  durationMs,
}: {
  lines: DocLine[];
  hasSpeakers: boolean;
  foundSpeakers: number | null;
  durationMs: number;
}) {
  const end = useRef<HTMLDivElement>(null);
  // Once, when the tab opens: a later change must not move the reader.
  useLayoutEffect(() => end.current?.scrollIntoView({ block: "end" }), []);

  const header = [speakerCount(foundSpeakers), formatClock(durationMs)].filter(Boolean).join(" · ");

  return (
    <div>
      <p className="text-[0.85rem] text-label-secondary">{header}</p>
      {!hasSpeakers && (
        <p className="mt-2 text-[0.85rem] text-label-secondary">
          Speaker separation failed for this session, so the transcript has no speaker marks.
        </p>
      )}
      {lines.length === 0 && <p className="mt-4 text-label-tertiary">Nothing was transcribed.</p>}
      {parts(lines).map((part, i) =>
        part.kind === "marker" ? (
          <Marker key={i} label={part.label} />
        ) : (
          <section key={i} className="mt-5">
            {part.speaker !== null && (
              <h3 className="mb-1 pl-[4.5rem] text-[0.85rem] font-semibold text-label-secondary">
                Speaker {part.speaker}
              </h3>
            )}
            {part.turns.map((turn, j) => {
              const time = formatPosition(turn.position_ms);
              return (
                <div key={j} className="group grid grid-cols-[4.5rem_1fr] py-1">
                  <span
                    aria-hidden
                    className="pt-[0.2rem] text-[0.8rem] text-label-tertiary tabular-nums opacity-0 group-hover:opacity-100"
                  >
                    {time}
                  </span>
                  <p className="leading-[1.55]">{turn.text}</p>
                </div>
              );
            })}
          </section>
        ),
      )}
      <div ref={end} className="h-6" />
    </div>
  );
}
