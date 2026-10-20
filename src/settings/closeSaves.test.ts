import { afterEach, describe, expect, test } from "vitest";
import { mockWindows } from "@tauri-apps/api/mocks";
import { type TauriMock, mockTauri } from "../test/tauri";
import { registerCloseSave } from "./closeSaves";

const called = (tauri: TauriMock, cmd: string) => tauri.calls.filter((c) => c.cmd === cmd).length;
const logLines = (tauri: TauriMock) =>
  tauri.calls.filter((c) => c.cmd === "log_from_webview").map((c) => `${c.args.level}: ${c.args.message}`);
const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until(check: () => boolean, ms = 1000) {
  const end = Date.now() + ms;
  while (!check()) {
    if (Date.now() > end) throw new Error("the condition did not become true");
    await pause(5);
  }
}

/// A flush that completes when the test calls `finish`.
function heldFlush(result = true) {
  let finish: () => void = () => {};
  const done = new Promise<void>((r) => (finish = r));
  let calls = 0;
  const flush = async () => {
    calls++;
    await done;
    return result;
  };
  return { flush, finish: () => finish(), calls: () => calls };
}

const removals: (() => void)[] = [];
const register = (...args: Parameters<typeof registerCloseSave>) => {
  const remove = registerCloseSave(...args);
  removals.push(remove);
  return remove;
};

afterEach(() => {
  for (const remove of removals.splice(0)) remove();
});

function setup() {
  const tauri = mockTauri({ "plugin:window|destroy": () => null, log_from_webview: () => null });
  mockWindows("settings");
  return tauri;
}

describe("closeSaves", () => {
  test("a window close waits for all registered saves, then destroys the window", async () => {
    const tauri = setup();
    const a = heldFlush();
    const b = heldFlush();
    register(a.flush);
    register(b.flush);
    // Let the close listener register.
    await pause(10);

    await tauri.emit("tauri://close-requested");
    await until(() => a.calls() === 1 && b.calls() === 1);
    a.finish();
    await pause(20);
    expect(called(tauri, "plugin:window|destroy")).toBe(0);
    b.finish();
    await until(() => called(tauri, "plugin:window|destroy") === 1);
    expect(logLines(tauri)).toEqual(["info: close requested", "info: the save is complete", "info: destroy called"]);
  });

  test("a failed save does not stop the close", async () => {
    const tauri = setup();
    register(async () => true);
    register(async () => false);
    await pause(10);
    await tauri.emit("tauri://close-requested");
    await until(() => called(tauri, "plugin:window|destroy") === 1);
    expect(logLines(tauri)[1]).toBe("error: the save failed; the window closes with text that is not saved");
  });

  test("the close waits at most the smallest registered limit", async () => {
    const tauri = setup();
    register(() => new Promise<boolean>(() => {}), 50);
    register(async () => true);
    await pause(10);
    await tauri.emit("tauri://close-requested");
    await until(() => called(tauri, "plugin:window|destroy") === 1);
    expect(logLines(tauri)).toEqual([
      "info: close requested",
      "warn: the save did not complete in 50 ms; the window closes",
      "info: destroy called",
    ]);
  });

  test("pagehide and beforeunload start each registered save", () => {
    setup();
    let a = 0;
    let b = 0;
    register(async () => (a++, true));
    register(async () => (b++, true));
    window.dispatchEvent(new Event("pagehide"));
    window.dispatchEvent(new Event("beforeunload"));
    expect([a, b]).toEqual([2, 2]);
  });

  test("a removed save is not called, and with no save the page events do nothing", () => {
    setup();
    let a = 0;
    const remove = register(async () => (a++, true));
    remove();
    window.dispatchEvent(new Event("pagehide"));
    expect(a).toBe(0);
  });

  test("with no save registered, a close request reaches no listener", async () => {
    const tauri = setup();
    const remove = register(async () => true);
    await pause(10);
    remove();
    await pause(10);
    const warn = console.warn;
    console.warn = () => {};
    try {
      await tauri.emit("tauri://close-requested");
      await pause(20);
    } finally {
      console.warn = warn;
    }
    expect(called(tauri, "plugin:window|destroy")).toBe(0);
  });
});
