// The file drop of the window (KTD14 of the audio import plan). Tauri's
// native drop gives the absolute paths of the files; the HTML `drop`
// event gives none. The hook tells whether files are over the window, so
// Home shows the drop target only then, and gives the paths of a drop.
//
// Some apps, for example Voice Memos, drag a file promise and no file. On
// macOS the app then sends `promisedFiles`: "over" when such a drag
// enters, and "received" when the other app wrote the files of the drop.

import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect, useRef, useState } from "react";
import { events } from "../bindings";

/// While `enabled`, listen for files that the user drags over the window.
/// Returns true while files are over the window. `onDrop` gets the paths
/// of the dropped files. `onFailed` runs when the other app of a drop did
/// not give one or more of its promised files.
export function useFileDrop(
  enabled: boolean,
  onDrop: (paths: string[]) => void,
  onFailed: () => void,
): boolean {
  // Files over the window, and promised files over the window. Tauri's
  // enter event has no paths for promised files, so each event sets only
  // its own value, and the order of the two events is not important.
  const [filesOver, setFilesOver] = useState(false);
  const [promisesOver, setPromisesOver] = useState(false);
  const dropped = useRef(onDrop);
  dropped.current = onDrop;
  const failed = useRef(onFailed);
  failed.current = onFailed;

  useEffect(() => {
    if (!enabled) return;
    const stops: (() => void)[] = [];
    let ended = false;
    const keep = (stop: () => void) => {
      if (ended) stop();
      else stops.push(stop);
    };
    const notStarted = (e: unknown) => {
      console.error("The file drop could not be started:", e);
    };
    void getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        if (payload.type === "enter") setFilesOver(payload.paths.length > 0);
        else if (payload.type === "leave" || payload.type === "drop") {
          setFilesOver(false);
          setPromisesOver(false);
          if (payload.type === "drop" && payload.paths.length > 0) dropped.current(payload.paths);
        }
      })
      .then(keep)
      .catch(notStarted);
    void events.promisedFiles
      .listen(({ payload }) => {
        if (payload.kind === "over") setPromisesOver(true);
        else {
          if (payload.paths.length > 0) dropped.current(payload.paths);
          if (payload.failed > 0) failed.current();
        }
      })
      .then(keep)
      .catch(notStarted);
    return () => {
      ended = true;
      for (const stop of stops) stop();
      setFilesOver(false);
      setPromisesOver(false);
    };
  }, [enabled]);

  return filesOver || promisesOver;
}
