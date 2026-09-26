import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockTauri } from "../test/tauri";
import { NotesTab, RELOAD_AFTER_INPUT_MS } from "./NotesTab";

/// A promise and the function that resolves it.
function deferred<T>() {
  let resolve: (v: T) => void = () => {};
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

/// Type text into the editor, then blur it, so the editor sends the document.
async function typeAndSend(box: HTMLElement, text: string) {
  await userEvent.setup().type(box, text);
  act(() => {
    fireEvent.blur(box);
  });
}

const saved = (fingerprint: string) => ({ kind: "saved", fingerprint });
const notes = (markdown: string, fingerprint: string) => ({ markdown, fingerprint });

const saves = (tauri: ReturnType<typeof mockTauri>) =>
  tauri.calls.filter((c) => c.cmd === "save_session_notes").map((c) => c.args);

// The clock that the tab reads for the time since the last keystroke.
let clock = 0;
let clockSpy: ReturnType<typeof spyOn> | undefined;
beforeEach(() => {
  clock = 100_000;
  clockSpy = spyOn(performance, "now").mockImplementation(() => clock);
});
afterEach(() => clockSpy?.mockRestore());

describe("NotesTab", () => {
  test("an error that reads the notes shows on the tab", async () => {
    mockTauri({
      load_session_notes: () => {
        throw "the notes file cannot be read";
      },
    });
    render(<NotesTab sessionId="s" autoFocus={false} />);
    expect(await screen.findByText("the notes file cannot be read")).toBeTruthy();
    expect(screen.queryByRole("textbox", { name: "My Notes" })).toBeNull();
  });

  test("an error that saves the notes shows after typing", async () => {
    mockTauri({
      load_session_notes: () => notes("hello", "f0"),
      save_session_notes: () => {
        throw "the disk is full";
      },
    });
    render(<NotesTab sessionId="s" autoFocus={false} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await typeAndSend(box, "!");
    expect(await screen.findByText("the disk is full")).toBeTruthy();
  });

  test("each save sends the fingerprint of the notes that the editor holds", async () => {
    let n = 0;
    const tauri = mockTauri({
      load_session_notes: () => notes("", "f0"),
      save_session_notes: () => saved(`f${++n}`),
    });
    render(<NotesTab sessionId="s" autoFocus={false} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await typeAndSend(box, "1");
    await act(async () => {});
    await typeAndSend(box, "2");
    await act(async () => {});
    expect(saves(tauri)).toEqual([
      { sessionId: "s", markdown: "1", loadedFingerprint: "f0" },
      { sessionId: "s", markdown: "12", loadedFingerprint: "f1" },
    ]);
  });

  test("saves go one at a time, and only the latest waiting document is sent", async () => {
    const first = deferred<unknown>();
    let n = 0;
    const tauri = mockTauri({
      load_session_notes: () => notes("", "f0"),
      save_session_notes: () => (n++ === 0 ? first.promise : saved("f2")),
    });
    render(<NotesTab sessionId="s" autoFocus={false} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await typeAndSend(box, "1");
    await typeAndSend(box, "2");
    await typeAndSend(box, "3");
    await act(async () => {});
    expect(saves(tauri).map((s) => s.markdown)).toEqual(["1"]);

    await act(async () => first.resolve(saved("f1")));
    await act(async () => {});
    expect(saves(tauri)).toEqual([
      { sessionId: "s", markdown: "1", loadedFingerprint: "f0" },
      { sessionId: "s", markdown: "123", loadedFingerprint: "f1" },
    ]);
  });

  test("a document that waits when the tab closes goes to its own session", async () => {
    const first = deferred<unknown>();
    let n = 0;
    const tauri = mockTauri({
      load_session_notes: () => notes("", "f0"),
      save_session_notes: () => (n++ === 0 ? first.promise : saved("f2")),
    });
    const view = render(<NotesTab sessionId="a" autoFocus={false} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await typeAndSend(box, "1");
    await userEvent.setup().type(box, "2");
    // The editor sends its pending document when it closes.
    view.unmount();
    await act(async () => first.resolve(saved("f1")));
    await act(async () => {});
    expect(saves(tauri).map((s) => [s.sessionId, s.markdown])).toEqual([
      ["a", "1"],
      ["a", "12"],
    ]);
  });

  // Covers KTD10.
  test("a synced change reloads the notes when no save waits", async () => {
    let file = notes("Old notes", "f0");
    mockTauri({ load_session_notes: () => file });
    const view = render(<NotesTab sessionId="s" autoFocus={false} synced={0} />);
    await screen.findByText("Old notes");
    file = notes("Notes from the other computer", "f1");
    view.rerender(<NotesTab sessionId="s" autoFocus={false} synced={1} />);
    expect(await screen.findByText("Notes from the other computer")).toBeTruthy();
  });

  // Covers KTD10.
  test("a synced change does not replace a local edit that still waits", async () => {
    let file = notes("Old notes", "f0");
    const save = deferred<unknown>();
    mockTauri({
      load_session_notes: () => file,
      save_session_notes: () => save.promise,
    });
    const view = render(<NotesTab sessionId="s" autoFocus={false} synced={0} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await typeAndSend(box, "!");
    file = notes("Notes from the other computer", "f1");
    view.rerender(<NotesTab sessionId="s" autoFocus={false} synced={1} />);
    await act(async () => {});
    expect(box.textContent).toBe("!Old notes");

    // The save ends, but the user typed less than 2 seconds ago.
    await act(async () => save.resolve(saved("f2")));
    await act(async () => {});
    expect(screen.getByRole("textbox", { name: "My Notes" }).textContent).toBe("!Old notes");

    // After 2 seconds with no input, the synced notes show.
    await act(async () => {
      clock += RELOAD_AFTER_INPUT_MS;
      await new Promise((r) => setTimeout(r, RELOAD_AFTER_INPUT_MS + 50));
    });
    expect(await screen.findByText("Notes from the other computer")).toBeTruthy();
  });

  // Covers AE8, KTD10.
  test("a save that conflicts names the draft, and the editor then shows the file", async () => {
    let file = notes("Old notes", "f0");
    const conflicts: string[] = [];
    mockTauri({
      load_session_notes: () => file,
      save_session_notes: () => {
        file = notes("Notes from the other computer", "f1");
        return { kind: "conflict", draft: "/n/Planning/notes (Longhand conflict 2026-09-26 1412).md" };
      },
    });
    render(<NotesTab sessionId="s" autoFocus={false} onConflict={(d) => conflicts.push(d)} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await typeAndSend(box, "!");
    await act(async () => {});
    expect(conflicts).toEqual(["notes (Longhand conflict 2026-09-26 1412).md"]);

    // The editor loads the file when the user has stopped typing.
    await act(async () => {
      clock += RELOAD_AFTER_INPUT_MS;
      await new Promise((r) => setTimeout(r, RELOAD_AFTER_INPUT_MS + 50));
    });
    expect(await screen.findByText("Notes from the other computer")).toBeTruthy();
  });

  // Covers AE7.
  test("an editor that is not editable takes no input", async () => {
    const tauri = mockTauri({ load_session_notes: () => notes("", "f0") });
    const view = render(<NotesTab sessionId="s" autoFocus={false} editable={false} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    expect(box.getAttribute("contenteditable")).toBe("false");
    await typeAndSend(box, "x");
    expect(box.textContent).toBe("");
    expect(saves(tauri)).toEqual([]);

    view.rerender(<NotesTab sessionId="s" autoFocus={false} editable />);
    expect(box.getAttribute("contenteditable")).toBe("true");
  });
});
