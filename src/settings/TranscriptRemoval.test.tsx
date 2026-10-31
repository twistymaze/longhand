import { describe, expect, test } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import userEvent from "@testing-library/user-event";
import { mockWindows } from "@tauri-apps/api/mocks";
import type { AudioRemovalSetting, Settings, TranscriptRemovalSetting } from "../bindings";
import { type CommandHandler, mockTauri } from "../test/tauri";
import { OldNoteRemoval, TranscriptRemoval } from "./TranscriptRemoval";
import { DestinationPicker } from "./DestinationPicker";

const base = {
  storage_folder: "/Users/me/Notes",
  shared: { availability: { kind: "available" }, newer_version: false, unknown_values: [], files_writable: true, damaged_file: null },
} as unknown as Settings;
const days = (d: number) => ({ kind: "after_days", days: d }) as const;
const OFF = { kind: "off" } as const;
const pair = (audio_removal: AudioRemovalSetting, transcript_removal: TranscriptRemovalSetting) =>
  ({ ...base, audio_removal, transcript_removal }) as Settings;
const readOnly = {
  ...pair(days(30), days(90)),
  shared: { ...base.shared, availability: { kind: "read_only", cause: "folder_not_available" } },
} as Settings;

function pane(settings: Settings, handlers: Record<string, CommandHandler> = {}) {
  const tauri = mockTauri({
    set_transcript_removal: (args) => pair(settings.audio_removal, args.removal as TranscriptRemovalSetting),
    ...handlers,
  });
  // A field with typed text registers a save for the window close.
  mockWindows("settings");
  const received: Settings[] = [];
  const view = render(<TranscriptRemoval settings={settings} onSettings={(s) => received.push(s)} />);
  return { tauri, received, view, user: userEvent.setup() };
}

const field = () => screen.getByRole("textbox", { name: "Days before the transcript is removed" }) as HTMLInputElement;
const audioField = () => screen.getByRole("textbox", { name: "Days before the audio is removed" }) as HTMLInputElement;
const toggle = () => screen.getByRole("switch", { name: "Remove old transcripts" }) as HTMLButtonElement;
const audioToggle = () => screen.getByRole("switch", { name: "Remove old audio" }) as HTMLButtonElement;
const cmds = (tauri: { calls: { cmd: string }[] }) => tauri.calls.map((c) => c.cmd);

async function enterDays(user: ReturnType<typeof userEvent.setup>, input: HTMLInputElement, text: string) {
  await user.clear(input);
  await user.type(input, `${text}{Enter}`);
}

