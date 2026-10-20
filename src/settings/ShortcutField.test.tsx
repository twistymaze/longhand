import { describe, expect, test } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import type { ShortcutStatus, ShortcutStatuses } from "../bindings";
import { mockTauri } from "../test/tauri";
import { ShortcutField } from "./ShortcutField";

const registered = (chord: string): ShortcutStatus => ({ chord, registered: true, error: null });
const off: ShortcutStatus = { chord: null, registered: false, error: null };
const resumed: ShortcutStatuses = { recording: registered("CommandOrControl+Alt+Shift+L"), show_window: off, pause: off, widget: off };

function mockField(handlers: Record<string, (args: Record<string, unknown>) => unknown> = {}) {
  return mockTauri({
    suspend_shortcuts: () => null,
    resume_shortcuts: () => null,
    shortcut_status: () => resumed,
    set_shortcut: (args) => ({ recording: registered(args.chord as string), show_window: off }),
    ...handlers,
  });
}

function renderField(
  status: ShortcutStatus,
  handlers: Record<string, (args: Record<string, unknown>) => unknown> = {},
) {
  const changed: ShortcutStatuses[] = [];
  const tauri = mockField(handlers);
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

async function capture(name: RegExp = /Recording shortcut/) {
  const button = screen.getByRole("button", { name });
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
    expect(names(tauri)).toEqual(["suspend_shortcuts", "set_shortcut", "resume_shortcuts", "shortcut_status"]);
    expect(tauri.calls[1].args).toEqual({ kind: "recording", chord: "Shift+Super+KeyR" });
    // The pane gets the statuses read after the resume, not the paused ones.
    expect(changed).toEqual([resumed]);
  });

  test("a refused chord shows the reason and keeps the old chord", async () => {
    const { tauri } = renderField(registered("CommandOrControl+Alt+Shift+L"), {
      set_shortcut: () => {
        throw "Another app uses these keys. Choose different keys.";
      },
    });
    const button = await capture();
    await act(async () => {
      fireEvent.keyDown(button, { key: " ", code: "Space", metaKey: true, shiftKey: true });
    });
    expect(screen.getByText("Another app uses these keys. Choose different keys.")).toBeTruthy();
    expect(names(tauri)).toEqual(["suspend_shortcuts", "set_shortcut", "resume_shortcuts", "shortcut_status"]);
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
    expect(names(tauri)).toEqual(["suspend_shortcuts", "resume_shortcuts", "shortcut_status"]);
    expect(screen.getByText("At rest")).toBeTruthy();
  });

  test("a blur during the pause resumes once, after the pause", async () => {
    let unpause = () => {};
    const { tauri } = renderField(registered("CommandOrControl+Alt+Shift+L"), {
      suspend_shortcuts: () => new Promise<null>((r) => (unpause = () => r(null))),
    });
    const button = await capture();
    await act(async () => {
      fireEvent.blur(button);
    });
    expect(names(tauri)).toEqual(["suspend_shortcuts"]);
    await act(async () => {
      unpause();
    });
    expect(names(tauri)).toEqual(["suspend_shortcuts", "resume_shortcuts", "shortcut_status"]);
    expect(screen.getByText("At rest")).toBeTruthy();
  });

  test("the button takes focus, so the keys reach it", async () => {
    renderField(registered("CommandOrControl+Alt+Shift+L"));
    const button = await capture();
    expect(document.activeElement).toBe(button);
  });

  test("starting one row ends the other row's wait and keeps the shortcuts paused", async () => {
    const tauri = mockField();
    render(
      <>
        <ShortcutField kind="recording" label="Recording shortcut" status={off} detail="" onChanged={() => {}} />
        <ShortcutField kind="show_window" label="Show Longhand" status={off} detail="" onChanged={() => {}} />
      </>,
    );
    await capture(/Recording shortcut/);
    const second = await capture(/Show Longhand/);
    // Only the second row waits for keys.
    expect(screen.getAllByText("Press keys…")).toHaveLength(1);
    expect(second.textContent).toBe("Press keys…");
    expect(names(tauri)).not.toContain("resume_shortcuts");
    await act(async () => {
      fireEvent.keyDown(second, { key: "Escape", code: "Escape" });
    });
    expect(names(tauri).filter((n) => n.endsWith("_shortcuts"))).toEqual(["suspend_shortcuts", "resume_shortcuts"]);
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
