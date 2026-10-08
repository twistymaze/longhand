import { describe, expect, mock, test } from "bun:test";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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
  const onStart = mock((_record: boolean) => {});
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
    // The first item: with no types, "No type".
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "No type" }));
    await userEvent.setup().keyboard("{Escape}");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(choices());
  });

  test("a click on the menu part focuses it before the menu opens", () => {
    renderButton(true);
    const button = choices();
    const focus = mock(() => {});
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
    { name: "Pathfinder", valid: true, shared: true, notes: 3 },
    { name: "One on One", valid: true, shared: true, notes: 2 },
    { name: "Bad:Name", valid: false, shared: false, notes: 1 },
  ];

  function renderTyped(records = true) {
    const onStart = mock((_record: boolean, _type?: NewNoteType) => {});
    const onTypesChanged = mock((_list: NoteTypeList) => {});
    render(
      <NewNoteButton records={records} readiness={ready} types={types} onStart={onStart} onTypesChanged={onTypesChanged} />,
    );
    return { onStart, onTypesChanged };
  }

  test("the menu lists the valid types in order, No type, the other choice and New Type…", async () => {
    mockTauri();
    renderTyped();
    await userEvent.setup().click(choices());
    const menu = screen.getByRole("menu");
    const entries = [...menu.querySelectorAll('[role="menuitem"], [role="separator"]')].map((el) =>
      el.getAttribute("role") === "separator" ? "—" : el.textContent,
    );
    expect(entries).toEqual(["Pathfinder", "One on One", "No type", "—", "New Note Without Recording", "—", "New Type…"]);
  });

  test("a type item starts a note of that type in the same way as the main part", async () => {
    mockTauri();
    const { onStart } = renderTyped(false);
    const user = userEvent.setup();
    await user.click(choices());
    await user.click(screen.getByRole("menuitem", { name: "Pathfinder" }));
    await user.click(choices());
    await user.click(screen.getByRole("menuitem", { name: "No type" }));
    expect(onStart.mock.calls).toEqual([
      [false, { kind: "named", name: "Pathfinder" }],
      [false, { kind: "no_type" }],
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
      types: [...types, { name: "Campaign", valid: true, shared: true, notes: 0 }],
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
});
