// The close coordinator of the Settings window (KTD9). Each Settings field
// that saves text registers its flush function here. One handler for the
// window close request waits for all registered flushes, then destroys the
// window. The `pagehide` and `beforeunload` events start each flush, but
// nothing waits for them.
//
// A pane change unmounts the pane, and each field then starts its own save.
// A window close destroys the page but does not unmount the pane, so the
// close waits for the saves, for at most the close limit. The window closes
// also if a save fails or does not complete, and the close asks nothing.
// Each close step goes to the app log.

import { getCurrentWindow } from "@tauri-apps/api/window";
import { commands } from "../bindings";
import { log, logText } from "../lib/log";

/// Starts the save of the pending text at once. Resolves to false when the
/// save failed.
export type CloseSave = () => Promise<boolean>;

/// The longest time that a window close waits for the saves, in
/// milliseconds. A save that does not complete must not keep the window
/// open.
export const defaultCloseSaveLimit = 2000;

/// The longest time that a window close waits for each later step (the
/// listener removal, the destroy), in milliseconds.
const stepLimit = 1000;

type Outcome<T> = { kind: "done"; value: T } | { kind: "error"; error: unknown } | { kind: "timeout" };

/// The result of `promise`, or a timeout when it does not settle in `ms`.
function within<T>(promise: Promise<T>, ms: number): Promise<Outcome<T>> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve({ kind: "timeout" }), ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve({ kind: "done", value });
      },
      (error: unknown) => {
        clearTimeout(timer);
        resolve({ kind: "error", error });
      },
    );
  });
}

type Entry = { flush: CloseSave; limit: number };

/// The registered saves. The handlers are installed while this set is not
/// empty.
const entries = new Set<Entry>();
let uninstall: (() => void) | null = null;

/// Starts all registered saves. Resolves to true when all of them saved.
function flushAll(): Promise<boolean> {
  return Promise.all([...entries].map((e) => e.flush())).then((results) => results.every(Boolean));
}

function install(): () => void {
  const onHide = () => void flushAll().catch(() => {});
  window.addEventListener("pagehide", onHide);
  window.addEventListener("beforeunload", onHide);
  const win = getCurrentWindow();
  let stop: (() => unknown) | undefined;
  let gone = false;
  let closing = false;
  const close = async () => {
    log("info", "close requested");
    const limit = Math.min(...[...entries].map((e) => e.limit));
    const saved = await within(flushAll(), limit);
    if (saved.kind === "timeout") log("warn", `the save did not complete in ${limit} ms; the window closes`);
    else if (saved.kind === "error") log("error", `the save failed: ${logText(saved.error)}`);
    else if (saved.value) log("info", "the save is complete");
    else log("error", "the save failed; the window closes with text that is not saved");
    // Remove the listener before the destroy. Tauri keeps the listener
    // record of a destroyed webview, and that record stops the close of
    // the next Settings window.
    const removed = await within(Promise.resolve(stop?.()), stepLimit);
    if (removed.kind !== "done") log("warn", `the close listener was not removed (${removed.kind})`);
    stop = undefined;
    log("info", "destroy called");
    const destroyed = await within(win.destroy(), stepLimit);
    if (destroyed.kind === "done") return;
    log(
      "error",
      destroyed.kind === "error"
        ? `destroy failed: ${logText(destroyed.error)}`
        : `destroy did not complete in ${stepLimit} ms`,
    );
    // The backend destroys the window with no window permission and no
    // close request.
    const closed = await within(commands.closeSettingsWindow(), stepLimit);
    if (closed.kind === "done" && closed.value.status === "ok") return;
    const why =
      closed.kind === "timeout"
        ? `no answer in ${stepLimit} ms`
        : closed.kind === "error"
          ? logText(closed.error)
          : closed.value.status === "error"
            ? closed.value.error
            : "";
    log("error", `the backend close failed: ${why}`);
  };
  win
    .onCloseRequested(async (event) => {
      event.preventDefault();
      // A second request while the window closes changes nothing.
      if (closing) return;
      closing = true;
      try {
        await close();
      } finally {
        closing = false;
      }
    })
    .then((f) => {
      // The handlers can be removed before the listener is registered.
      if (gone) f();
      else stop = f;
    })
    .catch((e: unknown) => log("error", `the close listener could not be registered: ${logText(e)}`));
  return () => {
    gone = true;
    stop?.();
    window.removeEventListener("pagehide", onHide);
    window.removeEventListener("beforeunload", onHide);
  };
}

/// Registers `flush` for the window close and the page events. A window
/// close waits at most the smallest `limit` of the registered saves. Returns
/// the function that removes the registration.
export function registerCloseSave(flush: CloseSave, limit = defaultCloseSaveLimit): () => void {
  const entry: Entry = { flush, limit };
  entries.add(entry);
  uninstall ??= install();
  return () => {
    if (!entries.delete(entry)) return;
    if (entries.size === 0) {
      uninstall?.();
      uninstall = null;
    }
  };
}
