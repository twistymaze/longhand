import { describe, expect, mock, test } from "bun:test";
import { fireEvent, render, screen } from "@testing-library/react";
import { ConfirmDialog } from "./ConfirmDialog";

function renderDialog(over: Partial<Parameters<typeof ConfirmDialog>[0]> = {}) {
  const onConfirm = mock(() => {});
  const onCancel = mock(() => {});
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
