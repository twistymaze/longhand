import { describe, expect, test } from "bun:test";
import type { NoteTypeEntry, TagEntry } from "../bindings";
import { generalIsChoice, keepExisting, menuOrder, noFilter, typesInUse } from "./filter";

const type = (name: string, over: Partial<NoteTypeEntry> = {}): NoteTypeEntry => ({
  name,
  valid: true,
  shared: true,
  notes: 0,
  general: false,
  prompt_choice: null,
  summary_prompt_id: "topic-summary",
  ...over,
});
const general = type("General", { shared: false, notes: 3, general: true });
const tag = (text: string): TagEntry => ({ text, key: text.trim().replace(/\s*::\s*/g, "::").toLowerCase(), count: 1 });

describe("types in use (R1, KTD1)", () => {
  test("a list with only General is not in use", () => {
    expect(typesInUse([general])).toBe(false);
  });

  test("a type with no notes puts types in use", () => {
    expect(typesInUse([general, type("Pathfinder")])).toBe(true);
  });

  test("a list that is not read yet is not in use", () => {
    expect(typesInUse(null)).toBe(false);
  });
});

describe("General as a choice (R2, R3, KTD1)", () => {
  const chosen = { ...noFilter, type: { kind: "general" as const } };

  test("a list that is not read yet keeps a chosen General", () => {
    expect(keepExisting(chosen, null, null, generalIsChoice(null))).toBe(chosen);
  });

  test("a list with only General drops a chosen General", () => {
    expect(keepExisting(chosen, [general], null, generalIsChoice([general])).type).toEqual({ kind: "all" });
  });

  test("a list with another type keeps a chosen General, also with no General note", () => {
    const types = [type("General", { shared: false, notes: 0, general: true }), type("Pathfinder")];
    expect(keepExisting(chosen, types, null, generalIsChoice(types))).toBe(chosen);
  });
});

describe("the Tags menu order (R7, KTD6)", () => {
  test("tags with no scope come first, then each scope A-Z with its values A-Z", () => {
    const order = menuOrder([
      tag("Topic::Roadmap"),
      tag("daily"),
      tag("Person::Dana Ortiz"),
      tag("person::alex"),
      tag("Action"),
    ]).map((t) => t.text);
    expect(order).toEqual(["Action", "daily", "person::alex", "Person::Dana Ortiz", "Topic::Roadmap"]);
  });

  test("a scope that differs only in case is one group", () => {
    const order = menuOrder([tag("Person::Bo"), tag("Topic::A"), tag("person::Al")]).map((t) => t.text);
    expect(order).toEqual(["person::Al", "Person::Bo", "Topic::A"]);
  });

  test("the order does not change the list it gets", () => {
    const list = [tag("b"), tag("a")];
    menuOrder(list);
    expect(list.map((t) => t.text)).toEqual(["b", "a"]);
  });
});
