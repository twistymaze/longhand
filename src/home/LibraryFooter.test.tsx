import { describe, expect, test } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockTauri } from "../test/tauri";
import { LibraryFooter } from "./LibraryFooter";

const info = { version: "1.2.14", commit: "abc1234" };

function mock(offer: string | null = null) {
  return mockTauri({
    build_info: () => info,
    update_offer: () => offer,
    check_for_updates: () => null,
    open_update_offer: () => null,
    open_settings: () => null,
  });
}

const calls = (tauri: ReturnType<typeof mockTauri>, cmd: string) => tauri.calls.filter((c) => c.cmd === cmd);

describe("LibraryFooter", () => {
  // Covers AE1, R3, R4: the version opens a menu, and only the menu item
  // starts a check.
  test("the version opens a menu, and only its item checks for updates", async () => {
    const tauri = mock();
    render(<LibraryFooter />);
    const user = userEvent.setup();
    const version = await screen.findByRole("button", { name: "Version 1.2.14" });

    await user.click(version);
    const item = await screen.findByRole("menuitem", { name: "Check for Updates…" });
    expect(calls(tauri, "check_for_updates")).toHaveLength(0);

    await user.click(item);
    expect(calls(tauri, "check_for_updates")).toHaveLength(1);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  // Covers AE2, R5: a waiting update replaces the version with a badge,
  // which opens the update window.
  test("a waiting update shows a badge that opens the update window", async () => {
    const tauri = mock("1.2.15");
    render(<LibraryFooter />);
    const badge = await screen.findByRole("button", { name: "Update available: 1.2.15" });
    expect(screen.queryByRole("button", { name: "Version 1.2.14" })).toBeNull();

    await userEvent.setup().click(badge);
    expect(calls(tauri, "open_update_offer")).toHaveLength(1);
    expect(calls(tauri, "check_for_updates")).toHaveLength(0);
  });

  // Covers AE3, R6: the footer follows the tray item's offer both ways.
  test("the badge follows the offer as it comes and goes", async () => {
    const tauri = mock();
    render(<LibraryFooter />);
    await screen.findByRole("button", { name: "Version 1.2.14" });

    await act(() => tauri.emit("update-offer-changed", { version: "1.2.15" }));
    await screen.findByRole("button", { name: "Update available: 1.2.15" });

    await act(() => tauri.emit("update-offer-changed", { version: null }));
    await screen.findByRole("button", { name: "Version 1.2.14" });
    expect(screen.queryByRole("button", { name: /Update available/ })).toBeNull();
  });

  // Covers R2.
  test("Settings opens the Settings window", async () => {
    const tauri = mock();
    render(<LibraryFooter />);
    await userEvent.setup().click(await screen.findByRole("button", { name: "Settings" }));
    expect(calls(tauri, "open_settings").map((c) => c.args)).toEqual([{ pane: null }]);
  });

  // Covers R7: the keys of a menu button, and the focus after Escape.
  test.each(["{Enter}", "{ArrowDown}"])("%s opens the menu, and Escape gives the focus back", async (key) => {
    mock();
    render(<LibraryFooter />);
    const user = userEvent.setup();
    const version = await screen.findByRole("button", { name: "Version 1.2.14" });
    version.focus();

    await user.keyboard(key);
    await screen.findByRole("menuitem", { name: "Check for Updates…" });
    expect(version.getAttribute("aria-expanded")).toBe("true");

    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    expect(document.activeElement).toBe(version);
  });

  test("with no version, Settings still works", async () => {
    const tauri = mockTauri({
      build_info: () => {
        throw "no build info";
      },
      update_offer: () => null,
      open_settings: () => null,
    });
    render(<LibraryFooter />);
    await userEvent.setup().click(await screen.findByRole("button", { name: "Settings" }));
    expect(calls(tauri, "open_settings")).toHaveLength(1);
    expect(screen.queryByRole("button", { name: /Version/ })).toBeNull();
  });
});
