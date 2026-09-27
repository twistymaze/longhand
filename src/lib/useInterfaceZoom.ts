import { useEffect, useState } from "react";
import { commands, type ZoomAction } from "../bindings";

export function zoomAction(event: KeyboardEvent, isMac: boolean): ZoomAction | null {
  if (event.isComposing || event.altKey || event.getModifierState("AltGraph")) return null;
  if (isMac ? !event.metaKey || event.ctrlKey : !event.ctrlKey || event.metaKey) return null;
  if (event.shiftKey && event.key !== "+" && event.key !== "=") return null;
  if (event.key === "+" || event.key === "=") return "increase";
  if (event.key === "-") return "decrease";
  if (event.key === "0") return "reset";
  return null;
}

export function useInterfaceZoom(isMac = navigator.userAgent.includes("Mac")) {
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let calls: Promise<unknown> = Promise.resolve();
    const handle = (event: KeyboardEvent) => {
      if (document.querySelector('[data-shortcut-capture="true"]')) return;
      const action = zoomAction(event, isMac);
      if (!action) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      calls = calls.then(() => commands.changeInterfaceZoom(action)).then((result) => {
        setError(result.status === "error" ? result.error : null);
      }, (reason: unknown) => setError(String(reason)));
    };
    window.addEventListener("keydown", handle, true);
    return () => window.removeEventListener("keydown", handle, true);
  }, [isMac]);
  return { error, dismiss: () => setError(null) };
}