describe("TranscriptRemoval", () => {
  test("AE4: with audio removal off, the control is unavailable and says why", () => {
    const { view } = pane(pair(OFF, OFF));
    expect(toggle().disabled).toBe(true);
    expect(toggle().getAttribute("aria-checked")).toBe("false");
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.getByText("Turn on Remove old audio first.")).toBeTruthy();
    view.unmount();
  });

  test("AE4: a transcript value with audio removal off shows as off", () => {
    const { view } = pane(pair(OFF, days(90)));
    expect(toggle().getAttribute("aria-checked")).toBe("false");
    expect(toggle().disabled).toBe(true);
    view.unmount();
  });

  test("AE5: 365 to 90 days with notes to remove asks first, and Cancel keeps 365 days", async () => {
    const { tauri, user, received, view } = pane(pair(days(30), days(365)), {
      transcript_removal_count: () => ({ remove: 4, skipped: 0 }),
    });
    await enterDays(user, field(), "90");
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("Remove the transcripts of 4 notes now?");
    expect(dialog.textContent).toContain("You can’t undo this.");
    expect(dialog.textContent).toContain("can’t use Regenerate Summary, Transcribe Again or Resume");
    expect(dialog.textContent).not.toContain("summary failed");
    expect(tauri.calls.find((c) => c.cmd === "transcript_removal_count")?.args).toEqual({ removal: days(90) });
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(cmds(tauri)).toEqual(["transcript_removal_count"]);
    expect(received).toEqual([]);
    await waitFor(() => expect(field().value).toBe("365"));
    view.unmount();
  });

  test("a confirmed change saves the new delay", async () => {
    const { tauri, user, received, view } = pane(pair(days(30), days(365)), {
      transcript_removal_count: () => ({ remove: 1, skipped: 0 }),
    });
    await enterDays(user, field(), "90");
    expect((await screen.findByRole("alertdialog")).textContent).toContain("Remove the transcripts of 1 note now?");
    await user.click(screen.getByRole("button", { name: "Remove Transcripts" }));
    await waitFor(() => expect(received).toEqual([pair(days(30), days(90))]));
    expect(cmds(tauri)).toEqual(["transcript_removal_count", "set_transcript_removal"]);
    view.unmount();
  });

  test("AE14: the confirmation names the notes that it skips because their summary failed", async () => {
    const { user, view } = pane(pair(days(30), days(90)), {
      transcript_removal_count: () => ({ remove: 3, skipped: 1 }),
    });
    await enterDays(user, field(), "60");
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("1 note keeps its transcript because its summary failed.");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    view.unmount();
  });

  test("a change that only skips notes still asks, and names them", async () => {
    const { tauri, user, received, view } = pane(pair(days(30), days(90)), {
      transcript_removal_count: () => ({ remove: 0, skipped: 2 }),
    });
    await enterDays(user, field(), "60");
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("2 notes keep their transcript because their summary failed.");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(received).toEqual([pair(days(30), days(60))]));
    expect(cmds(tauri)).toEqual(["transcript_removal_count", "set_transcript_removal"]);
    view.unmount();
  });

  test("turning the setting on with no notes to remove saves the audio delay or longer", async () => {
    const { tauri, user, received, view } = pane(pair(days(120), OFF), {
      transcript_removal_count: () => ({ remove: 0, skipped: 0 }),
    });
    expect(toggle().disabled).toBe(false);
    await user.click(toggle());
    await waitFor(() => expect(received).toEqual([pair(days(120), days(120))]));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(cmds(tauri)).toEqual(["transcript_removal_count", "set_transcript_removal"]);
    view.unmount();
  });

  test("turning the setting on with notes to remove asks first, and Cancel keeps it off", async () => {
    const { tauri, user, received, view } = pane(pair(days(30), OFF), {
      transcript_removal_count: () => ({ remove: 2, skipped: 0 }),
    });
    await user.click(toggle());
    await screen.findByRole("alertdialog");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(toggle().getAttribute("aria-checked")).toBe("false");
    expect(cmds(tauri)).toEqual(["transcript_removal_count"]);
    expect(received).toEqual([]);
    view.unmount();
  });

  test("turning the setting off saves it", async () => {
    const { user, received, view } = pane(pair(days(30), days(90)), {
      transcript_removal_count: () => ({ remove: 0, skipped: 0 }),
    });
    await user.click(toggle());
    await waitFor(() => expect(received).toEqual([pair(days(30), OFF)]));
    view.unmount();
  });

  test("a value below the audio delay is not saved, and a hint gives the minimum", async () => {
    const { tauri, user, received, view } = pane(pair(days(30), days(90)));
    await enterDays(user, field(), "20");
    expect(await screen.findByText("Enter at least 30 days, the audio delay.")).toBeTruthy();
    expect(cmds(tauri)).toEqual([]);
    expect(received).toEqual([]);
    await waitFor(() => expect(field().value).toBe("90"));
    view.unmount();
  });

  test("a value out of range is not saved", async () => {
    const { tauri, user, view } = pane(pair(days(30), days(90)));
    await enterDays(user, field(), "abc");
    expect(await screen.findByText("Enter a number of days from 1 to 3650.")).toBeTruthy();
    expect(cmds(tauri)).toEqual([]);
    await waitFor(() => expect(field().value).toBe("90"));
    view.unmount();
  });

  test("a transcript value shorter than the audio delay shows the audio delay", () => {
    const { view } = pane(pair(days(60), days(30)));
    expect(field().value).toBe("60");
    view.unmount();
  });

  test("a refused save shows the reason", async () => {
    const { user, received, view } = pane(pair(days(30), days(90)), {
      transcript_removal_count: () => ({ remove: 0, skipped: 0 }),
      set_transcript_removal: () => {
        throw "The transcript delay can’t be shorter than the audio delay.";
      },
    });
    await enterDays(user, field(), "120");
    expect(await screen.findByText("The transcript delay can’t be shorter than the audio delay.")).toBeTruthy();
    expect(received).toEqual([]);
    view.unmount();
  });

  test("read-only shared settings disable the control and show the reason", () => {
    const { view } = pane(readOnly);
    expect(toggle().disabled).toBe(true);
    expect(field().disabled).toBe(true);
    expect(screen.getAllByText(/The storage folder is not available\. This setting can’t be changed now\./)).toHaveLength(1);
    view.unmount();
  });

  test("a pane change saves typed days that remove no transcript", async () => {
    const { tauri, user, view } = pane(pair(days(30), days(90)), {
      transcript_removal_count: () => ({ remove: 0, skipped: 0 }),
    });
    await user.clear(field());
    await user.type(field(), "120");
    view.unmount();
    await waitFor(() => expect(cmds(tauri)).toEqual(["transcript_removal_count", "set_transcript_removal"]));
    expect(tauri.calls[1].args).toEqual({ removal: days(120) });
  });

  test("a pane change does not save or ask about a shorter delay that would remove transcripts", async () => {
    const { tauri, user, view } = pane(pair(days(30), days(365)), {
      transcript_removal_count: () => ({ remove: 4, skipped: 0 }),
      log_from_webview: () => null,
      "plugin:window|destroy": () => null,
    });
    await user.clear(field());
    await user.type(field(), "90");
    view.unmount();
    await waitFor(() => expect(cmds(tauri)).toEqual(["transcript_removal_count"]));
    await act(async () => {});
    expect(screen.queryByRole("alertdialog")).toBeNull();
    await tauri.emit("tauri://close-requested");
    expect(cmds(tauri)).toEqual(["transcript_removal_count"]);
  });

  test("turning removal off with typed days that were not saved keeps removal off", async () => {
    const tauri = mockTauri({
      transcript_removal_count: () => ({ remove: 0, skipped: 0 }),
      set_transcript_removal: (args) => pair(days(30), args.removal as TranscriptRemovalSetting),
    });
    mockWindows("settings");
    function Live() {
      const [settings, setSettings] = useState(pair(days(30), days(90)));
      return <TranscriptRemoval settings={settings} onSettings={setSettings} />;
    }
    const user = userEvent.setup();
    render(<Live />);
    await user.clear(field());
    await user.type(field(), "120");
    act(() => void fireEvent.click(toggle()));
    await waitFor(() => expect(screen.queryByRole("textbox")).toBeNull());
    await act(async () => {});
    await act(async () => {});
    expect(tauri.calls.filter((c) => c.cmd === "set_transcript_removal").map((c) => c.args.removal)).toEqual([OFF]);
  });
});

