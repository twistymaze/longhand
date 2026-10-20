import { describe, expect, test } from "vitest";
import { render, screen } from "@testing-library/react";
import { mockTauri } from "../test/tauri";
import { LicencesWindow } from "./LicencesWindow";

const text = [
  "Rust crates",
  "",
  "ggml-example 1.0.0",
  "Copyright (c) The ggml authors",
  "MIT licence text",
  "",
  "interface packages",
  "",
  "react 19.2.8",
  "",
  "Native libraries",
  "",
  "ONNX Runtime 1.22.0",
].join("\n");

describe("LicencesWindow", () => {
  test("shows the text from the command in a block that allows selection", async () => {
    mockTauri({ licence_text: () => text });
    render(<LicencesWindow />);
    const block = await screen.findByTestId("licence-text");
    expect(block.textContent).toBe(text);
    expect(block.className).toContain("select-text");
    expect(block.className).toContain("whitespace-pre-wrap");
  });

  test("marks the three part titles as headings, each before the text of its part", async () => {
    mockTauri({ licence_text: () => text });
    render(<LicencesWindow />);
    await screen.findByTestId("licence-text");
    const headings = screen.getAllByRole("heading");
    expect(headings.map((h) => h.textContent)).toEqual(["Rust crates", "interface packages", "Native libraries"]);
    expect(headings[0].nextSibling?.textContent).toContain("ggml-example 1.0.0");
    expect(headings[1].nextSibling?.textContent).toContain("react 19.2.8");
    expect(headings[2].nextSibling?.textContent).toContain("ONNX Runtime 1.22.0");
  });

  test("a title inside a line is not a heading", async () => {
    mockTauri({ licence_text: () => "Rust crates\nThe Rust crates below\n" });
    render(<LicencesWindow />);
    await screen.findByTestId("licence-text");
    expect(screen.getAllByRole("heading")).toHaveLength(1);
  });

  test("asks the person to open the window again when the command fails", async () => {
    mockTauri({});
    render(<LicencesWindow />);
    expect(await screen.findByText(/Close this window and open it again/)).toBeTruthy();
    expect(screen.queryByTestId("licence-text")).toBeNull();
  });
});
