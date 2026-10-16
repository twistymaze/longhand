import { describe, expect, test } from "bun:test";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockWindows } from "@tauri-apps/api/mocks";
import type { AudioRemovalSetting, Settings } from "../bindings";
import { type CommandHandler, mockTauri } from "../test/tauri";
import { AudioRemoval, parseDays } from "./AudioRemoval";
import { DestinationPicker } from "./DestinationPicker";

const base = {
  storage_folder: "/Users/me/Notes",
  shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [], files_writable: true, damaged_file: null },
} as unknown as Settings;
const withRemoval = (audio_removal: AudioRemovalSetting) => ({ ...base, audio_removal }) as Settings;
const after = (days: number) => withRemoval({ kind: "after_days", days });
const off = withRemoval({ kind: "off" });
const readOnly = {
  ...after(30),
  shared: { ...base.shared, availability: { kind: "read_only", cause: "folder_not_available" } },
} as Settings;

function pane(settings: Settings, handlers: Record<string, CommandHandler> = {}) {
  const tauri = mockTauri({
    set_audio_removal: (args) => withRemoval(args.removal as AudioRemovalSetting),
    ...handlers,
  });
  // A field with typed text registers a save for the window close.
  mockWindows("settings");
  const received: Settings[] = [];
  const view = render(<AudioRemoval settings={settings} onSettings={(s) => received.push(s)} />);
  return { tauri, received, view, user: userEvent.setup() };
}

const field = () => screen.getByRole("textbox", { name: "Days before the audio is removed" }) as HTMLInputElement;
const toggle = () => screen.getByRole("switch", { name: "Remove old audio" }) as HTMLButtonElement;
const cmds = (tauri: { calls: { cmd: string }[] }) => tauri.calls.map((c) => c.cmd);

async function enterDays(user: ReturnType<typeof userEvent.setup>, text: string) {
  await user.clear(field());
  await user.type(field(), `${text}{Enter}`);
}

