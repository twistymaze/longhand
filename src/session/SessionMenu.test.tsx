import { describe, expect, test } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { Menu, type MenuEntry, SessionMenu, separator } from "./SessionMenu";

const item = (label: string) => ({ label, onSelect: () => {} });

describe("Menu", () => {
  test("a separator shows as a separator and is not focusable", () => {
    render(<Menu items={[item("One"), separator("a"), item("Two")]} onClose={() => {}} />);
    const line = screen.getByRole("separator");
    expect(line.querySelector("button")).toBeNull();
    expect(line.tabIndex).toBe(-1);
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "One" }));
  });

  test("the arrow keys skip a separator", () => {
    render(<Menu items={[item("One"), separator("a"), item("Two")]} onClose={() => {}} />);
    const menu = screen.getByRole("menu");
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "Two" }));
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "One" }));
  });

  test("a menu with no submenu has a height limit and scrolls", () => {
    const items: MenuEntry[] = Array.from({ length: 40 }, (_, i) => item(`Type ${i}`));
    render(<Menu items={items} onClose={() => {}} />);
    const menu = screen.getByRole("menu");
    expect(menu.className).toContain("overflow-y-auto");
    expect(menu.className).toMatch(/max-h-/);
    fireEvent.keyDown(menu, { key: "ArrowUp" });
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "Type 39" }));
  });

  test("a menu with a submenu does not scroll, so the submenu is not cut off", () => {
    render(<Menu items={[item("One"), { ...item("More"), submenu: [item("Inner")] }]} onClose={() => {}} />);
    expect(screen.getByRole("menu").className).not.toContain("overflow-y-auto");
  });

  test("an item next to a separator opens its submenu with the right arrow", () => {
    render(<Menu items={[separator("a"), { ...item("More"), submenu: [item("Inner")] }]} onClose={() => {}} />);
    fireEvent.keyDown(screen.getByRole("menu"), { key: "ArrowRight" });
    expect(screen.getByRole("menuitem", { name: "Inner" })).toBeTruthy();
  });
});

describe("Menu in a floating layer", () => {
  test("without the option, the menu is under its button in the same parent", () => {
    render(
      <div data-testid="parent" className="relative">
        <Menu items={[item("One")]} onClose={() => {}} />
      </div>,
    );
    const menu = screen.getByRole("menu");
    expect(menu.parentElement).toBe(screen.getByTestId("parent"));
    expect(menu.className).toContain("absolute");
    expect(menu.className).toContain("top-full");
  });

  test("a floating menu is outside a parent that clips, and is fixed", () => {
    render(
      <div data-testid="clip" className="overflow-hidden">
        <SessionMenu floating label="Actions for Pathfinder" items={[item("Rename…")]} />
      </div>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Actions for Pathfinder" }));
    const menu = screen.getByRole("menu");
    expect(screen.getByTestId("clip").contains(menu)).toBe(false);
    expect(menu.className).toContain("fixed");
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "Rename…" }));
  });

  test("a press outside closes a floating menu, and a press in it does not", () => {
    render(
      <div>
        <p>Outside</p>
        <SessionMenu floating label="Actions" items={[item("One")]} />
      </div>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Actions" }));
    fireEvent.pointerDown(screen.getByRole("menuitem", { name: "One" }));
    expect(screen.queryByRole("menu")).not.toBeNull();
    fireEvent.pointerDown(screen.getByText("Outside"));
    expect(screen.queryByRole("menu")).toBeNull();
  });

  test("a click on the button of an open floating menu closes it", () => {
    render(<SessionMenu floating label="Actions" items={[item("One")]} />);
    const button = screen.getByRole("button", { name: "Actions" });
    fireEvent.click(button);
    expect(screen.queryByRole("menu")).not.toBeNull();
    fireEvent.pointerDown(button);
    fireEvent.click(button);
    expect(screen.queryByRole("menu")).toBeNull();
    expect(button.getAttribute("aria-expanded")).toBe("false");
  });

  test("Escape closes a floating menu and gives the focus to its button", () => {
    render(<SessionMenu floating label="Actions" items={[item("One")]} />);
    const button = screen.getByRole("button", { name: "Actions" });
    fireEvent.click(button);
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(button);
  });

  test("Tab in a floating menu closes it and gives the focus to its button", () => {
    render(<SessionMenu floating label="Actions" items={[item("One")]} />);
    const button = screen.getByRole("button", { name: "Actions" });
    fireEvent.click(button);
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Tab" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(button);
  });
});
