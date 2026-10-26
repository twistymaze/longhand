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

function setup(paths: string[], handler: () => unknown = () => imported) {
  const tauri = mockTauri({ import_files: handler as () => never });
  const onDone = vi.fn((_result: unknown) => {});
  const onCancel = vi.fn(() => {});
  render(<ImportSheet paths={paths} types={types} tags={["Team", "Client::Acme"]} onDone={onDone} onCancel={onCancel} />);
  const imports = () => tauri.calls.filter((c) => c.cmd === "import_files");
  return { user: userEvent.setup(), onDone, onCancel, imports };
}

const typeField = () => screen.getByRole("combobox", { name: "Type" }) as HTMLSelectElement;
const titleField = () => screen.getByRole("textbox", { name: "Title" }) as HTMLInputElement;
const notesField = () => screen.getByRole("textbox", { name: "Notes" });

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
      { paths: ["/Users/me/Recordings/Team sync.m4a"], noteType: { kind: "default" }, tags: [], title: null, notes: null },
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
      { paths, noteType: { kind: "named", name: "1:1" }, tags: ["Team"], title: null, notes: null },
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

  test("Escape does nothing while the import runs", async () => {
    let finish: (v: typeof imported) => void = () => {};
    const running = new Promise<typeof imported>((resolve) => (finish = resolve));
    const { user, onCancel, onDone, imports } = setup(["/a/One.m4a"], () => running);
    await user.click(screen.getByRole("button", { name: "Import" }));
    await waitFor(() => expect(imports()).toHaveLength(1));
    await user.keyboard("{Escape}");
    expect(onCancel).not.toHaveBeenCalled();
    finish(imported);
    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
  });

  test("the first field has the focus when the sheet opens", () => {
    setup(["/a/One.m4a"]);
    expect(document.activeElement).toBe(titleField());
  });

  test("with several files, the type field has the focus when the sheet opens", () => {
    setup(["/a/One.m4a", "/a/Two.m4a"]);
    expect(document.activeElement).toBe(typeField());
  });

  describe("Notes", () => {
    const modReturn = /Mac/.test(navigator.platform) ? "{Meta>}{Enter}{/Meta}" : "{Control>}{Enter}{/Control}";
    const selectAll = /Mac/.test(navigator.platform) ? "{Meta>}a{/Meta}" : "{Control>}a{/Control}";

    test("for one file, the text of the Notes field goes with the import", async () => {
      const { user, onDone, imports } = setup(["/a/One.m4a"]);
      await user.type(notesField(), "Weekly sync with Dana");
      await user.click(screen.getByRole("button", { name: "Import" }));
      await waitFor(() => expect(onDone).toHaveBeenCalled());
      expect(imports()[0].args.notes).toBe("Weekly sync with Dana");
    });

    test("a Notes field with only spaces sends no notes", async () => {
      const { user, onDone, imports } = setup(["/a/One.m4a"]);
      await user.type(notesField(), "   ");
      await user.click(screen.getByRole("button", { name: "Import" }));
      await waitFor(() => expect(onDone).toHaveBeenCalled());
      expect(imports()[0].args.notes).toBeNull();
    });

    test("for several files, the sheet has no Notes field", () => {
      setup(["/a/One.m4a", "/a/Two.m4a"]);
      expect(screen.queryByRole("textbox", { name: "Notes" })).toBeNull();
    });

    test("Return in the Notes field makes a new line and does not import", async () => {
      const { user, onDone, imports } = setup(["/a/One.m4a"]);
      await user.type(notesField(), "Dana{Enter}Budget");
      expect(imports()).toEqual([]);
      await user.click(screen.getByRole("button", { name: "Import" }));
      await waitFor(() => expect(onDone).toHaveBeenCalled());
      expect(imports()[0].args.notes).toBe("Dana\n\nBudget");
    });

    test("Cmd+Return in the Notes field imports once, with the notes", async () => {
      const { user, onDone, imports } = setup(["/a/One.m4a"]);
      await user.type(notesField(), "Dana");
      await user.keyboard(modReturn);
      await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
      expect(imports()).toHaveLength(1);
      expect(imports()[0].args.notes).toBe("Dana");
    });

    test("Cmd+Return in the title field imports", async () => {
      const { user, onDone, imports } = setup(["/a/One.m4a"]);
      await user.keyboard(modReturn);
      await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
      expect(imports()).toHaveLength(1);
    });

    test("Cmd+Return in a task item checks the task and does not import", async () => {
      const { user, onDone, imports } = setup(["/a/One.m4a"]);
      await user.type(notesField(), "- [[ ] Send the budget");
      expect(notesField().querySelector('li[data-type="taskItem"]')).not.toBeNull();
      await user.keyboard(modReturn);
      expect(notesField().querySelector('li[data-checked="true"]')).not.toBeNull();
      expect(imports()).toEqual([]);
      expect(onDone).not.toHaveBeenCalled();
    });

    test("Escape after plain text in the Notes field cancels the sheet", async () => {
      const { user, onCancel } = setup(["/a/One.m4a"]);
      await user.type(notesField(), "Dana");
      await user.keyboard("{Escape}");
      expect(onCancel).toHaveBeenCalledTimes(1);
    });

    test("Escape with the / menu open closes the menu, and a second Escape cancels the sheet", async () => {
      const { user, onCancel } = setup(["/a/One.m4a"]);
      await user.type(notesField(), "/");
      expect(screen.getByRole("button", { name: "Heading 1" })).toBeTruthy();
      await user.keyboard("{Escape}");
      expect(screen.queryByRole("button", { name: "Heading 1" })).toBeNull();
      expect(onCancel).not.toHaveBeenCalled();
      await user.keyboard("{Escape}");
      expect(onCancel).toHaveBeenCalledTimes(1);
    });

    test("Escape in the link field closes only the link field", async () => {
      const { user, onCancel } = setup(["/a/One.m4a"]);
      await user.type(notesField(), "Dana");
      await user.keyboard(selectAll);
      await user.click(screen.getByRole("button", { name: "Link" }));
      expect(document.activeElement).toBe(screen.getByLabelText("Link address"));
      await user.keyboard("{Escape}");
      expect(screen.queryByLabelText("Link address")).toBeNull();
      expect(onCancel).not.toHaveBeenCalled();
    });

    test("Tab in the Notes field keeps the focus in the field", async () => {
      const { user } = setup(["/a/One.m4a"]);
      await user.click(notesField());
      await user.keyboard("{Tab}");
      expect(document.activeElement).toBe(notesField());
    });
  });
});
