import { expect, test } from "bun:test";
import { mockTauri } from "../test/tauri";
import { forwardErrorsToLog, logText } from "./log";

const lines = (tauri: ReturnType<typeof mockTauri>) =>
  tauri.calls.filter((c) => c.cmd === "log_from_webview").map((c) => `${c.args.level}: ${c.args.message}`);

test("console errors, uncaught errors and unhandled rejections go to the app log, and the console still gets them", async () => {
  const tauri = mockTauri({ log_from_webview: () => null });
  const printed: unknown[][] = [];
  const original = console.error;
  console.error = (...args: unknown[]) => void printed.push(args);
  try {
    forwardErrorsToLog();
    // A second call does not forward each line twice.
    forwardErrorsToLog();
    console.error("The save failed:", new Error("disk full"), { id: "c1" });
    window.dispatchEvent(new ErrorEvent("error", { error: new TypeError("x is undefined"), message: "x" }));
    const rejection = new Event("unhandledrejection") as Event & { reason: unknown };
    rejection.reason = "no answer";
    window.dispatchEvent(rejection);
    await new Promise((r) => setTimeout(r, 10));
  } finally {
    console.error = original;
  }
  expect(printed).toHaveLength(1);
  expect(printed[0][0]).toBe("The save failed:");
  const sent = lines(tauri);
  expect(sent).toHaveLength(3);
  expect(sent[0]).toStartWith('error: The save failed: Error: disk full\n');
  expect(sent[0]).toEndWith(' {"id":"c1"}');
  expect(sent[1]).toStartWith("error: Uncaught error: TypeError: x is undefined");
  expect(sent[2]).toBe("error: Unhandled rejection: no answer");
});

test("log text of a value that JSON cannot write is its string form", () => {
  const cycle: Record<string, unknown> = {};
  cycle.self = cycle;
  expect(logText(cycle)).toBe("[object Object]");
  expect(logText(undefined)).toBe("undefined");
});
