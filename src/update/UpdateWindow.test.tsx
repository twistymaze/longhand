import { describe, expect, spyOn, test } from "bun:test";
import { mockWindows } from "@tauri-apps/api/mocks";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { UpdateState } from "../bindings";
import { mockTauri } from "../test/tauri";
import { UpdateWindow } from "./UpdateWindow";

const CURRENT = "1.0.3";
const NEW = "1.0.4";

function open(state: UpdateState, withKey = false) {
  const tauri = mockTauri({ update_state: () => ({ state, with_key: withKey }), choose_update: () => null });
  render(<UpdateWindow />);
  return tauri;
}

const buttons = () => screen.getAllByRole("button").map((b) => b.textContent);
const choices = (tauri: ReturnType<typeof mockTauri>) =>
  tauri.calls.filter((c) => c.cmd === "choose_update").map((c) => c.args.choice);

describe("the update window", () => {
  test("the offer names both versions, with Remind Me Later and Install and Relaunch", async () => {
    open({ state: "offered", version: NEW, current: CURRENT });
    expect(await screen.findByText(`Longhand ${NEW} is available. You have ${CURRENT}.`)).toBeTruthy();
    expect(buttons()).toEqual(["Remind Me Later", "Install and Relaunch"]);
  });

  test("Install and Relaunch sends the choice, and the state event shows the progress", async () => {
    const tauri = open({ state: "offered", version: NEW, current: CURRENT });
    await userEvent.setup().click(await screen.findByRole("button", { name: "Install and Relaunch" }));
    expect(choices(tauri)).toEqual(["install"]);
    await act(() =>
      tauri.emit("update-state-changed", {
        state: { state: "downloading", version: NEW, downloaded: 3_000_000, total: 12_000_000 },
        with_key: false,
      }),
    );
    expect(await screen.findByText(`Downloading Longhand ${NEW}…`)).toBeTruthy();
    expect(screen.getByText("3 MB of 12 MB")).toBeTruthy();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("25");
  });

  test("the ready view offers Relaunch Now and Later", async () => {
    open({ state: "ready", version: NEW });
    expect(await screen.findByText(`Longhand ${NEW} is ready`)).toBeTruthy();
    expect(buttons()).toEqual(["Later", "Relaunch Now"]);
  });

  test("the failed view shows the error and Try Again", async () => {
    const tauri = open({ state: "failed", version: NEW, error: "the signature does not match the download" });
    expect(
      await screen.findByText("The signature does not match the download. Your installed version did not change."),
    ).toBeTruthy();
    await userEvent.setup().click(screen.getByRole("button", { name: "Try Again" }));
    expect(choices(tauri)).toEqual(["try_again"]);
  });

  test("a manual check says up to date, or that local builds get no updates", async () => {
    open({ state: "up_to_date", current: CURRENT });
    expect(await screen.findByText("Longhand is up to date")).toBeTruthy();
    expect(screen.getByText(`You have the newest version, ${CURRENT}.`)).toBeTruthy();
  });

  test("a local build says that it gets no updates", async () => {
    open({ state: "local_build" });
    expect(await screen.findByText("This build gets no updates")).toBeTruthy();
    expect(screen.getByText("Local builds do not update. To get updates, install the latest release.")).toBeTruthy();
    expect(buttons()).toEqual(["OK"]);
  });

  // The window can open while the user types in another app. A key press
  // that was meant for that app must not choose.
  test("no button has the focus when the window opens", async () => {
    open({ state: "offered", version: NEW, current: CURRENT });
    await screen.findByRole("button", { name: "Install and Relaunch" });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(document.activeElement?.tagName).not.toBe("BUTTON");
  });

  test("Return, Space and Escape send no choice", async () => {
    const tauri = open({ state: "offered", version: NEW, current: CURRENT });
    await screen.findByRole("button", { name: "Install and Relaunch" });
    await userEvent.setup().keyboard("{Enter} {Escape}");
    expect(choices(tauri)).toEqual([]);
  });

  test("a key press on a button that has the focus sends no choice", async () => {
    const tauri = open({ state: "offered", version: NEW, current: CURRENT });
    const install = await screen.findByRole("button", { name: "Install and Relaunch" });
    install.focus();
    await userEvent.setup().keyboard("{Enter} ");
    expect(choices(tauri)).toEqual([]);
  });

  // The user opened the window from the tray or "Check for Updates…", so
  // the keyboard belongs to it.
  test("a window that the user asked for gives the default button the focus, and Return chooses it", async () => {
    const tauri = open({ state: "offered", version: NEW, current: CURRENT }, true);
    const install = await screen.findByRole("button", { name: "Install and Relaunch" });
    await waitFor(() => expect(document.activeElement).toBe(install));
    await userEvent.setup().keyboard("{Enter}");
    expect(choices(tauri)).toEqual(["install"]);
  });

  test("a request for a window that shows already gives the default button the focus", async () => {
    const state: UpdateState = { state: "offered", version: NEW, current: CURRENT };
    const tauri = open(state);
    const install = await screen.findByRole("button", { name: "Install and Relaunch" });
    await act(() => tauri.emit("update-state-changed", { state, with_key: true }));
    await waitFor(() => expect(document.activeElement).toBe(install));
    await userEvent.setup().keyboard("{Enter}");
    expect(choices(tauri)).toEqual(["install"]);
  });
});

describe("the update window size", () => {
  // The visible page is the window's viewport. The test sets it directly.
  function setViewport(width: number, height: number) {
    Object.defineProperty(window, "innerWidth", { value: width, configurable: true });
    Object.defineProperty(window, "innerHeight", { value: height, configurable: true });
  }

  // The window opens hidden. It shows only after it has the content's size,
  // so it does not first show at the wrong size.
  test("a window that shows less than the content grows by the part that does not show, and then shows", async () => {
    mockWindows("update");
    const measure = spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(
      DOMRect.fromRect({ width: 480, height: 132.2 }),
    );
    const viewport = [window.innerWidth, window.innerHeight] as const;
    setViewport(480, 200);
    try {
      const tauri = mockTauri({
        update_state: () => ({ state: { state: "up_to_date", current: CURRENT }, with_key: false }),
        "plugin:window|set_size": () => null,
        show_update_window: () => null,
      });
      const sizes = () =>
        tauri.calls.filter((c) => c.cmd === "plugin:window|set_size").map((c) => JSON.stringify(c.args));
      const shows = () => tauri.calls.filter((c) => c.cmd === "show_update_window").length;
      render(<UpdateWindow />);
      await screen.findByText("Longhand is up to date");
      await waitFor(() => expect(sizes().slice(-1)[0]).toContain('"width":480,"height":133'));

      // The title bar takes 32 points of the requested height.
      setViewport(480, 101);
      act(() => void window.dispatchEvent(new Event("resize")));
      await waitFor(() => expect(sizes().slice(-1)[0]).toContain('"width":480,"height":165'));
      expect(shows()).toBe(0);

      // All of the content shows, so the window keeps its size and shows.
      const count = sizes().length;
      setViewport(480, 133);
      act(() => void window.dispatchEvent(new Event("resize")));
      expect(sizes().length).toBe(count);
      await waitFor(() => expect(shows()).toBe(1));

      // A later resize does not show the window again.
      act(() => void window.dispatchEvent(new Event("resize")));
      expect(shows()).toBe(1);
    } finally {
      measure.mockRestore();
      setViewport(...viewport);
    }
  });
});
