import { describe, expect, mock, test } from "bun:test";
import { fireEvent, render, screen } from "@testing-library/react";
import { ConfirmDialog } from "./ConfirmDialog";

const button = (name: string) => screen.getByRole("button", { name }) as HTMLButtonElement;

/// The text of the elements that the aria-describedby of `el` names.
function description(el: HTMLElement): string {
  return (el.getAttribute("aria-describedby") ?? "")
    .split(/\s+/)
    .filter(Boolean)
    .map((id) => document.getElementById(id)?.textContent ?? "")
    .join(" ");
}

function renderOne(props: Partial<Parameters<typeof ConfirmDialog>[0]> = {}) {
  const onConfirm = mock(() => {});
  const onCancel = mock(() => {});
  render(
    <ConfirmDialog
      title="Delete this note?"
      message="The recording and the transcript are removed."
      action="Delete"
      onConfirm={onConfirm}
      onCancel={onCancel}
      {...props}
    />,
  );
  return { onConfirm, onCancel };
}

function renderTwo(
  secondary: Partial<NonNullable<Parameters<typeof ConfirmDialog>[0]["secondary"]>> = {},
  props: Partial<Parameters<typeof ConfirmDialog>[0]> = {},
) {
  const onSecondary = mock(() => {});
  const handlers = renderOne({
    title: "Delete models?",
    message: "You can download them again later.",
    action: "Delete all",
    secondary: { label: "Delete unused", onClick: onSecondary, ...secondary },
    ...props,
  });
  return { ...handlers, onSecondary };
}

describe("ConfirmDialog", () => {
  test("without a second action, it shows Cancel and one action, and Cancel has the focus", () => {
    const { onConfirm } = renderOne();
    const buttons = screen.getAllByRole("button");
    expect(buttons.map((b) => b.textContent)).toEqual(["Cancel", "Delete"]);
    expect(document.activeElement).toBe(button("Cancel"));
    expect(button("Delete").disabled).toBe(false);
    fireEvent.click(button("Delete"));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  test("the dialog has its title as its name and the message as its description", () => {
    renderOne();
    const dialog = screen.getByRole("alertdialog", { name: "Delete this note?" });
    expect(description(dialog)).toBe("The recording and the transcript are removed.");
  });

  test("with a second action, it shows between Cancel and the action, and each calls its own handler", () => {
    const { onConfirm, onSecondary, onCancel } = renderTwo();
    const buttons = screen.getAllByRole("button");
    expect(buttons.map((b) => b.textContent)).toEqual(["Cancel", "Delete unused", "Delete all"]);
    expect(document.activeElement).toBe(button("Cancel"));

    fireEvent.click(button("Delete unused"));
    expect(onSecondary).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();

    fireEvent.click(button("Delete all"));
    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(onSecondary).toHaveBeenCalledTimes(1);
    expect(onCancel).not.toHaveBeenCalled();
  });

  test("a disabled second action cannot be clicked, and its reason shows", () => {
    const { onSecondary } = renderTwo({ disabled: true, reason: "No model is unused." });
    const secondary = button("Delete unused");
    expect(secondary.disabled).toBe(true);
    fireEvent.click(secondary);
    expect(onSecondary).not.toHaveBeenCalled();
    expect(screen.getByText("No model is unused.")).toBeTruthy();
    expect(description(secondary)).toBe("No model is unused.");
    expect(button("Delete all").disabled).toBe(false);
  });

  test("a disabled primary action cannot be clicked, and its reason shows", () => {
    const { onConfirm, onSecondary } = renderTwo(
      {},
      { actionDisabled: true, actionReason: "A note is transcribing." },
    );
    const primary = button("Delete all");
    expect(primary.disabled).toBe(true);
    fireEvent.click(primary);
    expect(onConfirm).not.toHaveBeenCalled();
    expect(screen.getByText("A note is transcribing.")).toBeTruthy();
    expect(description(primary)).toBe("A note is transcribing.");
    expect(button("Delete unused").disabled).toBe(false);
    fireEvent.click(button("Delete unused"));
    expect(onSecondary).toHaveBeenCalledTimes(1);
  });

  test("a disabled primary action without a second action shows its reason", () => {
    const { onConfirm } = renderOne({ actionDisabled: true, actionReason: "A note is recording." });
    expect(button("Delete").disabled).toBe(true);
    fireEvent.click(button("Delete"));
    expect(onConfirm).not.toHaveBeenCalled();
    expect(description(button("Delete"))).toBe("A note is recording.");
  });

  test("a reason does not show while its action is on", () => {
    renderTwo({ reason: "No model is unused." }, { actionReason: "A note is transcribing." });
    expect(screen.queryByText("No model is unused.")).toBeNull();
    expect(screen.queryByText("A note is transcribing.")).toBeNull();
    expect(button("Delete unused").getAttribute("aria-describedby")).toBeNull();
    expect(button("Delete all").getAttribute("aria-describedby")).toBeNull();
  });

  test("busy turns off both actions, but not Cancel", () => {
    const { onConfirm, onSecondary, onCancel } = renderTwo({}, { busy: true });
    expect(button("Delete unused").disabled).toBe(true);
    expect(button("Delete all").disabled).toBe(true);
    fireEvent.click(button("Delete unused"));
    fireEvent.click(button("Delete all"));
    expect(onSecondary).not.toHaveBeenCalled();
    expect(onConfirm).not.toHaveBeenCalled();
    expect(button("Cancel").disabled).toBe(false);
    fireEvent.click(button("Cancel"));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  test("Escape cancels without a second action", () => {
    const { onCancel, onConfirm } = renderOne();
    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  test("Escape cancels with a second action", () => {
    const { onCancel, onConfirm, onSecondary } = renderTwo();
    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
    expect(onSecondary).not.toHaveBeenCalled();
  });

  test("Escape cancels while the focus is on the dialog, not on a button", () => {
    // WKWebView does not focus a clicked button, so the focus can be on the
    // dialog or the page body. A key event inside the dialog still cancels.
    const { onCancel } = renderTwo();
    fireEvent.keyDown(screen.getByRole("alertdialog"), { key: "Escape" });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});
