import { describe, expect, test, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { NewNoteType, NoteTypeEntry, NoteTypeList, Readiness } from "../bindings";
import { mockTauri } from "../test/tauri";
import { NewNoteButton } from "./NewNoteButton";

const ready: Readiness = {
  microphone_authorized: true,
  system_audio: "allowed",
  destination: { kind: "ready", path: "/x", free_bytes: null },
  armed: true,
  shortcut: { chord: "Alt+Shift+L", registered: true, error: null },
  missing_models: [],
  endpoint_configured: true,
  can_start_note: true,
  note_start_refusal: null,
};

const unavailable: Readiness = {
  ...ready,
  can_start_note: false,
  note_start_refusal: "The storage folder is not available.",
};

function renderButton(records: boolean, readiness: Readiness = ready) {
  const onStart = vi.fn((_record: boolean) => {});
  const view = render(<NewNoteButton records={records} readiness={readiness} onStart={onStart} />);
  return { onStart, view };
}

const main = (name: string) => screen.getByRole("button", { name });
const choices = () => screen.getByRole("button", { name: "New Note Options" });
const reasonOf = (el: HTMLElement) => el.closest("[title]")?.getAttribute("title") ?? null;

describe("NewNoteButton", () => {
  test("with the recording default, the main part records and the menu opens a note without recording", async () => {
    const { onStart } = renderButton(true);
    const user = userEvent.setup();
    await user.click(main("Record New Note"));
    expect(onStart.mock.calls).toEqual([[true]]);
    await user.click(choices());
    await user.click(screen.getByRole("menuitem", { name: "New Note Without Recording" }));
    expect(onStart.mock.calls).toEqual([[true], [false]]);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  test("the menu is as wide as its label, so the label is not cut off", async () => {
    renderButton(true);
    await userEvent.setup().click(choices());
    const classes = screen.getByRole("menu").className.split(" ");
    expect(classes).toContain("w-max");
    expect(classes).not.toContain("w-52");
  });

  test("with the default that does not record, the main part opens a note and the menu records", async () => {
    const { onStart } = renderButton(false);
    const user = userEvent.setup();
    await user.click(main("New Note"));
    expect(onStart.mock.calls).toEqual([[false]]);
    await user.click(choices());
    await user.click(screen.getByRole("menuitem", { name: "Record New Note" }));
    expect(onStart.mock.calls).toEqual([[false], [true]]);
  });

  test("the main part's label follows the setting when it changes", () => {
    const onStart = () => {};
    const view = render(<NewNoteButton records readiness={ready} onStart={onStart} />);
    expect(main("Record New Note")).toBeTruthy();
    view.rerender(<NewNoteButton records={false} readiness={ready} onStart={onStart} />);
    expect(main("New Note")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Record New Note" })).toBeNull();
  });

  test("while a note cannot start, both parts are unavailable and the reason shows", async () => {
    const { onStart } = renderButton(true, unavailable);
    expect(main("Record New Note").hasAttribute("disabled")).toBe(true);
    expect(choices().hasAttribute("disabled")).toBe(true);
    expect(reasonOf(main("Record New Note"))).toBe("The storage folder is not available.");
    await userEvent.setup().click(main("Record New Note"));
    expect(onStart).not.toHaveBeenCalled();
  });

  test("while recording is not ready, a note without recording can still start from the menu", async () => {
    const { onStart } = renderButton(true, { ...ready, armed: false, microphone_authorized: false });
    expect(main("Record New Note").hasAttribute("disabled")).toBe(true);
    expect(reasonOf(main("Record New Note"))).toBe("To record, allow microphone access in Settings.");
    const user = userEvent.setup();
    await user.click(choices());
    await user.click(screen.getByRole("menuitem", { name: "New Note Without Recording" }));
    expect(onStart.mock.calls).toEqual([[false]]);
  });

  test("with the default that does not record, the record choice in the menu is unavailable while recording is not ready", async () => {
    renderButton(false, { ...ready, armed: false, microphone_authorized: false });
    expect(main("New Note").hasAttribute("disabled")).toBe(false);
    await userEvent.setup().click(choices());
    const item = screen.getByRole("menuitem", { name: "Record New Note" });
    expect(item.hasAttribute("disabled")).toBe(true);
    expect(item.getAttribute("title")).toBe("To record, allow microphone access in Settings.");
  });

  // WKWebView does not focus a clicked button. A plain click event, which
  // moves no focus, must still put the focus in the menu, and Escape must
  // give it back to the menu part.
  test("the menu part takes the focus on a click that moves no focus, and Escape gives it back", async () => {
    renderButton(true);
    fireEvent.click(choices());
    expect(choices().getAttribute("aria-expanded")).toBe("true");
    // The first item: with no types read, General.
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "General" }));
    await userEvent.setup().keyboard("{Escape}");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(choices());
  });

  test("a click on the menu part focuses it before the menu opens", () => {
    renderButton(true);
    const button = choices();
    const focus = vi.fn(() => {});
    const original = button.focus.bind(button);
    button.focus = () => {
      focus();
      original();
    };
    fireEvent.click(button);
    expect(focus).toHaveBeenCalled();
  });
});

describe("NewNoteButton: note types (R24)", () => {
  const types: NoteTypeEntry[] = [
    { name: "General", valid: true, shared: false, notes: 4, general: true, prompt_choice: null, prompt_name: null, summary_prompt_id: "topic-summary" },
    { name: "Pathfinder", valid: true, shared: true, notes: 3, general: false, prompt_choice: null, prompt_name: null, summary_prompt_id: "topic-summary" },
    { name: "One on One", valid: true, shared: true, notes: 2, general: false, prompt_choice: null, prompt_name: null, summary_prompt_id: "topic-summary" },
    { name: "Bad:Name", valid: false, shared: false, notes: 1, general: false, prompt_choice: null, prompt_name: null, summary_prompt_id: "topic-summary" },
  ];

  function renderTyped(records = true) {
    const onStart = vi.fn((_record: boolean, _type?: NewNoteType) => {});
    const onTypesChanged = vi.fn((_list: NoteTypeList) => {});
    render(
      <NewNoteButton records={records} readiness={ready} types={types} onStart={onStart} onTypesChanged={onTypesChanged} />,
    );
    return { onStart, onTypesChanged };
  }

  test("the menu lists General, the valid types in order, the other choice and New Type…", async () => {
    mockTauri();
    renderTyped();
    await userEvent.setup().click(choices());
    const menu = screen.getByRole("menu");
    const entries = [...menu.querySelectorAll('[role="menuitem"], [role="separator"]')].map((el) =>
      el.getAttribute("role") === "separator" ? "—" : el.textContent,
    );
    expect(entries).toEqual(["General", "Pathfinder", "One on One", "—", "New Note Without Recording", "—", "New Type…"]);
  });

  test("a type item starts a note of that type in the same way as the main part", async () => {
    mockTauri();
    const { onStart } = renderTyped(false);
    const user = userEvent.setup();
    await user.click(choices());
    await user.click(screen.getByRole("menuitem", { name: "Pathfinder" }));
    await user.click(choices());
    await user.click(screen.getByRole("menuitem", { name: "General" }));
    expect(onStart.mock.calls).toEqual([
      [false, { kind: "named", name: "Pathfinder" }],
      [false, { kind: "general" }],
    ]);
  });

  test("New Type… with a refused name shows the reason in the dialog and starts no note", async () => {
    const tauri = mockTauri({
      add_note_type: () => {
        throw "A type with this name already exists.";
      },
    });
    const { onStart } = renderTyped();
    const user = userEvent.setup();
    await user.click(choices());
    await user.click(screen.getByRole("menuitem", { name: "New Type…" }));
    const field = screen.getByRole("textbox", { name: "Type name" });
    expect(document.activeElement).toBe(field);
    await user.type(field, "pathfinder{Enter}");
    expect((await screen.findByRole("alert")).textContent).toBe("A type with this name already exists.");
    expect(tauri.calls.find((c) => c.cmd === "add_note_type")?.args).toEqual({ name: "pathfinder" });
    expect(screen.getByRole("dialog", { name: "New Type" })).toBeTruthy();
    expect(onStart).not.toHaveBeenCalled();
  });

  test("Escape in the New Type dialog closes it, starts no note and gives the focus back", async () => {
    mockTauri();
    const { onStart } = renderTyped();
    const user = userEvent.setup();
    await user.click(choices());
    await user.click(screen.getByRole("menuitem", { name: "New Type…" }));
    await user.type(screen.getByRole("textbox", { name: "Type name" }), "Campaign");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(choices());
    expect(onStart).not.toHaveBeenCalled();
  });

  test("Create adds the type and starts a note of that type", async () => {
    const added: NoteTypeList = {
      types: [...types, { name: "Campaign", valid: true, shared: true, notes: 0, general: false, prompt_choice: null, prompt_name: null, summary_prompt_id: "topic-summary" }],
      default_type: null,
    };
    mockTauri({ add_note_type: () => added });
    const { onStart, onTypesChanged } = renderTyped();
    const user = userEvent.setup();
    await user.click(choices());
    await user.click(screen.getByRole("menuitem", { name: "New Type…" }));
    await user.type(screen.getByRole("textbox", { name: "Type name" }), "  campaign ");
    await user.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(onStart.mock.calls).toEqual([[true, { kind: "named", name: "Campaign" }]]));
    expect(onTypesChanged.mock.calls).toEqual([[added]]);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  test("Cancel while the type is created starts no note", async () => {
    let finish: (l: NoteTypeList) => void = () => {};
    const tauri = mockTauri({ add_note_type: () => new Promise<NoteTypeList>((r) => (finish = r)) });
    const { onStart } = renderTyped();
    const user = userEvent.setup();
    await user.click(choices());
    await user.click(screen.getByRole("menuitem", { name: "New Type…" }));
    await user.type(screen.getByRole("textbox", { name: "Type name" }), "Campaign");
    await user.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "add_note_type")).toBe(true));
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    await act(async () => finish({ types: [...types, { name: "Campaign", valid: true, shared: true, notes: 0, general: false, prompt_choice: null, prompt_name: null, summary_prompt_id: "topic-summary" }], default_type: null }));
    await act(() => new Promise((r) => setTimeout(r, 20)));
    expect(onStart).not.toHaveBeenCalled();
  });
});
