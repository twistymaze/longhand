// The labels for a note's type and tags (R14). A tag with a scope ("Person
// :: Nathan") shows in two parts: the scope in a neutral colour, then the
// value in the scope's colour. A tag with no scope shows in one neutral
// part. Each scope in the library has its own colour on every screen. A
// type label has an outline and a folder symbol, because the type is the
// note's folder. The labels do not move or change by themselves, so they
// keep the attention budget of the recording page (R17).

import { createContext, type CSSProperties, type ReactNode, useCallback, useContext, useEffect, useRef, useState } from "react";
import { commands } from "../bindings";
import { useReloadOn } from "./useReloadOn";

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
/// set, because it is too near to orange at that contrast. Two colours that
/// are next to each other in the list have very different hues, because a
/// scope whose colour is in use gets the next free colour.
export const scopeTones: ScopeTone[] = [
  { light: "#ae0011", dark: "#ff9791" }, // red
  { light: "#006071", dark: "#5de6ff" }, // teal
  { light: "#a22a00", dark: "#ffb340" }, // orange
  { light: "#0040dd", dark: "#76b8ff" }, // blue
  { light: "#1b662d", dark: "#30db5b" }, // green
  { light: "#aa0c38", dark: "#ff93a8" }, // pink
  { light: "#005e89", dark: "#70d7ff" }, // cyan
  { light: "#6a5439", dark: "#cab294" }, // brown
  { light: "#3634a3", dark: "#afadff" }, // indigo
  { light: "#09625e", dark: "#66d4cf" }, // mint
  { light: "#793c97", dark: "#dd99ff" }, // purple
];

/// The opacity of the scope colour under the value text. The class names in
/// `TagLabel` repeat it as "14%", because Tailwind reads only literal text.
export const SCOPE_TINT = 0.14;

