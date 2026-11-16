import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { mockWindows } from "@tauri-apps/api/mocks";
import { type TauriMock, mockTauri } from "../test/tauri";
import { defaultCloseSaveLimit, registerCloseSave } from "./closeSaves";

const called = (tauri: TauriMock, cmd: string) => tauri.calls.filter((c) => c.cmd === cmd).length;
const logLines = (tauri: TauriMock) =>
  tauri.calls.filter((c) => c.cmd === "log_from_webview").map((c) => `${c.args.level}: ${c.args.message}`);

/// Runs the queued microtasks, but no timer. The IPC mock and the close
/// steps use only microtasks, so each step that can complete is then
/// complete, and the count of microtasks does not depend on the speed of
/// the computer.
async function settle() {
  for (let i = 0; i < 100; i++) await Promise.resolve();
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

// All tests use the fake clock: no step waits for real time. The setup file
// sets the real clock again after each test.
beforeEach(() => {
  vi.useFakeTimers();
});

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
    await settle();

    await tauri.emit("tauri://close-requested");
    await settle();
    expect([a.calls(), b.calls()]).toEqual([1, 1]);
    a.finish();
    await settle();
    expect(called(tauri, "plugin:window|destroy")).toBe(0);
    b.finish();
    await settle();
    expect(called(tauri, "plugin:window|destroy")).toBe(1);
    expect(logLines(tauri)).toEqual(["info: close requested", "info: the save is complete", "info: destroy called"]);
  });

  test("a failed save does not stop the close", async () => {
    const tauri = setup();
    register(async () => true);
    register(async () => false);
    await settle();
    await tauri.emit("tauri://close-requested");
    await settle();
    expect(called(tauri, "plugin:window|destroy")).toBe(1);
    expect(logLines(tauri)[1]).toBe("error: the save failed; the window closes with text that is not saved");
  });

  test("the close waits at most the smallest registered limit", async () => {
    const tauri = setup();
    register(() => new Promise<boolean>(() => {}), 50);
    register(() => new Promise<boolean>(() => {}));
    await settle();
    await tauri.emit("tauri://close-requested");
    await settle();
    // The window stays open until the smallest limit ends.
    vi.advanceTimersByTime(49);
    await settle();
    expect(called(tauri, "plugin:window|destroy")).toBe(0);
    vi.advanceTimersByTime(1);
    await settle();
    expect(called(tauri, "plugin:window|destroy")).toBe(1);
    expect(logLines(tauri)).toEqual([
      "info: close requested",
      "warn: the save did not complete in 50 ms; the window closes",
      "info: destroy called",
    ]);
  });

  test("with the default limit, the close waits for the save until the limit ends", async () => {
    const tauri = setup();
    register(() => new Promise<boolean>(() => {}));
    await settle();
    await tauri.emit("tauri://close-requested");
    await settle();
    vi.advanceTimersByTime(defaultCloseSaveLimit - 1);
    await settle();
    expect(called(tauri, "plugin:window|destroy")).toBe(0);
    vi.advanceTimersByTime(1);
    await settle();
    expect(called(tauri, "plugin:window|destroy")).toBe(1);
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
    await settle();
    remove();
    await settle();
    const warn = console.warn;
    console.warn = () => {};
    try {
      await tauri.emit("tauri://close-requested");
      await settle();
      // Past all limits of the close steps.
      vi.advanceTimersByTime(10 * defaultCloseSaveLimit);
      await settle();
    } finally {
      console.warn = warn;
    }
    expect(called(tauri, "plugin:window|destroy")).toBe(0);
  });
});
