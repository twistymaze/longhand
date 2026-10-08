import { describe, expect, mock, test } from "bun:test";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { SegmentedControl, type Segment } from "./SegmentedControl";

const items: Segment<string>[] = [
  { value: "summary", label: "Summary" },
  { value: "notes", label: "My Notes" },
  { value: "transcript", label: "Transcript" },
];

function Harness({ kind, onChange }: { kind?: "tabs" | "radio"; onChange: (v: string, click: boolean) => void }) {
  const [value, setValue] = useState("summary");
  return (
    <SegmentedControl
      label="Show"
      kind={kind}
      items={items}
      value={value}
      onChange={(v, click) => {
        setValue(v);
        onChange(v, click);
      }}
    />
  );
}

describe("SegmentedControl", () => {
  test("the arrow keys move the selection and the focus, and wrap at the ends", () => {
    const onChange = mock((_v: string, _click: boolean) => {});
    render(<Harness onChange={onChange} />);
    const list = screen.getByRole("tablist", { name: "Show" });
    fireEvent.keyDown(list, { key: "ArrowRight" });
    expect(screen.getByRole("tab", { name: "My Notes" }).getAttribute("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(screen.getByRole("tab", { name: "My Notes" }));
    fireEvent.keyDown(list, { key: "ArrowLeft" });
    fireEvent.keyDown(list, { key: "ArrowLeft" });
    expect(screen.getByRole("tab", { name: "Transcript" }).getAttribute("aria-selected")).toBe("true");
    expect(onChange.mock.calls).toEqual([
      ["notes", false],
      ["summary", false],
      ["transcript", false],
    ]);
  });

  test("only the chosen segment is in the tab order", () => {
    render(<Harness onChange={() => {}} />);
    expect(screen.getAllByRole("tab").map((t) => t.tabIndex)).toEqual([0, -1, -1]);
  });

  test("a click chooses a segment and reports a click", async () => {
    const onChange = mock((_v: string, _click: boolean) => {});
    render(<Harness onChange={onChange} />);
    await userEvent.setup().click(screen.getByRole("tab", { name: "Transcript" }));
    expect(onChange.mock.calls).toEqual([["transcript", true]]);
  });

  test("the radio kind is a radio group with checked segments", () => {
    render(<Harness kind="radio" onChange={() => {}} />);
    const group = screen.getByRole("radiogroup", { name: "Show" });
    expect(screen.getByRole("radio", { name: "Summary" }).getAttribute("aria-checked")).toBe("true");
    fireEvent.keyDown(group, { key: "ArrowRight" });
    expect(screen.getByRole("radio", { name: "My Notes" }).getAttribute("aria-checked")).toBe("true");
  });

  test("an accessory shows inside its segment", () => {
    render(
      <SegmentedControl
        label="Show"
        items={[{ value: "a", label: "A", accessory: <span data-testid="meter" /> }]}
        value="a"
        onChange={() => {}}
      />,
    );
    expect(screen.getByRole("tab", { name: "A" }).contains(screen.getByTestId("meter"))).toBe(true);
  });
});
