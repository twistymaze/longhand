import { describe, expect, test } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Settings, WatchedFolderStatus } from "../bindings";
import { type CommandHandler, mockTauri } from "../test/tauri";
import { WatchedFolder } from "./WatchedFolder";

const RECORDINGS = "/Users/me/Library/Mobile Documents/com~apple~CloudDocs/Recordings";
const off = { watched_folder: null } as unknown as Settings;
const on = { watched_folder: RECORDINGS } as unknown as Settings;

function pane(
  handlers: Record<string, CommandHandler> = {},
  options: { settings?: Settings; status?: WatchedFolderStatus } = {},
) {
  const settings = options.settings ?? off;
  const status =
    options.status ?? (settings.watched_folder ? { kind: "watching", path: settings.watched_folder } : { kind: "off" });
  const tauri = mockTauri({
    watched_folder_status: () => status,
    pick_watched_folder: () => RECORDINGS,
    set_watched_folder: (args) => ({ watched_folder: args.folder }),
    ...handlers,
  });
  const received: Settings[] = [];
  const view = render(<WatchedFolder settings={settings} onSettings={(s) => received.push(s)} />);
  return { tauri, received, view, user: userEvent.setup() };
}

const cmds = (tauri: { calls: { cmd: string }[] }) =>
  tauri.calls.map((c) => c.cmd).filter((c) => c !== "watched_folder_status");
const setCalls = (tauri: { calls: { cmd: string; args: Record<string, unknown> }[] }) =>
  tauri.calls.filter((c) => c.cmd === "set_watched_folder").map((c) => c.args);

describe("WatchedFolder", () => {
  test("is off by default, and a folder with no new audio files is set with no question", async () => {
    const { tauri, received, user, view } = pane({ watched_folder_preview: () => ({ count: 0, bytes: 0 }) });
    expect(view.getByText("Off")).toBeTruthy();
    await user.click(view.getByRole("button", { name: "Choose Folder…" }));
    await waitFor(() => expect(received).toHaveLength(1));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(cmds(tauri)).toEqual(["pick_watched_folder", "watched_folder_preview", "set_watched_folder"]);
    expect(setCalls(tauri)).toEqual([{ folder: RECORDINGS, existing: "import" }]);
    view.unmount();
  });

  test("closing the folder dialog changes nothing", async () => {
    const { tauri, user, view } = pane({ pick_watched_folder: () => null });
    await user.click(view.getByRole("button", { name: "Choose Folder…" }));
    await waitFor(() => expect(cmds(tauri)).toEqual(["pick_watched_folder"]));
    view.unmount();
  });

  // R6, AE4: the question gives the count and the size, and the safe
  // answer has the focus.
  test("asks once about the audio files in the folder; Only New Files has the focus and skips them", async () => {
    const { tauri, user, view } = pane({ watched_folder_preview: () => ({ count: 40, bytes: 1_200_000_000 }) });
    await user.click(view.getByRole("button", { name: "Choose Folder…" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("Recordings");
    expect(dialog.textContent).toContain("40 audio files (1.2 GB)");
    await waitFor(() => expect(document.activeElement?.textContent).toBe("Only New Files"));
    await user.click(screen.getByRole("button", { name: "Only New Files" }));
    await waitFor(() => expect(setCalls(tauri)).toEqual([{ folder: RECORDINGS, existing: "skip" }]));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    view.unmount();
  });

  test("Import Them imports the files in the folder", async () => {
    const { tauri, user, view } = pane({ watched_folder_preview: () => ({ count: 1, bytes: 2_000_000 }) });
    await user.click(view.getByRole("button", { name: "Choose Folder…" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("1 audio file (2 MB)");
    await user.click(screen.getByRole("button", { name: "Import Them" }));
    await waitFor(() => expect(setCalls(tauri)).toEqual([{ folder: RECORDINGS, existing: "import" }]));
    view.unmount();
  });

  test("Cancel in the question leaves the setting as it was", async () => {
    const { tauri, received, user, view } = pane({ watched_folder_preview: () => ({ count: 3, bytes: 3000 }) });
    await user.click(view.getByRole("button", { name: "Choose Folder…" }));
    await user.click(await screen.findByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(setCalls(tauri)).toEqual([]);
    expect(received).toEqual([]);
    view.unmount();
  });

  // R9: a refused folder says why, and the setting stays.
  test("a refused folder shows the reason and changes nothing", async () => {
    const { tauri, user, view } = pane({
      watched_folder_preview: () => {
        throw "This folder is inside the storage folder. Choose a folder outside it.";
      },
    });
    await user.click(view.getByRole("button", { name: "Choose Folder…" }));
    expect(await view.findByText("This folder is inside the storage folder. Choose a folder outside it.")).toBeTruthy();
    expect(setCalls(tauri)).toEqual([]);
    expect(view.getByText("Off")).toBeTruthy();
    view.unmount();
  });

  test("while the folder is read, the pane says so and the buttons wait", async () => {
    let finish: (value: unknown) => void = () => {};
    const { user, view } = pane({
      watched_folder_preview: () => new Promise((resolve) => (finish = resolve)),
    });
    await user.click(view.getByRole("button", { name: "Choose Folder…" }));
    expect(await view.findByText("Reading the folder…")).toBeTruthy();
    expect((view.getByRole("button", { name: "Choose Folder…" }) as HTMLButtonElement).disabled).toBe(true);
    await act(async () => finish({ count: 0, bytes: 0 }));
    await waitFor(() => expect(view.queryByText("Reading the folder…")).toBeNull());
    view.unmount();
  });

  // R10: Turn Off stops new imports.
  test("a watched folder shows its path, and Turn Off turns it off", async () => {
    const { tauri, received, user, view } = pane({}, { settings: on });
    expect(view.getByText("Recordings")).toBeTruthy();
    expect(view.getByText(RECORDINGS)).toBeTruthy();
    // Normal watching shows no message.
    expect(view.queryByRole("status")).toBeNull();
    await user.click(view.getByRole("button", { name: "Turn Off" }));
    await waitFor(() => expect(received).toHaveLength(1));
    expect(setCalls(tauri)).toEqual([{ folder: null, existing: "import" }]);
    view.unmount();
  });

  test("Change… picks another folder", async () => {
    const other = "/Users/me/Dropbox/Memos";
    const { tauri, user, view } = pane(
      { pick_watched_folder: () => other, watched_folder_preview: () => ({ count: 0, bytes: 0 }) },
      { settings: on },
    );
    await user.click(view.getByRole("button", { name: "Change…" }));
    await waitFor(() => expect(setCalls(tauri)).toEqual([{ folder: other, existing: "import" }]));
    view.unmount();
  });

  test("a folder that is not available shows its state, and the event clears it", async () => {
    const { tauri, view } = pane({}, { settings: on, status: { kind: "unavailable", path: RECORDINGS } });
    expect(await view.findByText("Folder not available")).toBeTruthy();
    await act(() => tauri.emit("watched-folder-changed", { status: { kind: "watching", path: RECORDINGS } }));
    await waitFor(() => expect(view.queryByText("Folder not available")).toBeNull());
    await act(() => tauri.emit("watched-folder-changed", { status: { kind: "unavailable", path: RECORDINGS } }));
    expect(await view.findByText("Folder not available")).toBeTruthy();
    view.unmount();
  });
});
