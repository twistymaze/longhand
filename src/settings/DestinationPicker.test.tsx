import { describe, expect, test } from "vitest";
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
  const tauri = mockTauri({
    move_status: () => null,
    agent_instructions_status: () => ({ kind: "not_added" }),
    watched_folder_status: () => ({ kind: "off" }),
    ...handlers,
  });
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

const RECORDINGS = "/Users/me/Recordings";
/// The result of a storage folder change that did nothing to the watched
/// folder.
const done = (s: Settings) => ({ settings: s, watched_folder_off: null, watched_folder_files: null });
const change = () => screen.getAllByRole("button", { name: "Change…" })[0] as HTMLButtonElement;
const show = () => screen.getByRole("button", { name: /^Show in / }) as HTMLButtonElement;
const cmds = (tauri: { calls: { cmd: string }[] }) => tauri.calls.map((c) => c.cmd).filter((c) => c !== "move_status" && c !== "agent_instructions_status" && c !== "watched_folder_status");

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
      change_storage_folder: () => done(changed),
    });
    await user.click(change());
    await user.click(await screen.findByRole("button", { name: "Leave Them" }));
    await waitFor(() => expect(received).toEqual([changed]));
    expect(tauri.calls.find((c) => c.cmd === "change_storage_folder")?.args).toEqual({
      folder: "/Volumes/Disk/Notes",
      oldNotes: "leave",
    });
    expect(screen.queryByRole("alertdialog")).toBeNull();
    view.unmount();
  });

  // R6 after Leave Them: the new folder's import record holds none of
  // the 5 audio files in the watched folder, so the question of the
  // watched folder shows, and the answer goes to the watched folder.
  test("Leave Them with 5 audio files in the watched folder asks about 5 files", async () => {
    const changed = { ...settings, storage_folder: "/Volumes/Disk/Notes", watched_folder: RECORDINGS } as Settings;
    const { tauri, user, received, view } = picker(
      {
        pick_storage_folder: () => "/Volumes/Disk/Notes",
        change_storage_folder: () => ({ ...done(changed), watched_folder_files: { count: 5, bytes: 5_000_000 } }),
        set_watched_folder: () => changed,
      },
      { settings: { ...settings, watched_folder: RECORDINGS } as Settings },
    );
    await user.click(change());
    await user.click(await screen.findByRole("button", { name: "Leave Them" }));
    const dialog = await screen.findByText(/This folder has 5 audio files \(5 MB\)/);
    expect(dialog).toBeTruthy();
    await waitFor(() => expect(document.activeElement?.textContent).toBe("Only New Files"));
    expect(cmds(tauri)).not.toContain("set_watched_folder");
    await user.click(screen.getByRole("button", { name: "Import Them" }));
    await waitFor(() =>
      expect(tauri.calls.find((c) => c.cmd === "set_watched_folder")?.args).toEqual({
        folder: RECORDINGS,
        existing: "import",
      }),
    );
    await waitFor(() => expect(received).toEqual([changed]));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    view.unmount();
  });

  // R9 at a change of the storage folder: the user must choose
  // another folder to watch, so the reason shows in the watched folder
  // group.
  test("a change that turns the watched folder off says why in the watched folder group", async () => {
    const changed = { ...settings, storage_folder: "/Users/me/Library", watched_folder: null } as Settings;
    const reason = "The watched folder is off because it is inside the new storage folder. Choose another folder to watch.";
    const { user, received, view } = picker(
      {
        pick_storage_folder: () => "/Users/me/Library",
        change_storage_folder: () => ({ ...done(changed), watched_folder_off: reason }),
      },
      { settings: { ...settings, watched_folder: RECORDINGS } as Settings },
    );
    await user.click(change());
    await user.click(await screen.findByRole("button", { name: "Leave Them" }));
    await waitFor(() => expect(received).toEqual([changed]));
    view.rerender(<DestinationPicker settings={changed} status={ready} onSettings={(s) => received.push(s)} />);
    const group = screen.getByRole("button", { name: "Choose Folder…" }).closest("section");
    expect(group?.textContent).toContain(reason);
    expect(screen.queryByRole("alertdialog")).toBeNull();
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
        return done({ ...settings, storage_folder: "/Volumes/Disk/Notes" } as Settings);
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

  test("a status read that ends after the Move finished does not show the Move again", async () => {
    let reply: (status: MoveStatus) => void = () => {};
    const { tauri, view } = picker({ move_status: () => new Promise<MoveStatus>((resolve) => (reply = resolve)) });
    await act(() => tauri.emit("move-progress", { moved: 10, total: 10, state: "finished" }));
    expect(await screen.findByText(/^Moved 10 notes/)).toBeTruthy();
    await act(async () => reply(moving));
    expect(screen.queryByText("3 of 10 notes moved")).toBeNull();
    expect(screen.queryByRole("progressbar")).toBeNull();
    expect(screen.getByText(/^Moved 10 notes/)).toBeTruthy();
    view.unmount();
  });

  test("a status read that ends after newer Move events gives only the folder", async () => {
    const replies: ((status: MoveStatus) => void)[] = [];
    const { tauri, view } = picker({ move_status: () => new Promise<MoveStatus>((resolve) => replies.push(resolve)) });
    await act(() => tauri.emit("move-progress", { moved: 4, total: 10, state: "running" }));
    await act(() => tauri.emit("move-progress", { moved: 5, total: 10, state: "running" }));
    expect(await screen.findByText("5 of 10 notes moved")).toBeTruthy();
    await act(async () => replies.forEach((reply) => reply(moving)));
    expect(await screen.findByText(/to “Notes”/)).toBeTruthy();
    expect(screen.getByText("5 of 10 notes moved")).toBeTruthy();
    view.unmount();
  });

  test("a status read that started before the last Move finished does not give its folder to the next Move", async () => {
    const replies: ((status: MoveStatus) => void)[] = [];
    const { tauri, view } = picker({ move_status: () => new Promise<MoveStatus>((resolve) => replies.push(resolve)) });
    const first = { ...moving, new_folder: "/Volumes/A/Alpha" };
    const next = { ...moving, new_folder: "/Volumes/B/Beta", moved: 1 };
    await act(() => tauri.emit("move-progress", { moved: 3, total: 10, state: "running" }));
    await act(() => tauri.emit("move-progress", { moved: 10, total: 10, state: "finished" }));
    await act(() => tauri.emit("move-progress", { moved: 1, total: 10, state: "running" }));
    expect(await screen.findByText("1 of 10 notes moved")).toBeTruthy();
    // The reads that started before the finish reply with the first Move.
    const early = replies.length - 1;
    await act(async () => replies.slice(0, early).forEach((reply) => reply(first)));
    expect(screen.queryByText(/Alpha/)).toBeNull();
    // A fresh read starts for the folder of the next Move.
    expect(replies.length).toBeGreaterThan(early + 1);
    await act(async () => replies.slice(early).forEach((reply) => reply(next)));
    expect(await screen.findByText(/to “Beta”/)).toBeTruthy();
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

  describe("agent instructions", () => {
    const addButton = () => screen.getByRole("button", { name: "Add Agent Instructions" }) as HTMLButtonElement;

    test("the button adds the instructions and shows Added", async () => {
      const { tauri, user, view } = picker({ add_agent_instructions: () => ({ kind: "added" }) });
      expect(await screen.findByText(/AGENTS\.md and CLAUDE\.md/)).toBeTruthy();
      await user.click(addButton());
      expect(cmds(tauri)).toEqual(["add_agent_instructions"]);
      expect(await screen.findByText("Added")).toBeTruthy();
      expect(screen.queryByRole("button", { name: "Add Agent Instructions" })).toBeNull();
      view.unmount();
    });

    test("instructions that are in the files already show Added and no button", async () => {
      const { view } = picker({ agent_instructions_status: () => ({ kind: "added" }) });
      expect(await screen.findByText("Added")).toBeTruthy();
      expect(screen.queryByRole("button", { name: "Add Agent Instructions" })).toBeNull();
      view.unmount();
    });

    test("damaged markers show an error that names the file, and the button is off", async () => {
      const { view } = picker({ agent_instructions_status: () => ({ kind: "damaged", file: "CLAUDE.md" }) });
      expect(await screen.findByText(/The agent instructions in CLAUDE\.md are damaged/)).toBeTruthy();
      expect(addButton().disabled).toBe(true);
      view.unmount();
    });

    test("a folder that Longhand can't write turns the button off and gives the reason", async () => {
      const { view } = picker({
        agent_instructions_status: () => ({ kind: "unavailable", reason: "The storage folder is not available." }),
      });
      await waitFor(() => expect(addButton().disabled).toBe(true));
      expect(screen.getAllByText("The storage folder is not available.").length).toBeGreaterThan(0);
      view.unmount();
    });

    test("a storage folder that Longhand can't write turns the button off and gives the reason", async () => {
      const locked: DestinationStatus = { kind: "not_writable", path: settings.storage_folder, message: "Permission denied" };
      const { view } = picker({}, { status: locked });
      expect(await screen.findByText("Longhand can’t write to the storage folder: Permission denied")).toBeTruthy();
      expect(addButton().disabled).toBe(true);
      view.unmount();
    });

    test("a new status read or a folder change removes the error of a failed write", async () => {
      let fail = true;
      const { user, view } = picker({
        add_agent_instructions: () => {
          throw "Longhand can’t write AGENTS.md: Permission denied";
        },
        agent_instructions_status: () => (fail ? { kind: "not_added" } : { kind: "added" }),
      });
      await user.click(await screen.findByRole("button", { name: "Add Agent Instructions" }));
      expect(await screen.findByText("Longhand can’t write AGENTS.md: Permission denied")).toBeTruthy();
      // The user fixes the file, then comes back to the window.
      fail = false;
      act(() => {
        window.dispatchEvent(new Event("focus"));
      });
      expect(await screen.findByText("Added")).toBeTruthy();
      expect(screen.queryByText("Longhand can’t write AGENTS.md: Permission denied")).toBeNull();

      // A folder change removes the error too.
      fail = true;
      act(() => {
        window.dispatchEvent(new Event("focus"));
      });
      await user.click(await screen.findByRole("button", { name: "Add Agent Instructions" }));
      expect(await screen.findByText("Longhand can’t write AGENTS.md: Permission denied")).toBeTruthy();
      view.rerender(
        <DestinationPicker
          settings={{ ...settings, storage_folder: "/Volumes/Disk/Notes" }}
          status={{ ...ready, path: "/Volumes/Disk/Notes" }}
          onSettings={() => {}}
        />,
      );
      await waitFor(() => expect(screen.queryByText("Longhand can’t write AGENTS.md: Permission denied")).toBeNull());
      view.unmount();
    });

    test("a failed write shows the error that names the file", async () => {
      const { user, view } = picker({
        add_agent_instructions: () => {
          throw "Longhand can’t write AGENTS.md: Permission denied";
        },
      });
      await user.click(await screen.findByRole("button", { name: "Add Agent Instructions" }));
      expect(await screen.findByText("Longhand can’t write AGENTS.md: Permission denied")).toBeTruthy();
      expect(addButton().disabled).toBe(false);
      view.unmount();
    });
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
