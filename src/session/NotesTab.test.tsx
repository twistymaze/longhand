import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { notesSaved } from "../lib/notesSaves";
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

  test("a later edit after a conflict makes one more draft, not one for each save", async () => {
    let file = notes("Old notes", "f0");
    const conflicts: string[] = [];
    let drafts = 0;
    const tauri = mockTauri({
      load_session_notes: () => file,
      // As save_notes does: a save with another fingerprint writes a draft.
      save_session_notes: (args) => {
        if (args.loadedFingerprint === file.fingerprint) return saved(file.fingerprint);
        return { kind: "conflict", draft: `/n/Planning/notes (Longhand conflict ${++drafts}).md` };
      },
    });
    render(<NotesTab sessionId="s" autoFocus={false} onConflict={(d) => conflicts.push(d)} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    // Another computer changes the file, and the sync has not told the tab.
    file = notes("Notes from the other computer", "f1");
    await typeAndSend(box, "1");
    await act(async () => {});
    // The user types on after the conflict: these saves wait.
    await typeAndSend(box, "2");
    await act(async () => {});
    await typeAndSend(box, "3");
    await act(async () => {});
    expect(saves(tauri).map((s) => s.markdown)).toEqual(["1Old notes"]);

    // When the user stops, the latest edit goes into one draft, and the
    // editor shows the file.
    await act(async () => {
      clock += RELOAD_AFTER_INPUT_MS;
      await new Promise((r) => setTimeout(r, RELOAD_AFTER_INPUT_MS + 50));
    });
    expect(await screen.findByText("Notes from the other computer")).toBeTruthy();
    expect(saves(tauri)).toEqual([
      { sessionId: "s", markdown: "1Old notes", loadedFingerprint: "f0" },
      { sessionId: "s", markdown: "1Old notes23", loadedFingerprint: "f0" },
    ]);
    expect(drafts).toBe(2);
    expect(conflicts).toEqual(["notes (Longhand conflict 1).md", "notes (Longhand conflict 2).md"]);
  });

  test("an edit that waits after a conflict is sent when the tab closes", async () => {
    let file = notes("Old notes", "f0");
    const tauri = mockTauri({
      load_session_notes: () => file,
      save_session_notes: () => {
        file = notes("Notes from the other computer", "f1");
        return { kind: "conflict", draft: "/n/Planning/notes (Longhand conflict).md" };
      },
    });
    const view = render(<NotesTab sessionId="s" autoFocus={false} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await typeAndSend(box, "1");
    await act(async () => {});
    await typeAndSend(box, "2");
    await act(async () => {});
    view.unmount();
    await act(async () => {});
    expect(saves(tauri)).toEqual([
      { sessionId: "s", markdown: "1Old notes", loadedFingerprint: "f0" },
      { sessionId: "s", markdown: "1Old notes2", loadedFingerprint: "f0" },
    ]);
  });

  test("an older read that ends last does not replace a newer read", async () => {
    const reads = [deferred<unknown>(), deferred<unknown>()];
    let n = 0;
    mockTauri({ load_session_notes: () => reads[n++].promise });
    const view = render(<NotesTab sessionId="s" autoFocus={false} synced={0} />);
    await act(async () => {});
    view.rerender(<NotesTab sessionId="s" autoFocus={false} synced={1} />);
    await act(async () => {});
    expect(n).toBe(2);
    await act(async () => reads[1].resolve(notes("New notes", "f1")));
    expect(await screen.findByText("New notes")).toBeTruthy();
    await act(async () => reads[0].resolve(notes("Old notes", "f0")));
    await act(async () => {});
    expect(screen.getByRole("textbox", { name: "My Notes" }).textContent).toBe("New notes");
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

  test("a mouse edit that waits when a sync reloads the notes goes into a draft", async () => {
    let file = notes("- [ ] task", "f0");
    const markdowns: string[] = [];
    let drafts = 0;
    const tauri = mockTauri({
      load_session_notes: () => file,
      // As save_notes does: a save with another fingerprint writes a draft.
      save_session_notes: (args) => {
        if (args.loadedFingerprint === file.fingerprint) return saved(file.fingerprint);
        return { kind: "conflict", draft: `/n/Planning/notes (Longhand conflict ${++drafts}).md` };
      },
    });
    const view = render(
      <NotesTab sessionId="s" autoFocus={false} synced={0} onMarkdown={(m) => markdowns.push(m)} />,
    );
    await screen.findByText("task");
    // A click on the checkbox fires no key or input event, and the editor
    // waits before it sends the change.
    act(() => {
      fireEvent.click(document.querySelector(".task-checkbox")!);
    });
    file = notes("- [ ] task from the other computer", "f1");
    view.rerender(
      <NotesTab sessionId="s" autoFocus={false} synced={1} onMarkdown={(m) => markdowns.push(m)} />,
    );
    expect(await screen.findByText("task from the other computer")).toBeTruthy();
    expect(saves(tauri)).toEqual([{ sessionId: "s", markdown: "- [x] task", loadedFingerprint: "f0" }]);
    expect(drafts).toBe(1);
    // The last text that the page holds is the text that the editor shows.
    expect(markdowns[markdowns.length - 1]).toBe("- [ ] task from the other computer");
  });

  test("a mouse edit that the read flushes after a conflict goes into a draft", async () => {
    let file = notes("- [ ] task", "f0");
    const first = deferred<unknown>();
    let n = 0;
    const tauri = mockTauri({
      load_session_notes: () => file,
      save_session_notes: () => {
        if (n++ === 0) return first.promise;
        return { kind: "conflict", draft: "/n/Planning/notes (Longhand conflict 2).md" };
      },
    });
    render(<NotesTab sessionId="s" autoFocus={false} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    const checkbox = () => document.querySelector(".task-checkbox")!;
    act(() => {
      fireEvent.click(checkbox());
    });
    act(() => {
      fireEvent.blur(box);
    });
    // While the save runs, another click waits in the editor. No key or
    // input event counts it as typing.
    act(() => {
      fireEvent.click(checkbox());
    });
    file = notes("- [ ] task from the other computer", "f1");
    clock += RELOAD_AFTER_INPUT_MS;
    await act(async () => first.resolve({ kind: "conflict", draft: "/n/Planning/notes (Longhand conflict).md" }));
    expect(await screen.findByText("task from the other computer")).toBeTruthy();
    expect(saves(tauri)).toEqual([
      { sessionId: "s", markdown: "- [x] task", loadedFingerprint: "f0" },
      { sessionId: "s", markdown: "- [ ] task", loadedFingerprint: "f0" },
    ]);
  });

  test("a leave waits for an edit that a conflict holds while the leave runs", async () => {
    const file = notes("Old notes", "f0");
    const first = deferred<unknown>();
    const later = deferred<unknown>();
    let n = 0;
    const tauri = mockTauri({
      load_session_notes: () => file,
      save_session_notes: () => (n++ === 0 ? first.promise : later.promise),
    });
    render(<NotesTab sessionId="s" autoFocus={false} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await typeAndSend(box, "1");
    // The user types on while the first save runs, then leaves the note.
    await userEvent.setup().type(box, "2");
    let left = false;
    const leave = notesSaved("s").then(() => (left = true));
    await act(async () => {});
    // The running save conflicts: the waiting edit is held.
    await act(async () => first.resolve({ kind: "conflict", draft: "/n/Planning/notes (Longhand conflict).md" }));
    await waitFor(() => expect(saves(tauri)).toHaveLength(2));
    expect(saves(tauri)[1]).toEqual({ sessionId: "s", markdown: "1Old notes2", loadedFingerprint: "f0" });
    expect(left).toBe(false);
    await act(async () => later.resolve({ kind: "conflict", draft: "/n/Planning/notes (Longhand conflict 2).md" }));
    await leave;
    expect(left).toBe(true);
  });

  test("an edit after a conflict is saved when the read finds the same notes", async () => {
    const file = notes("Old notes", "f0");
    const first = deferred<unknown>();
    let n = 0;
    const tauri = mockTauri({
      load_session_notes: () => file,
      // The first save cannot read notes.md and conflicts. The store does
      // not change.
      save_session_notes: () =>
        n++ === 0 ? first.promise : saved("f2"),
    });
    render(<NotesTab sessionId="s" autoFocus={false} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await typeAndSend(box, "1");
    // The user stops typing, then the save conflicts and the tab reads the
    // notes again: they did not change.
    clock += RELOAD_AFTER_INPUT_MS;
    await act(async () => first.resolve({ kind: "conflict", draft: "/n/Planning/notes (Longhand conflict).md" }));
    await act(async () => {});
    await typeAndSend(box, "2");
    await waitFor(() => expect(saves(tauri)).toHaveLength(2));
    expect(saves(tauri)[1]).toEqual({ sessionId: "s", markdown: "1Old notes2", loadedFingerprint: "f0" });
  });

  test("a leave of the note sends an edit that waits after a conflict, and waits for it", async () => {
    const file = notes("Old notes", "f0");
    const later = deferred<unknown>();
    let n = 0;
    const tauri = mockTauri({
      load_session_notes: () => file,
      save_session_notes: () =>
        n++ === 0 ? { kind: "conflict", draft: "/n/Planning/notes (Longhand conflict).md" } : later.promise,
    });
    render(<NotesTab sessionId="s" autoFocus={false} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await typeAndSend(box, "1");
    await act(async () => {});
    // The user types on: the edit waits in the tab, and the editor still
    // holds the last key.
    await typeAndSend(box, "2");
    await userEvent.setup().type(box, "3");
    let left = false;
    const leave = notesSaved("s").then(() => (left = true));
    await waitFor(() => expect(saves(tauri)).toHaveLength(2));
    expect(saves(tauri)[1]).toEqual({ sessionId: "s", markdown: "1Old notes23", loadedFingerprint: "f0" });
    await act(async () => {});
    expect(left).toBe(false);
    await act(async () => later.resolve({ kind: "conflict", draft: "/n/Planning/notes (Longhand conflict 2).md" }));
    await leave;
    expect(left).toBe(true);
  });
});
