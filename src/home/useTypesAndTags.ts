// The note types and the tag list for Home: the filter bar and the New
// Note menu use them. A settings change reads the types again (another
// computer, or the Settings window, changed the list). A library change
// reads both again, because a folder or a tag edit can change them. Home
// also reads both when it shows again, after the note page changed a note.

import { useCallback, useEffect, useState } from "react";
import { commands, type NoteTypeList, type TagEntry } from "../bindings";
import { useReloadOn } from "../lib/useReloadOn";

export function useTypesAndTags(active: boolean) {
  // Null until the first read. A failed read keeps the last list.
  const [types, setTypes] = useState<NoteTypeList | null>(null);
  const [tags, setTags] = useState<TagEntry[] | null>(null);

  const readTypes = useCallback(async () => {
    try {
      const result = await commands.noteTypes();
      if (result.status === "ok") setTypes(result.data);
    } catch {
      // The list stays as it is. The next event reads it again.
    }
  }, []);
  const readTags = useCallback(async () => {
    try {
      const result = await commands.listTags();
      if (result.status === "ok") setTags(result.data);
    } catch {
      // As above.
    }
  }, []);

  useEffect(() => {
    if (!active) return;
    void readTypes();
    void readTags();
  }, [active, readTypes, readTags]);

  const readBoth = useCallback(() => {
    void readTypes();
    void readTags();
  }, [readTypes, readTags]);
  useReloadOn(readTypes, readBoth);

  return { types, tags, setTypes };
}
