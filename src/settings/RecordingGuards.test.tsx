import { describe, expect, test } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockWindows } from "@tauri-apps/api/mocks";
import type { CheckTimeSetting, Settings } from "../bindings";
import { type CommandHandler, mockTauri } from "../test/tauri";
import { RecordingGuards } from "./RecordingGuards";

const base = {
  storage_folder: "/Users/me/Notes",
  silence_check_minutes: { kind: "after", time: 30 },
  recording_time_limit_hours: { kind: "after", time: 6 },
  long_silence_cut_minutes: { kind: "after", time: 10 },
  stop_countdown_minutes: 2,
  shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [], files_writable: true, damaged_file: null },
} as unknown as Settings;
const readOnly = {
  ...base,
  shared: { ...base.shared, availability: { kind: "read_only", cause: "folder_not_available" } },
} as Settings;
const after = (time: number): CheckTimeSetting => ({ kind: "after", time });
const off: CheckTimeSetting = { kind: "off" };

function pane(settings: Settings, handlers: Record<string, CommandHandler> = {}) {
  const tauri = mockTauri({
    set_silence_check_minutes: (args) => ({ ...settings, silence_check_minutes: args.check }),
    set_recording_time_limit_hours: (args) => ({ ...settings, recording_time_limit_hours: args.limit }),
    set_long_silence_cut_minutes: (args) => ({ ...settings, long_silence_cut_minutes: args.cut }),
    set_stop_countdown_minutes: (args) => ({ ...settings, stop_countdown_minutes: args.minutes }),
    ...handlers,
  });
  // A field with typed text registers a save for the window close.
  mockWindows("settings");
  const received: Settings[] = [];
  const view = render(<RecordingGuards settings={settings} onSettings={(s) => received.push(s)} />);
  return { tauri, received, view, user: userEvent.setup() };
}

const toggle = (name: string) => screen.getByRole("switch", { name }) as HTMLButtonElement;
const field = (name: string) => screen.getByRole("textbox", { name }) as HTMLInputElement;
const calls = (tauri: { calls: { cmd: string; args: Record<string, unknown> }[] }) =>
  tauri.calls.map((c) => [c.cmd, c.args]);

const SILENCE = "Ask after silence";
const LIMIT = "Ask after a time limit";
const CUT = "Remove long silence";
const SILENCE_FIELD = "Minutes with no speech before the question";
const LIMIT_FIELD = "Hours of recording before the question";
const CUT_FIELD = "Shortest silence to remove, in minutes";
const COUNTDOWN_FIELD = "Minutes to answer before the note stops";

async function enter(user: ReturnType<typeof userEvent.setup>, name: string, text: string) {
  await user.clear(field(name));
  await user.type(field(name), `${text}{Enter}`);
}

