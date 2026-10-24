// The library notices of Home (KTD14 of the safe agent edits plan). They
// come from the latest library scans, so they are read again at each
// library change. The import notices are read again at each change of the
// imports or the watched folder.

import { useCallback, useEffect, useRef, useState } from "react";
import { commands, events, type LibraryNotices } from "../bindings";
import { useReloadOn } from "../lib/useReloadOn";

const ignore = () => {};

/// The library notices, or null until the first read. A read that fails
/// keeps the last notices.
export function useLibraryNotices(): { notices: LibraryNotices | null; reload: () => Promise<void> } {
  const [notices, setNotices] = useState<LibraryNotices | null>(null);
  // Reads can end out of order. Only the newest read changes the notices.
  const request = useRef(0);
  const reload = useCallback(async () => {
    const current = ++request.current;
    try {
      const result = await commands.libraryNotices();
      if (current === request.current && result.status === "ok") setNotices(result.data);
    } catch (e) {
      console.error("The library notices could not be read:", e);
    }
  }, []);
  useEffect(() => {
    void reload();
  }, [reload]);
  useReloadOn(ignore, reload);
  // The import notices change with the import worker and the watched
  // folder (U8 of the audio import plan).
  useEffect(() => {
    const unlisteners = [
      events.importNoticesChanged.listen(() => void reload()),
      events.watchedFolderChanged.listen(() => void reload()),
    ];
    return () => {
      for (const u of unlisteners) void u.then((f) => f());
    };
  }, [reload]);
  return { notices, reload };
}
