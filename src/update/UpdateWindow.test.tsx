import { readFileSync } from "node:fs";
import { describe, expect, test, vi } from "vitest";
import { mockWindows } from "@tauri-apps/api/mocks";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ChangelogEntry, UpdateState } from "../bindings";
import { PAGES_URL, prAddress } from "../changelog/months";
import { mockTauri } from "../test/tauri";
import { UpdateWindow } from "./UpdateWindow";

const CURRENT = "1.0.3";
const NEW = "1.0.4";

function open(state: UpdateState, withKey = false) {
  const tauri = mockTauri({ update_state: () => ({ state, with_key: withKey }), choose_update: () => null });
  render(<UpdateWindow />);
  return tauri;
}

// The window buttons. The links in the list of changes are buttons too, so
// the list is left out.
const buttons = () =>
  screen
    .getAllByRole("button")
    .filter((b) => !b.closest("section"))
    .map((b) => b.textContent);
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

  test("before the first bytes arrive, the bar is empty and shows no byte count", async () => {
    open({ state: "downloading", version: NEW, downloaded: 0, total: null });
    expect(await screen.findByText(`Downloading Longhand ${NEW}…`)).toBeTruthy();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("0");
    expect(screen.queryByText("0 bytes")).toBeNull();
  });

  test("a download with no total size shows the byte count and no percentage", async () => {
    open({ state: "downloading", version: NEW, downloaded: 3_000_000, total: null });
    expect(await screen.findByText("3 MB")).toBeTruthy();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBeNull();
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

  test("in a window that the user asked for, Space clicks the button that has the focus", async () => {
    const tauri = open({ state: "offered", version: NEW, current: CURRENT }, true);
    const install = await screen.findByRole("button", { name: "Install and Relaunch" });
    await waitFor(() => expect(document.activeElement).toBe(install));
    screen.getByRole("button", { name: "Remind Me Later" }).focus();
    await userEvent.setup().keyboard(" ");
    expect(choices(tauri)).toEqual(["remind_later"]);
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
    const measure = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(
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

describe("the changes in an offer (U6 of the monthly changelog plan)", () => {
  let next = 0;
  function change(line: string, pr: number | null = null, month = "2026-10"): ChangelogEntry {
    next += 1;
    return { id: `id-${next}`, time: next, month, group: "Fixes", rank: 1, line, pr };
  }

  const offer: UpdateState = { state: "offered", version: NEW, current: CURRENT };

  /// The offer, with `offered_changes` answered by `changes`.
  function openOffer(changes: () => unknown, withKey = false) {
    const tauri = mockTauri({
      update_state: () => ({ state: offer, with_key: withKey }),
      choose_update: () => null,
      offered_changes: changes,
      "plugin:opener|open_url": () => null,
    });
    render(<UpdateWindow />);
    return tauri;
  }

  const area = () => screen.findByRole("region", { name: `Changes in Longhand ${NEW}` });
  const opened = (tauri: ReturnType<typeof mockTauri>) =>
    tauri.calls.filter((c) => c.cmd === "plugin:opener|open_url").map((c) => c.args.url);

  test("while the request runs, the title, the text and both buttons show, and the area is empty", async () => {
    const tauri = openOffer(() => new Promise(() => {}));
    expect(await screen.findByText("A new version of Longhand is available")).toBeTruthy();
    expect(screen.getByText(`Longhand ${NEW} is available. You have ${CURRENT}.`)).toBeTruthy();
    expect(buttons()).toEqual(["Remind Me Later", "Install and Relaunch"]);
    expect((await area()).textContent).toBe("");
    await waitFor(() => expect(tauri.calls.find((c) => c.cmd === "offered_changes")?.args).toEqual({ version: NEW }));
  });

  // Covers AE5.
  test("changes that are not available give the full-changelog link, and Install and Relaunch still works", async () => {
    const tauri = openOffer(() => null);
    const link = await within(await area()).findByRole("button", { name: /Full changelog/ });
    await userEvent.setup().click(link);
    expect(opened(tauri)).toEqual([PAGES_URL]);
    await userEvent.setup().click(screen.getByRole("button", { name: "Install and Relaunch" }));
    expect(choices(tauri)).toEqual(["install"]);
  });

  test("a failed request gives the full-changelog link and no error", async () => {
    openOffer(() => {
      throw "no connection";
    });
    expect(await within(await area()).findByRole("button", { name: /Full changelog/ })).toBeTruthy();
    expect(screen.queryByText(/connection|could not|error/i)).toBeNull();
  });

  test("an empty list gives the full-changelog link and no error", async () => {
    openOffer(() => []);
    expect(await within(await area()).findByRole("button", { name: /Full changelog/ })).toBeTruthy();
    expect(within(await area()).queryAllByRole("listitem")).toHaveLength(0);
    expect(screen.queryByText(/could not|error/i)).toBeNull();
  });

  test("the changes show by month and heading", async () => {
    openOffer(() => [change("Fix the timer", 72), change("Fix the tray", null, "2026-09")]);
    await within(await area()).findByText("Fix the tray");
    expect(within(await area()).getAllByRole("heading", { level: 2 }).map((h) => h.textContent)).toEqual([
      "October 2026",
      "September 2026",
    ]);
    expect(within(await area()).getAllByRole("heading", { level: 3 }).map((h) => h.textContent)).toEqual(["Fixes", "Fixes"]);
    expect(within(await area()).queryByRole("button", { name: /Full changelog/ })).toBeNull();
  });

  test("a list of 60 lines scrolls in its area, and both buttons stay in the window", async () => {
    openOffer(() => Array.from({ length: 60 }, (_, n) => change(`Change ${n + 1}`)));
    await within(await area()).findByText("Change 60");
    expect(within(await area()).getAllByRole("listitem")).toHaveLength(60);
    expect((await area()).className).toContain("overflow-y-auto");
    // The area has a fixed height, so the list does not change the window size.
    expect((await area()).className).toMatch(/(^|\s)h-\[/);
    for (const name of ["Remind Me Later", "Install and Relaunch"]) {
      expect((await area()).contains(screen.getByRole("button", { name }))).toBe(false);
    }
  });

  test("an automatic offer gives no focus to a link or a button", async () => {
    openOffer(() => [change("Fix the timer", 72)]);
    await within(await area()).findByText("Fix the timer");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(document.activeElement?.tagName).not.toBe("BUTTON");
  });

  test("in an automatic offer, Return and Space on a PR line open nothing and choose nothing", async () => {
    const tauri = openOffer(() => [change("Fix the timer", 72)]);
    const line = await within(await area()).findByRole("button", { name: /Fix the timer/ });
    line.focus();
    await userEvent.setup().keyboard("{Enter} ");
    expect(choices(tauri)).toEqual([]);
    expect(opened(tauri)).toEqual([]);
  });

  test("a click on a PR line opens the PR page", async () => {
    const tauri = openOffer(() => [change("Fix the timer", 72)]);
    await userEvent.setup().click(await within(await area()).findByRole("button", { name: /Fix the timer/ }));
    expect(opened(tauri)).toEqual([prAddress(72)]);
  });

  test("the window opener allows the same pages as Recent changes", () => {
    const read = (name: string) =>
      JSON.parse(readFileSync(`${import.meta.dirname}/../../src-tauri/capabilities/${name}.json`, "utf8"));
    const opener = (capability: { permissions: unknown[] }) =>
      (
        capability.permissions.find(
          (p) => typeof p === "object" && (p as { identifier: string }).identifier === "opener:allow-open-url",
        ) as { allow: { url: string }[] }
      ).allow.map((a) => a.url);
    const update = read("update");
    expect(update.windows).toEqual(["update"]);
    expect(opener(update)).toEqual(opener(read("changelog")));
  });
});
