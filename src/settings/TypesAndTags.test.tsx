import { describe, expect, test } from "bun:test";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockWindows } from "@tauri-apps/api/mocks";
import type { NoteTypeEntry, NoteTypeList, SummaryPromptList, TagEntry } from "../bindings";
import { type CommandHandler, type TauriMock, mockTauri } from "../test/tauri";
import { TypesAndTags } from "./TypesAndTags";

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
const key = (text: string) => text.trim().replace(/\s*::\s*/g, "::").toLowerCase();

const prompts: SummaryPromptList = {
  prompts: [
    { id: "topic-summary", name: "Topic summary", text: "", title_text: "", built_in: true, over_limit: false, too_large: false },
    { id: "play-by-play", name: "Play-by-play summary", text: "", title_text: "", built_in: false, over_limit: false, too_large: false },
  ],
  active: "topic-summary",
  text_limit: 1000,
  title_text_limit: 100,
};

const entry = (name: string, notes: number, valid = true, general = false): NoteTypeEntry => ({
  name,
  valid,
  shared: !general,
  notes,
  general,
  prompt_choice: null,
  prompt_name: null,
  summary_prompt_id: "topic-summary",
});

/// A fake of the type, tag and prompt commands that keeps its own lists, as
/// the backend does. A rename onto another type or tag merges the two.
function backend(
  over: Record<string, CommandHandler> = {},
  start: NoteTypeEntry[] = [
    entry("General", 5, true, true),
    entry("Pathfinder", 30),
    entry("Projects", 4),
    entry("Bad:Name", 2, false),
  ],
) {
  let types = start;
  let defaultType: string | null = null;
  let tags: TagEntry[] = [
    { text: "Person :: Nathan", key: "person::nathan", count: 12 },
    { text: "urgent", key: "urgent", count: 3 },
  ];
  const list = (): NoteTypeList => ({ types: types.map((t) => ({ ...t })), default_type: defaultType });
  return mockTauri({
    note_types: () => list(),
    list_tags: () => tags.map((t) => ({ ...t })),
    summary_prompts: () => prompts,
    add_note_type: ({ name }) => {
      const n = name as string;
      if (types.some((t) => same(t.name, n))) throw "A type with this name already exists.";
      types = [...types, entry(n, 0)];
      return list();
    },
    set_default_note_type: ({ name }) => {
      defaultType = name as string | null;
      return list();
    },
    set_note_type_prompt: ({ name, promptId }) => {
      const id = promptId as string | null;
      types = types.map((t) =>
        t.name === name
          ? {
              ...t,
              prompt_choice: id,
              prompt_name: prompts.prompts.find((p) => p.id === id)?.name ?? null,
              summary_prompt_id: id ?? prompts.active,
            }
          : t,
      );
      return list();
    },
    rename_note_type: ({ name, newName }) => {
      const to = newName as string;
      if (to.includes("/")) throw "A type name can’t contain “/”.";
      if (same(to, "General")) throw "General is a built-in type.";
      const from = types.find((t) => t.name === name)!;
      const into = types.find((t) => t !== from && same(t.name, to));
      if (into) {
        into.notes += from.notes;
        types = types.filter((t) => t !== from);
      } else {
        types = types.map((t) => (t === from ? { ...t, name: to, valid: true } : t));
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
const rowsOf = (list: string) => within(screen.getByRole("list", { name: list })).queryAllByRole("listitem");
/// The name that each row shows as text, or null while the row has a field.
const namesOf = (list: string) =>
  rowsOf(list).map((row) => {
    const id = row.getAttribute("aria-labelledby");
    return (id && document.getElementById(id)?.textContent) || null;
  });
const typeNames = () => namesOf("Types");
const tagNames = () => namesOf("Tags");
const actions = (name: string) => screen.getByRole("button", { name: `Actions for ${name}` }) as HTMLButtonElement;
const promptPopup = (name: string) => screen.getByRole("combobox", { name: `Summary prompt for ${name}` }) as HTMLSelectElement;
const defaultPopup = () => screen.getByRole("combobox", { name: "New notes get the type" }) as HTMLSelectElement;
const optionTexts = (select: HTMLSelectElement) => [...select.options].map((o) => o.text);
const selectedText = (select: HTMLSelectElement) => select.options[select.selectedIndex]?.text;

/// Opens the ⋯ menu of a row and chooses an item.
async function choose(name: string, item: "Rename…" | "Delete…") {
  const user = userEvent.setup();
  await user.click(actions(name));
  await user.click(screen.getByRole("menuitem", { name: item }));
}

/// Renames with Rename… in the ⋯ menu, then Return.
async function rename(name: string, text: string, label: string) {
  const user = userEvent.setup();
  await choose(name, "Rename…");
  const field = screen.getByRole("textbox", { name: label });
  await user.clear(field);
  await user.type(field, `${text}{Enter}`);
}
const renameType = (name: string, text: string) => rename(name, text, `Name of type ${name}`);
const renameTag = (name: string, text: string) => rename(name, text, `Name of tag ${name}`);

const loaded = () => waitFor(() => expect(typeNames()).toHaveLength(4));

describe("TypesAndTags", () => {
  test("lists the types and the tags as text with their counts", async () => {
    backend();
    renderPane();
    await waitFor(() => expect(typeNames()).toEqual(["General", "Pathfinder", "Projects", "Bad:Name"]));
    expect(screen.queryAllByRole("textbox")).toHaveLength(0);
    expect(screen.getByText("30 notes")).toBeTruthy();
    expect(screen.getByText(/can’t be a folder name/)).toBeTruthy();
    await waitFor(() => expect(tagNames()).toEqual(["Person :: Nathan", "urgent"]));
    expect(screen.getByText("12 notes")).toBeTruthy();
  });

  test("the default type popup offers General and the valid types, and sets the default", async () => {
    const tauri = backend();
    renderPane();
    const user = userEvent.setup();
    await loaded();
    expect(screen.queryByRole("radio")).toBeNull();
    expect(optionTexts(defaultPopup())).toEqual(["General", "Pathfinder", "Projects"]);
    expect(selectedText(defaultPopup())).toBe("General");

    await user.selectOptions(defaultPopup(), "Projects");
    await waitFor(() => expect(selectedText(defaultPopup())).toBe("Projects"));
    expect(called(tauri, "set_default_note_type")[0].args).toEqual({ name: "Projects" });

    await user.selectOptions(defaultPopup(), "General");
    await waitFor(() => expect(selectedText(defaultPopup())).toBe("General"));
    expect(called(tauri, "set_default_note_type")[1].args).toEqual({ name: null });
  });

  test("General is first, with its count, a note and a prompt popup, and has no ⋯ menu", async () => {
    backend();
    renderPane();
    await loaded();
    const first = rowsOf("Types")[0];
    expect(within(first).getByText("General")).toBeTruthy();
    expect(within(first).getByText("Notes at the top of the storage folder.")).toBeTruthy();
    expect(within(first).getByText("5 notes")).toBeTruthy();
    expect(within(first).getByRole("combobox", { name: "Summary prompt for General" })).toBeTruthy();
    expect(within(first).queryByRole("button")).toBeNull();
  });

  test("the prompt popup of a type applies a choice at once", async () => {
    const tauri = backend();
    renderPane();
    const user = userEvent.setup();
    await loaded();
    expect(optionTexts(promptPopup("Pathfinder"))).toEqual(["Default", "Topic summary", "Play-by-play summary"]);
    expect(selectedText(promptPopup("Pathfinder"))).toBe("Default");

    await user.selectOptions(promptPopup("Pathfinder"), "Play-by-play summary");
    await waitFor(() => expect(selectedText(promptPopup("Pathfinder"))).toBe("Play-by-play summary"));
    expect(called(tauri, "set_note_type_prompt").map((c) => c.args)).toEqual([
      { name: "Pathfinder", promptId: "play-by-play" },
    ]);

    await user.selectOptions(promptPopup("General"), "Topic summary");
    await waitFor(() => expect(selectedText(promptPopup("General"))).toBe("Topic summary"));
    expect(called(tauri, "set_note_type_prompt")[1].args).toEqual({ name: "General", promptId: "topic-summary" });

    await user.selectOptions(promptPopup("Pathfinder"), "Default");
    await waitFor(() => expect(selectedText(promptPopup("Pathfinder"))).toBe("Default"));
    expect(called(tauri, "set_note_type_prompt")[2].args).toEqual({ name: "Pathfinder", promptId: null });
  });

  test("a choice whose prompt file is missing shows the prompt that summaries use, and Default resets it (R7)", async () => {
    const recap = { ...entry("Pathfinder", 30), prompt_choice: "recap", prompt_name: "Recap", summary_prompt_id: "recap" };
    const tauri = backend({}, [entry("General", 5, true, true), recap, entry("Projects", 4), entry("Bad:Name", 2, false)]);
    renderPane();
    const user = userEvent.setup();
    await loaded();
    expect(optionTexts(promptPopup("Pathfinder"))).toEqual(["Default", "Topic summary", "Play-by-play summary", "Recap"]);
    expect(selectedText(promptPopup("Pathfinder"))).toBe("Recap");
    // Only the type that chooses the prompt offers it.
    expect(optionTexts(promptPopup("General"))).toEqual(["Default", "Topic summary", "Play-by-play summary"]);

    await user.selectOptions(promptPopup("Pathfinder"), "Default");
    await waitFor(() => expect(selectedText(promptPopup("Pathfinder"))).toBe("Default"));
    expect(called(tauri, "set_note_type_prompt").map((c) => c.args)).toEqual([{ name: "Pathfinder", promptId: null }]);
  });

  test("a refused prompt choice shows its reason", async () => {
    backend({
      set_note_type_prompt: () => {
        throw "The prompt could not be saved.";
      },
    });
    renderPane();
    const user = userEvent.setup();
    await loaded();
    await user.selectOptions(promptPopup("Pathfinder"), "Play-by-play summary");
    expect(await screen.findByText("The prompt could not be saved.")).toBeTruthy();
    expect(selectedText(promptPopup("Pathfinder"))).toBe("Default");
  });

  test("Rename… opens a field with the focus, and Escape leaves the name and calls nothing (AE8)", async () => {
    const tauri = backend();
    renderPane();
    const user = userEvent.setup();
    await loaded();
    await choose("Pathfinder", "Rename…");
    const field = screen.getByRole("textbox", { name: "Name of type Pathfinder" }) as HTMLInputElement;
    expect(field.value).toBe("Pathfinder");
    expect(document.activeElement).toBe(field);
    await user.clear(field);
    await user.type(field, "Pathfind{Escape}");
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(typeNames()).toEqual(["General", "Pathfinder", "Projects", "Bad:Name"]);
    expect(called(tauri, "rename_note_type")).toHaveLength(0);
    expect(document.activeElement).toBe(actions("Pathfinder"));
  });

  test("Rename… then a click elsewhere leaves the name and calls nothing", async () => {
    const tauri = backend();
    renderPane();
    const user = userEvent.setup();
    await loaded();
    await choose("Pathfinder", "Rename…");
    await user.type(screen.getByRole("textbox", { name: "Name of type Pathfinder" }), "X");
    await user.click(screen.getByRole("heading", { name: "Types and Tags" }));
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(typeNames()).toEqual(["General", "Pathfinder", "Projects", "Bad:Name"]);
    expect(called(tauri, "rename_note_type")).toHaveLength(0);
  });

  test("Rename… then Return renames the type and asks nothing", async () => {
    const tauri = backend();
    renderPane();
    await loaded();
    await renameType("Projects", "Work");
    await waitFor(() => expect(typeNames()).toEqual(["General", "Pathfinder", "Work", "Bad:Name"]));
    expect(called(tauri, "rename_note_type")[0].args).toEqual({ name: "Projects", newName: "Work" });
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(screen.queryByRole("textbox")).toBeNull();
    await waitFor(() => expect(document.activeElement).toBe(actions("Work")));
  });

  test("a rename to an existing name shows the merge and the list holds one type", async () => {
    backend();
    renderPane();
    await loaded();
    await renameType("Pathfinder", "projects");
    await waitFor(() => expect(typeNames()).toEqual(["General", "Projects", "Bad:Name"]));
    expect(screen.getByRole("status").textContent).toContain("Merged “Pathfinder” into “Projects”");
    expect(screen.getByText("34 notes")).toBeTruthy();
  });

  test("a case-only rename is not a merge", async () => {
    backend();
    renderPane();
    await loaded();
    await renameType("Projects", "PROJECTS");
    await waitFor(() => expect(typeNames()).toEqual(["General", "Pathfinder", "PROJECTS", "Bad:Name"]));
    expect(screen.queryByText(/Merged/)).toBeNull();
  });

  test("a refused rename keeps the field open with the text and the focus, and shows why", async () => {
    const tauri = backend();
    renderPane();
    await loaded();
    await renameType("Projects", "General");
    expect(await screen.findByText("General is a built-in type.")).toBeTruthy();
    const field = screen.getByRole("textbox", { name: "Name of type Projects" }) as HTMLInputElement;
    expect(field.value).toBe("General");
    expect(document.activeElement).toBe(field);
    expect(called(tauri, "rename_note_type")).toHaveLength(1);
    // The other rows are unchanged; the row of Projects still has the field.
    expect(typeNames()).toEqual(["General", "Pathfinder", null, "Bad:Name"]);
  });

  test("Delete… asks one confirmation with the count, and deletes only after the user confirms", async () => {
    const tauri = backend();
    renderPane();
    const user = userEvent.setup();
    await loaded();
    await choose("Pathfinder", "Delete…");
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("Delete “Pathfinder”?");
    expect(dialog.textContent).toContain("30 notes move from the Pathfinder folder to General.");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(called(tauri, "delete_note_type")).toHaveLength(0);
    expect(typeNames()).toContain("Pathfinder");

    await choose("Pathfinder", "Delete…");
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(typeNames()).toEqual(["General", "Projects", "Bad:Name"]));
    expect(called(tauri, "delete_note_type")).toHaveLength(1);
    expect(called(tauri, "delete_note_type")[0].args).toEqual({ name: "Pathfinder" });
  });

  test("deleting the default type makes the default General", async () => {
    backend();
    renderPane();
    const user = userEvent.setup();
    await loaded();
    await user.selectOptions(defaultPopup(), "Projects");
    await waitFor(() => expect(selectedText(defaultPopup())).toBe("Projects"));
    await choose("Projects", "Delete…");
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(selectedText(defaultPopup())).toBe("General"));
  });

  test("Add Type… then Return adds the type, and Escape adds nothing", async () => {
    const tauri = backend();
    renderPane();
    const user = userEvent.setup();
    await loaded();
    await user.click(screen.getByRole("button", { name: "Add Type…" }));
    const field = screen.getByRole("textbox", { name: "New type" });
    expect(document.activeElement).toBe(field);
    await user.type(field, "Clients{Enter}");
    await waitFor(() => expect(typeNames()).toContain("Clients"));
    expect(called(tauri, "add_note_type")[0].args).toEqual({ name: "Clients" });
    expect(screen.queryByRole("textbox")).toBeNull();

    await user.click(screen.getByRole("button", { name: "Add Type…" }));
    await user.type(screen.getByRole("textbox", { name: "New type" }), "Other{Escape}");
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(called(tauri, "add_note_type")).toHaveLength(1);
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Add Type…" }));
  });

  test("a refused new type keeps the field open with the text and shows why", async () => {
    backend();
    renderPane();
    const user = userEvent.setup();
    await loaded();
    await user.click(screen.getByRole("button", { name: "Add Type…" }));
    await user.type(screen.getByRole("textbox", { name: "New type" }), "pathfinder{Enter}");
    expect(await screen.findByText("A type with this name already exists.")).toBeTruthy();
    expect((screen.getByRole("textbox", { name: "New type" }) as HTMLInputElement).value).toBe("pathfinder");
  });

  test("a type with an invalid name shows its warning and a ⋯ menu, and no prompt popup", async () => {
    backend();
    renderPane();
    await loaded();
    const bad = rowsOf("Types")[3];
    expect(within(bad).getByText(/can’t be a folder name/)).toBeTruthy();
    expect(within(bad).getByRole("button", { name: "Actions for Bad:Name" })).toBeTruthy();
    expect(within(bad).queryByRole("combobox")).toBeNull();
  });

  test("a folder named General is a type to rename, beside the built-in General", async () => {
    backend({
      note_types: () => ({ types: [entry("General", 1, true, true), entry("general", 2, false)], default_type: null }),
    });
    renderPane();
    await waitFor(() => expect(typeNames()).toEqual(["General", "general"]));
    expect(optionTexts(defaultPopup())).toEqual(["General"]);
    expect(actions("general")).toBeTruthy();
    expect(screen.getByText("General is a built-in type. Rename this type.")).toBeTruthy();
  });

  test("read-only shared settings disable the type popups and menus and show why", async () => {
    backend();
    renderPane("The storage folder is not available.");
    await loaded();
    expect(screen.getByText(/The storage folder is not available\. Types can’t be changed now\./)).toBeTruthy();
    expect(defaultPopup().disabled).toBe(true);
    expect(promptPopup("General").disabled).toBe(true);
    expect(promptPopup("Pathfinder").disabled).toBe(true);
    expect(actions("Pathfinder").disabled).toBe(true);
    expect(actions("Bad:Name").disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Add Type…" }) as HTMLButtonElement).disabled).toBe(true);
  });

  test("a tag row shows a ⋯ menu with Rename… and Delete…, and no popup", async () => {
    backend();
    renderPane();
    const user = userEvent.setup();
    await waitFor(() => expect(tagNames()).toHaveLength(2));
    const urgent = rowsOf("Tags")[1];
    expect(within(urgent).queryByRole("combobox")).toBeNull();
    await user.click(actions("urgent"));
    expect(screen.getAllByRole("menuitem").map((i) => i.textContent)).toEqual(["Rename…", "Delete…"]);
  });

  test("a tag rename calls the command, and a rename onto a tag merges them", async () => {
    const tauri = backend();
    renderPane();
    await waitFor(() => expect(tagNames()).toHaveLength(2));
    await renameTag("urgent", "Urgent now");
    await waitFor(() => expect(tagNames()).toEqual(["Person :: Nathan", "Urgent now"]));
    expect(called(tauri, "rename_tag")[0].args).toEqual({ name: "urgent", newName: "Urgent now" });
    expect(screen.queryByRole("alertdialog")).toBeNull();

    await renameTag("Urgent now", "person::nathan");
    await waitFor(() => expect(tagNames()).toEqual(["Person :: Nathan"]));
    expect(screen.getByRole("status").textContent).toContain("Merged “Urgent now” into “Person :: Nathan”");
    expect(screen.getByText("15 notes")).toBeTruthy();
  });

  test("a tag rename with Escape calls nothing", async () => {
    const tauri = backend();
    renderPane();
    const user = userEvent.setup();
    await waitFor(() => expect(tagNames()).toHaveLength(2));
    await choose("urgent", "Rename…");
    await user.type(screen.getByRole("textbox", { name: "Name of tag urgent" }), "X{Escape}");
    expect(tagNames()).toEqual(["Person :: Nathan", "urgent"]);
    expect(called(tauri, "rename_tag")).toHaveLength(0);
  });

  test("Delete… on a tag shows the count and deletes only after the user confirms", async () => {
    const tauri = backend();
    renderPane();
    const user = userEvent.setup();
    await waitFor(() => expect(tagNames()).toHaveLength(2));
    await choose("Person :: Nathan", "Delete…");
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("12 notes lose this tag");
    expect(called(tauri, "delete_tag")).toHaveLength(0);
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(tagNames()).toEqual(["urgent"]));
    expect(called(tauri, "delete_tag")[0].args).toEqual({ name: "Person :: Nathan" });
  });

  test("the ⋯ menu of the last row opens outside the group that clips", async () => {
    backend();
    renderPane();
    const user = userEvent.setup();
    await waitFor(() => expect(tagNames()).toHaveLength(2));
    const last = rowsOf("Tags")[1];
    await user.click(actions("urgent"));
    const menu = screen.getByRole("menu");
    const clip = last.closest(".overflow-hidden");
    expect(clip).not.toBeNull();
    expect(clip!.contains(menu)).toBe(false);
    expect(menu.className).toContain("fixed");
  });

  test("a change on another computer or in the library reads the lists again", async () => {
    const tauri = backend();
    renderPane();
    await loaded();
    const before = called(tauri, "note_types").length;
    const promptsBefore = called(tauri, "summary_prompts").length;
    await tauri.emit("settings-changed", null);
    await waitFor(() => expect(called(tauri, "note_types").length).toBeGreaterThan(before));
    await waitFor(() => expect(called(tauri, "summary_prompts").length).toBeGreaterThan(promptsBefore));
    const tagsBefore = called(tauri, "list_tags").length;
    await tauri.emit("library-changed", { changed: [], removed: [] });
    await waitFor(() => expect(called(tauri, "list_tags").length).toBeGreaterThan(tagsBefore));
  });
});

const typesNamed = (...names: string[]): NoteTypeList => ({
  types: names.map((name) => entry(name, 0)),
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
    await renameType("Projects", "Work");
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
