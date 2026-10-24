// The file drop of the window (KTD14 of the audio import plan). Tauri's
// native drop gives the absolute paths of the files; the HTML `drop`
// event gives none. The hook tells whether files are over the window, so
// Home shows the drop target only then, and gives the paths of a drop.

import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect, useRef, useState } from "react";

/// While `enabled`, listen for files that the user drags over the window.
/// Returns true while files are over the window. `onDrop` gets the paths
/// of the dropped files.
export function useFileDrop(enabled: boolean, onDrop: (paths: string[]) => void): boolean {
  const [over, setOver] = useState(false);
  const dropped = useRef(onDrop);
  dropped.current = onDrop;

  useEffect(() => {
    if (!enabled) return;
    let unlisten: (() => void) | null = null;
    let ended = false;
    void getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        if (payload.type === "enter") setOver(payload.paths.length > 0);
        else if (payload.type === "leave") setOver(false);
        else if (payload.type === "drop") {
          setOver(false);
          if (payload.paths.length > 0) dropped.current(payload.paths);
        }
      })
      .then((stop) => {
        if (ended) stop();
        else unlisten = stop;
      })
      .catch((e: unknown) => {
        console.error("The file drop could not be started:", e);
      });
    return () => {
      ended = true;
      unlisten?.();
      setOver(false);
    };
  }, [enabled]);

  return over;
}
