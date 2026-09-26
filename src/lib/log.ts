// Writes lines from the webview to the app log (Longhand.log). The webview
// console does not reach the log, so the errors of each window are sent to
// it with `forwardErrorsToLog`.

import { commands, type WebviewLogLevel } from "../bindings";

/// Write `message` to the app log. A failure writes nothing and throws
/// nothing: a log call must not stop the code that calls it.
export function log(level: WebviewLogLevel, message: string): void {
  try {
    void commands.logFromWebview(level, message).catch(() => {});
  } catch {
    // No Tauri IPC (a test, a plain browser): nothing to write to.
  }
}

/// One value as log text. An Error gives its name, message and stack.
export function logText(value: unknown): string {
  if (value instanceof Error) return value.stack ? `${value.name}: ${value.message}\n${value.stack}` : String(value);
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value) ?? String(value);
  } catch {
    return String(value);
  }
}

let forwarding = false;

/// Send each `console.error` call, each uncaught error and each unhandled
/// promise rejection of this window to the app log. The console output
/// stays as it was. Call it once, at the start of the page.
export function forwardErrorsToLog(): void {
  if (forwarding) return;
  forwarding = true;
  // Set while a line is sent, so that an error in the send does not send
  // again.
  let sending = false;
  const send = (message: string) => {
    if (sending) return;
    sending = true;
    try {
      log("error", message);
    } finally {
      sending = false;
    }
  };

  const consoleError = console.error.bind(console);
  console.error = (...args: unknown[]) => {
    consoleError(...args);
    send(args.map(logText).join(" "));
  };
  window.addEventListener("error", (event) => {
    send(`Uncaught error: ${logText(event.error ?? event.message)}`);
  });
  window.addEventListener("unhandledrejection", (event) => {
    send(`Unhandled rejection: ${logText(event.reason)}`);
  });
}
