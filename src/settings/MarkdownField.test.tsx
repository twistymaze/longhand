import { describe, expect, test } from "bun:test";
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { MarkdownField, type MarkdownFieldProps } from "./MarkdownField";

function renderField(props: Partial<MarkdownFieldProps> = {}) {
  const changes: string[] = [];
  let blurs = 0;
  const all: MarkdownFieldProps = {
    id: "background",
    label: "Background",
    hint: "Names, terms and people that summaries use.",
    value: "Erv Walter",
    count: 10,
    limit: 4000,
    onChange: (m) => changes.push(m),
    onBlur: () => blurs++,
    ...props,
  };
  const view = render(<MarkdownField {...all} />);
  return { changes, blurs: () => blurs, view, all, user: userEvent.setup() };
}

const frameOf = (box: HTMLElement) => box.closest("[data-markdown-field]") as HTMLElement;

describe("MarkdownField", () => {
  test("the textbox has the label as its name and the hint as its description", () => {
    renderField();
    const box = screen.getByRole("textbox", { name: "Background" });
    const hint = document.getElementById(box.getAttribute("aria-describedby")!);
    expect(hint?.textContent).toBe("Names, terms and people that summaries use.");
    expect(box.textContent).toBe("Erv Walter");
  });

  test("two fields on one page are found by name", () => {
    render(
      <>
        <MarkdownField id="a" label="Prompt text" hint="How." value="one" count={3} limit={10} onChange={() => {}} />
        <MarkdownField id="b" label="Title instructions" hint="Title." value="two" count={3} limit={10} onChange={() => {}} />
      </>,
    );
    expect(screen.getByRole("textbox", { name: "Prompt text" }).textContent).toBe("one");
    expect(screen.getByRole("textbox", { name: "Title instructions" }).textContent).toBe("two");
  });

  test("a typed character goes to onChange at once", async () => {
    const { changes, user } = renderField();
    const box = screen.getByRole("textbox", { name: "Background" });
    await user.click(box);
    await user.keyboard("!");
    // happy-dom puts the cursor at the start on a click.
    expect(changes).toEqual(["!Erv Walter"]);
  });

  test("the field does not take the focus at mount", () => {
    renderField();
    expect(document.activeElement).not.toBe(screen.getByRole("textbox", { name: "Background" }));
  });

  test("a click on the label puts the focus in the editor", async () => {
    const { user } = renderField();
    await user.click(screen.getByText("Background"));
    expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "Background" }));
  });

  test("onBlur fires when the focus leaves the field", () => {
    const { blurs } = renderField();
    act(() => {
      fireEvent.blur(screen.getByRole("textbox", { name: "Background" }));
    });
    expect(blurs()).toBe(1);
  });

  test("an empty field shows no note placeholder", () => {
    renderField({ value: "", count: 0 });
    const box = screen.getByRole("textbox", { name: "Background" });
    expect(box.querySelector("[data-placeholder]")).toBeNull();
  });

  test("shows the count and the limit", () => {
    renderField({ count: 1234 });
    expect(screen.getByText("1,234 / 4,000")).toBeTruthy();
  });

  test("showCount false hides the count", () => {
    renderField({ showCount: false });
    expect(screen.queryByText("10 / 4,000")).toBeNull();
  });

  test("over the limit, the count and the frame are red", () => {
    renderField({ count: 4001 });
    expect(screen.getByText("4,001 / 4,000").className).toContain("text-negative");
    expect(frameOf(screen.getByRole("textbox", { name: "Background" })).className).toContain("border-negative");
  });

  test("within the limit, the frame has the normal border", () => {
    renderField();
    const frame = frameOf(screen.getByRole("textbox", { name: "Background" }));
    expect(frame.className).toContain("border-control-border");
    expect(frame.className).not.toContain("border-negative");
  });

  test("a read-only field is marked read-only, has no toolbar and sends no change", async () => {
    const { changes, user } = renderField({ readOnly: true, value: "- [ ] send notes" });
    const box = screen.getByRole("textbox", { name: "Background" });
    expect(box.getAttribute("aria-readonly")).toBe("true");
    expect(frameOf(box).className).toContain("text-label-secondary");
    await user.click(box);
    await user.keyboard("/abc");
    expect(screen.queryByRole("button", { name: "Heading 1" })).toBeNull();
    expect(screen.queryByRole("toolbar")).toBeNull();
    act(() => {
      fireEvent.click(box.querySelector("input[type=checkbox]")!);
    });
    expect(changes).toEqual([]);
    expect(box.querySelector("ul li")?.textContent).toContain("send notes");
  });

  test("a mount with a text and no edit calls onChange zero times", () => {
    const { changes, view } = renderField({ value: "line one\nline two\n* item" });
    act(() => {
      fireEvent.blur(screen.getByRole("textbox", { name: "Background" }));
    });
    view.unmount();
    expect(changes).toEqual([]);
  });
});
