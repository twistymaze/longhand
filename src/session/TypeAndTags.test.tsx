import { describe, expect, mock, test } from "bun:test";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { NoteTypeList, TypeChangeResult } from "../bindings";
import type { Notice } from "./TitleField";
import { type CommandHandler, mockTauri } from "../test/tauri";
import { TypeAndTags } from "./TypeAndTags";

const typeList: NoteTypeList = {
  types: [
    { name: "General", valid: true, shared: false, notes: 2, general: true, prompt_choice: null, summary_prompt_id: "topic-summary" },
    { name: "One on One", valid: true, shared: true, notes: 4, general: false, prompt_choice: null, summary_prompt_id: "topic-summary" },
    { name: "Pathfinder", valid: true, shared: true, notes: 30, general: false, prompt_choice: null, summary_prompt_id: "topic-summary" },
    { name: "Bad:Name", valid: false, shared: false, notes: 1, general: false, prompt_choice: null, summary_prompt_id: "topic-summary" },
  ],
  default_type: null,
};

const tagList = [
  { text: "Person::Nathan", key: "person::nathan", count: 2 },
  { text: "urgent", key: "urgent", count: 1 },
];

function mockRow(extra: Record<string, CommandHandler> = {}) {
  return mockTauri({
    note_types: () => typeList,
    list_tags: () => tagList,
    set_note_type: (): TypeChangeResult => ({ kind: "done" }),
    set_note_tags: (args) => args.add,
    ...extra,
  });
}

type Props = { noteType: string | null; tags: string[]; recording?: boolean };

function renderRow(props: Props) {
  const onNotice = mock((_n: Notice | null) => {});
  const onChanged = mock(() => {});
  const ui = (p: Props) => (
    <TypeAndTags
      sessionId="s1"
      present
      noteType={p.noteType}
      tags={p.tags}
      recording={p.recording ?? false}
      onNotice={onNotice}
      onChanged={onChanged}
    />
  );
  const view = render(ui(props));
  return { ...view, onNotice, onChanged, user: userEvent.setup(), rerenderWith: (p: Props) => view.rerender(ui(p)) };
}

const row = () => screen.getByRole("group", { name: "Type and tags" });
const typeButton = () => screen.getByRole("button", { name: /^Type: / });

