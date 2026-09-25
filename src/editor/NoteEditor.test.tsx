import { afterEach, beforeEach, describe, expect, jest, test } from "bun:test";
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { NoteEditor, SAVE_DEBOUNCE_MS, SAVE_MAX_WAIT_MS } from "./NoteEditor";

beforeEach(() => {
  jest.useFakeTimers();
});

afterEach(() => {
  jest.useRealTimers();
});

function setup(markdown: string, autoFocus = true) {
  const changes: string[] = [];
  const view = render(
    <NoteEditor markdown={markdown} onChange={(m) => changes.push(m)} autoFocus={autoFocus} />,
  );
  // No delay between keys: the test moves the clock itself.
  const user = userEvent.setup({ delay: null });
  return { changes, view, user, box: screen.getByRole("textbox") };
}

function advance(ms: number) {
  act(() => {
    jest.advanceTimersByTime(ms);
  });
}

describe("NoteEditor", () => {
  test("shows the markdown as formatted text", () => {
    const { box } = setup("# Plan\n\n- **first**", false);
    expect(box.querySelector("h1")?.textContent).toBe("Plan");
    expect(box.querySelector("ul li strong")?.textContent).toBe("first");
  });

  test("autoFocus puts the cursor in the editor", () => {
    const { box } = setup("hello");
    expect(document.activeElement).toBe(box);
  });

  test("sends the document once after typing stops", async () => {
    const { changes, user, box } = setup("hello");
    await user.type(box, " world");

    // The fake clock can fire a timer up to a millisecond early, so the test
    // checks well before and at the debounce time.
    advance(SAVE_DEBOUNCE_MS - 50);
    expect(changes).toEqual([]);
    advance(50);
    expect(changes).toEqual(["hello world"]);
    advance(5000);
    expect(changes).toEqual(["hello world"]);
  });

  test("sends a pending change on blur, and only once", async () => {
    const { changes, user, box } = setup("hello");
    await user.type(box, "!");
    act(() => {
      fireEvent.blur(box);
    });
    expect(changes).toEqual(["hello!"]);
    advance(5000);
    expect(changes).toEqual(["hello!"]);
  });

  test("sends nothing on blur when nothing changed", () => {
    const { changes, box } = setup("hello");
    act(() => {
      fireEvent.blur(box);
    });
    advance(5000);
    expect(changes).toEqual([]);
  });

  test("sends at least once each second during continuous typing", async () => {
    const { changes, user, box } = setup("");
    // 3 s of typing, one key each 100 ms, with no pause as long as the
    // debounce.
    for (let i = 0; i < 30; i++) {
      await user.type(box, "a");
      advance(100);
    }
    expect(changes.length).toBeGreaterThanOrEqual(2);
    expect(changes.length).toBeGreaterThanOrEqual(Math.floor(3000 / SAVE_MAX_WAIT_MS));
    advance(SAVE_DEBOUNCE_MS);
    expect(changes[changes.length - 1]).toBe("a".repeat(30));
  });

  test("sends a pending change on unmount", async () => {
    const { changes, user, box, view } = setup("hello");
    await user.type(box, "!");
    view.unmount();
    expect(changes).toEqual(["hello!"]);
  });

  test("a click on a task checkbox writes - [x]", () => {
    const { changes, box } = setup("- [ ] send notes", false);
    const checkbox = box.querySelector("input[type=checkbox]") as HTMLInputElement;
    expect(checkbox.checked).toBe(false);
    act(() => {
      fireEvent.click(checkbox);
    });
    advance(SAVE_DEBOUNCE_MS);
    expect(changes).toEqual(["- [x] send notes"]);
    expect((box.querySelector("input[type=checkbox]") as HTMLInputElement).checked).toBe(true);
  });

  test("typed markdown becomes a heading", async () => {
    const { changes, user, box } = setup("");
    await user.type(box, "## Actions");
    advance(SAVE_DEBOUNCE_MS);
    expect(box.querySelector("h2")?.textContent).toBe("Actions");
    expect(changes[changes.length - 1]).toBe("## Actions");
  });
});
