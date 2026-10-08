import { describe, expect, test } from "bun:test";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import type { TagEntry } from "../bindings";
import { tagKey } from "../lib/Labels";
import { TagFilterMenu } from "./TagFilterMenu";

const entries = (...texts: string[]): TagEntry[] => texts.map((text) => ({ text, key: tagKey(text), count: 1 }));
const library = entries("Topic::Roadmap", "daily", "Person::Dana Ortiz", "Person::Alex");

let setTags: (tags: TagEntry[]) => void = () => {};

function Harness({ initial }: { initial: TagEntry[] }) {
  const [tags, setList] = useState(initial);
  const [chosen, setChosen] = useState<string[]>([]);
  setTags = setList;
  return (
    <>
      <TagFilterMenu
        tags={tags}
        chosen={chosen}
        onToggle={(key) => setChosen((c) => (c.includes(key) ? c.filter((k) => k !== key) : [...c, key]))}
      />
      <output aria-label="Chosen">{chosen.join(",")}</output>
      <button type="button">Outside</button>
    </>
  );
}

function setup(initial = library) {
  render(<Harness initial={initial} />);
  return { user: userEvent.setup(), button: screen.getByRole("button", { name: "Tags" }) };
}

const field = () => screen.getByRole("combobox", { name: "Filter Tags" }) as HTMLInputElement;
const shown = () => screen.queryAllByRole("option").map((o) => o.getAttribute("aria-label"));
const chosen = () => screen.getByRole("status", { name: "Chosen" }).textContent;

describe("TagFilterMenu", () => {
  // R10, R7.
  test("on open, the field has the focus and the tags are in the menu order", async () => {
    const { user, button } = setup();
    await user.click(button);
    expect(document.activeElement).toBe(field());
    expect(shown()).toEqual(["daily", "Person: Alex", "Person: Dana Ortiz", "Topic: Roadmap"]);
  });

  // AE5, R6.
  test("typing filters the tags by their whole text", async () => {
    const { user, button } = setup();
    await user.click(button);
    await user.keyboard("da");
    expect(shown()).toEqual(["daily", "Person: Dana Ortiz"]);
    await user.clear(field());
    await user.keyboard("person :: dana");
    expect(shown()).toEqual(["Person: Dana Ortiz"]);
  });

  // KTD4.
  test("with text that matches nothing, the field stays and the menu says so", async () => {
    const { user, button } = setup();
    await user.click(button);
    await user.keyboard("zzz");
    expect(shown()).toEqual([]);
    expect(screen.getByText("No matching tags")).toBeTruthy();
    expect(document.activeElement).toBe(field());
  });

  // AE6, R8, R9.
  test("a click chooses a tag, the menu stays open, and the chosen tags show a check mark", async () => {
    const { user, button } = setup();
    await user.click(button);
    await user.click(screen.getByRole("option", { name: "daily" }));
    await user.click(screen.getByRole("option", { name: "Person: Alex" }));
    expect(chosen()).toBe("daily,person::alex");
    expect(screen.getByRole("listbox", { name: "Tags" })).toBeTruthy();
    expect(screen.getByRole("option", { name: "daily" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("option", { name: "Person: Alex" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("option", { name: "Topic: Roadmap" }).getAttribute("aria-selected")).toBe("false");
    expect(document.activeElement).toBe(field());
  });

  // R10.
  test("the arrow keys move the active tag, and Return chooses or clears it", async () => {
    const { user, button } = setup();
    await user.click(button);
    await user.keyboard("{Enter}");
    expect(chosen()).toBe("");
    await user.keyboard("{ArrowDown}{ArrowDown}{Enter}");
    expect(chosen()).toBe("person::alex");
    await user.keyboard("{Enter}");
    expect(chosen()).toBe("");
  });

  // R10.
  test("ArrowUp with no active tag goes to the last tag, and the keys stop at each end", async () => {
    const { user, button } = setup();
    await user.click(button);
    await user.keyboard("{ArrowUp}{ArrowDown}{Enter}");
    expect(chosen()).toBe("topic::roadmap");
    await user.keyboard("{ArrowUp}{ArrowUp}{ArrowUp}{ArrowUp}{ArrowUp}{Enter}");
    expect(chosen()).toBe("topic::roadmap,daily");
  });

  // R10.
  test("typing clears the active tag, so Return does nothing until a key moves to a tag", async () => {
    const { user, button } = setup();
    await user.click(button);
    await user.keyboard("{ArrowDown}{ArrowDown}");
    await user.keyboard("dana");
    expect(field().getAttribute("aria-activedescendant")).toBeNull();
    await user.keyboard("{Enter}");
    expect(chosen()).toBe("");
  });

  test("Tab closes the menu and gives the focus to the Tags button", async () => {
    const { user, button } = setup();
    await user.click(button);
    await user.keyboard("{Tab}");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(document.activeElement).toBe(button);
  });

  test("a press on the field keeps its default, so it can move the caret", async () => {
    const { user, button } = setup();
    await user.click(button);
    const press = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    field().dispatchEvent(press);
    expect(press.defaultPrevented).toBe(false);
    const rowPress = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    screen.getByRole("option", { name: "daily" }).dispatchEvent(rowPress);
    expect(rowPress.defaultPrevented).toBe(true);
  });

  // R10, KTD4.
  test("Escape closes the menu, clears the text, and gives the focus to the Tags button", async () => {
    const { user, button } = setup();
    await user.click(button);
    await user.keyboard("da{Escape}");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(document.activeElement).toBe(button);
    await user.click(button);
    expect(field().value).toBe("");
  });

  test("a press on the Tags button of an open menu closes it", async () => {
    const { user, button } = setup();
    await user.click(button);
    await user.click(button);
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(button.getAttribute("aria-expanded")).toBe("false");
  });

  test("a press outside closes the menu", async () => {
    const { user, button } = setup();
    await user.click(button);
    await user.click(screen.getByRole("button", { name: "Outside" }));
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  test("when the tag list changes while the menu is open, the menu shows the new list and no tag is active", async () => {
    const { user, button } = setup();
    await user.click(button);
    await user.keyboard("{ArrowDown}");
    expect(field().getAttribute("aria-activedescendant")).toBeTruthy();
    act(() => setTags(entries("Person::Alex", "urgent")));
    expect(shown()).toEqual(["urgent", "Person: Alex"]);
    expect(field().getAttribute("aria-activedescendant")).toBeNull();
  });

  // R11.
  test("each tag has its full text as its hover text", async () => {
    const { user, button } = setup();
    await user.click(button);
    expect(screen.getByRole("option", { name: "Person: Dana Ortiz" }).getAttribute("title")).toBe("Person::Dana Ortiz");
  });
});
