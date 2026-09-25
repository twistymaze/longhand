import { describe, expect, test } from "bun:test";
import { act, fireEvent, render, screen } from "@testing-library/react";
import type { ShortcutStatus, ShortcutStatuses } from "../bindings";
import { mockTauri } from "../test/tauri";
import { ShortcutField } from "./ShortcutField";

const registered = (chord: string): ShortcutStatus => ({ chord, registered: true, error: null });
const off: ShortcutStatus = { chord: null, registered: false, error: null };

function renderField(
  status: ShortcutStatus,
  handlers: Record<string, (args: Record<string, unknown>) => unknown> = {},
) {
  const changed: ShortcutStatuses[] = [];
  const tauri = mockTauri({
    suspend_shortcuts: () => null,
    resume_shortcuts: () => null,
    set_shortcut: (args) => ({ recording: registered(args.chord as string), show_window: off }),
    ...handlers,
  });
  render(
    <ShortcutField
      kind="recording"
      label="Recording shortcut"
      status={status}
      resetTo="CommandOrControl+Alt+Shift+L"
      detail="At rest"
      onChanged={(s) => changed.push(s)}
    />,
  );
  return { tauri, changed };
}

const names = (tauri: ReturnType<typeof mockTauri>) => tauri.calls.map((c) => c.cmd);

async function capture() {
  const button = screen.getByRole("button", { name: /Recording shortcut/ });
  await act(async () => {
    fireEvent.click(button);
  });
  return button;
}

describe("ShortcutField", () => {
  test("pauses the shortcuts first, sends the chord, then resumes them", async () => {
    const { tauri, changed } = renderField(registered("CommandOrControl+Alt+Shift+L"));
    const button = await capture();
    expect(screen.getByText("Press the new shortcut")).toBeTruthy();
    await act(async () => {
      fireEvent.keyDown(button, { key: "r", code: "KeyR", metaKey: true, shiftKey: true });
    });
    expect(names(tauri)).toEqual(["suspend_shortcuts", "set_shortcut", "resume_shortcuts"]);
    expect(tauri.calls[1].args).toEqual({ kind: "recording", chord: "Shift+Super+KeyR" });
    expect(changed).toHaveLength(1);
  });

  test("a refused chord shows the reason and keeps the old chord", async () => {
    const { tauri, changed } = renderField(registered("CommandOrControl+Alt+Shift+L"), {
      set_shortcut: () => {
        throw "Another app uses these keys. Choose different keys.";
      },
    });
    const button = await capture();
    await act(async () => {
      fireEvent.keyDown(button, { key: " ", code: "Space", metaKey: true, shiftKey: true });
    });
    expect(screen.getByText("Another app uses these keys. Choose different keys.")).toBeTruthy();
    expect(changed).toHaveLength(0);
    expect(names(tauri)).toEqual(["suspend_shortcuts", "set_shortcut", "resume_shortcuts"]);
  });

  test("a modifier key alone waits for the rest of the chord", async () => {
    const { tauri } = renderField(registered("CommandOrControl+Alt+Shift+L"));
    const button = await capture();
    await act(async () => {
      fireEvent.keyDown(button, { key: "Shift", code: "ShiftLeft", shiftKey: true });
    });
    expect(names(tauri)).toEqual(["suspend_shortcuts"]);
    expect(screen.getByText("Press the new shortcut")).toBeTruthy();
  });

  test("Escape cancels with no change and resumes the shortcuts", async () => {
    const { tauri } = renderField(registered("CommandOrControl+Alt+Shift+L"));
    const button = await capture();
    await act(async () => {
      fireEvent.keyDown(button, { key: "Escape", code: "Escape" });
    });
    expect(names(tauri)).toEqual(["suspend_shortcuts", "resume_shortcuts"]);
    expect(screen.getByText("At rest")).toBeTruthy();
  });

  test("Clear sends no chord", async () => {
    const { tauri } = renderField(registered("Super+KeyR"));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    });
    expect(tauri.calls[tauri.calls.length - 1]).toEqual({ cmd: "set_shortcut", args: { kind: "recording", chord: null } });
  });

  test("an off shortcut shows Set Shortcut and Reset", () => {
    renderField(off);
    expect(screen.getByRole("button", { name: "Set Recording shortcut" }).textContent).toBe("Set Shortcut");
    expect(screen.getByRole("button", { name: "Reset" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Clear" })).toBeNull();
  });
});
