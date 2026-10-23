import { afterEach, describe, expect, test } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Readiness } from "../bindings";
import { mockTauri } from "../test/tauri";
import { MicrophoneAccessRow, SystemAudioAccessRow } from "./Permissions";

const realAgent = navigator.userAgent;

function setUserAgent(value: string) {
  Object.defineProperty(navigator, "userAgent", { value, configurable: true });
}

afterEach(() => setUserAgent(realAgent));

function renderMicrophone(authorized: boolean, handlers: Record<string, (args: Record<string, unknown>) => unknown> = {}) {
  const tauri = mockTauri(handlers);
  let refreshed = 0;
  render(
    <MicrophoneAccessRow
      authorized={authorized}
      onRefresh={async () => {
        refreshed += 1;
      }}
    />,
  );
  const row = () => screen.getByText("Microphone access").closest(".settings-row") as HTMLElement;
  return { tauri, row, refreshed: () => refreshed };
}

describe("the microphone access row", () => {
  test("allowed: says so and shows no button", () => {
    setUserAgent("Macintosh");
    const { row } = renderMicrophone(true);
    expect(row().textContent).toContain("Allowed");
    expect(within(row()).queryAllByRole("button")).toHaveLength(0);
  });

  test("on another platform, a microphone that is not allowed shows no button", () => {
    setUserAgent("Windows NT 10.0");
    const { row } = renderMicrophone(false);
    expect(row().textContent).toContain("Longhand needs permission to hear the room");
    expect(within(row()).queryAllByRole("button")).toHaveLength(0);
  });

  test("Allow… asks, waits for the answer, and refreshes readiness", async () => {
    setUserAgent("Macintosh");
    let checks = 0;
    const { tauri, row, refreshed } = renderMicrophone(false, {
      "plugin:macos-permissions|request_microphone_permission": () => null,
      "plugin:macos-permissions|check_microphone_permission": () => {
        checks += 1;
        return checks >= 2;
      },
    });
    await userEvent.setup().click(within(row()).getByRole("button", { name: "Allow…" }));
    expect(await within(row()).findByRole("button", { name: "Waiting…" })).toBeTruthy();
    await waitFor(() => expect(refreshed()).toBe(1), { timeout: 3000 });
    expect(tauri.calls.filter((c) => c.cmd === "plugin:macos-permissions|request_microphone_permission")).toHaveLength(1);
    expect(checks).toBe(2);
  });
});

function renderSystemAudio(
  state: Readiness["system_audio"],
  handlers: Record<string, (args: Record<string, unknown>) => unknown> = {},
) {
  const tauri = mockTauri(handlers);
  let refreshed = 0;
  render(
    <SystemAudioAccessRow
      permission={state}
      onRefresh={async () => {
        refreshed += 1;
      }}
    />,
  );
  const row = () => screen.getByText("App audio access").closest(".settings-row") as HTMLElement;
  return { tauri, row, refreshed: () => refreshed };
}

describe("the App audio access row", () => {
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

  test("Allow… asks, checks the status again, and refreshes", async () => {
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
