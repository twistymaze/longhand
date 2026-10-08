// The labels for a note's type and tags (R14). A tag with a scope ("Person
// :: Nathan") shows in two parts: the scope in the scope's colour, then the
// value. A tag with no scope shows in one part. A type label has an outline
// and a folder symbol, because the type is the note's folder. The labels do
// not move or change by themselves, so they keep the attention budget of
// the recording page (R17).

import type { CSSProperties } from "react";

/// The text of a tag in its parts. The scope is the text before the first
/// `::`. A tag with an empty scope or an empty value has no scope.
export function tagParts(text: string): { scope: string | null; value: string } {
  const trimmed = text.trim();
  const at = trimmed.indexOf("::");
  if (at >= 0) {
    const scope = trimmed.slice(0, at).trim();
    const value = trimmed.slice(at + 2).trim();
    if (scope && value) return { scope, value };
  }
  return { scope: null, value: trimmed };
}

/// The key that matches two tags (R30, KTD10): the trimmed text, with no
/// spaces around `::`, in lower case.
/// This must match `session_core::tags::tag_key`.
export function tagKey(text: string): string {
  return text.trim().replace(/\s*::\s*/g, "::").toLowerCase();
}

/// Type names compare without regard to case (KTD8).
export const sameType = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/// The text that VoiceOver reads for a tag, such as "Person: Nathan".
export function tagName(text: string): string {
  const { scope, value } = tagParts(text);
  return scope ? `${scope}: ${value}` : value;
}

/// The text that VoiceOver reads for a note type, such as "Type: 1:1".
export function typeName(name: string): string {
  return `Type: ${name}`;
}

export type ScopeTone = { light: string; dark: string };

/// The scope colours, for light and dark. They start from the macOS system
/// colours in their increased-contrast form. The light values are darker and
/// the dark values are lighter, until the text meets 4.5:1 on its tint over
/// the window, a group and a row under the pointer. Yellow is not in the
/// set, because it is too near to orange at that contrast.
export const scopeTones: ScopeTone[] = [
  { light: "#ae0011", dark: "#ff9791" }, // red
  { light: "#a22a00", dark: "#ffb340" }, // orange
  { light: "#1b662d", dark: "#30db5b" }, // green
  { light: "#09625e", dark: "#66d4cf" }, // mint
  { light: "#006071", dark: "#5de6ff" }, // teal
  { light: "#005e89", dark: "#70d7ff" }, // cyan
  { light: "#0040dd", dark: "#76b8ff" }, // blue
  { light: "#3634a3", dark: "#afadff" }, // indigo
  { light: "#793c97", dark: "#dd99ff" }, // purple
  { light: "#aa0c38", dark: "#ff93a8" }, // pink
  { light: "#6a5439", dark: "#cab294" }, // brown
];

/// The opacity of the scope colour under the scope text. The class names in
/// `TagLabel` repeat it as "14%", because Tailwind reads only literal text.
export const SCOPE_TINT = 0.14;

/// The colour of a scope. A 32-bit FNV-1a hash of the scope key chooses it,
/// so a scope has the same colour on every screen and every computer.
export function scopeTone(scope: string): ScopeTone {
  let hash = 0x811c9dc5;
  for (const char of tagKey(scope)) {
    hash ^= char.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return scopeTones[hash % scopeTones.length];
}

const labelText = "text-[0.72rem] leading-[1.3] font-medium whitespace-nowrap";

function RemoveButton({ name, onRemove }: { name: string; onRemove: () => void }) {
  return (
    <button
      type="button"
      aria-label={`Remove ${name}`}
      onClick={onRemove}
      className="flex size-[13px] shrink-0 items-center justify-center rounded-[3px] text-label-secondary hover:bg-fill-pressed hover:text-label outline-offset-0 focus-visible:outline-2 focus-visible:outline-accent"
    >
      <svg aria-hidden viewBox="0 0 8 8" className="size-[7px]" fill="none" stroke="currentColor" strokeWidth="1.4">
        <path d="M1.5 1.5l5 5M6.5 1.5l-5 5" strokeLinecap="round" />
      </svg>
    </button>
  );
}

/// A tag label. With `onRemove`, the label ends with a control that removes
/// the tag.
export function TagLabel({ text, onRemove }: { text: string; onRemove?: () => void }) {
  const { scope, value } = tagParts(text);
  const name = tagName(text);
  const remove = onRemove && <RemoveButton name={name} onRemove={onRemove} />;
  if (!scope) {
    return (
      <span
        data-label="tag"
        className={`inline-flex max-w-full items-center gap-0.5 rounded-[4px] bg-fill px-[5px] py-px text-label-secondary ${labelText}`}
      >
        <span className="min-w-0 truncate">{value}</span>
        {remove}
      </span>
    );
  }
  const tone = scopeTone(scope);
  return (
    <span data-label="tag" className={`inline-flex max-w-full items-stretch overflow-hidden rounded-[4px] ${labelText}`}>
      <span className="sr-only">{name}</span>
      <span aria-hidden className="inline-flex min-w-0">
        <span
          data-scope
          style={{ "--scope-light": tone.light, "--scope-dark": tone.dark } as CSSProperties}
          className={`shrink-0 px-[5px] py-px text-[color:var(--scope-light)] dark:text-[color:var(--scope-dark)] bg-[color-mix(in_srgb,var(--scope-light)_14%,transparent)] dark:bg-[color-mix(in_srgb,var(--scope-dark)_14%,transparent)]`}
        >
          {scope}
        </span>
        <span className="min-w-0 truncate bg-fill px-[5px] py-px text-label">{value}</span>
      </span>
      {remove && <span className="flex items-center bg-fill pr-[3px]">{remove}</span>}
    </span>
  );
}

/// A note type label: an outline, a folder symbol and the type name.
export function TypeLabel({ name }: { name: string }) {
  return (
    <span
      data-label="type"
      className={`inline-flex max-w-full items-center gap-[3px] rounded-[4px] px-[4px] py-px text-label-secondary shadow-[inset_0_0_0_1px_var(--control-border)] ${labelText}`}
    >
      <span className="sr-only">Type: </span>
      <svg aria-hidden viewBox="0 0 12 10" className="h-[8px] w-[10px] shrink-0" fill="none" stroke="currentColor" strokeWidth="1.2">
        <path d="M1 2.2c0-.6.4-1 1-1h2.4l1 1.2H10c.6 0 1 .4 1 1v4.4c0 .6-.4 1-1 1H2c-.6 0-1-.4-1-1z" strokeLinejoin="round" />
      </svg>
      <span className="min-w-0 truncate">{name}</span>
    </span>
  );
}
