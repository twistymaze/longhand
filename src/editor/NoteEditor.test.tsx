import { afterEach, beforeEach, describe, expect, jest, test } from "bun:test";
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { NoteEditor, type NoteEditorProps, SAVE_DEBOUNCE_MS, SAVE_MAX_WAIT_MS } from "./NoteEditor";

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

// The Settings form of the editor (KTD7). My Notes passes none of these props.
describe("NoteEditor in Settings", () => {
  const selectAll = /Mac/.test(navigator.platform) ? "{Meta>}a{/Meta}" : "{Control>}a{/Control}";

  function field(props: Partial<NoteEditorProps> = {}) {
    const changes: string[] = [];
    const all: NoteEditorProps = {
      markdown: "hello",
      onChange: (m: string) => changes.push(m),
      ...props,
    };
    const view = render(<NoteEditor {...all} />);
    const user = userEvent.setup({ delay: null });
    return { changes, view, user, all };
  }

  test("with immediate, one typed character calls onChange at once", async () => {
    const { changes, user } = field({ immediate: true, autoFocus: true });
    await user.type(screen.getByRole("textbox"), "!");
    // No timer moved: the change came without a delay.
    expect(changes).toEqual(["hello!"]);
    advance(5000);
    expect(changes).toEqual(["hello!"]);
  });

  test("labelledBy and describedBy give the textbox its name and hint", () => {
    render(
      <>
        <span id="a-label">Background</span>
        <span id="a-hint">Names and terms.</span>
        <span id="b-label">Summary prompt</span>
        <NoteEditor markdown="one" onChange={() => {}} labelledBy="a-label" describedBy="a-hint" />
        <NoteEditor markdown="two" onChange={() => {}} labelledBy="b-label" />
      </>,
    );
    const a = screen.getByRole("textbox", { name: "Background" });
    expect(a.textContent).toBe("one");
    expect(a.getAttribute("aria-describedby")).toBe("a-hint");
    expect(a.getAttribute("aria-label")).toBeNull();
    expect(screen.getByRole("textbox", { name: "Summary prompt" }).textContent).toBe("two");
    expect(screen.queryByRole("textbox", { name: "My Notes" })).toBeNull();
  });

  test("without the new props, the textbox keeps the name My Notes and the placeholder", () => {
    field({ markdown: "" });
    const box = screen.getByRole("textbox", { name: "My Notes" });
    expect(box.querySelector("[data-placeholder]")?.getAttribute("data-placeholder")).toContain("Take notes.");
  });

  test("placeholder null shows no note placeholder in an empty field", () => {
    field({ markdown: "", placeholder: null, labelledBy: "x" });
    const box = screen.getByRole("textbox");
    expect(box.querySelector("[data-placeholder]")).toBeNull();
    expect(box.textContent).toBe("");
  });

  test("className adds a class to the textbox", () => {
    field({ className: "settings-markdown" });
    expect(screen.getByRole("textbox").classList.contains("settings-markdown")).toBe(true);
  });

  test("onBlur fires when focus leaves the editor", () => {
    let blurs = 0;
    field({ onBlur: () => blurs++, autoFocus: true });
    act(() => {
      fireEvent.blur(screen.getByRole("textbox"));
    });
    expect(blurs).toBe(1);
  });

  test("a mount with a text and no edit calls onChange zero times", () => {
    const { changes, view } = field({ markdown: "- a\n- b\nline one\nline two", immediate: true });
    act(() => {
      fireEvent.blur(screen.getByRole("textbox"));
    });
    advance(5000);
    view.unmount();
    expect(changes).toEqual([]);
  });

  test("the selection toolbar shows only while the editor is editable", async () => {
    const { user, view, all } = field({ autoFocus: true });
    await user.keyboard(selectAll);
    expect(window.getSelection()!.isCollapsed).toBe(false);
    expect(screen.getByRole("toolbar", { name: "Format selection" })).toBeTruthy();
    view.rerender(<NoteEditor {...all} editable={false} />);
    expect(screen.queryByRole("toolbar", { name: "Format selection" })).toBeNull();
  });

  test("the / menu shows only while the editor is editable", async () => {
    const { user, view, all } = field({ markdown: "", autoFocus: true });
    await user.type(screen.getByRole("textbox"), "/");
    expect(screen.getByRole("button", { name: "Heading 1" })).toBeTruthy();
    view.rerender(<NoteEditor {...all} editable={false} />);
    expect(screen.queryByRole("button", { name: "Heading 1" })).toBeNull();
  });

  test("a read-only editor marks itself read-only and sends no change", async () => {
    const { changes, user, view } = field({
      markdown: "- [ ] send notes",
      editable: false,
      immediate: true,
      labelledBy: "x",
    });
    const box = screen.getByRole("textbox");
    expect(box.getAttribute("aria-readonly")).toBe("true");
    await user.type(box, "abc");
    act(() => {
      fireEvent.click(box.querySelector("input[type=checkbox]")!);
    });
    advance(5000);
    view.unmount();
    expect(changes).toEqual([]);
  });
});
