import { describe, expect, test, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TagPicker } from "./TagPicker";

const tags = ["Person::Nathan", "Person::Ana", "Project::Atlas", "urgent", "follow up"];

function setup(applied: string[] = []) {
  const onAdd = vi.fn((_tag: string) => {});
  const onOpen = vi.fn(() => {});
  render(<TagPicker tags={tags} applied={applied} onAdd={onAdd} onOpen={onOpen} />);
  const opener = screen.getByRole("button", { name: "Add Tag" });
  return { user: userEvent.setup(), opener, onAdd, onOpen };
}

const optionTexts = () => screen.getAllByRole("option").map((o) => o.getAttribute("aria-label"));
const input = () => screen.getByRole("combobox") as HTMLInputElement;

describe("TagPicker", () => {
  test("after a click on the opener, the input has the focus and the list asks for the tags", async () => {
    const { user, opener, onOpen } = setup();
    await user.click(opener);
    expect(document.activeElement).toBe(input());
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  test("the list leaves out the tags on the note", async () => {
    const { user, opener } = setup(["URGENT", "person :: ana"]);
    await user.click(opener);
    expect(optionTexts()).toEqual(["Person: Nathan", "Project: Atlas", "follow up"]);
  });

  test("typing suggests matches from any part of the text, in any case", async () => {
    const { user, opener } = setup();
    await user.click(opener);
    await user.keyboard("NA");
    expect(optionTexts()).toEqual(["Person: Nathan", "Person: Ana", 'Add "NA"']);
  });

  test("Enter on a typed new tag adds it, and the input stays open for another tag", async () => {
    const { user, opener, onAdd } = setup();
    await user.click(opener);
    await user.keyboard("Client :: Acme{Enter}");
    expect(onAdd.mock.calls).toEqual([["Client :: Acme"]]);
    expect(input().value).toBe("");
    expect(document.activeElement).toBe(input());
  });

  test("Enter on the exact text of a listed tag adds the listed spelling", async () => {
    const { user, opener, onAdd } = setup();
    await user.click(opener);
    await user.keyboard("project::atlas{Enter}");
    expect(onAdd.mock.calls).toEqual([["Project::Atlas"]]);
  });

  test("the arrow keys and Enter add a suggestion", async () => {
    const { user, opener, onAdd } = setup();
    await user.click(opener);
    await user.keyboard("person{ArrowDown}{ArrowDown}{Enter}");
    expect(onAdd.mock.calls).toEqual([["Person::Ana"]]);
  });

  test("a click on a suggestion adds it", async () => {
    const { user, opener, onAdd } = setup();
    await user.click(opener);
    await user.click(screen.getByRole("option", { name: "urgent" }));
    expect(onAdd.mock.calls).toEqual([["urgent"]]);
  });

  test("a tag that is on the note is not added again", async () => {
    const { user, opener, onAdd } = setup(["urgent"]);
    await user.click(opener);
    await user.keyboard(" Urgent {Enter}");
    expect(onAdd).not.toHaveBeenCalled();
    expect(screen.queryByRole("option", { name: 'Add "Urgent"' })).toBeNull();
  });

  test("Enter with no text does nothing", async () => {
    const { user, opener, onAdd } = setup();
    await user.click(opener);
    await user.keyboard("   {Enter}");
    expect(onAdd).not.toHaveBeenCalled();
  });

  test("Escape closes the picker and returns the focus to the opener", async () => {
    const { user, opener } = setup();
    await user.click(opener);
    await user.keyboard("abc{Escape}");
    expect(screen.queryByRole("combobox")).toBeNull();
    const again = screen.getByRole("button", { name: "Add Tag" });
    expect(document.activeElement).toBe(again);
  });

  test("Escape does not reach the page", async () => {
    const outer = vi.fn(() => {});
    const onAdd = vi.fn((_tag: string) => {});
    render(
      <div onKeyDown={(e) => e.key === "Escape" && outer()}>
        <TagPicker tags={tags} applied={[]} onAdd={onAdd} />
      </div>,
    );
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Add Tag" }));
    await user.keyboard("{Escape}");
    expect(outer).not.toHaveBeenCalled();
  });

  test("a move of the focus out of the input closes the picker without adding the text", async () => {
    const { user, opener, onAdd } = setup();
    await user.click(opener);
    await user.keyboard("draft");
    await user.click(document.body);
    expect(screen.queryByRole("combobox")).toBeNull();
    expect(onAdd).not.toHaveBeenCalled();
  });
});
