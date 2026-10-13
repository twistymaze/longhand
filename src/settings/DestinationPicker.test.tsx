import { describe, expect, test } from "bun:test";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { DestinationStatus, MoveStatus, Settings } from "../bindings";
import { type CommandHandler, mockTauri } from "../test/tauri";
import { DestinationPicker } from "./DestinationPicker";

const settings = {
  storage_folder: "/Users/me/Library/Application Support/Longhand/Notes",
  shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [], files_writable: true, damaged_file: null },
} as unknown as Settings;
const readOnly = {
  ...settings,
  shared: { ...settings.shared, availability: { kind: "read_only", cause: "folder_not_available" } },
} as Settings;
const ready: DestinationStatus = { kind: "ready", path: settings.storage_folder, free_bytes: null };
const missing: DestinationStatus = { kind: "missing", path: settings.storage_folder };
const moving: MoveStatus = {
  old_folder: settings.storage_folder,
  new_folder: "/Volumes/Disk/Notes",
  moved: 3,
  total: 10,
  state: "running",
  cause: null,
};

function picker(
  handlers: Record<string, CommandHandler> = {},
  options: { settings?: Settings; status?: DestinationStatus } = {},
) {
  const tauri = mockTauri({ move_status: () => null, ...handlers });
  const received: Settings[] = [];
  const view = render(
    <DestinationPicker
      settings={options.settings ?? settings}
      status={options.status ?? ready}
      onSettings={(s) => received.push(s)}
    />,
  );
  return { tauri, received, view, user: userEvent.setup() };
}

const change = () => screen.getByRole("button", { name: "Change…" }) as HTMLButtonElement;
const show = () => screen.getByRole("button", { name: /^Show in / }) as HTMLButtonElement;
const cmds = (tauri: { calls: { cmd: string }[] }) => tauri.calls.map((c) => c.cmd).filter((c) => c !== "move_status");

