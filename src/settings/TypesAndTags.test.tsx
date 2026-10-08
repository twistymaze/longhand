import { describe, expect, test } from "bun:test";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockWindows } from "@tauri-apps/api/mocks";
import type { NoteTypeEntry, NoteTypeList, TagEntry } from "../bindings";
import { type CommandHandler, type TauriMock, mockTauri } from "../test/tauri";
import { TypesAndTags } from "./TypesAndTags";

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
const key = (text: string) => text.trim().replace(/\s*::\s*/g, "::").toLowerCase();

/// A fake of the type and tag commands that keeps its own lists, as the
/// backend does. A rename onto another type or tag merges the two.
function backend(over: Record<string, CommandHandler> = {}) {
  let types: NoteTypeEntry[] = [
    { name: "Pathfinder", valid: true, shared: true, notes: 30 },
    { name: "Projects", valid: true, shared: true, notes: 4 },
    { name: "Bad:Name", valid: false, shared: false, notes: 2 },
  ];
  let defaultType: string | null = null;
  let tags: TagEntry[] = [
    { text: "Person :: Nathan", key: "person::nathan", count: 12 },
    { text: "urgent", key: "urgent", count: 3 },
  ];
  const list = (): NoteTypeList => ({ types: types.map((t) => ({ ...t })), default_type: defaultType });
  return mockTauri({
    note_types: () => list(),
    list_tags: () => tags.map((t) => ({ ...t })),
    add_note_type: ({ name }) => {
      const n = name as string;
      if (types.some((t) => same(t.name, n))) throw "A type with this name already exists.";
      types = [...types, { name: n, valid: true, shared: true, notes: 0 }];
      return list();
    },
    set_default_note_type: ({ name }) => {
      defaultType = name as string | null;
      return list();
    },
    rename_note_type: ({ name, newName }) => {
      const to = newName as string;
      if (to.includes("/")) throw "A type name can’t contain “/”.";
      const from = types.find((t) => t.name === name)!;
      const into = types.find((t) => t !== from && same(t.name, to));
      if (into) {
        into.notes += from.notes;
        types = types.filter((t) => t !== from);
      } else {
        types = types.map((t) => (t === from ? { ...t, name: to } : t));
      }
      if (defaultType === name) defaultType = into ? into.name : to;
      return { changed: from.notes, list: list() };
    },
    delete_note_type: ({ name }) => {
      const from = types.find((t) => t.name === name)!;
      types = types.filter((t) => t !== from);
      if (defaultType === name) defaultType = null;
      return { changed: from.notes, list: list() };
    },
    rename_tag: ({ name, newName }) => {
      const to = newName as string;
      if (to.trim() === "") throw "A tag can’t be empty.";
      const from = tags.find((t) => t.text === name)!;
      const into = tags.find((t) => t !== from && t.key === key(to));
      if (into) {
        into.count += from.count;
        tags = tags.filter((t) => t !== from);
      } else {
        tags = tags.map((t) => (t === from ? { text: to, key: key(to), count: t.count } : t));
      }
      return { changed: from.count, tags: tags.map((t) => ({ ...t })) };
    },
    delete_tag: ({ name }) => {
      const from = tags.find((t) => t.text === name)!;
      tags = tags.filter((t) => t !== from);
      return { changed: from.count, tags: tags.map((t) => ({ ...t })) };
    },
    ...over,
  });
}

function renderPane(readOnly: string | null = null) {
  mockWindows("settings");
  render(<TypesAndTags readOnly={readOnly} />);
}

const called = (tauri: TauriMock, cmd: string) => tauri.calls.filter((c) => c.cmd === cmd);
const typeField = (name: string) => screen.getByRole("textbox", { name: `Name of type ${name}` }) as HTMLInputElement;
const tagField = (text: string) => screen.getByRole("textbox", { name: `Name of tag ${text}` }) as HTMLInputElement;
const typeNames = () =>
  screen.queryAllByRole("textbox", { name: /^Name of type / }).map((f) => (f as HTMLInputElement).value);
const tagNames = () =>
  screen.queryAllByRole("textbox", { name: /^Name of tag / }).map((f) => (f as HTMLInputElement).value);

