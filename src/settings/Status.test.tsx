import { describe, expect, test } from "bun:test";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Readiness } from "../bindings";
import { mockTauri, shortcutStatuses } from "../test/tauri";
import { StatusPane } from "./Status";

const readiness: Readiness = {
  microphone_authorized: true,
  destination: { kind: "ready", path: "/x", free_bytes: null },
  armed: true,
  shortcut: shortcutStatuses.recording,
  missing_models: [],
  endpoint_configured: true,
};

function renderPane(playResult: () => unknown = () => null) {
  const tauri = mockTauri({
    shortcut_status: () => shortcutStatuses,
    play_alert_sound: playResult,
  });
  render(<StatusPane readiness={readiness} onRefresh={async () => {}} onNavigate={() => {}} />);
  const row = () => screen.getByText("Alert sound").closest(".settings-row") as HTMLElement;
  return { tauri, row };
}

describe("the alert sound row", () => {
  test("says what the sound is for and asks for no test", () => {
    const { row } = renderPane();
    expect(row().textContent).toContain("Plays when recording stops and cannot restart");
    expect(row().textContent).not.toMatch(/Not tested|Heard at setup|Tested/);
  });

  test("Play Sound plays the sound and asks no question", async () => {
    const { tauri, row } = renderPane();
    await userEvent.setup().click(within(row()).getByRole("button", { name: "Play Sound" }));
    expect(tauri.calls.filter((c) => c.cmd === "play_alert_sound")).toHaveLength(1);
    expect(screen.queryByText("Did you hear it?")).toBeNull();
    expect(screen.queryByRole("button", { name: "Yes" })).toBeNull();
    expect(screen.queryByRole("button", { name: "No" })).toBeNull();
    expect(within(row()).getByRole("button", { name: "Play Sound" })).toBeTruthy();
    expect(tauri.calls.some((c) => c.cmd === "confirm_alert_sound")).toBe(false);
  });

  test("a sound that cannot play shows the error", async () => {
    const { row } = renderPane(() => {
      throw "no output device";
    });
    await userEvent.setup().click(within(row()).getByRole("button", { name: "Play Sound" }));
    expect(await within(row()).findByText("no output device")).toBeTruthy();
  });
});
