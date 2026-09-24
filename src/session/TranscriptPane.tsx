// The landed transcript. A followed note scrolls its turn to the middle and
// tints it briefly, so the eye finds the place without a search; the tint
// is a colour change, which reduced motion keeps, and the scroll jumps
// instead of gliding when motion is reduced.

import { useEffect, useRef } from "react";
import type { DocLine } from "../bindings";
import { formatPosition } from "../lib/timeline";

function Marker({ label }: { label: string }) {
  return (
    <div className="flex items-center gap-3 py-2 text-[0.85rem] text-label-tertiary">
      <span className="h-px flex-1 bg-separator" />
      {label}
      <span className="h-px flex-1 bg-separator" />
    </div>
  );
}

export function TranscriptPane({
  lines,
  hasSpeakers,
  target,
}: {
  lines: DocLine[];
  hasSpeakers: boolean;
  /// The line to bring into view, with a counter so following the same
  /// note twice scrolls again.
  target: { index: number; nonce: number } | null;
}) {
  const refs = useRef<(HTMLElement | null)[]>([]);

  useEffect(() => {
    if (!target) return;
    const el = refs.current[target.index];
    if (!el) return;
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    el.scrollIntoView({ block: "center", behavior: reduce ? "auto" : "smooth" });
    el.classList.remove("followed");
    // Restart the tint even when the same line is followed again.
    void el.offsetWidth;
    el.classList.add("followed");
  }, [target]);

  let previousSpeaker: number | null | undefined;
  return (
    <div className="px-6 py-4">
      {!hasSpeakers && (
        <p className="mb-3 text-[0.85rem] text-label-secondary">
          Speaker separation failed for this session, so the transcript has no speaker marks.
        </p>
      )}
      {lines.length === 0 && <p className="text-label-tertiary">Nothing was transcribed.</p>}
      {lines.map((line, i) => {
        const ref = (el: HTMLElement | null) => {
          refs.current[i] = el;
        };
        if (line.kind === "gap") {
          previousSpeaker = undefined;
          return (
            <div key={i} ref={ref} className="rounded-md">
              <Marker label={`Not recorded for ${formatPosition(line.duration_ms)}`} />
            </div>
          );
        }
        if (line.kind === "silence") {
          previousSpeaker = undefined;
          return (
            <div key={i} ref={ref} className="rounded-md">
              <Marker label={`Silence for ${formatPosition(line.duration_ms)}`} />
            </div>
          );
        }
        const newSpeaker = line.speaker !== previousSpeaker;
        previousSpeaker = line.speaker;
        return (
          <p
            key={i}
            ref={ref}
            className={`-mx-2 flex gap-3 rounded-md px-2 py-1 ${newSpeaker && i > 0 ? "mt-2" : ""}`}
          >
            <span className="w-[4.25rem] shrink-0 pt-px text-[0.85rem] text-label-tertiary tabular-nums">
              {formatPosition(line.position_ms)}
            </span>
            <span className="min-w-0 flex-1 leading-relaxed">
              {line.speaker !== null && newSpeaker && (
                <span className="mr-1.5 font-semibold">Speaker {line.speaker}</span>
              )}
              {line.text}
            </span>
          </p>
        );
      })}
    </div>
  );
}
