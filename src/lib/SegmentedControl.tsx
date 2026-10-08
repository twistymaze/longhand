// A segmented control after the macOS control: one row of segments with
// one width, and one chosen segment. The session page uses it for its tabs,
// and Home uses it for the type filter. The segments change at once, with
// no motion: they are used many times a day.

import type { ReactNode } from "react";

export type Segment<T extends string> = {
  value: T;
  label: string;
  /// Content after the label inside the segment, such as the level bars on
  /// Transcript.
  accessory?: ReactNode;
};

/// The segments have one width, so the control keeps its shape when an
/// accessory shows. The arrow keys move the choice and the focus through
/// `items`, and wrap at the ends. Only the chosen segment is in the tab
/// order. `kind` "tabs" is a tab list that shows a panel. `kind` "radio" is
/// a group of radio buttons, for a choice that does not show a panel.
export function SegmentedControl<T extends string>({
  items,
  value,
  onChange,
  label,
  kind = "tabs",
}: {
  items: Segment<T>[];
  value: T;
  /// `click` is true for a click, false for an arrow key.
  onChange: (value: T, click: boolean) => void;
  /// The accessible name of the control.
  label: string;
  kind?: "tabs" | "radio";
}) {
  const tabs = kind === "tabs";
  return (
    <div
      role={tabs ? "tablist" : "radiogroup"}
      aria-label={label}
      className="inline-grid auto-cols-fr grid-flow-col gap-0.5 rounded-[7px] bg-fill p-[2px]"
      onKeyDown={(e) => {
        if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
        e.preventDefault();
        const at = items.findIndex((t) => t.value === value);
        const next = items[(at + (e.key === "ArrowRight" ? 1 : items.length - 1)) % items.length];
        onChange(next.value, false);
        e.currentTarget.querySelector<HTMLElement>(`[data-segment="${CSS.escape(next.value)}"]`)?.focus();
      }}
    >
      {items.map((t) => {
        const chosen = value === t.value;
        return (
          <button
            key={t.value}
            type="button"
            role={tabs ? "tab" : "radio"}
            data-segment={t.value}
            aria-selected={tabs ? chosen : undefined}
            aria-checked={tabs ? undefined : chosen}
            tabIndex={chosen ? 0 : -1}
            onClick={() => onChange(t.value, true)}
            className={`flex items-center justify-center gap-1.5 rounded-[5px] px-4 py-[3px] whitespace-nowrap ${
              chosen
                ? "bg-group text-label shadow-[0_0.5px_1.5px_rgb(0_0_0/0.18),0_0_0_0.5px_rgb(0_0_0/0.04)] dark:bg-white/[0.2] dark:shadow-[0_0.5px_1px_rgb(0_0_0/0.4)]"
                : "text-label-secondary hover:text-label active:bg-fill-pressed"
            } outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent`}
          >
            {t.label}
            {t.accessory}
          </button>
        );
      })}
    </div>
  );
}
