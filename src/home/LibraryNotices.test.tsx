import { describe, expect, test } from "vitest";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ImportNotices, LibraryNotices as Notices } from "../bindings";
import { mockTauri } from "../test/tauri";
import { LibraryNotices } from "./LibraryNotices";
import { useLibraryNotices } from "./useLibraryNotices";

const noImports: ImportNotices = { watched_folder: null, waiting_for_space: [], failed: [] };

const notices = (imports: Partial<ImportNotices>): Notices => ({
  conversion: null,
  problems: [],
  imports: { ...noImports, ...imports },
});

/// The notices of Home, as Home reads and shows them.
function Notices() {
  const { notices, reload } = useLibraryNotices();
  return <LibraryNotices notices={notices} onChanged={() => void reload()} />;
}

const failedRegion = "Audio files that could not be imported";
const diskRegion = "Imports that wait for disk space";
const folderRegion = "Watched folder not available";

describe("LibraryNotices: audio import (U8 of the audio import plan)", () => {
  // Covers AE8, R20: one notice names the failed file. Dismiss removes it,
  // and later reads do not show it again. A new attempt is a new notice.
  test("a failed import shows one notice until the user dismisses it", async () => {
    let current = notices({ failed: [{ note_id: "n1", file_name: "Damaged.m4a" }] });
    const tauri = mockTauri({
      library_notices: () => current,
      dismiss_import_failure: () => {
        current = notices({});
        return null;
      },
    });
    render(<Notices />);
    const notice = await screen.findByRole("region", { name: failedRegion });
    expect(
      within(notice).getByText("Longhand can’t read the audio in “Damaged.m4a”. The file may be damaged."),
    ).toBeTruthy();
    expect(screen.getAllByRole("region")).toHaveLength(1);

    await userEvent.setup().click(within(notice).getByRole("button", { name: "Dismiss “Damaged.m4a”" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: failedRegion })).toBeNull());
    expect(tauri.calls.find((c) => c.cmd === "dismiss_import_failure")?.args).toEqual({ noteId: "n1" });

    // A later scan changes nothing: the notice stays away.
    const reads = tauri.calls.filter((c) => c.cmd === "library_notices").length;
    await act(() => tauri.emit("library-changed", { changed: [], removed: [] }));
    await waitFor(() => expect(tauri.calls.filter((c) => c.cmd === "library_notices").length).toBe(reads + 1));
    expect(screen.queryByRole("region", { name: failedRegion })).toBeNull();

    // A second import of the same file failed again: a new notice.
    current = notices({ failed: [{ note_id: "n2", file_name: "Damaged.m4a" }] });
    await act(() => tauri.emit("import-notices-changed", null));
    expect(await screen.findByRole("region", { name: failedRegion })).toBeTruthy();
  });

  // Covers R22, KTD9: the disk notice names the file, and it clears when
  // the copy starts.
  test("an import that waits for disk space shows a notice until the copy starts", async () => {
    let current = notices({ waiting_for_space: ["Team sync.m4a"] });
    const tauri = mockTauri({ library_notices: () => current });
    render(<Notices />);
    const notice = await screen.findByRole("region", { name: diskRegion });
    expect(
      within(notice).getByText(
        "There is not enough disk space to import “Team sync.m4a”. Free some space, and the import continues.",
      ),
    ).toBeTruthy();
    expect(within(notice).queryByRole("button")).toBeNull();

    current = notices({});
    await act(() => tauri.emit("import-notices-changed", null));
    await waitFor(() => expect(screen.queryByRole("region", { name: diskRegion })).toBeNull());
  });

  // Covers R20: the folder notice has no Dismiss, and it clears when the
  // folder is available again.
  test("the folder notice shows and clears with the folder state", async () => {
    let current = notices({ watched_folder: "/Users/a/Phone recordings" });
    const tauri = mockTauri({ library_notices: () => current, open_settings: () => null });
    render(<Notices />);
    const notice = await screen.findByRole("region", { name: folderRegion });
    expect(
      within(notice).getByText(
        "Longhand can’t open the watched folder “Phone recordings”. Connect its disk, or choose another folder in Settings.",
      ),
    ).toBeTruthy();
    expect(within(notice).queryByRole("button", { name: /Dismiss/ })).toBeNull();
    await userEvent.setup().click(within(notice).getByRole("button", { name: "Open Settings…" }));
    expect(tauri.calls.find((c) => c.cmd === "open_settings")?.args).toEqual({ pane: "destination" });

    current = notices({});
    await act(() =>
      tauri.emit("watched-folder-changed", { status: { kind: "watching", path: "/Users/a/Phone recordings" } }),
    );
    await waitFor(() => expect(screen.queryByRole("region", { name: folderRegion })).toBeNull());
  });
});
