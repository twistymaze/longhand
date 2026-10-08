import { describe, expect, test } from "bun:test";
import { fireEvent, render, screen } from "@testing-library/react";
import { Menu, type MenuEntry, separator } from "./SessionMenu";

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
