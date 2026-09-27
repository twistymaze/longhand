import { expect, test } from "bun:test";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockTauri, shortcutStatuses } from "../test/tauri";
import { StatusPane } from "./Status";

function setup(handlers: Parameters<typeof mockTauri>[0] = {}) {
  const mock = mockTauri({ shortcut_status: () => shortcutStatuses, login_item_status: () => "disabled", ...handlers });
  render(<StatusPane readiness={{ microphone_authorized: true, system_audio: "allowed", destination: { kind: "ready", path: "/x", free_bytes: null }, armed: true, shortcut: shortcutStatuses.recording, missing_models: [], endpoint_configured: true }} onRefresh={async () => {}} onNavigate={() => {}} />);
  return mock;
}

test("Start at login starts off and does not register on read", async () => {
  const mock = setup();
  const toggle = await screen.findByRole("switch", { name: "Start at login" });
  await waitFor(() => expect(toggle.hasAttribute("disabled")).toBe(false));
  expect(toggle.getAttribute("aria-checked")).toBe("false");
  expect(mock.calls.some(c => c.cmd === "set_login_item")).toBe(false);
});

test("a pending change disables the switch until the OS result arrives", async () => {
  let finish!: (value: string) => void;
  setup({ set_login_item: () => new Promise<string>(resolve => { finish = resolve; }) });
  const toggle = await screen.findByRole("switch", { name: "Start at login" });
  await waitFor(() => expect(toggle.hasAttribute("disabled")).toBe(false));
  await userEvent.setup().click(toggle);
  expect(toggle.hasAttribute("disabled")).toBe(true);
  expect(toggle.getAttribute("aria-checked")).toBe("false");
  finish("enabled");
  await waitFor(() => expect(toggle.getAttribute("aria-checked")).toBe("true"));
});

test("registration failure keeps the observed state and reports the error", async () => {
  setup({ set_login_item: () => { throw "Registration failed"; } });
  const toggle = await screen.findByRole("switch", { name: "Start at login" });
  await waitFor(() => expect(toggle.hasAttribute("disabled")).toBe(false));
  await userEvent.setup().click(toggle);
  expect(await screen.findByText("Registration failed")).toBeTruthy();
  expect(toggle.getAttribute("aria-checked")).toBe("false");
});

test("General refreshes actual state when its window regains focus", async () => {
  let state = "enabled";
  setup({ login_item_status: () => state, set_login_item: () => "disabled" });
  const toggle = await screen.findByRole("switch", { name: "Start at login" });
  await waitFor(() => expect(toggle.getAttribute("aria-checked")).toBe("true"));
  await userEvent.setup().click(toggle);
  await waitFor(() => expect(toggle.getAttribute("aria-checked")).toBe("false"));
  fireEvent(window, new Event("focus"));
  await waitFor(() => expect(toggle.getAttribute("aria-checked")).toBe("true"));
});

test("approval is distinct from enabled and opens system settings", async () => {
  const mock = setup({ login_item_status: () => "approval_required", open_login_item_settings: () => null });
  const button = await screen.findByRole("button", { name: "Open System Settings" });
  expect(screen.getByRole("switch", { name: "Start at login" }).getAttribute("aria-checked")).toBe("false");
  await userEvent.setup().click(button);
  expect(mock.calls.some(c => c.cmd === "open_login_item_settings")).toBe(true);
});

test("a failed status query is unavailable, not confirmed off", async () => {
  setup({ login_item_status: () => { throw "Cannot read login items"; } });
  expect(await screen.findByText("Cannot read login items")).toBeTruthy();
  expect(screen.getByRole("switch", { name: "Start at login" }).hasAttribute("disabled")).toBe(true);
});

test("a pending approval can be cancelled", async () => {
  const mock = setup({ login_item_status: () => "approval_required", set_login_item: () => "disabled" });
  await userEvent.setup().click(await screen.findByRole("button", { name: "Cancel Request" }));
  await waitFor(() => expect(screen.queryByRole("button", { name: "Cancel Request" })).toBeNull());
  expect(mock.calls.find(c => c.cmd === "set_login_item")?.args).toEqual({ enabled: false });
});

test("a late focus query cannot replace a newer result", async () => {
  let resolveOld!: (value: string) => void;
  let count = 0;
  setup({ login_item_status: () => ++count === 1 ? new Promise<string>(resolve => { resolveOld = resolve; }) : "enabled" });
  fireEvent(window, new Event("focus"));
  const toggle = await screen.findByRole("switch", { name: "Start at login" });
  await waitFor(() => expect(toggle.getAttribute("aria-checked")).toBe("true"));
  await act(async () => { resolveOld("disabled"); });
  expect(toggle.getAttribute("aria-checked")).toBe("true");
});
