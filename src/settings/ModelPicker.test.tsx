import { describe, expect, mock, test } from "bun:test";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ModelPicker } from "./ModelPicker";

// About 100 ids, as OpenAI lists, with some that contain "mini" in any case.
const many = [
  ...Array.from({ length: 95 }, (_, i) => `model-${String(i).padStart(3, "0")}`),
  "gpt-4o",
  "gpt-4o-mini",
  "o4-MINI",
  "text-embedding-mini",
  "whisper-1",
];

function setup(value: string, models: string[]) {
  const onCommit = mock((_model: string) => {});
  render(<ModelPicker value={value} models={models} onCommit={onCommit} />);
  const input = screen.getByRole("combobox") as HTMLInputElement;
  return { user: userEvent.setup(), input, onCommit };
}

const optionTexts = () => screen.getAllByRole("option").map((o) => o.textContent);

describe("ModelPicker", () => {
  test("typing filters the list by a substring in any case, with no other filter (AE6)", async () => {
    const { user, input } = setup("", many);
    await user.type(input, "mini");
    expect(optionTexts()).toEqual(["gpt-4o-mini", "o4-MINI", "text-embedding-mini", 'Use "mini"']);
  });

  test("the chevron opens the list, focuses the field, and closes the list again", async () => {
    const { user, input } = setup("", ["gpt-4o", "whisper-1"]);
    const chevron = screen.getByRole("button", { name: "Show models" });
    await user.click(chevron);
    expect(document.activeElement).toBe(input);
    expect(optionTexts()).toEqual(["gpt-4o", "whisper-1"]);
    await user.click(chevron);
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(document.activeElement).toBe(input);
  });

  test("Arrow Down then Enter commits the first match", async () => {
    const { user, input, onCommit } = setup("", many);
    await user.type(input, "mini");
    await user.keyboard("{ArrowDown}{Enter}");
    expect(onCommit.mock.calls).toEqual([["gpt-4o-mini"]]);
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  test("a name that matches no id is offered as a Use row, and Enter commits it", async () => {
    const { user, input, onCommit } = setup("", many);
    await user.type(input, "my-deployment");
    expect(optionTexts()).toEqual(['Use "my-deployment"']);
    await user.keyboard("{Enter}");
    expect(onCommit.mock.calls).toEqual([["my-deployment"]]);
  });

  test("a click on a row commits it", async () => {
    const { user, input, onCommit } = setup("", many);
    await user.type(input, "whisper");
    await user.click(screen.getByRole("option", { name: "whisper-1" }));
    expect(onCommit.mock.calls).toEqual([["whisper-1"]]);
  });

  test("an id that matches exactly shows no Use row", async () => {
    const { user, input } = setup("", many);
    await user.type(input, "gpt-4o");
    expect(optionTexts()).toEqual(["gpt-4o", "gpt-4o-mini"]);
  });

  test("leaving the field restores the saved model and commits nothing (AE7)", async () => {
    const { user, input, onCommit } = setup("gpt-4o", many);
    await user.clear(input);
    await user.type(input, "mini");
    await user.tab();
    expect(input.value).toBe("gpt-4o");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(onCommit).not.toHaveBeenCalled();
  });

  test("Escape closes the popup and restores the saved model", async () => {
    const { user, input, onCommit } = setup("gpt-4o", many);
    await user.clear(input);
    await user.type(input, "mini");
    expect(screen.getByRole("listbox")).toBeTruthy();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(input.value).toBe("gpt-4o");
    expect(onCommit).not.toHaveBeenCalled();
  });

  test("a saved model that the list does not contain stays, with a note (AE3)", () => {
    const { input } = setup("gpt-4o", ["gpt-4.1", "o3"]);
    expect(input.value).toBe("gpt-4o");
    expect(screen.getByText("This server doesn't list this model")).toBeTruthy();
  });

  test("a saved model that the list contains shows no note", () => {
    setup("gpt-4o", many);
    expect(screen.queryByText("This server doesn't list this model")).toBeNull();
  });

  test("with no saved model, the field is empty and shows the placeholder", () => {
    const { input } = setup("", many);
    expect(input.value).toBe("");
    expect(input.placeholder).toBe("Choose a model");
  });

  test("the input is an ARIA combobox that references the active row", async () => {
    const { user, input } = setup("", many);
    expect(input.getAttribute("aria-expanded")).toBe("false");
    await user.type(input, "mini");
    expect(input.getAttribute("aria-expanded")).toBe("true");
    const listbox = screen.getByRole("listbox");
    expect(input.getAttribute("aria-controls")).toBe(listbox.id);
    expect(input.getAttribute("aria-activedescendant")).toBeNull();
    await user.keyboard("{ArrowDown}{ArrowDown}");
    const active = screen.getByRole("option", { name: "o4-MINI" });
    expect(input.getAttribute("aria-activedescendant")).toBe(active.id);
  });
});
