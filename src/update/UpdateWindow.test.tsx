import { describe, expect, spyOn, test } from "bun:test";
import { mockWindows } from "@tauri-apps/api/mocks";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { UpdateState } from "../bindings";
import { mockTauri } from "../test/tauri";
import { UpdateWindow, windowSize } from "./UpdateWindow";

const CURRENT = "0.1.0-preview.20260928143012";
const NEW = "0.1.0-preview.20260929080000";

function open(state: UpdateState) {
  const tauri = mockTauri({ update_state: () => state, choose_update: () => null });
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
    expect(buttons()).toEqual(["OK"]);
  });

  test("the default button has the focus, so Return activates it", async () => {
    const tauri = open({ state: "offered", version: NEW, current: CURRENT });
    const install = await screen.findByRole("button", { name: "Install and Relaunch" });
    await waitFor(() => expect(document.activeElement).toBe(install));
    await userEvent.setup().keyboard("{Enter}");
    expect(choices(tauri)).toEqual(["install"]);
  });

  test("Escape sends the left button's choice", async () => {
    const tauri = open({ state: "offered", version: NEW, current: CURRENT });
    await screen.findByRole("button", { name: "Install and Relaunch" });
    await userEvent.setup().keyboard("{Escape}");
    expect(choices(tauri)).toEqual(["remind_later"]);
  });
});

describe("the update window size", () => {
  test("the window fits the content at the interface zoom", async () => {
    mockWindows("update");
    const scaled = window as { __interfaceScale?: string };
    scaled.__interfaceScale = "1.5";
    const measure = spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(
      DOMRect.fromRect({ width: 480, height: 171 }),
    );
    try {
      const tauri = mockTauri({
        update_state: () => ({ state: "offered", version: NEW, current: CURRENT }),
        "plugin:window|set_size": () => null,
      });
      render(<UpdateWindow />);
      await screen.findByText("A new version of Longhand is available");
      await waitFor(() => {
        const sizes = tauri.calls.filter((c) => c.cmd === "plugin:window|set_size").map((c) => JSON.stringify(c.args));
        expect(sizes[sizes.length - 1]).toContain('"width":720,"height":257');
      });
    } finally {
      measure.mockRestore();
      delete scaled.__interfaceScale;
    }
  });

  test("one CSS pixel is the scale in logical pixels, rounded up", () => {
    const size = windowSize(480, 170.2, 1.3);
    expect([size.width, size.height]).toEqual([624, 222]);
  });
});