async function rename(field: HTMLInputElement, text: string) {
  const user = userEvent.setup();
  await user.clear(field);
  await user.type(field, `${text}{Enter}`);
}

describe("TypesAndTags", () => {
  test("lists the types, the default type and the tags with their counts", async () => {
    backend();
    renderPane();
    await waitFor(() => expect(typeNames()).toEqual(["Pathfinder", "Projects", "Bad:Name"]));
    const group = screen.getByRole("radiogroup", { name: "Default type" });
    // A type that is not valid is never offered as the default.
    expect(within(group).getAllByRole("radio").map((r) => r.getAttribute("aria-label"))).toEqual([
      "No type",
      "Pathfinder",
      "Projects",
    ]);
    expect(within(group).getByRole("radio", { name: "No type" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getByText("30 notes")).toBeTruthy();
    expect(screen.getByText(/can’t be a folder name/)).toBeTruthy();
    expect(tagNames()).toEqual(["Person :: Nathan", "urgent"]);
    expect(screen.getByText("12 notes")).toBeTruthy();
  });

  test("adding a type calls the command and refreshes the list", async () => {
    const tauri = backend();
    renderPane();
    const user = userEvent.setup();
    await waitFor(() => expect(typeNames()).toHaveLength(3));
    await user.type(screen.getByRole("textbox", { name: "New type" }), "Clients");
    await user.click(screen.getByRole("button", { name: "Add Type" }));
    await waitFor(() => expect(typeNames()).toContain("Clients"));
    expect(called(tauri, "add_note_type")[0].args).toEqual({ name: "Clients" });
    expect((screen.getByRole("textbox", { name: "New type" }) as HTMLInputElement).value).toBe("");
  });

  test("renaming a type calls the command and asks nothing", async () => {
    const tauri = backend();
    renderPane();
    await waitFor(() => expect(typeNames()).toHaveLength(3));
    await rename(typeField("Projects"), "Work");
    await waitFor(() => expect(typeNames()).toEqual(["Pathfinder", "Work", "Bad:Name"]));
    expect(called(tauri, "rename_note_type")[0].args).toEqual({ name: "Projects", newName: "Work" });
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  test("renaming a type to an existing name shows the merge and the list holds one type", async () => {
    backend();
    renderPane();
    await waitFor(() => expect(typeNames()).toHaveLength(3));
    await rename(typeField("Pathfinder"), "projects");
    await waitFor(() => expect(typeNames()).toEqual(["Projects", "Bad:Name"]));
    expect(screen.getByRole("status").textContent).toContain("Merged “Pathfinder” into “Projects”");
    expect(screen.getByText("34 notes")).toBeTruthy();
  });

  test("a case-only rename is not a merge", async () => {
    backend();
    renderPane();
    await waitFor(() => expect(typeNames()).toHaveLength(3));
    await rename(typeField("Projects"), "PROJECTS");
    await waitFor(() => expect(typeNames()).toEqual(["Pathfinder", "PROJECTS", "Bad:Name"]));
    expect(screen.queryByText(/Merged/)).toBeNull();
  });

  test("deleting a type shows the count and calls delete only after the user confirms", async () => {
    const tauri = backend();
    renderPane();
    const user = userEvent.setup();
    await waitFor(() => expect(typeNames()).toHaveLength(3));
    await user.click(screen.getByRole("button", { name: "Delete type Pathfinder" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("Delete “Pathfinder”?");
    expect(dialog.textContent).toContain("30 notes move out of the Pathfinder folder");
    expect(called(tauri, "delete_note_type")).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(called(tauri, "delete_note_type")).toHaveLength(0);
    expect(typeNames()).toContain("Pathfinder");

    await user.click(screen.getByRole("button", { name: "Delete type Pathfinder" }));
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(typeNames()).toEqual(["Projects", "Bad:Name"]));
    expect(called(tauri, "delete_note_type")[0].args).toEqual({ name: "Pathfinder" });
  });

  test("setting the default type checks it in the radio group", async () => {
    const tauri = backend();
    renderPane();
    const user = userEvent.setup();
    await waitFor(() => expect(typeNames()).toHaveLength(3));
    const radio = screen.getByRole("radio", { name: "Projects" });
    await user.click(radio);
    await waitFor(() => expect(radio.getAttribute("aria-checked")).toBe("true"));
    expect(document.activeElement).toBe(radio);
    expect(called(tauri, "set_default_note_type")[0].args).toEqual({ name: "Projects" });
    expect(screen.getByRole("radio", { name: "No type" }).getAttribute("aria-checked")).toBe("false");

    await user.click(screen.getByRole("radio", { name: "No type" }));
    await waitFor(() => expect(screen.getByRole("radio", { name: "No type" }).getAttribute("aria-checked")).toBe("true"));
    expect(called(tauri, "set_default_note_type")[1].args).toEqual({ name: null });

    // The down arrow on a radio moves the default; in a name field it does not.
    await user.keyboard("{ArrowDown}");
    await waitFor(() => expect(screen.getByRole("radio", { name: "Pathfinder" }).getAttribute("aria-checked")).toBe("true"));
    expect(document.activeElement).toBe(screen.getByRole("radio", { name: "Pathfinder" }));
    await user.click(typeField("Projects"));
    await user.keyboard("{ArrowDown}");
    expect(called(tauri, "set_default_note_type")).toHaveLength(3);
  });

  test("deleting the default type makes the default No type", async () => {
    backend();
    renderPane();
    const user = userEvent.setup();
    await waitFor(() => expect(typeNames()).toHaveLength(3));
    await user.click(screen.getByRole("radio", { name: "Projects" }));
    await waitFor(() => expect(screen.getByRole("radio", { name: "Projects" }).getAttribute("aria-checked")).toBe("true"));
    await user.click(screen.getByRole("button", { name: "Delete type Projects" }));
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(screen.getByRole("radio", { name: "No type" }).getAttribute("aria-checked")).toBe("true"));
  });

  test("renaming a tag calls the command, and a rename onto a tag merges them", async () => {
    const tauri = backend();
    renderPane();
    await waitFor(() => expect(tagNames()).toHaveLength(2));
    await rename(tagField("urgent"), "Urgent now");
    await waitFor(() => expect(tagNames()).toEqual(["Person :: Nathan", "Urgent now"]));
    expect(called(tauri, "rename_tag")[0].args).toEqual({ name: "urgent", newName: "Urgent now" });
    expect(screen.queryByRole("alertdialog")).toBeNull();

    await rename(tagField("Urgent now"), "person::nathan");
    await waitFor(() => expect(tagNames()).toEqual(["Person :: Nathan"]));
    expect(screen.getByRole("status").textContent).toContain("Merged “Urgent now” into “Person :: Nathan”");
    expect(screen.getByText("15 notes")).toBeTruthy();
  });

  test("deleting a tag shows the count and calls delete only after the user confirms", async () => {
    const tauri = backend();
    renderPane();
    const user = userEvent.setup();
    await waitFor(() => expect(tagNames()).toHaveLength(2));
    await user.click(screen.getByRole("button", { name: "Delete tag Person :: Nathan" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("12 notes lose this tag");
    expect(called(tauri, "delete_tag")).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(tagNames()).toEqual(["urgent"]));
    expect(called(tauri, "delete_tag")[0].args).toEqual({ name: "Person :: Nathan" });
  });

  test("a refused name shows its reason and changes nothing", async () => {
    backend();
    renderPane();
    const user = userEvent.setup();
    await waitFor(() => expect(typeNames()).toHaveLength(3));
    await rename(typeField("Projects"), "A/B");
    expect(await screen.findByText("A type name can’t contain “/”.")).toBeTruthy();
    expect(typeNames()).toEqual(["Pathfinder", "A/B", "Bad:Name"]);
    // The list still holds the old name: Escape shows it again.
    await user.type(typeField("Projects"), "{Escape}");
    expect(typeField("Projects").value).toBe("Projects");

    await user.type(screen.getByRole("textbox", { name: "New type" }), "pathfinder");
    await user.click(screen.getByRole("button", { name: "Add Type" }));
    expect(await screen.findByText("A type with this name already exists.")).toBeTruthy();
    expect((screen.getByRole("textbox", { name: "New type" }) as HTMLInputElement).value).toBe("pathfinder");

    await rename(tagField("urgent"), " ");
    expect(await screen.findByText("A tag can’t be empty.")).toBeTruthy();
    expect(tagNames()).toEqual(["Person :: Nathan", " "]);
  });

  test("a change on another computer or in the library reads the lists again", async () => {
    const tauri = backend();
    renderPane();
    await waitFor(() => expect(typeNames()).toHaveLength(3));
    const before = called(tauri, "note_types").length;
    await tauri.emit("settings-changed", null);
    await waitFor(() => expect(called(tauri, "note_types").length).toBeGreaterThan(before));
    const tagsBefore = called(tauri, "list_tags").length;
    await tauri.emit("library-changed", { changed: [], removed: [] });
    await waitFor(() => expect(called(tauri, "list_tags").length).toBeGreaterThan(tagsBefore));
  });

  test("read-only shared settings disable the type controls and show why", async () => {
    backend();
    renderPane("The storage folder is not available.");
    await waitFor(() => expect(typeNames()).toHaveLength(3));
    expect(screen.getByText(/The storage folder is not available\. Types can’t be changed now\./)).toBeTruthy();
    expect(typeField("Projects").disabled).toBe(true);
    expect((screen.getByRole("radio", { name: "Projects" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Add Type" }) as HTMLButtonElement).disabled).toBe(true);
  });
});

const typesNamed = (...names: string[]): NoteTypeList => ({
  types: names.map((name) => ({ name, valid: true, shared: true, notes: 0 })),
  default_type: null,
});
const settle = () => act(() => new Promise((r) => setTimeout(r, 20)));

describe("TypesAndTags reads", () => {
  test("a read that ends after a newer read does not replace the newer list", async () => {
    const reads: ((l: NoteTypeList) => void)[] = [];
    const tauri = backend({ note_types: () => new Promise<NoteTypeList>((r) => reads.push(r)) });
    renderPane();
    await waitFor(() => expect(reads).toHaveLength(1));
    await act(() => tauri.emit("settings-changed", null));
    await waitFor(() => expect(reads).toHaveLength(2));
    await act(async () => reads[1](typesNamed("New")));
    await waitFor(() => expect(typeNames()).toEqual(["New"]));
    await act(async () => reads[0](typesNamed("Old")));
    await settle();
    expect(typeNames()).toEqual(["New"]);
  });

  test("a read that started before a command does not replace the command result", async () => {
    const reads: ((l: NoteTypeList) => void)[] = [];
    const tauri = backend({
      note_types: () => new Promise<NoteTypeList>((r) => reads.push(r)),
      rename_note_type: () => ({ changed: 0, list: typesNamed("Work") }),
    });
    renderPane();
    await waitFor(() => expect(reads).toHaveLength(1));
    await act(async () => reads[0](typesNamed("Projects")));
    await waitFor(() => expect(typeNames()).toEqual(["Projects"]));
    await act(() => tauri.emit("settings-changed", null));
    await waitFor(() => expect(reads).toHaveLength(2));
    await rename(typeField("Projects"), "Work");
    await waitFor(() => expect(typeNames()).toEqual(["Work"]));
    await act(async () => reads[1](typesNamed("Projects")));
    await settle();
    expect(typeNames()).toEqual(["Work"]);
  });

  test("a read error goes away after a later good read", async () => {
    let failed = 0;
    const tauri = backend({
      note_types: () => {
        if (failed++ === 0) throw "The types could not be found.";
        return typesNamed("Projects");
      },
      list_tags: () => {
        if (failed < 2) throw "The tags could not be found.";
        return [];
      },
    });
    renderPane();
    expect(await screen.findByText("The types could not be found.")).toBeTruthy();
    expect(await screen.findByText("The tags could not be found.")).toBeTruthy();
    await act(() => tauri.emit("settings-changed", null));
    await waitFor(() => expect(typeNames()).toEqual(["Projects"]));
    await settle();
    expect(screen.queryByText("The types could not be found.")).toBeNull();
    expect(screen.queryByText("The tags could not be found.")).toBeNull();
  });
});
