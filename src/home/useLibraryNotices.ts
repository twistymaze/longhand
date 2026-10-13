// The library notices of Home (KTD14 of the safe agent edits plan). They
// come from the latest library scans, so they are read again at each
// library change.

import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type LibraryNotices } from "../bindings";
import { useReloadOn } from "../lib/useReloadOn";

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
  const ignore = useCallback(() => {}, []);
  useReloadOn(ignore, reload);
  return { notices, reload };
}
