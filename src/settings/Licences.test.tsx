import { describe, expect, test } from "bun:test";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ModelStatus } from "../bindings";
import { Licences } from "./Licences";

const model = (id: string, name: string, role: ModelStatus["role"]): ModelStatus => ({
  id,
  role,
  name,
  description: "",
  streaming: false,
  size_bytes: 1,
  required: true,
  state: { state: "present" },
  repository: `example/${id}`,
  revision: "0123456789abcdef",
  licence: { name: "MIT", url: `https://example.com/${id}/LICENSE`, attribution: `${name} authors` },
});

describe("Licences", () => {
  test("each model is one row, and a click shows its details", async () => {
    render(<Licences models={[model("a", "Speech A", "recognition"), model("b", "Voices B", "embedding")]} />);
    const rows = screen.getAllByRole("button");
    expect(rows.map((r) => r.getAttribute("aria-expanded"))).toEqual(["false", "false"]);
    expect(screen.queryByText("Speech A authors")).toBeNull();

    await userEvent.setup().click(screen.getByRole("button", { name: /Speech A/ }));
    expect(screen.getByRole("button", { name: /Speech A/ }).getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("Speech A authors")).toBeTruthy();
    expect(screen.getByText("https://example.com/a/LICENSE")).toBeTruthy();
    expect(screen.getByText("0123456789ab")).toBeTruthy();
    expect(screen.queryByText("Voices B authors")).toBeNull();
  });
});