describe("RecordingGuards", () => {
  test("the defaults show each check on, with 30 minutes, 6 hours, 10 minutes and 2 minutes", () => {
    const { view } = pane(base);
    for (const name of [SILENCE, LIMIT, CUT]) expect(toggle(name).getAttribute("aria-checked")).toBe("true");
    expect(field(SILENCE_FIELD).value).toBe("30");
    expect(field(LIMIT_FIELD).value).toBe("6");
    expect(field(CUT_FIELD).value).toBe("10");
    expect(field(COUNTDOWN_FIELD).value).toBe("2");
    expect(screen.getByText(/apply on every computer that uses this storage folder/)).toBeTruthy();
    view.unmount();
  });

  const switches = [
    { name: SILENCE, cmd: "set_silence_check_minutes", arg: "check", key: "silence_check_minutes", time: 45 },
    { name: LIMIT, cmd: "set_recording_time_limit_hours", arg: "limit", key: "recording_time_limit_hours", time: 8 },
    { name: CUT, cmd: "set_long_silence_cut_minutes", arg: "cut", key: "long_silence_cut_minutes", time: 15 },
  ] as const;

  for (const s of switches) {
    test(`the “${s.name}” switch saves off, and then on with the last time`, async () => {
      const tauri = mockTauri({
        [s.cmd]: (args) => ({ ...base, [s.key]: args[s.arg] }),
      });
      mockWindows("settings");
      let settings = { ...base, [s.key]: after(s.time) } as Settings;
      const user = userEvent.setup();
      const view = render(<RecordingGuards settings={settings} onSettings={(next) => (settings = next)} />);
      await user.click(toggle(s.name));
      await waitFor(() => expect(calls(tauri)).toEqual([[s.cmd, { [s.arg]: off }]]));
      view.rerender(<RecordingGuards settings={settings} onSettings={(next) => (settings = next)} />);
      expect(toggle(s.name).getAttribute("aria-checked")).toBe("false");
      await user.click(toggle(s.name));
      await waitFor(() =>
        expect(calls(tauri)).toEqual([
          [s.cmd, { [s.arg]: off }],
          [s.cmd, { [s.arg]: after(s.time) }],
        ]),
      );
      view.unmount();
    });
  }

  test("a switch that is off at the start turns on with the default time", async () => {
    const { tauri, user, view } = pane({ ...base, recording_time_limit_hours: off } as Settings);
    expect(screen.queryByRole("textbox", { name: LIMIT_FIELD })).toBeNull();
    await user.click(toggle(LIMIT));
    await waitFor(() => expect(calls(tauri)).toEqual([["set_recording_time_limit_hours", { limit: after(6) }]]));
    view.unmount();
  });

  test("a typed time saves at Return", async () => {
    const { tauri, user, received, view } = pane(base);
    await enter(user, SILENCE_FIELD, "45");
    await waitFor(() => expect(calls(tauri)).toEqual([["set_silence_check_minutes", { check: after(45) }]]));
    expect(received).toHaveLength(1);
    view.unmount();
  });

  const ranges = [
    { name: SILENCE_FIELD, bad: ["4", "241", "abc", "1.5", ""], error: "Enter a number of minutes from 5 to 240.", saved: "30" },
    { name: LIMIT_FIELD, bad: ["0", "25"], error: "Enter a number of hours from 1 to 24.", saved: "6" },
    { name: CUT_FIELD, bad: ["4", "241"], error: "Enter a number of minutes from 5 to 240.", saved: "10" },
    { name: COUNTDOWN_FIELD, bad: ["0", "11"], error: "Enter a number of minutes from 1 to 10.", saved: "2" },
  ];
  for (const r of ranges) {
    for (const bad of r.bad) {
      test(`“${bad}” in “${r.name}” shows the error and saves nothing`, async () => {
        const { tauri, user, received, view } = pane(base);
        if (bad === "") {
          await user.clear(field(r.name));
          await user.keyboard("{Enter}");
        } else await enter(user, r.name, bad);
        expect(await screen.findByText(r.error)).toBeTruthy();
        expect(tauri.calls).toEqual([]);
        expect(received).toEqual([]);
        await waitFor(() => expect(field(r.name).value).toBe(r.saved));
        view.unmount();
      });
    }
  }

  test("the countdown saves at Return", async () => {
    const { tauri, user, view } = pane(base);
    await enter(user, COUNTDOWN_FIELD, "5");
    await waitFor(() => expect(calls(tauri)).toEqual([["set_stop_countdown_minutes", { minutes: 5 }]]));
    view.unmount();
  });

  test("typed times save when the pane unmounts", async () => {
    const { tauri, user, view } = pane(base);
    await user.clear(field(LIMIT_FIELD));
    await user.type(field(LIMIT_FIELD), "12");
    view.unmount();
    await waitFor(() => expect(calls(tauri)).toEqual([["set_recording_time_limit_hours", { limit: after(12) }]]));
  });

  test("a typed countdown saves when the window closes", async () => {
    const { tauri, user, view } = pane(base, {
      log_from_webview: () => null,
      "plugin:window|destroy": () => null,
    });
    await user.clear(field(COUNTDOWN_FIELD));
    await user.type(field(COUNTDOWN_FIELD), "3");
    await tauri.emit("tauri://close-requested");
    await waitFor(() =>
      expect(tauri.calls.find((c) => c.cmd === "set_stop_countdown_minutes")?.args).toEqual({ minutes: 3 }),
    );
    view.unmount();
  });

  test("read-only shared settings disable the controls and show the reason", () => {
    const { view } = pane(readOnly);
    for (const name of [SILENCE, LIMIT, CUT]) expect(toggle(name).disabled).toBe(true);
    for (const name of [SILENCE_FIELD, LIMIT_FIELD, CUT_FIELD, COUNTDOWN_FIELD]) expect(field(name).disabled).toBe(true);
    expect(screen.getByText(/The storage folder is not available\. These settings can’t be changed now\./)).toBeTruthy();
    view.unmount();
  });
});
