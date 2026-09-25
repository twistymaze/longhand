import { describe, expect, test } from "bun:test";
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockTauri } from "../test/tauri";
import { NotesTab } from "./NotesTab";

/// A promise and the function that resolves it.
function deferred() {
  let resolve: (v: null) => void = () => {};
  const promise = new Promise<null>((r) => {
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

const saves = (tauri: ReturnType<typeof mockTauri>) =>
  tauri.calls.filter((c) => c.cmd === "set_session_notes").map((c) => c.args);

describe("NotesTab", () => {
  test("an error that reads the notes shows on the tab", async () => {
    mockTauri({
      session_notes: () => {
        throw "the notes file cannot be read";
      },
    });
    render(<NotesTab sessionId="s" autoFocus={false} />);
    expect(await screen.findByText("the notes file cannot be read")).toBeTruthy();
    expect(screen.queryByRole("textbox", { name: "My Notes" })).toBeNull();
  });

  test("an error that saves the notes shows after typing", async () => {
    mockTauri({
      session_notes: () => "hello",
      set_session_notes: () => {
        throw "the disk is full";
      },
    });
    render(<NotesTab sessionId="s" autoFocus={false} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await typeAndSend(box, "!");
    expect(await screen.findByText("the disk is full")).toBeTruthy();
  });

  test("saves go one at a time, and only the latest waiting document is sent", async () => {
    const first = deferred();
    let n = 0;
    const tauri = mockTauri({
      session_notes: () => "",
      set_session_notes: () => (n++ === 0 ? first.promise : null),
    });
    render(<NotesTab sessionId="s" autoFocus={false} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await typeAndSend(box, "1");
    await typeAndSend(box, "2");
    await typeAndSend(box, "3");
    await act(async () => {});
    expect(saves(tauri)).toEqual([{ sessionId: "s", markdown: "1" }]);

    await act(async () => first.resolve(null));
    await act(async () => {});
    expect(saves(tauri)).toEqual([
      { sessionId: "s", markdown: "1" },
      { sessionId: "s", markdown: "123" },
    ]);
  });

  test("a document that waits when the tab closes goes to its own session", async () => {
    const first = deferred();
    let n = 0;
    const tauri = mockTauri({
      session_notes: () => "",
      set_session_notes: () => (n++ === 0 ? first.promise : null),
    });
    const view = render(<NotesTab sessionId="a" autoFocus={false} />);
    const box = await screen.findByRole("textbox", { name: "My Notes" });
    await typeAndSend(box, "1");
    await userEvent.setup().type(box, "2");
    // The editor sends its pending document when it closes.
    view.unmount();
    await act(async () => first.resolve(null));
    await act(async () => {});
    expect(saves(tauri)).toEqual([
      { sessionId: "a", markdown: "1" },
      { sessionId: "a", markdown: "12" },
    ]);
  });
});
