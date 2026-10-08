import { describe, expect, mock, test } from "bun:test";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { TagEntry } from "../bindings";
import { mockTauri } from "../test/tauri";
import {
  assignScopeTones,
  DARK_TEXT,
  ScopeTonesProvider,
  scopeTone,
  scopeTones,
  TagLabel,
  tagKey,
  tagParts,
  TypeLabel,
  WHITE_TEXT,
} from "./Labels";

const tagList = (...texts: string[]): TagEntry[] => texts.map((text) => ({ text, key: tagKey(text), count: 1 }));
// The colour of the value part of a rendered tag.
const toneOf = (container: HTMLElement) => container.querySelector("[data-label='tag']")?.getAttribute("style");

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
// The hue of an sRGB colour, in degrees.
const hue = (hex: string) => {
  const [r, g, b] = rgb(hex).map((v) => v / 255);
  const max = Math.max(r, g, b);
  const d = max - Math.min(r, g, b);
  const h = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return (h * 60 + 360) % 360;
};
const hueDistance = (a: string, b: string) => {
  const d = Math.abs(hue(a) - hue(b));
  return Math.min(d, 360 - d);
};

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
    const a = toneOf(first.container);
    first.unmount();
    const second = render(<TagLabel text=" person :: Ana" />);
    const b = toneOf(second.container);
    expect(a).toBeTruthy();
    expect(a).toBe(b);
    expect(scopeTone("Person")).toBe(scopeTone("PERSON "));
  });

  test("the scopes use more than one colour", () => {
    const used = new Set(["Person", "Project", "Client", "Topic", "Team", "Place"].map((s) => scopeTone(s).light));
    expect(used.size).toBeGreaterThan(2);
  });

  test("Person and Project have the same hash colour, but different library colours", () => {
    expect(scopeTone("Person")).toBe(scopeTone("Project"));
    const tones = assignScopeTones(["Person", "Project"]);
    expect(tones.get("person")).toBeTruthy();
    expect(tones.get("project")).toBeTruthy();
    expect(tones.get("person")).not.toBe(tones.get("project"));
  });

  test("the same scopes in any order, case or count give the same colours", () => {
    const scopes = ["Person", "Vendor", "Topic", "Project", "Client", "Team", "Place"];
    const a = assignScopeTones(scopes);
    const b = assignScopeTones([...scopes].reverse().flatMap((s) => [s.toUpperCase(), ` ${s} `]));
    expect([...b.entries()].sort()).toEqual([...a.entries()].sort());
  });

  test("a scope keeps its hash colour when that colour is free, and a collision takes the next free colour", () => {
    const tones = assignScopeTones(["Project", "Person", "Topic"]);
    expect(tones.get("person")).toBe(scopeTone("Person"));
    expect(tones.get("topic")).toBe(scopeTone("Topic"));
    const next = scopeTones[(scopeTones.indexOf(scopeTone("Project")) + 1) % scopeTones.length];
    expect(tones.get("project")).toBe(next);
  });

  test("up to one scope for each colour, no two scopes share a colour", () => {
    const scopes = Array.from({ length: scopeTones.length }, (_, i) => `Scope ${i}`);
    expect(new Set(assignScopeTones(scopes).values()).size).toBe(scopeTones.length);
    const more = assignScopeTones([...scopes, "One more"]);
    expect(more.size).toBe(scopeTones.length + 1);
    expect(new Set(more.values()).size).toBe(scopeTones.length);
  });

  test("with the provider, tags in Person and Project show different colours, and a new scope updates the colours", async () => {
    const state = { tags: tagList("Person::Nathan", "Topic::Loot") };
    const tauri = mockTauri({ list_tags: () => state.tags });
    const { container } = render(
      <ScopeTonesProvider>
        <span data-testid="person">
          <TagLabel text="Person::Nathan" />
        </span>
        <span data-testid="project">
          <TagLabel text="Project::Acme" />
        </span>
      </ScopeTonesProvider>,
    );
    const person = () => toneOf(screen.getByTestId("person"));
    const project = () => toneOf(screen.getByTestId("project"));
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "list_tags")).toBe(true));
    // "Project" is not in the library yet: it has its hash colour, as "Person" has.
    await waitFor(() => expect(person()).toBe(project()));
    state.tags = tagList("Person::Nathan", "Topic::Loot", "Project::Acme");
    await act(() => tauri.emit("library-changed", { changed: [], removed: [] }));
    await waitFor(() => expect(person()).not.toBe(project()));
    expect(container.textContent).toContain("Acme");
  });

  test("there are 10 colours, and each one is different", () => {
    expect(scopeTones).toHaveLength(10);
    expect(new Set(scopeTones.map((t) => t.light)).size).toBe(10);
    expect(new Set(scopeTones.map((t) => t.dark)).size).toBe(10);
  });

  test("the text on each fill is white or near-black and meets 4.5:1, in light and dark", () => {
    for (const tone of scopeTones) {
      for (const [fill, text] of [
        [tone.light, tone.lightText],
        [tone.dark, tone.darkText],
      ]) {
        expect([WHITE_TEXT, DARK_TEXT]).toContain(text);
        expect(contrast(rgb(text), rgb(fill))).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  test("two colours next to each other in the list have very different hues", () => {
    scopeTones.forEach((tone, i) => {
      const next = scopeTones[(i + 1) % scopeTones.length];
      expect(hueDistance(tone.light, next.light)).toBeGreaterThanOrEqual(45);
      expect(hueDistance(tone.dark, next.dark)).toBeGreaterThanOrEqual(45);
    });
  });
});

describe("labels", () => {
  test("a scoped tag shows two parts, and its accessible name has the scope and the value", () => {
    const { container } = render(<TagLabel text="Person::Nathan" />);
    expect(screen.getByText("Person: Nathan").className).toContain("sr-only");
    const parts = container.querySelectorAll("[aria-hidden] > span");
    expect([...parts].map((p) => p.textContent)).toEqual(["Person", "Nathan"]);
  });

  test("the value has the scope colour, and the scope is neutral", () => {
    const { container } = render(<TagLabel text="Person::Nathan" />);
    const scope = container.querySelector("[data-scope]");
    const value = container.querySelector("[data-value]");
    expect(scope?.textContent).toBe("Person");
    expect(scope?.className).toContain("bg-fill");
    expect(scope?.className).toContain("text-label-secondary");
    expect(scope?.className).not.toContain("--scope-");
    expect(value?.textContent).toBe("Nathan");
    // The scope colour is the fill of the value, under white or near-black text.
    expect(value?.className).toContain("bg-[var(--scope-light)]");
    expect(value?.className).toContain("dark:bg-[var(--scope-dark)]");
    expect(value?.className).toContain("text-[color:var(--scope-text-light)]");
    expect(value?.className).toContain("dark:text-[color:var(--scope-text-dark)]");
    const style = container.querySelector<HTMLElement>("[data-label='tag']")?.style;
    const tone = scopeTone("Person");
    expect(style?.getPropertyValue("--scope-light")).toBe(tone.light);
    expect(style?.getPropertyValue("--scope-text-light")).toBe(tone.lightText);
    expect(style?.getPropertyValue("--scope-dark")).toBe(tone.dark);
    expect(style?.getPropertyValue("--scope-text-dark")).toBe(tone.darkText);
  });

  test("the remove control of a scoped tag has the text colour of the fill", () => {
    const { container } = render(<TagLabel text="Person::Nathan" onRemove={() => {}} />);
    const button = screen.getByRole("button", { name: "Remove Person: Nathan" });
    expect(button.className).toContain("text-current");
    // The pressed state uses the text colour, so it shows on every fill.
    expect(button.className).toContain("hover:bg-[color-mix(in_srgb,currentColor_22%,transparent)]");
    const holder = button.parentElement;
    expect(holder?.className).toContain("bg-[var(--scope-light)]");
    expect(holder?.className).toContain("text-[color:var(--scope-text-light)]");
    expect(container.querySelector("[data-value]")).toBeTruthy();
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
