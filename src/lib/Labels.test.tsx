import { describe, expect, mock, test } from "bun:test";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SCOPE_TINT, scopeTone, scopeTones, TagLabel, tagKey, tagParts, TypeLabel } from "./Labels";

// The WCAG contrast ratio of two sRGB colours.
const rgb = (hex: string) => [1, 3, 5].map((i) => Number.parseInt(hex.slice(i, i + 2), 16));
const luminance = (c: number[]) => {
  const [r, g, b] = c.map((v) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const contrast = (a: number[], b: number[]) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};
const over = (top: number[], bottom: number[], alpha: number) => top.map((v, i) => v * alpha + bottom[i] * (1 - alpha));

// The window, group and hover backgrounds of index.css, as opaque colours.
const lightBackgrounds = ["#f3f1ec", "#fffdfa", "#e9e6df"];
const darkBackgrounds = ["#1d1b18", "#2c2925", "#36332e"];

describe("tag text", () => {
  test("the part before the first :: is the scope, and spaces at its ends are removed", () => {
    expect(tagParts("Person :: Nathan")).toEqual({ scope: "Person", value: "Nathan" });
    expect(tagParts("a::b::c")).toEqual({ scope: "a", value: "b::c" });
  });

  test("a tag with no :: or an empty part has no scope", () => {
    expect(tagParts("urgent")).toEqual({ scope: null, value: "urgent" });
    expect(tagParts("::x")).toEqual({ scope: null, value: "::x" });
    expect(tagParts("x:: ")).toEqual({ scope: null, value: "x::" });
  });

  test("the key ignores case and spaces at the ends and around ::", () => {
    expect(tagKey("  Person :: Nathan ")).toBe(tagKey("person::nathan"));
    expect(tagKey("Big Idea")).not.toBe(tagKey("BigIdea"));
  });
});

describe("scope colour", () => {
  test("the same scope gives the same colour in two renders, in any case and spacing", () => {
    const first = render(<TagLabel text="Person::Nathan" />);
    const a = first.container.querySelector("[data-scope]")?.getAttribute("style");
    first.unmount();
    const second = render(<TagLabel text=" person :: Ana" />);
    const b = second.container.querySelector("[data-scope]")?.getAttribute("style");
    expect(a).toBeTruthy();
    expect(a).toBe(b);
    expect(scopeTone("Person")).toBe(scopeTone("PERSON "));
  });

  test("the scopes use more than one colour", () => {
    const used = new Set(["Person", "Project", "Client", "Topic", "Team", "Place"].map((s) => scopeTone(s).light));
    expect(used.size).toBeGreaterThan(2);
  });

  test("each colour meets 4.5:1 on its tint in light and dark", () => {
    for (const tone of scopeTones) {
      for (const bg of lightBackgrounds) {
        const back = over(rgb(tone.light), rgb(bg), SCOPE_TINT);
        expect(contrast(rgb(tone.light), back)).toBeGreaterThanOrEqual(4.5);
      }
      for (const bg of darkBackgrounds) {
        const back = over(rgb(tone.dark), rgb(bg), SCOPE_TINT);
        expect(contrast(rgb(tone.dark), back)).toBeGreaterThanOrEqual(4.5);
      }
    }
  });
});

describe("labels", () => {
  test("a scoped tag shows two parts, and its accessible name has the scope and the value", () => {
    const { container } = render(<TagLabel text="Person::Nathan" />);
    expect(screen.getByText("Person: Nathan").className).toContain("sr-only");
    const parts = container.querySelectorAll("[aria-hidden] > span");
    expect([...parts].map((p) => p.textContent)).toEqual(["Person", "Nathan"]);
  });

  test("a tag with no scope shows one part", () => {
    const { container } = render(<TagLabel text="urgent" />);
    expect(container.querySelector("[data-scope]")).toBeNull();
    expect(container.textContent).toBe("urgent");
  });

  test("the remove control names the tag and reports a click", async () => {
    const onRemove = mock(() => {});
    render(<TagLabel text="Person::Nathan" onRemove={onRemove} />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Remove Person: Nathan" }));
    expect(onRemove).toHaveBeenCalledTimes(1);
  });

  test("a type label is a different kind of label and reads as a type", () => {
    const { container } = render(<TypeLabel name="1:1" />);
    expect(screen.getByText("Type:").className).toContain("sr-only");
    expect(container.textContent).toBe("Type: 1:1");
    expect(container.querySelector("[data-label='type']")).toBeTruthy();
  });
});
