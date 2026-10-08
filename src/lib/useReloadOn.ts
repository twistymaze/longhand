// Reads data again when the settings or the library change. Another
// computer, the Settings window or a note page can change the note types
// and the tags.

import { useEffect } from "react";
import { events } from "../bindings";

/// Calls `onSettings` at each settings change and `onLibrary` at each
/// library change. With no `onLibrary`, a library change calls
/// `onSettings`. Each function must keep its identity between renders.
export function useReloadOn(onSettings: () => unknown, onLibrary: () => unknown = onSettings) {
  useEffect(() => {
    const unlisteners = [
      events.settingsChanged.listen(() => void onSettings()),
      events.libraryChanged.listen(() => void onLibrary()),
    ];
    return () => {
      for (const u of unlisteners) void u.then((f) => f());
    };
  }, [onSettings, onLibrary]);
}
