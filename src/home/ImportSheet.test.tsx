import { describe, expect, test, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { NoteTypeEntry, NoteTypeList } from "../bindings";
import { mockTauri } from "../test/tauri";
import { ImportSheet } from "./ImportSheet";

const entry = (name: string, general = false): NoteTypeEntry => ({
  name,
  valid: true,
  shared: !general,
  notes: 0,
  general,
  prompt_choice: null,
  prompt_name: null,
  summary_prompt_id: "",
});

const types: NoteTypeList = {
  types: [entry("General", true), entry("1:1"), entry("Pathfinder")],
  default_type: "Pathfinder",
};

const imported = { note_ids: ["n1"], refused: [] };

function setup(paths: string[]) {
  const tauri = mockTauri({ import_files: () => imported });
  const onDone = vi.fn((_result: unknown) => {});
  const onCancel = vi.fn(() => {});
  render(<ImportSheet paths={paths} types={types} tags={["Team", "Client::Acme"]} onDone={onDone} onCancel={onCancel} />);
  const imports = () => tauri.calls.filter((c) => c.cmd === "import_files");
  return { user: userEvent.setup(), onDone, onCancel, imports };
}

const typeField = () => screen.getByRole("combobox", { name: "Type" }) as HTMLSelectElement;
const titleField = () => screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement;

describe("ImportSheet", () => {
  test("for one file, the sheet shows the file name as the title, the default type and no tags; Import sends them", async () => {
    const { user, onDone, imports } = setup(["/Users/me/Recordings/Team sync.m4a"]);
    expect(titleField().value).toBe("Team sync");
    expect(typeField().value).toBe("Pathfinder");
    expect(screen.queryAllByText("Team")).toEqual([]);
    await user.click(screen.getByRole("button", { name: "Import" }));
    await waitFor(() => expect(onDone).toHaveBeenCalledWith({ status: "ok", data: imported }));
    // An unchanged file name stays a proposed title, so the summary can
    // replace it (R15).
    expect(imports().map((c) => c.args)).toEqual([
      { paths: ["/Users/me/Recordings/Team sync.m4a"], noteType: { kind: "default" }, tags: [], title: null },
    ]);
  });

  test("an edited title is sent as a user title", async () => {
    const { user, onDone, imports } = setup(["/Users/me/Team sync.m4a"]);
    await user.clear(titleField());
    await user.type(titleField(), "Budget review{Enter}");
    await waitFor(() => expect(onDone).toHaveBeenCalled());
    expect(imports()[0].args.title).toBe("Budget review");
  });

  test("for three files, the sheet lists the names, has no title field, and one type and tag set goes with all", async () => {
    const paths = ["/a/One.m4a", "/a/Two.mp3", "/a/Three.wav"];
    const { user, onDone, imports } = setup(paths);
    const names = screen.getByRole("list", { name: "Files" });
    expect([...names.querySelectorAll("li")].map((li) => li.textContent)).toEqual(["One.m4a", "Two.mp3", "Three.wav"]);
    expect(screen.queryByRole("textbox", { name: "Title" })).toBeNull();
    await user.selectOptions(typeField(), "1:1");
    await user.click(screen.getByRole("button", { name: "Add Tag" }));
    await user.keyboard("Team{Enter}");
    await user.click(screen.getByRole("button", { name: "Import" }));
    await waitFor(() => expect(onDone).toHaveBeenCalled());
    expect(imports().map((c) => c.args)).toEqual([
      { paths, noteType: { kind: "named", name: "1:1" }, tags: ["Team"], title: null },
    ]);
  });

  test("General is sent as General", async () => {
    const { user, onDone, imports } = setup(["/a/One.m4a", "/a/Two.m4a"]);
    await user.selectOptions(typeField(), "General");
    await user.click(screen.getByRole("button", { name: "Import" }));
    await waitFor(() => expect(onDone).toHaveBeenCalled());
    expect(imports()[0].args.noteType).toEqual({ kind: "general" });
  });

  test("Cancel calls nothing", async () => {
    const { user, onCancel, onDone, imports } = setup(["/a/One.m4a"]);
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onDone).not.toHaveBeenCalled();
    expect(imports()).toEqual([]);
  });

  test("Escape cancels", async () => {
    const { user, onCancel, imports } = setup(["/a/One.m4a"]);
    await user.keyboard("{Escape}");
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(imports()).toEqual([]);
  });

  test("the first field has the focus when the sheet opens", () => {
    setup(["/a/One.m4a"]);
    expect(document.activeElement).toBe(titleField());
  });

  test("with several files, the type field has the focus when the sheet opens", () => {
    setup(["/a/One.m4a", "/a/Two.m4a"]);
    expect(document.activeElement).toBe(typeField());
  });
});