describe("DestinationPicker", () => {
  test("shows the one storage folder and no earlier folders", () => {
    const { view } = picker();
    expect(view.getByText("Notes")).toBeTruthy();
    expect(view.getByText(settings.storage_folder)).toBeTruthy();
    expect(view.queryByText("Earlier folders")).toBeNull();
    view.unmount();
  });

  test("Show in Finder opens the storage folder from Rust", async () => {
    const { tauri, user, view } = picker({ show_storage_folder: () => null });
    await user.click(show());
    expect(cmds(tauri)).toEqual(["show_storage_folder"]);
    view.unmount();
  });

  test("Change, then Cancel, calls no change command", async () => {
    const { tauri, user, received, view } = picker({ pick_storage_folder: () => "/Volumes/Disk/Notes" });
    await user.click(change());
    await user.click(await screen.findByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(cmds(tauri)).toEqual(["pick_storage_folder"]);
    expect(received).toEqual([]);
    view.unmount();
  });

  test("the question has the focus on Move Notes, and Escape cancels after a click in it", async () => {
    const { tauri, user, view } = picker({ pick_storage_folder: () => "/Volumes/Disk/Notes" });
    await user.click(change());
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("Move your notes to “Notes”?");
    await waitFor(() => expect(document.activeElement?.textContent).toBe("Move Notes"));
    await user.keyboard("{Tab}");
    expect(document.activeElement?.textContent).toBe("Cancel");
    await user.keyboard("{Shift>}{Tab}{/Shift}");
    expect(document.activeElement?.textContent).toBe("Move Notes");
    // WKWebView: a click does not move the focus to a button.
    (document.activeElement as HTMLElement).blur();
    await user.keyboard("{Tab}");
    expect(document.activeElement?.textContent).toBe("Move Notes");
    (document.activeElement as HTMLElement).blur();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(cmds(tauri)).toEqual(["pick_storage_folder"]);
    view.unmount();
  });

  test("Change, then Leave Them, changes the folder and keeps the notes where they are", async () => {
    const changed = { ...settings, storage_folder: "/Volumes/Disk/Notes" } as Settings;
    const { tauri, user, received, view } = picker({
      pick_storage_folder: () => "/Volumes/Disk/Notes",
      change_storage_folder: () => changed,
    });
    await user.click(change());
    await user.click(await screen.findByRole("button", { name: "Leave Them" }));
    await waitFor(() => expect(received).toEqual([changed]));
    expect(tauri.calls.find((c) => c.cmd === "change_storage_folder")?.args).toEqual({
      folder: "/Volumes/Disk/Notes",
      oldNotes: "leave",
    });
    view.unmount();
  });

  test("a closed folder dialog changes nothing", async () => {
    const { tauri, user, received, view } = picker({ pick_storage_folder: () => null });
    await user.click(change());
    await waitFor(() => expect(cmds(tauri)).toEqual(["pick_storage_folder"]));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(received).toEqual([]);
    view.unmount();
  });

  test("a refused folder shows its cause in the pane", async () => {
    const { user, view } = picker({
      pick_storage_folder: () => settings.storage_folder,
      change_storage_folder: () => {
        throw "This folder is already the storage folder.";
      },
    });
    await user.click(change());
    await user.click(await screen.findByRole("button", { name: "Move Notes" }));
    expect(await screen.findByText("This folder is already the storage folder.")).toBeTruthy();
    view.unmount();
  });

  test("Change, then Move, shows progress, and a pause shows the cause and Retry", async () => {
    let status: MoveStatus | null = null;
    const { tauri, user, view } = picker({
      pick_storage_folder: () => "/Volumes/Disk/Notes",
      change_storage_folder: () => {
        status = moving;
        return { ...settings, storage_folder: "/Volumes/Disk/Notes" };
      },
      move_status: () => status,
      retry_move: () => null,
    });
    await user.click(change());
    await user.click(await screen.findByRole("button", { name: "Move Notes" }));
    expect(tauri.calls.find((c) => c.cmd === "change_storage_folder")?.args.oldNotes).toBe("move");
    expect(await screen.findByText("3 of 10 notes moved")).toBeTruthy();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("30");
    expect(change().disabled).toBe(true);

    await act(() => tauri.emit("move-progress", { moved: 4, total: 10, state: "running" }));
    expect(await screen.findByText("4 of 10 notes moved")).toBeTruthy();

    await act(() => tauri.emit("move-paused", { cause: { kind: "disk_full", message: "The new folder's disk is full." } }));
    expect(await screen.findByText("The new folder's disk is full.")).toBeTruthy();
    expect(screen.getByText("Move paused")).toBeTruthy();
    expect(change().disabled).toBe(true);
    status = { ...moving, moved: 4, state: "running" };
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(tauri.calls.some((c) => c.cmd === "retry_move")).toBe(true);
    await waitFor(() => expect(screen.queryByText("Move paused")).toBeNull());
    expect(screen.getByText("4 of 10 notes moved")).toBeTruthy();
    view.unmount();
  });

  test("a finished Move shows the number of notes moved, and Change is available again", async () => {
    const { tauri, view } = picker({ move_status: () => moving });
    expect(await screen.findByText("3 of 10 notes moved")).toBeTruthy();
    await act(() => tauri.emit("move-progress", { moved: 10, total: 10, state: "finished" }));
    expect(await screen.findByText("Moved 10 notes to “Notes”.")).toBeTruthy();
    expect(screen.queryByRole("progressbar")).toBeNull();
    expect(change().disabled).toBe(false);
    view.unmount();
  });

  test("a Move that waits for a busy note says so", async () => {
    const { view } = picker({ move_status: () => ({ ...moving, state: "waiting" }) });
    expect(await screen.findByText(/waits for a note that is in use/)).toBeTruthy();
    view.unmount();
  });

  test("a paused Move at open shows its cause and Retry, and Change is not available", async () => {
    const { view } = picker({
      move_status: () => ({
        ...moving,
        state: "paused",
        cause: { kind: "folder_not_available", message: "The old folder is not available." },
      }),
    });
    expect(await screen.findByText("The old folder is not available.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
    expect(change().disabled).toBe(true);
    view.unmount();
  });

  test("a read-only state says that the folder is not available, and Show in Finder is off", () => {
    const { view } = picker({}, { settings: readOnly, status: missing });
    expect(screen.getByText("The storage folder is not available.")).toBeTruthy();
    expect(show().disabled).toBe(true);
    expect(change().disabled).toBe(false);
    view.unmount();
  });

  for (const [agent, label] of [
    ["Mozilla/5.0 (Macintosh; Mac OS X)", "Show in Finder"],
    ["Mozilla/5.0 (Windows NT 10.0)", "Show in Explorer"],
  ]) {
    test(`${label} on ${agent.includes("Mac") ? "macOS" : "Windows"}`, () => {
      Object.defineProperty(navigator, "userAgent", { value: agent, configurable: true });
      try {
        const { view } = picker();
        expect(show().textContent).toBe(label);
        view.unmount();
      } finally {
        delete (navigator as { userAgent?: string }).userAgent;
      }
    });
  }
});
