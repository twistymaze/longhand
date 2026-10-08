import { describe, expect, mock, test } from "bun:test";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { TagEntry } from "../bindings";
import { mockTauri } from "../test/tauri";
import {
  assignScopeTones,
  SCOPE_TINT,
  ScopeTonesProvider,
  scopeTone,
  scopeTones,
  TagLabel,
  tagKey,
  tagParts,
  TypeLabel,
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

  test("Person and Vendor have the same hash colour, but different library colours", () => {
    expect(scopeTone("Person")).toBe(scopeTone("Vendor"));
    const tones = assignScopeTones(["Person", "Vendor"]);
    expect(tones.get("person")).toBeTruthy();
    expect(tones.get("vendor")).toBeTruthy();
    expect(tones.get("person")).not.toBe(tones.get("vendor"));
  });

  test("the same scopes in any order, case or count give the same colours", () => {
    const scopes = ["Person", "Vendor", "Topic", "Project", "Client", "Team", "Place"];
    const a = assignScopeTones(scopes);
    const b = assignScopeTones([...scopes].reverse().flatMap((s) => [s.toUpperCase(), ` ${s} `]));
    expect([...b.entries()].sort()).toEqual([...a.entries()].sort());
  });

  test("a scope keeps its hash colour when that colour is free, and a collision takes the next free colour", () => {
    const tones = assignScopeTones(["Vendor", "Person", "Topic"]);
    expect(tones.get("person")).toBe(scopeTone("Person"));
    expect(tones.get("topic")).toBe(scopeTone("Topic"));
    const next = scopeTones[(scopeTones.indexOf(scopeTone("Vendor")) + 1) % scopeTones.length];
    expect(tones.get("vendor")).toBe(next);
  });

  test("up to one scope for each colour, no two scopes share a colour", () => {
    const scopes = Array.from({ length: scopeTones.length }, (_, i) => `Scope ${i}`);
    expect(new Set(assignScopeTones(scopes).values()).size).toBe(scopeTones.length);
    const more = assignScopeTones([...scopes, "One more"]);
    expect(more.size).toBe(scopeTones.length + 1);
    expect(new Set(more.values()).size).toBe(scopeTones.length);
  });

  test("with the provider, tags in Person and Vendor show different colours, and a new scope updates the colours", async () => {
    const state = { tags: tagList("Person::Nathan", "Topic::Loot") };
    const tauri = mockTauri({ list_tags: () => state.tags });
    const { container } = render(
      <ScopeTonesProvider>
        <span data-testid="person">
          <TagLabel text="Person::Nathan" />
        </span>
        <span data-testid="vendor">
          <TagLabel text="Vendor::Acme" />
        </span>
      </ScopeTonesProvider>,
    );
    const person = () => toneOf(screen.getByTestId("person"));
    const vendor = () => toneOf(screen.getByTestId("vendor"));
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "list_tags")).toBe(true));
    // "Vendor" is not in the library yet: it has its hash colour, as "Person" has.
    await waitFor(() => expect(person()).toBe(vendor()));
    state.tags = tagList("Person::Nathan", "Topic::Loot", "Vendor::Acme");
    await act(() => tauri.emit("library-changed", { changed: [], removed: [] }));
    await waitFor(() => expect(person()).not.toBe(vendor()));
    expect(container.textContent).toContain("Acme");
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

  test("the value has the scope colour, and the scope is neutral", () => {
    const { container } = render(<TagLabel text="Person::Nathan" />);
    const scope = container.querySelector("[data-scope]");
    const value = container.querySelector("[data-value]");
    expect(scope?.textContent).toBe("Person");
    expect(scope?.className).toContain("bg-fill");
    expect(scope?.className).toContain("text-label-secondary");
    expect(scope?.className).not.toContain("--scope-");
    expect(value?.textContent).toBe("Nathan");
    expect(value?.className).toContain("text-[color:var(--scope-light)]");
    expect(value?.className).toContain("bg-[color-mix(in_srgb,var(--scope-light)_14%,transparent)]");
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