describe("OldNoteRemoval", () => {
  // The Settings window reads the settings again after a save. These tests
  // keep the props: the pane must show the saved value at once.
  function both(settings: Settings, handlers: Record<string, CommandHandler>) {
    const tauri = mockTauri(handlers);
    mockWindows("settings");
    const view = render(<OldNoteRemoval settings={settings} onSettings={() => {}} />);
    return { tauri, view, user: userEvent.setup() };
  }

  test("AE2: a longer audio delay shows the moved transcript delay at once and asks nothing", async () => {
    const { tauri, user, view } = both(pair(days(30), days(60)), {
      audio_removal_count: () => 0,
      set_audio_removal: () => pair(days(90), days(90)),
    });
    expect(field().value).toBe("60");
    await enterDays(user, audioField(), "90");
    await waitFor(() => expect(field().value).toBe("90"));
    expect(audioField().value).toBe("90");
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(cmds(tauri)).toEqual(["audio_removal_count", "set_audio_removal"]);
    view.unmount();
  });

  test("AE3: turning audio removal off shows transcript removal off at once", async () => {
    const { user, view } = both(pair(days(30), days(60)), {
      audio_removal_count: () => 0,
      set_audio_removal: () => pair(OFF, OFF),
    });
    await user.click(audioToggle());
    await waitFor(() => expect(toggle().getAttribute("aria-checked")).toBe("false"));
    expect(toggle().disabled).toBe(true);
    expect(screen.getByText("Turn on Remove old audio first.")).toBeTruthy();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    view.unmount();
  });

  test("new settings from a read replace the saved value", async () => {
    mockTauri({ audio_removal_count: () => 0, set_audio_removal: () => pair(days(90), days(90)) });
    mockWindows("settings");
    const user = userEvent.setup();
    const view = render(<OldNoteRemoval settings={pair(days(30), days(60))} onSettings={() => {}} />);
    await enterDays(user, audioField(), "90");
    await waitFor(() => expect(field().value).toBe("90"));
    view.rerender(<OldNoteRemoval settings={pair(days(45), days(200))} onSettings={() => {}} />);
    await waitFor(() => expect(field().value).toBe("200"));
    expect(audioField().value).toBe("45");
    view.unmount();
  });

  test("the Storage pane shows the transcript setting below the audio setting", () => {
    mockTauri({ move_status: () => null, agent_instructions_status: () => ({ kind: "not_added" }) });
    const view = render(
      <DestinationPicker
        settings={pair(days(30), OFF)}
        status={{ kind: "ready", path: base.storage_folder, free_bytes: null }}
        onSettings={() => {}}
      />,
    );
    const switches = screen.getAllByRole("switch").map((s) => s.getAttribute("aria-label") ?? s.textContent);
    expect(switches.indexOf("Remove old transcripts")).toBeGreaterThan(switches.indexOf("Remove old audio"));
    expect(switches.indexOf("Remove old audio")).toBeGreaterThanOrEqual(0);
    view.unmount();
  });
});
