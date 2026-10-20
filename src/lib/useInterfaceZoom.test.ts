import { expect, test } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { mockTauri } from "../test/tauri";
import { zoomAction, useInterfaceZoom } from "./useInterfaceZoom";

test("platform zoom keys reject unrelated modifiers and composition", () => {
  const key = (value: string, extra = {}) => new KeyboardEvent("keydown", { key: value, metaKey: true, ...extra });
  expect(zoomAction(key("+", { shiftKey: true }), true)).toBe("increase");
  expect(zoomAction(key("="), true)).toBe("increase");
  expect(zoomAction(key("-"), true)).toBe("decrease");
  expect(zoomAction(key("0"), true)).toBe("reset");
  for (const extra of [{ altKey: true }, { ctrlKey: true }, { isComposing: true }, { shiftKey: true }]) {
    expect(zoomAction(key("-", extra), true)).toBe(null);
  }
  expect(zoomAction(key("=", { metaKey: false, ctrlKey: true }), false)).toBe("increase");
  expect(zoomAction(key("="), false)).toBe(null);
});

test("capture consumes editor keys but leaves shortcut recording exclusive", async () => {
  const tauri = mockTauri({ change_interface_zoom: () => 110 });
  const hook = renderHook(() => useInterfaceZoom({ isMac: false }));
  const editor = document.createElement("textarea");
  document.body.append(editor);
  let editorKeys = 0;
  editor.addEventListener("keydown", () => editorKeys++);
  const press = () => {
    const event = new KeyboardEvent("keydown", { key: "=", ctrlKey: true, bubbles: true, cancelable: true });
    editor.dispatchEvent(event);
    return event;
  };
  await act(async () => { expect(press().defaultPrevented).toBe(true); });
  expect(editorKeys).toBe(0);
  expect(tauri.calls.length).toBe(1);
  editor.dataset.shortcutCapture = "true";
  await act(async () => { expect(press().defaultPrevented).toBe(false); });
  expect(editorKeys).toBe(1);
  expect(tauri.calls.length).toBe(1);
  hook.unmount();
  editor.remove();
});

test("a window without interface zoom leaves the zoom keys alone", async () => {
  const tauri = mockTauri({ change_interface_zoom: () => 110 });
  const hook = renderHook(() => useInterfaceZoom({ isMac: false, enabled: false }));
  const event = new KeyboardEvent("keydown", { key: "=", ctrlKey: true, cancelable: true });
  await act(async () => void window.dispatchEvent(event));
  expect(event.defaultPrevented).toBe(false);
  expect(tauri.calls.length).toBe(0);
  hook.unmount();
});

test("keys stay ordered while a prior command is pending and errors remain visible", async () => {
  let finish!: (value: number) => void;
  let count = 0;
  const tauri = mockTauri({ change_interface_zoom: () => {
    count++;
    if (count === 1) return new Promise<number>((resolve) => { finish = resolve; });
    throw "Could not save";
  } });
  const hook = renderHook(() => useInterfaceZoom({ isMac: false }));
  const press = (key: string) => window.dispatchEvent(new KeyboardEvent("keydown", { key, ctrlKey: true, cancelable: true }));
  await act(async () => { press("="); press("0"); });
  expect(tauri.calls.length).toBe(1);
  await act(async () => { finish(110); });
  expect(tauri.calls.map((c) => c.args.action)).toEqual(["increase", "reset"]);
  expect(hook.result.current.error).toBe("Could not save");
  act(() => hook.result.current.dismiss());
  expect(hook.result.current.error).toBe(null);
  hook.unmount();
});
