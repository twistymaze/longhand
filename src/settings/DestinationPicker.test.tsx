import { describe, expect, test } from "bun:test";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { DestinationStatus, Settings } from "../bindings";
import { mockTauri } from "../test/tauri";
import { DestinationPicker } from "./DestinationPicker";

const settings = { storage_folder: "/Users/me/Library/Application Support/Longhand/Notes" } as Settings;
const ready: DestinationStatus = { kind: "ready", path: settings.storage_folder, free_bytes: null };

function picker(onSettings: (s: Settings) => void = () => {}) {
  return render(<DestinationPicker settings={settings} status={ready} onSettings={onSettings} />);
}

describe("DestinationPicker", () => {
  test("shows the one storage folder and no earlier folders", () => {
    mockTauri();
    const view = picker();
    expect(view.getByText("Notes")).toBeTruthy();
    expect(view.getByText(settings.storage_folder)).toBeTruthy();
    expect(view.queryByText("Earlier folders")).toBeNull();
    view.unmount();
  });

  test("Change picks a folder and changes to it with Leave them", async () => {
    const changed = { ...settings, storage_folder: "/Volumes/Disk/Notes" } as Settings;
    const mock = mockTauri({
      pick_storage_folder: () => "/Volumes/Disk/Notes",
      change_storage_folder: () => changed,
    });
    const received: Settings[] = [];
    const view = picker((s) => received.push(s));
    await userEvent.click(view.getByRole("button", { name: "Change…" }));
    await waitFor(() => expect(received).toEqual([changed]));
    expect(mock.calls.map((c) => c.cmd)).toEqual(["pick_storage_folder", "change_storage_folder"]);
    expect(mock.calls[1].args).toEqual({ folder: "/Volumes/Disk/Notes", oldNotes: "leave" });
    view.unmount();
  });

  test("a closed dialog changes nothing", async () => {
    const mock = mockTauri({ pick_storage_folder: () => null });
    const received: Settings[] = [];
    const view = picker((s) => received.push(s));
    await userEvent.click(view.getByRole("button", { name: "Change…" }));
    await waitFor(() => expect(mock.calls.length).toBe(1));
    expect(received).toEqual([]);
    view.unmount();
  });

  test("a refused folder shows its cause", async () => {
    mockTauri({
      pick_storage_folder: () => settings.storage_folder,
      change_storage_folder: () => {
        throw "This folder is already the storage folder.";
      },
    });
    const view = picker();
    await userEvent.click(view.getByRole("button", { name: "Change…" }));
    expect(await screen.findByText("This folder is already the storage folder.")).toBeTruthy();
    view.unmount();
  });
});
