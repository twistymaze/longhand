import { describe, expect, test, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { ConfirmDialog } from "./ConfirmDialog";

function renderDialog(over: Partial<Parameters<typeof ConfirmDialog>[0]> = {}) {
  const onConfirm = vi.fn(() => {});
  const onCancel = vi.fn(() => {});
  render(
    <ConfirmDialog
      title="Delete note?"
      message="The note goes to the Trash."
      action="Delete Permanently"
      onConfirm={onConfirm}
      onCancel={onCancel}
      {...over}
    />,
  );
  return { onConfirm, onCancel };
}

describe("ConfirmDialog", () => {
  test("Cancel has the focus when the dialog opens", () => {
    renderDialog();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Cancel" }));
  });

  test("Escape cancels", () => {
    const { onCancel, onConfirm } = renderDialog();
    fireEvent.keyDown(screen.getByRole("alertdialog"), { key: "Escape" });
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  test("Tab stays inside the dialog", () => {
    renderDialog();
    const cancel = screen.getByRole("button", { name: "Cancel" });
    const confirm = screen.getByRole("button", { name: "Delete Permanently" });
    confirm.focus();
    fireEvent.keyDown(confirm, { key: "Tab" });
    expect(document.activeElement).toBe(cancel);
    fireEvent.keyDown(cancel, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(confirm);
  });

  test("Tab keeps the focus on Cancel while the action is busy", () => {
    renderDialog({ busy: true });
    const cancel = screen.getByRole("button", { name: "Cancel" });
    fireEvent.keyDown(cancel, { key: "Tab" });
    expect(document.activeElement).toBe(cancel);
  });

  test("Escape and Tab work when the focus is on the page body", () => {
    // WKWebView: a click does not move the focus to a button.
    const { onCancel } = renderDialog();
    (document.activeElement as HTMLElement).blur();
    fireEvent.keyDown(document.body, { key: "Tab" });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Cancel" }));
    (document.activeElement as HTMLElement).blur();
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  test("a second action sits between Cancel and the action, and Tab moves over the three", () => {
    const onSecondary = vi.fn(() => {});
    const { onConfirm, onCancel } = renderDialog({
      action: "Move Notes",
      destructive: false,
      secondary: { label: "Leave Them", onClick: onSecondary },
      focus: "confirm",
    });
    const buttons = screen.getAllByRole("button").map((b) => b.textContent);
    expect(buttons).toEqual(["Cancel", "Leave Them", "Move Notes"]);
    const confirm = screen.getByRole("button", { name: "Move Notes" });
    expect(document.activeElement).toBe(confirm);
    fireEvent.keyDown(confirm, { key: "Tab" });
    expect(document.activeElement?.textContent).toBe("Cancel");
    fireEvent.keyDown(document.activeElement!, { key: "Tab" });
    expect(document.activeElement?.textContent).toBe("Leave Them");
    fireEvent.keyDown(document.activeElement!, { key: "Tab", shiftKey: true });
    expect(document.activeElement?.textContent).toBe("Cancel");
    (document.activeElement as HTMLElement).blur();
    fireEvent.keyDown(document.body, { key: "Tab" });
    expect(document.activeElement).toBe(confirm);
    fireEvent.click(screen.getByRole("button", { name: "Leave Them" }));
    expect(onSecondary).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
    expect(onCancel).not.toHaveBeenCalled();
  });

  test("an action that is not destructive uses the accent colour", () => {
    renderDialog({ destructive: false, action: "Transcribe Again" });
    const confirm = screen.getByRole("button", { name: "Transcribe Again" });
    expect(confirm.className).toContain("bg-accent");
    expect(confirm.className).not.toContain("bg-negative");
  });

  test("the panel and the message are limited to the window", () => {
    renderDialog({ message: "x".repeat(400) });
    const panel = screen.getByRole("alertdialog");
    expect(panel.className).toContain("max-w-[calc(100vw-2rem)]");
    expect(panel.className).toContain("max-h-");
    const message = screen.getByText("x".repeat(400)).parentElement!;
    expect(message.className).toContain("overflow-y-auto");
    expect(message.className).toContain("[overflow-wrap:anywhere]");
    expect(message.className).toContain("grid-cols-[minmax(0,1fr)]");
  });
});
