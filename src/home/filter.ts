// The Home filter (R19, R20, R21): one type choice and every chosen tag.
// The filter lives in Home only, so it stays while Home is mounted and
// resets when the app quits (R33).

import type { NoteTypeEntry, SessionEntry, TagEntry } from "../bindings";
import { isOpen } from "../processing/describe";
import { sameType, tagKey } from "../lib/Labels";

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

/// True when General is a choice: at least one listed note is in General.
export const hasGeneralNote = (entries: SessionEntry[]) => entries.some((e) => e.note_type === null);

/// The filter without a type or a tag that no longer exists. General drops
/// out when `general` is false. A null list is not read yet, so its
/// choices stay. A type that changed only its case keeps its place with the
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
