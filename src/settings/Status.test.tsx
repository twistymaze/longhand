import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Readiness } from "../bindings";
import { mockTauri, shortcutStatuses } from "../test/tauri";
import { StatusPane } from "./Status";
import { features } from "../lib/features";

const readiness: Readiness = {
  microphone_authorized: true,
  system_audio: "allowed",
  destination: { kind: "ready", path: "/x", free_bytes: null },
  armed: true,
  shortcut: shortcutStatuses.recording,
  missing_models: [],
  endpoint_configured: true,
};

function renderPane(playResult: () => unknown = () => null) {
  const tauri = mockTauri({
    shortcut_status: () => shortcutStatuses,
    login_item_status: () => "disabled",
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
    expect(document.body.textContent).not.toMatch(/Not tested|Heard at setup|Tested|Do Not Disturb|silence the notification/);
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

function renderSystemAudio(
  state: Readiness["system_audio"],
  handlers: Record<string, (args: Record<string, unknown>) => unknown> = {},
) {
  const tauri = mockTauri({ shortcut_status: () => shortcutStatuses,
    login_item_status: () => "disabled", ...handlers });
  let refreshed = 0;
  render(
    <StatusPane
      readiness={{ ...readiness, system_audio: state }}
      onRefresh={async () => {
        refreshed += 1;
      }}
      onNavigate={() => {}}
    />,
  );
  const row = () => screen.getByText("Computer audio").closest(".settings-row") as HTMLElement;
  return { tauri, row, refreshed: () => refreshed };
}

test("while Computer audio is hidden, the General pane has no Computer audio row", () => {
  mockTauri({ shortcut_status: () => shortcutStatuses, login_item_status: () => "disabled" });
  render(<StatusPane readiness={{ ...readiness, system_audio: "not_asked" }} onRefresh={async () => {}} onNavigate={() => {}} />);
  expect(screen.queryByText("Computer audio")).toBeNull();
});

describe("the Computer audio permission row", () => {
  beforeEach(() => {
    features.systemAudio = true;
  });
  afterEach(() => {
    features.systemAudio = false;
  });

  test("not asked: says so and offers Allow…", () => {
    const { row } = renderSystemAudio("not_asked");
    expect(row().textContent).toContain("Not allowed yet");
    expect(within(row()).getAllByRole("button").map((b) => b.textContent)).toEqual(["Allow…"]);
  });

  test("denied: says so and opens System Settings", async () => {
    const { tauri, row } = renderSystemAudio("denied", { open_system_audio_settings: () => null });
    expect(row().textContent).toContain("Not allowed");
    expect(row().textContent).not.toContain("Not allowed yet");
    const buttons = within(row()).getAllByRole("button");
    expect(buttons.map((b) => b.textContent)).toEqual(["Open System Settings"]);
    await userEvent.setup().click(buttons[0]);
    expect(tauri.calls.filter((c) => c.cmd === "open_system_audio_settings")).toHaveLength(1);
  });

  test("unknown: cannot check, and opens System Settings", () => {
    const { row } = renderSystemAudio("unknown");
    expect(row().textContent).toContain("Cannot check");
    expect(within(row()).getAllByRole("button").map((b) => b.textContent)).toEqual(["Open System Settings"]);
  });

  test("allowed: says so and shows no button", () => {
    const { row } = renderSystemAudio("allowed");
    expect(row().textContent).toContain("Allowed");
    expect(within(row()).queryAllByRole("button")).toHaveLength(0);
  });

  test("Allow… asks, checks the status again, and refreshes the pane", async () => {
    let checks = 0;
    const { tauri, row, refreshed } = renderSystemAudio("not_asked", {
      request_system_audio_permission: () => "not_asked",
      system_audio_permission: () => {
        checks += 1;
        return checks < 2 ? "not_asked" : "allowed";
      },
    });
    await userEvent.setup().click(within(row()).getByRole("button", { name: "Allow…" }));
    await waitFor(() => expect(refreshed()).toBe(1), { timeout: 3000 });
    expect(tauri.calls.filter((c) => c.cmd === "request_system_audio_permission")).toHaveLength(1);
    expect(checks).toBe(2);
  });
});