describe("TypeAndTags", () => {
  test("the row shows the type and each tag with a control to remove it", () => {
    mockRow();
    renderRow({ noteType: "One on One", tags: ["Person::Nathan", "urgent"] });
    expect(typeButton().textContent).toContain("One on One");
    expect(screen.getByRole("button", { name: "Remove Person: Nathan" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove urgent" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Add Tag" })).toBeTruthy();
  });

  test("the type menu lists General, the valid types and New Type…, and the first item has the focus", async () => {
    mockRow();
    const { user } = renderRow({ noteType: "One on One", tags: [] });
    await user.click(typeButton());
    const menu = await screen.findByRole("menu");
    await waitFor(() => expect(within(menu).getAllByRole("menuitemradio").length).toBe(3));
    const labels = [...menu.querySelectorAll("button")].map((i) => i.textContent);
    expect(labels).toEqual(["General", "One on One", "Pathfinder", "New Type…"]);
    expect(within(menu).getByRole("menuitemradio", { name: "One on One" }).getAttribute("aria-checked")).toBe("true");
    expect(document.activeElement).toBe(within(menu).getByRole("menuitemradio", { name: "General" }));
  });

  test("changing the type calls set_note_type and shows the new type", async () => {
    const tauri = mockRow();
    const { user, onNotice, onChanged } = renderRow({ noteType: "One on One", tags: [] });
    await user.click(typeButton());
    await user.click(await screen.findByRole("menuitemradio", { name: "Pathfinder" }));
    await waitFor(() => expect(typeButton().textContent).toContain("Pathfinder"));
    expect(tauri.calls.find((c) => c.cmd === "set_note_type")?.args).toEqual({ sessionId: "s1", noteType: "Pathfinder" });
    expect(onNotice).toHaveBeenLastCalledWith(null);
    expect(onChanged).toHaveBeenCalled();
    expect(screen.queryByRole("menu")).toBeNull();
  });

  test("General calls set_note_type with null", async () => {
    const tauri = mockRow();
    const { user } = renderRow({ noteType: "One on One", tags: [] });
    await user.click(typeButton());
    await user.click(await screen.findByRole("menuitemradio", { name: "General" }));
    await waitFor(() => expect(typeButton().textContent).toContain("General"));
    expect(tauri.calls.find((c) => c.cmd === "set_note_type")?.args).toEqual({ sessionId: "s1", noteType: null });
  });

  // The app moves the folder later without help: the user gets no notice.
  for (const [what, result] of [
    ["a busy note", { kind: "later" }],
    ["a folder that could not move now", { kind: "kept", message: "the folder is in use" }],
  ] as const) {
    test(`${what} shows the new type and no notice`, async () => {
      mockRow({ set_note_type: () => result });
      const { user, onNotice } = renderRow({ noteType: null, tags: [] });
      await user.click(typeButton());
      await user.click(await screen.findByRole("menuitemradio", { name: "Pathfinder" }));
      await waitFor(() => expect(typeButton().textContent).toContain("Pathfinder"));
      await waitFor(() => expect(onNotice).toHaveBeenCalled());
      expect(onNotice).toHaveBeenLastCalledWith(null);
      expect(typeButton().getAttribute("title")).toBeNull();
    });
  }

  test("a refused type change shows the error and keeps the type", async () => {
    mockRow({
      set_note_type: () => {
        throw "The note type isn't in the list";
      },
    });
    const { user, onNotice } = renderRow({ noteType: "One on One", tags: [] });
    await user.click(typeButton());
    await user.click(await screen.findByRole("menuitemradio", { name: "Pathfinder" }));
    await waitFor(() => expect(onNotice).toHaveBeenCalled());
    expect(onNotice).toHaveBeenLastCalledWith({ tone: "negative", text: "The note type isn't in the list." });
    expect(typeButton().textContent).toContain("One on One");
  });

  test("New Type… creates the type and gives it to the note", async () => {
    const tauri = mockRow({
      add_note_type: (args) => ({
        ...typeList,
        types: [...typeList.types, { name: args.name as string, valid: true, shared: true, notes: 0, general: false, prompt_choice: null, summary_prompt_id: "topic-summary" }],
      }),
    });
    const { user } = renderRow({ noteType: null, tags: [] });
    await user.click(typeButton());
    await user.click(await screen.findByRole("menuitem", { name: "New Type…" }));
    const field = screen.getByRole("textbox", { name: "Type name" });
    expect(document.activeElement).toBe(field);
    await user.keyboard("Customer Call{Enter}");
    await waitFor(() => expect(typeButton().textContent).toContain("Customer Call"));
    expect(tauri.calls.find((c) => c.cmd === "add_note_type")?.args).toEqual({ name: "Customer Call" });
    expect(tauri.calls.find((c) => c.cmd === "set_note_type")?.args).toEqual({
      sessionId: "s1",
      noteType: "Customer Call",
    });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  test("New Type… with a refused name shows the reason in the dialog and changes nothing", async () => {
    const tauri = mockRow({
      add_note_type: () => {
        throw "A type name can't contain a slash";
      },
    });
    const { user } = renderRow({ noteType: null, tags: [] });
    await user.click(typeButton());
    await user.click(await screen.findByRole("menuitem", { name: "New Type…" }));
    await user.keyboard("a/b{Enter}");
    const dialog = screen.getByRole("dialog", { name: "New Type" });
    expect(await within(dialog).findByText("A type name can't contain a slash.")).toBeTruthy();
    expect(tauri.calls.some((c) => c.cmd === "set_note_type")).toBe(false);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(typeButton());
  });

  test("adding and removing a tag calls set_note_tags", async () => {
    const tauri = mockRow({
      set_note_tags: (args) => {
        const remove = args.remove as string[];
        const add = args.add as string[];
        return [...["Person::Nathan"].filter((t) => !remove.includes(t)), ...add];
      },
    });
    const { user, onChanged } = renderRow({ noteType: null, tags: ["Person::Nathan"] });
    await user.click(screen.getByRole("button", { name: "Add Tag" }));
    await user.keyboard("Follow up{Enter}");
    await screen.findByRole("button", { name: "Remove Follow up" });
    expect(tauri.calls.filter((c) => c.cmd === "set_note_tags").map((c) => c.args)).toEqual([
      { sessionId: "s1", add: ["Follow up"], remove: [] },
    ]);
    await user.click(screen.getByRole("button", { name: "Remove Person: Nathan" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Remove Person: Nathan" })).toBeNull());
    expect(tauri.calls.filter((c) => c.cmd === "set_note_tags").map((c) => c.args)[1]).toEqual({
      sessionId: "s1",
      add: [],
      remove: ["Person::Nathan"],
    });
    expect(onChanged).toHaveBeenCalled();
  });

  test("a refused tag shows the error", async () => {
    mockRow({
      set_note_tags: () => {
        throw "A tag can't contain a line break or a tab.";
      },
    });
    const { user, onNotice } = renderRow({ noteType: null, tags: [] });
    await user.click(screen.getByRole("button", { name: "Add Tag" }));
    await user.keyboard("x{Enter}");
    await waitFor(() =>
      expect(onNotice).toHaveBeenLastCalledWith({ tone: "negative", text: "A tag can't contain a line break or a tab." }),
    );
    expect(screen.queryByRole("button", { name: "Remove x" })).toBeNull();
  });

  test("the picker suggests the tags of the tag list", async () => {
    mockRow();
    const { user } = renderRow({ noteType: null, tags: [] });
    await user.click(screen.getByRole("button", { name: "Add Tag" }));
    await user.keyboard("n");
    await waitFor(() => expect(screen.getByRole("option", { name: "Person: Nathan" })).toBeTruthy());
  });

  test("opening the picker or the type menu takes the selection out of the page text", async () => {
    mockRow();
    const { user } = renderRow({ noteType: null, tags: [] });
    const editor = document.createElement("div");
    editor.contentEditable = "true";
    editor.textContent = "my notes";
    document.body.append(editor);
    const select = () => {
      const range = document.createRange();
      range.selectNodeContents(editor);
      window.getSelection()!.removeAllRanges();
      window.getSelection()!.addRange(range);
    };
    select();
    await user.click(screen.getByRole("button", { name: "Add Tag" }));
    expect(editor.contains(window.getSelection()?.anchorNode ?? null)).toBe(false);
    expect(document.activeElement).toBe(screen.getByRole("combobox"));
    await user.keyboard("{Escape}");
    select();
    await user.click(typeButton());
    expect(editor.contains(window.getSelection()?.anchorNode ?? null)).toBe(false);
    editor.remove();
  });

  test("a tag edit from another computer shows when no recording runs, and the typed tag text stays", async () => {
    mockRow();
    const { user, rerenderWith } = renderRow({ noteType: null, tags: [] });
    await user.click(screen.getByRole("button", { name: "Add Tag" }));
    await user.keyboard("Proj");
    rerenderWith({ noteType: "Pathfinder", tags: ["urgent"] });
    expect(within(row()).getByText("urgent")).toBeTruthy();
    expect(typeButton().textContent).toContain("Pathfinder");
    const field = screen.getByRole("combobox") as HTMLInputElement;
    expect(field.value).toBe("Proj");
    expect(document.activeElement).toBe(field);
  });

  test("during a recording, an edit from another computer waits until the recording stops", async () => {
    mockRow();
    const { rerenderWith } = renderRow({ noteType: "One on One", tags: ["urgent"], recording: true });
    const before = row().innerHTML;
    rerenderWith({ noteType: "Pathfinder", tags: ["urgent", "Person::Nathan"], recording: true });
    expect(row().innerHTML).toBe(before);
    rerenderWith({ noteType: "Pathfinder", tags: ["urgent", "Person::Nathan"], recording: false });
    expect(typeButton().textContent).toContain("Pathfinder");
    expect(screen.getByRole("button", { name: "Remove Person: Nathan" })).toBeTruthy();
  });

  test("during a recording, the user's own tag edit shows at once", async () => {
    mockRow({ set_note_tags: (args) => ["urgent", ...(args.add as string[])] });
    const { user } = renderRow({ noteType: null, tags: ["urgent"], recording: true });
    await user.click(screen.getByRole("button", { name: "Add Tag" }));
    await user.keyboard("Follow up{Enter}");
    await screen.findByRole("button", { name: "Remove Follow up" });
  });

  test("a change to the shared settings loads the type list again", async () => {
    let list = typeList;
    const tauri = mockRow({ note_types: () => list });
    const { user } = renderRow({ noteType: null, tags: [] });
    await waitFor(() => expect(tauri.calls.filter((c) => c.cmd === "note_types").length).toBe(1));
    list = { ...typeList, types: [{ name: "Standup", valid: true, shared: true, notes: 0, general: false, prompt_choice: null, summary_prompt_id: "topic-summary" }] };
    await act(() => tauri.emit("settings-changed", { settings: {} }));
    await waitFor(() => expect(tauri.calls.filter((c) => c.cmd === "note_types").length).toBe(2));
    await user.click(typeButton());
    expect(await screen.findByRole("menuitemradio", { name: "Standup" })).toBeTruthy();
  });

  test("a type read that ends after a newer read does not replace the newer list", async () => {
    const reads: ((l: NoteTypeList) => void)[] = [];
    const tauri = mockRow({ note_types: () => new Promise<NoteTypeList>((r) => reads.push(r)) });
    const { user } = renderRow({ noteType: null, tags: [] });
    await waitFor(() => expect(reads).toHaveLength(1));
    await act(() => tauri.emit("settings-changed", { settings: {} }));
    await waitFor(() => expect(reads).toHaveLength(2));
    const one = (name: string): NoteTypeList => ({ types: [{ name, valid: true, shared: true, notes: 0, general: false, prompt_choice: null, summary_prompt_id: "topic-summary" }], default_type: null });
    await act(async () => reads[1](one("New")));
    await act(async () => reads[0](one("Old")));
    await act(() => new Promise((r) => setTimeout(r, 20)));
    // The click reads the list again; that read stays open here.
    await user.click(typeButton());
    const menu = await screen.findByRole("menu");
    expect([...menu.querySelectorAll("button")].map((i) => i.textContent)).toEqual(["General", "New", "New Type…"]);
  });

  test("Escape while the type is created does not give the type to the note", async () => {
    let finish: (l: NoteTypeList) => void = () => {};
    const tauri = mockRow({ add_note_type: () => new Promise<NoteTypeList>((r) => (finish = r)) });
    const { user, onChanged } = renderRow({ noteType: null, tags: [] });
    await user.click(typeButton());
    await user.click(await screen.findByRole("menuitem", { name: "New Type…" }));
    await user.keyboard("Customer Call{Enter}");
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "add_note_type")).toBe(true));
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    await act(async () =>
      finish({ ...typeList, types: [...typeList.types, { name: "Customer Call", valid: true, shared: true, notes: 0, general: false, prompt_choice: null, summary_prompt_id: "topic-summary" }] }),
    );
    await act(() => new Promise((r) => setTimeout(r, 20)));
    expect(tauri.calls.some((c) => c.cmd === "set_note_type")).toBe(false);
    expect(onChanged).not.toHaveBeenCalled();
    expect(typeButton().textContent).toContain("General");
  });
});
