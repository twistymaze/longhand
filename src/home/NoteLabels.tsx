// The type and tag labels of a Home row (R22). The labels stay on one line.
// A label that does not fit in full is hidden, and "+N" after the last
// label that shows gives the number of hidden labels. The first label always
// shows. A long first label shows in part.

import { useLayoutEffect, useRef, useState } from "react";
import { TagLabel, tagName, TypeLabel, typeName } from "../lib/Labels";

/// The space that "+N" needs after the last label that shows.
const MORE_WIDTH = 28;
const GAP = 4;

export function NoteLabels({
  noteType,
  tags,
  onAccent = false,
}: {
  noteType: string | null;
  tags: string[];
  /// The row is selected: each label gets a solid surface, so it keeps its
  /// contrast on the accent colour.
  onAccent?: boolean;
}) {
  const area = useRef<HTMLDivElement>(null);
  const count = (noteType === null ? 0 : 1) + tags.length;
  // The number of labels that show, and the position of "+N".
  const [fit, setFit] = useState({ shown: count, more: 0 });

  // A hidden label keeps its place in the layout, so each measurement
  // reads the full width of every label.
  useLayoutEffect(() => {
    const el = area.current;
    if (!el) return;
    const measure = () => {
      const width = el.clientWidth;
      const labels = [...el.querySelectorAll<HTMLElement>(":scope > [data-index]")];
      // With no layout (a test DOM), every label shows.
      if (width === 0) return setFit({ shown: labels.length, more: 0 });
      let shown = 0;
      for (const [i, label] of labels.entries()) {
        const right = label.offsetLeft + label.offsetWidth;
        const last = i === labels.length - 1;
        if (right > width - (last ? 0 : GAP + MORE_WIDTH)) break;
        shown = i + 1;
      }
      shown = Math.max(1, shown);
      const end = labels[shown - 1];
      const more = end ? end.offsetLeft + end.offsetWidth + GAP : 0;
      setFit((f) => (f.shown === shown && f.more === more ? f : { shown, more }));
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [noteType, tags.join("\n")]);

  if (count === 0) return null;
  const shown = Math.min(fit.shown, count);
  const names = [...(noteType === null ? [] : [typeName(noteType)]), ...tags.map(tagName)];
  const hidden = names.slice(shown);
  const surface = onAccent ? "rounded-[4px] bg-group" : "";
  const items = [
    ...(noteType === null ? [] : [<TypeLabel key="type" name={noteType} />]),
    ...tags.map((tag) => <TagLabel key={`tag:${tag}`} text={tag} />),
  ];
  return (
    <div ref={area} className="relative flex min-w-0 flex-1 items-center gap-1 overflow-hidden">
      {items.map((item, i) => (
        <span
          key={item.key}
          data-index={i}
          // A label never shrinks. The first label is at most as wide as
          // the space less "+N", so a long name shows in part.
          className={`flex shrink-0 ${i === 0 && count > 1 ? "max-w-[calc(100%-32px)]" : "max-w-full"} ${surface} ${
            i < shown ? "" : "invisible"
          }`}
        >
          {item}
        </span>
      ))}
      {hidden.length > 0 && (
        <span
          title={hidden.join(", ")}
          aria-label={`${hidden.length} more: ${hidden.join(", ")}`}
          style={{ left: fit.more }}
          className={`absolute top-1/2 -translate-y-1/2 rounded-[4px] px-[5px] py-px text-[0.72rem] leading-[1.3] font-medium whitespace-nowrap ${
            onAccent ? "bg-white/20 text-white" : "bg-fill text-label-secondary"
          }`}
        >
          +{hidden.length}
        </span>
      )}
    </div>
  );
}
