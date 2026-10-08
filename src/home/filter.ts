// The Home filter (R19, R20, R21): one type choice and every chosen tag.
// The filter lives in Home only, so it stays while Home is mounted and
// resets when the app quits (R33).

import type { NoteTypeEntry, SessionEntry, TagEntry } from "../bindings";
import { isOpen } from "../processing/describe";
import { sameType, tagKey, tagParts } from "../lib/Labels";

/// The type choice of the filter: every note, the notes of General (at the
/// top of the storage folder), or the notes of one other type.
export type TypeChoice = { kind: "all" } | { kind: "general" } | { kind: "named"; name: string };

/// `tags` holds tag keys (KTD10), in the order the user chose them.
export type NoteFilter = { type: TypeChoice; tags: string[] };

export const noFilter: NoteFilter = { type: { kind: "all" }, tags: [] };

export const isFiltered = (filter: NoteFilter) => filter.type.kind !== "all" || filter.tags.length > 0;

/// True when the note has the chosen type and every chosen tag. A note that
/// records, or whose recording was interrupted, always matches (R21).
export function matches(entry: SessionEntry, filter: NoteFilter): boolean {
  if (isOpen(entry.state)) return true;
  const { type } = filter;
  if (type.kind === "general" && entry.note_type !== null) return false;
  if (type.kind === "named" && (entry.note_type === null || !sameType(entry.note_type, type.name))) return false;
  if (filter.tags.length === 0) return true;
  const keys = new Set(entry.tags.map(tagKey));
  return filter.tags.every((key) => keys.has(key));
}

/// True when note types are in use: the type list has a type other than
/// General, also a type with no notes (R1, KTD1). A list that is not read
/// yet is not in use.
export const typesInUse = (types: NoteTypeEntry[] | null) => types?.some((t) => !t.general) ?? false;

/// True when General is a choice of the type filter (R2, R3). A list that is
/// not read yet keeps a chosen General, so the choice does not drop at
/// launch.
export const generalIsChoice = (types: NoteTypeEntry[] | null) => types === null || typesInUse(types);

const byText = new Intl.Collator(undefined, { sensitivity: "base" }).compare;

/// The tags in the order of the Tags menu (R7, KTD6): the tags with no
/// scope first, then the scopes, each A-Z without regard to case. A scope
/// that differs only in case is one scope.
export function menuOrder(tags: TagEntry[]): TagEntry[] {
  const rows = tags.map((tag) => {
    const { scope, value } = tagParts(tag.text);
    return { tag, scope: scope === null ? null : tagKey(scope), value };
  });
  rows.sort((a, b) => {
    if (a.scope !== b.scope) {
      if (a.scope === null) return -1;
      if (b.scope === null) return 1;
      const scope = byText(a.scope, b.scope);
      if (scope !== 0) return scope;
    }
    return byText(a.value, b.value) || byText(a.tag.text, b.tag.text);
  });
  return rows.map((row) => row.tag);
}

/// The filter without a type or a tag that no longer exists. General drops
/// out when `general` is false (see `generalIsChoice`). A null list is not
/// read yet, so its choices stay. A type that changed only its case keeps its place with the
/// new spelling. Returns the same object when nothing changes.
export function keepExisting(
  filter: NoteFilter,
  types: NoteTypeEntry[] | null,
  tags: TagEntry[] | null,
  general: boolean,
): NoteFilter {
  let type = filter.type;
  if (!general && type.kind === "general") type = { kind: "all" };
  if (types && type.kind === "named") {
    const name = type.name;
    const found = types.find((t) => !t.general && sameType(t.name, name));
    if (!found) type = { kind: "all" };
    else if (found.name !== name) type = { kind: "named", name: found.name };
  }
  let chosen = filter.tags;
  if (tags) {
    const keys = new Set(tags.map((t) => t.key));
    const kept = chosen.filter((key) => keys.has(key));
    if (kept.length !== chosen.length) chosen = kept;
  }
  return type === filter.type && chosen === filter.tags ? filter : { type, tags: chosen };
}
