import { describe, expect, test } from "vitest";
import { render, screen } from "@testing-library/react";
import { ScopeTonesProvider } from "../lib/Labels";
import { mockTauri } from "../test/tauri";
import { type SceneName, scenes } from "./scenes";

// Each scene renders the real components with its own command results. A
// command with no result fails the screenshot, so a change to the app that
// needs a new command shows here first.
function renderScene(name: SceneName) {
  const scene = scenes[name]();
  const tauri = mockTauri(scene.handlers);
  render(<ScopeTonesProvider>{scene.view}</ScopeTonesProvider>);
  const unmocked = () =>
    tauri.calls.map((c) => c.cmd).filter((cmd) => !(cmd in scene.handlers) && !cmd.startsWith("plugin:"));
  return { unmocked };
}

describe("screenshot scenes", () => {
  test("the recording scene shows the notes being written and the recording", async () => {
    const { unmocked } = renderScene("recording");
    expect(await screen.findByText("Weekly team meeting")).toBeTruthy();
    expect(screen.getByRole("tab", { name: "My Notes" }).getAttribute("aria-selected")).toBe("true");
    expect(unmocked()).toEqual([]);
  });

  test("the summary scene shows the summary of the recorded team meeting", async () => {
    const { unmocked } = renderScene("summary");
    expect(await screen.findByText("Card update flow")).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Summary" }).getAttribute("aria-selected")).toBe("true");
    expect(unmocked()).toEqual([]);
  });

  test("the library scene lists every note, with the team meeting selected", async () => {
    const { unmocked } = renderScene("library");
    expect(await screen.findByText(/The Lantern Keeper's Bargain/)).toBeTruthy();
    expect(screen.getAllByRole("option")).toHaveLength(11);
    expect(unmocked()).toEqual([]);
  });
});
