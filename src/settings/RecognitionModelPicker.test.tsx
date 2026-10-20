import { describe, expect, test } from "vitest";
import { render, screen } from "@testing-library/react";
import type { ModelStatus } from "../bindings";
import { RecognitionModelPicker } from "./RecognitionModelPicker";

function model(id: string): ModelStatus {
  return {
    speaker_preferences: null,
    id,
    role: "recognition",
    name: id,
    description: "",
    streaming: false,
    languages: ["en"],
    recommended: false,
    size_bytes: 1000,
    required: false,
    state: { state: "present" },
    repository: "",
    revision: "",
    licence: {},
    assessment: null,
  } as unknown as ModelStatus;
}

function renderPicker() {
  render(
    <RecognitionModelPicker
      models={[model("alpha"), model("beta")]}
      value="alpha"
      onChange={() => {}}
      modelAction={(m) => <button type="button">Delete {m.name}</button>}
    />,
  );
  const row = (name: string) =>
    screen.getByRole("radio", { name }).parentElement as HTMLElement;
  return { row };
}

describe("RecognitionModelPicker row hover (R12)", () => {
  test("a row that is not selected has the hover class on its wrapper, not on its radio button", () => {
    const { row } = renderPicker();
    const wrapper = row("beta");
    expect(wrapper.className.split(/\s+/)).toContain("hover:bg-row-hover");
    expect(wrapper.className.split(/\s+/)).toContain("rounded-lg");
    const radio = screen.getByRole("radio", { name: "beta" });
    expect(radio.className).not.toContain("hover:bg-row-hover");
    // The Delete… button is inside the wrapper, so its area gets the highlight.
    expect(
      wrapper.contains(screen.getByRole("button", { name: "Delete beta" })),
    ).toBe(true);
  });

  test("the selected row has no hover class and keeps its selected background", () => {
    const { row } = renderPicker();
    const wrapper = row("alpha");
    expect(wrapper.className).not.toContain("hover:bg-row-hover");
    expect(wrapper.className.split(/\s+/)).toContain("bg-accent/10");
    const radio = screen.getByRole("radio", { name: "alpha" });
    expect(radio.className).not.toContain("hover:bg-row-hover");
    expect(radio.className).toContain("focus-visible:outline-2");
    expect(radio.className).toContain("active:bg-fill-pressed");
  });
});
