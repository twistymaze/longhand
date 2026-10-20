import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { TaskCheckbox } from "./task-checkbox";

describe("TaskCheckbox", () => {
  it("calls onToggle when interactive", () => {
    const onToggle = vi.fn();

    render(<TaskCheckbox checked={false} isInteractive onToggle={onToggle} />);

    fireEvent.click(screen.getByRole("checkbox"));

    expect(onToggle).toHaveBeenCalledTimes(1);
  });

  it("renders as checked only when checked", () => {
    const done = render(<TaskCheckbox checked />);

    expect(
      (done.container.querySelector("input") as HTMLInputElement).checked,
    ).toBe(true);
    done.unmount();

    const open = render(<TaskCheckbox checked={false} />);

    expect(
      (open.container.querySelector("input") as HTMLInputElement).checked,
    ).toBe(false);
  });

  it("does not call onToggle when read-only", () => {
    const onToggle = vi.fn();
    const view = render(<TaskCheckbox checked onToggle={onToggle} />);

    const checkbox = view.container.querySelector(
      'input[type="checkbox"]',
    ) as HTMLInputElement | null;

    expect(checkbox).not.toBeNull();
    if (!checkbox) {
      return;
    }

    fireEvent.click(checkbox);

    expect(checkbox.getAttribute("data-interactive")).toBe("false");
    expect(onToggle).not.toHaveBeenCalled();
  });
});