describe("AudioRemoval", () => {
  test("AE7: 90 to 30 days with eligible notes asks first, and Cancel keeps 90 days", async () => {
    const { tauri, user, received, view } = pane(after(90), { audio_removal_count: () => 4 });
    await enterDays(user, "30");
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("Remove the audio of 4 notes now?");
    expect(dialog.textContent).toContain("You can’t undo this.");
    expect(dialog.textContent).toContain("transcribe these notes again or resume their recordings");
    expect(tauri.calls.find((c) => c.cmd === "audio_removal_count")?.args).toEqual({
      removal: { kind: "after_days", days: 30 },
    });
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(cmds(tauri)).toEqual(["audio_removal_count"]);
    expect(received).toEqual([]);
    await waitFor(() => expect(field().value).toBe("90"));
    view.unmount();
  });

  test("a confirmed change saves the new delay", async () => {
    const { tauri, user, received, view } = pane(after(90), { audio_removal_count: () => 1 });
    await enterDays(user, "30");
    expect((await screen.findByRole("alertdialog")).textContent).toContain("Remove the audio of 1 note now?");
    await user.click(screen.getByRole("button", { name: "Remove Audio" }));
    await waitFor(() => expect(received).toEqual([after(30)]));
    expect(cmds(tauri)).toEqual(["audio_removal_count", "set_audio_removal"]);
    view.unmount();
  });

  test("30 to 90 days saves with no confirmation", async () => {
    const { tauri, user, received, view } = pane(after(30), { audio_removal_count: () => 0 });
    await enterDays(user, "90");
    await waitFor(() => expect(received).toEqual([after(90)]));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(tauri.calls.find((c) => c.cmd === "set_audio_removal")?.args).toEqual({
      removal: { kind: "after_days", days: 90 },
    });
    view.unmount();
  });

  test("turning the setting on with no eligible notes saves with no confirmation", async () => {
    const { tauri, user, received, view } = pane(off, { audio_removal_count: () => 0 });
    expect(toggle().getAttribute("aria-checked")).toBe("false");
    expect(screen.queryByRole("textbox")).toBeNull();
    await user.click(toggle());
    await waitFor(() => expect(received).toEqual([after(30)]));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(cmds(tauri)).toEqual(["audio_removal_count", "set_audio_removal"]);
    view.unmount();
  });

  test("turning the setting on with eligible notes asks first, and Cancel keeps it off", async () => {
    const { tauri, user, received, view } = pane(off, { audio_removal_count: () => 2 });
    await user.click(toggle());
    await screen.findByRole("alertdialog");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(toggle().getAttribute("aria-checked")).toBe("false");
    expect(cmds(tauri)).toEqual(["audio_removal_count"]);
    expect(received).toEqual([]);
    view.unmount();
  });

  test("turning the setting off saves it", async () => {
    const { user, received, view } = pane(after(30), { audio_removal_count: () => 0 });
    await user.click(toggle());
    await waitFor(() => expect(received).toEqual([off]));
    view.unmount();
  });

  test("read-only shared settings disable the control and show the reason", () => {
    const { view } = pane(readOnly);
    expect(toggle().disabled).toBe(true);
    expect(field().disabled).toBe(true);
    expect(screen.getByText(/The storage folder is not available\. This setting can’t be changed now\./)).toBeTruthy();
    view.unmount();
  });

  for (const bad of ["0", "3651", "abc", "1.5", "-3", ""]) {
    test(`the value “${bad}” is refused and the field shows the saved delay`, async () => {
      const { tauri, user, received, view } = pane(after(30));
      if (bad === "") {
        await user.clear(field());
        await user.keyboard("{Enter}");
      } else await enterDays(user, bad);
      expect(await screen.findByText("Enter a number of days from 1 to 3650.")).toBeTruthy();
      expect(cmds(tauri)).toEqual([]);
      expect(received).toEqual([]);
      await waitFor(() => expect(field().value).toBe("30"));
      view.unmount();
    });
  }

  test("a value that the settings view does not give shows the default, on at 30 days", () => {
    const { view } = pane(base);
    expect(toggle().getAttribute("aria-checked")).toBe("true");
    expect(field().value).toBe("30");
    view.unmount();
  });

  test("the day range is 1 to 3650 whole days", () => {
    expect(parseDays("1")).toBe(1);
    expect(parseDays(" 3650 ")).toBe(3650);
    expect(parseDays("0")).toBeNull();
    expect(parseDays("3651")).toBeNull();
    expect(parseDays("2e1")).toBeNull();
  });

  test("the Storage pane shows the audio setting", () => {
    mockTauri({ move_status: () => null, agent_instructions_status: () => ({ kind: "not_added" }) });
    const view = render(
      <DestinationPicker
        settings={after(30)}
        status={{ kind: "ready", path: base.storage_folder, free_bytes: null }}
        onSettings={() => {}}
      />,
    );
    expect(screen.getByRole("switch", { name: "Remove old audio" })).toBeTruthy();
    view.unmount();
  });

  test("a pane change saves typed days that remove no audio", async () => {
    const { tauri, user, view } = pane(after(90), { audio_removal_count: () => 0 });
    await user.clear(field());
    await user.type(field(), "60");
    view.unmount();
    await waitFor(() => expect(cmds(tauri)).toEqual(["audio_removal_count", "set_audio_removal"]));
    expect(tauri.calls[1].args).toEqual({ removal: { kind: "after_days", days: 60 } });
  });

  test("a pane change does not save or ask about typed days that would remove audio", async () => {
    const { tauri, user, view } = pane(after(90), {
      audio_removal_count: () => 4,
      log_from_webview: () => null,
      "plugin:window|destroy": () => null,
    });
    await user.clear(field());
    await user.type(field(), "30");
    view.unmount();
    await waitFor(() => expect(cmds(tauri)).toEqual(["audio_removal_count"]));
    await act(async () => {});
    expect(screen.queryByRole("alertdialog")).toBeNull();
    // The commit ended, so no close save stays registered: a window close
    // does not go through the close coordinator.
    await tauri.emit("tauri://close-requested");
    expect(cmds(tauri)).toEqual(["audio_removal_count"]);
  });
});