/// The position of the hash colour of a scope key in `scopeTones`. A 32-bit
/// FNV-1a hash of the key chooses it, so the result is the same on every
/// computer.
function hashIndex(key: string): number {
  let hash = 0x811c9dc5;
  for (const char of key) {
    hash ^= char.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash % scopeTones.length;
}

/// The hash colour of a scope. Two scopes can have the same hash colour.
/// `TagLabel` uses it for a scope that is not in the library list yet, and
/// when no `ScopeTonesProvider` is above it.
export function scopeTone(scope: string): ScopeTone {
  return scopeTones[hashIndex(tagKey(scope))];
}

/// The colours of the scopes in the library, by scope key. The function
/// sorts the keys. In that order, each scope gets its hash colour when no
/// other scope has it. Then each scope that did not get a colour gets the
/// next free colour after its hash colour, in the order of `scopeTones`.
/// Two scopes have the same colour only when there are more scopes than
/// colours. The same scopes, in any order, give the same colours.
export function assignScopeTones(scopes: Iterable<string>): Map<string, ScopeTone> {
  const keys = [...new Set([...scopes].map(tagKey))].sort();
  const used = new Set<number>();
  const result = new Map<string, ScopeTone>();
  const waiting: string[] = [];
  for (const key of keys) {
    const index = hashIndex(key);
    if (used.has(index)) {
      waiting.push(key);
    } else {
      used.add(index);
      result.set(key, scopeTones[index]);
    }
  }
  for (const key of waiting) {
    // All colours are in use: a new round starts, and colours repeat.
    if (used.size === scopeTones.length) used.clear();
    let index = hashIndex(key);
    while (used.has(index)) index = (index + 1) % scopeTones.length;
    used.add(index);
    result.set(key, scopeTones[index]);
  }
  return result;
}

const ScopeTonesContext = createContext<Map<string, ScopeTone> | null>(null);

const ignore = () => {};

/// Gives the library scope colours to each `TagLabel` in the window. It
/// reads the tag list once, and again at each library change, because a
/// tag edit can add or remove a scope.
export function ScopeTonesProvider({ children }: { children: ReactNode }) {
  const [tones, setTones] = useState<Map<string, ScopeTone>>(() => new Map());
  // Reads can end out of order. Each read takes a number, and only the
  // newest read changes the colours.
  const request = useRef(0);
  const read = useCallback(async () => {
    const current = ++request.current;
    try {
      const result = await commands.listTags();
      if (current !== request.current || result.status !== "ok") return;
      const scopes = result.data.flatMap((t) => tagParts(t.text).scope ?? []);
      setTones(assignScopeTones(scopes));
    } catch {
      // The colours stay as they are. The next library change reads again.
    }
  }, []);
  useEffect(() => {
    void read();
  }, [read]);
  useReloadOn(ignore, read);
  return <ScopeTonesContext.Provider value={tones}>{children}</ScopeTonesContext.Provider>;
}

/// The colour of a scope: the library colour, or the hash colour when the
/// library list does not have the scope.
function useScopeTone(scope: string | null): ScopeTone | null {
  const tones = useContext(ScopeTonesContext);
  if (scope === null) return null;
  return tones?.get(tagKey(scope)) ?? scopeTone(scope);
}

// About 11.4 px text in a 20 px label. The remove control is 16 px.
const labelText = "text-[0.875rem] leading-[1.4] font-medium whitespace-nowrap";
const part = "px-[6px] py-[2px]";

function RemoveButton({ name, onRemove }: { name: string; onRemove: () => void }) {
  return (
    <button
      type="button"
      aria-label={`Remove ${name}`}
      onClick={onRemove}
      className="flex size-[16px] shrink-0 items-center justify-center rounded-[4px] text-current opacity-70 hover:bg-fill-pressed hover:opacity-100 outline-offset-0 focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-accent"
    >
      <svg aria-hidden viewBox="0 0 8 8" className="size-[8px]" fill="none" stroke="currentColor" strokeWidth="1.5">
        <path d="M1.5 1.5l5 5M6.5 1.5l-5 5" strokeLinecap="round" />
      </svg>
    </button>
  );
}

/// A tag label. With `onRemove`, the label ends with a control that removes
/// the tag.
export function TagLabel({ text, onRemove }: { text: string; onRemove?: () => void }) {
  const { scope, value } = tagParts(text);
  const tone = useScopeTone(scope);
  const name = tagName(text);
  const remove = onRemove && <RemoveButton name={name} onRemove={onRemove} />;
  if (!scope || !tone) {
    return (
      <span
        data-label="tag"
        className={`inline-flex max-w-full items-center rounded-[5px] bg-fill text-label-secondary ${labelText}`}
      >
        <span className={`min-w-0 truncate ${part} ${remove ? "pr-[2px]" : ""}`}>{value}</span>
        {remove && <span className="flex items-center pr-[2px]">{remove}</span>}
      </span>
    );
  }
  // The value part and the remove control have the scope colour.
  const tinted =
    "text-[color:var(--scope-light)] dark:text-[color:var(--scope-dark)] bg-[color-mix(in_srgb,var(--scope-light)_14%,transparent)] dark:bg-[color-mix(in_srgb,var(--scope-dark)_14%,transparent)]";
  return (
    <span
      data-label="tag"
      style={{ "--scope-light": tone.light, "--scope-dark": tone.dark } as CSSProperties}
      className={`inline-flex max-w-full items-stretch overflow-hidden rounded-[5px] ${labelText}`}
    >
      <span className="sr-only">{name}</span>
      <span aria-hidden className="inline-flex min-w-0">
        <span data-scope className={`shrink-0 bg-fill text-label-secondary ${part}`}>
          {scope}
        </span>
        <span data-value className={`min-w-0 truncate ${tinted} ${part} ${remove ? "pr-[2px]" : ""}`}>
          {value}
        </span>
      </span>
      {remove && <span className={`flex items-center pr-[2px] ${tinted}`}>{remove}</span>}
    </span>
  );
}

/// A note type label: an outline, a folder symbol and the type name.
export function TypeLabel({ name }: { name: string }) {
  return (
    <span
      data-label="type"
      className={`inline-flex max-w-full items-center gap-[4px] rounded-[5px] ${part} text-label-secondary shadow-[inset_0_0_0_1px_var(--control-border)] ${labelText}`}
    >
      <span className="sr-only">Type: </span>
      <svg aria-hidden viewBox="0 0 12 10" className="h-[9px] w-[11px] shrink-0" fill="none" stroke="currentColor" strokeWidth="1.2">
        <path d="M1 2.2c0-.6.4-1 1-1h2.4l1 1.2H10c.6 0 1 .4 1 1v4.4c0 .6-.4 1-1 1H2c-.6 0-1-.4-1-1z" strokeLinejoin="round" />
      </svg>
      <span className="min-w-0 truncate">{name}</span>
    </span>
  );
}
