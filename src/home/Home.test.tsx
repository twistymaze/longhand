import { describe, expect, mock, test } from "bun:test";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { SessionDetail, SessionDocument, SessionEntry } from "../bindings";
import { sessionTitle } from "../processing/describe";
import { mockTauri } from "../test/tauri";
import { Home, type RecordingNow } from "./Home";
import { useSessions } from "../session/useSessions";

const now = new Date(2026, 8, 24, 18, 0);

const entry = (id: string, over: Partial<SessionEntry> = {}): SessionEntry => ({
  id,
  started: new Date(2026, 8, 24, 14, 5).toISOString(),
  duration_ms: 42 * 60_000,
  state: "saved",
  stage: null,
  title: null,
  found_speakers: 3,
  storage_unavailable: false,
  remote: null,
  note_type: null,
  type_move_waits: false,
  tags: [],
  ...over,
});

const detail = (e: SessionEntry, home: string | null): SessionDetail => ({
  entry: e,
  done: [],
  current: null,
  windows_done: null,
  windows_total: null,
  speakers: null,
  failed_stage: null,
  message: null,
  summary_problem: null,
  resumable: false,
  record: null,
  home_path: home,
  endpoint: null,
  audio_kept: true,
  transcript_kept: true,
  audio: "present",
  asr_model: null,
  live_transcript: false,
  transcribe_again: false,
  transcribe_again_error: null,
  summary_not_updated: null,
  waiting: [],
  conflict_files: [],
  not_saved_yet: null,
});

const document = (body: string): SessionDocument => ({
  has_speakers: true,
  lines: [],
  transcript_markdown: "",
  summary: { body, stub: false, host: null, model: null, prompt: null },
});

function renderHome({
  entries = [] as SessionEntry[],
  recording = null as RecordingNow | null,
  processingWaits = false,
  chord = "⌥⇧L" as string | null,
  onOpen = (_id: string) => {},
  onChanged = () => {},
} = {}) {
  return render(
    <Home
      entries={entries}
      stages={{}}
      recording={recording}
      processingWaits={processingWaits}
      readiness={null}
      chord={chord}
      onOpen={onOpen}
      onChanged={onChanged}
      now={now}
    />,
  );
}

const row = (name: string) =>
  screen.getByRole("option", { name: new RegExp(name) });

describe("Home", () => {
  test("a new note starts without a separate recognition chooser", () => {
    mockTauri();
    render(
      <Home
        entries={[]}
        stages={{}}
        recording={null}
        readiness={{
          armed: true,
          microphone_authorized: true,
          destination: { kind: "ready", path: "/notes", free_bytes: null },
          shortcut: { chord: null, registered: false, error: null },
          system_audio: "allowed",
          missing_models: [],
          endpoint_configured: true, can_start_note: true, note_start_refusal: null,
        }}
        chord={null}
        onOpen={() => {}}
        onChanged={() => {}}
      />,
    );
    expect(
      screen.queryByRole("button", { name: "Choices for next note…" }),
    ).toBeNull();
    expect(screen.getByRole("button", { name: "Record New Note" })).toBeTruthy();
  });
  test("with no sessions, shows the line that names the shortcut and says it starts a new note", () => {
    mockTauri();
    renderHome();
    expect(screen.getByRole("heading", { name: "Notes" })).toBeTruthy();
    expect(screen.getByText("No notes yet")).toBeTruthy();
    expect(
      screen.getByText(
        (_, el) =>
          el?.tagName === "P" &&
          /Press ⌥⇧L .*to start a new note/.test(el.textContent ?? ""),
      ),
    ).toBeTruthy();
    expect(screen.queryByRole("option")).toBeNull();
  });

  test("with no sessions and no shortcut, the line names New Note and shows no key cap", () => {
    mockTauri();
    const { container } = renderHome({ chord: null });
    expect(
      screen.getByText(
        (_, el) => el?.tagName === "P" && /New Note/.test(el.textContent ?? ""),
      ),
    ).toBeTruthy();
    expect(container.querySelector("kbd")).toBeNull();
  });

  test("New Note starts a recording", async () => {
    const tauri = mockTauri({ start_recording: () => "s" });
    render(
      <Home
        entries={[]}
        stages={{}}
        recording={null}
        readiness={{
          microphone_authorized: true,
          system_audio: "allowed",
          destination: { kind: "ready", path: "/x", free_bytes: null },
          armed: true,
          shortcut: { chord: "Alt+Shift+L", registered: true, error: null },
          missing_models: [],
          endpoint_configured: true, can_start_note: true, note_start_refusal: null,
        }}
        chord="⌥⇧L"
        onOpen={() => {}}
        onChanged={() => {}}
        now={now}
      />,
    );
    expect(screen.queryByRole("button", { name: "Record" })).toBeNull();
    await userEvent
      .setup()
      .click(screen.getByRole("button", { name: "Record New Note" }));
    expect(tauri.calls.find((c) => c.cmd === "start_recording")?.args).toEqual({
      record: true,
      config: null,
    });
    // The saved default configuration applies; no override is set.
    expect(tauri.calls.some((c) => c.cmd === "set_recognition_override")).toBe(false);
  });

  test("a session with no title shows its start time, and a titled session shows its title", () => {
    mockTauri();
    renderHome({
      entries: [entry("a"), entry("b", { title: "Budget review" })],
    });
    expect(row(sessionTitle(entry("a").started, now))).toBeTruthy();
    expect(row("Budget review")).toBeTruthy();
  });

  test("a loaded nonempty library selects its newest note and shows the preview", () => {
    mockTauri();
    renderHome({ entries: [entry("a")] });
    expect(screen.getByRole("region", { name: "Preview" })).toBeTruthy();
  });

  test("the selected overview remains after the pointer leaves the list", async () => {
    const e = entry("a", { title: "Budget review" });
    mockTauri({
      session_document: () =>
        document(
          "## Overview\n\nWe agreed the **budget** for next year.\n\n## Decisions\n\n- More",
        ),
      session_detail: () =>
        detail(e, "/Users/me/Longhand/2026-09-24 1405 Budget review"),
    });
    const onOpen = mock((_id: string) => {});
    renderHome({ entries: [e], onOpen });

    const user = userEvent.setup();
    await user.hover(row("Budget review"));
    const preview = await screen.findByRole("region", { name: "Preview" });
    expect(
      await within(preview).findByText("We agreed the budget for next year."),
    ).toBeTruthy();
    expect(
      within(preview).getByText("2026-09-24 1405 Budget review"),
    ).toBeTruthy();
    expect(within(preview).getByText("3")).toBeTruthy();
    expect(onOpen).not.toHaveBeenCalled();
    await user.unhover(row("Budget review"));
    // Pointer movement does not clear the selection.
    await act(() => new Promise((done) => setTimeout(done, 300)));
    expect(screen.getByRole("region", { name: "Preview" })).toBeTruthy();
  });

  test("the arrow keys select a row and show its preview", async () => {
    mockTauri({
      session_document: () => document("An overview."),
      session_detail: () => detail(entry("a"), null),
    });
    renderHome({
      entries: [
        entry("a", { title: "Budget review" }),
        entry("b", { title: "Standup" }),
      ],
    });
    row("Budget review").focus();
    await userEvent.setup().keyboard("{ArrowDown}");
    expect(row("Standup").getAttribute("aria-selected")).toBe("true");
    const preview = await screen.findByRole("region", { name: "Preview" });
    expect(
      within(preview).getByRole("heading", { name: "Standup" }),
    ).toBeTruthy();
  });

  test("a click selects and focuses; double-click and Return open the note", async () => {
    mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("a"), null),
    });
    const onOpen = mock((_id: string) => {});
    renderHome({ entries: [entry("a", { title: "Budget review" })], onOpen });
    const user = userEvent.setup();
    await user.click(row("Budget review"));
    expect(onOpen).not.toHaveBeenCalled();
    expect(globalThis.document.activeElement).toBe(row("Budget review"));
    await user.dblClick(row("Budget review"));
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(onOpen).toHaveBeenCalledWith("a");
    onOpen.mockClear();
    row("Budget review").focus();
    await user.keyboard("{Enter}");
    expect(onOpen).toHaveBeenCalledWith("a");
  });

  test("the row menu opens without opening the note", async () => {
    mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("a"), null),
    });
    const onOpen = mock((_id: string) => {});
    renderHome({ entries: [entry("a", { title: "Budget review" })], onOpen });
    await userEvent
      .setup()
      .click(
        within(row("Budget review")).getByRole("button", { name: "Actions" }),
      );
    expect(screen.getByRole("menuitem", { name: "Rename" })).toBeTruthy();
    expect(onOpen).not.toHaveBeenCalled();
  });

  // Covers AE6.
  test("with a recording in progress, selects the recording row with its elapsed time and status preview", async () => {
    mockTauri();
    const onOpen = mock((_id: string) => {});
    renderHome({
      entries: [entry("rec", { state: "capturing" })],
      recording: {
        sessionId: "rec",
        elapsedMs: 5 * 60_000 + 7_000,
        interrupted: false,
      },
      onOpen,
    });
    const recording = screen.getByRole("option", { name: /Recording/ });
    expect(recording.textContent).toContain("00:05:07");
    // The open session is the recording row, not a row in the list.
    expect(recording.getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("region", { name: "Preview" })).toBeTruthy();
    await userEvent.setup().dblClick(recording);
    expect(onOpen).toHaveBeenCalledWith("rec");
  });

  test("while processing waits for a recording, a processing row and its preview say so", () => {
    mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("p"), null),
    });
    renderHome({
      entries: [entry("p", { state: "processing", stage: "assemble", title: "Budget review" })],
      processingWaits: true,
    });
    expect(within(row("Budget review")).getByText("Waiting for the recording")).toBeTruthy();
    expect(screen.queryByText("Preparing the audio")).toBeNull();
  });

  // Covers AE5.
  test("the menu of a processing session has Delete disabled", async () => {
    mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("p"), null),
    });
    renderHome({
      entries: [entry("p", { state: "processing", title: "Budget review" })],
    });
    const user = userEvent.setup();
    await user.click(
      within(row("Budget review")).getByRole("button", { name: "Actions" }),
    );
    const del = screen.getByRole("menuitem", { name: "Delete…" });
    expect(del.hasAttribute("disabled")).toBe(true);
    expect(
      screen.getByRole("menuitem", { name: "Rename" }).hasAttribute("disabled"),
    ).toBe(false);
  });

  // Covers AE10.
  test("a Trash failure offers Delete Permanently, and Cancel leaves the row", async () => {
    const tauri = mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("s"), "/x/Budget review"),
      delete_session: () => {
        throw {
          kind: "cannot_move_to_trash",
          path: "/x/Budget review",
          reason: "the volume has no Trash",
        };
      },
      delete_session_permanently: () => null,
    });
    const onChanged = mock(() => {});
    renderHome({
      entries: [entry("s", { title: "Budget review" })],
      onChanged,
    });
    const user = userEvent.setup();
    await user.click(
      within(row("Budget review")).getByRole("button", { name: "Actions" }),
    );
    await user.click(screen.getByRole("menuitem", { name: "Delete…" }));
    const confirm = screen.getByRole("alertdialog");
    await user.click(within(confirm).getByRole("button", { name: "Delete" }));
    const permanent = await screen.findByRole("alertdialog", {
      name: /can’t be moved to the Trash/,
    });
    expect(
      within(permanent).getByRole("button", { name: "Delete Permanently" }),
    ).toBeTruthy();
    await user.click(within(permanent).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(row("Budget review")).toBeTruthy();
    expect(
      tauri.calls.some((c) => c.cmd === "delete_session_permanently"),
    ).toBe(false);
    expect(onChanged).not.toHaveBeenCalled();
  });

  // Covers AE10: the second question deletes the session for good.
  test("a Trash failure offers Delete Permanently, which deletes the session and its row", async () => {
    let entries = [entry("s", { title: "Budget review" })];
    const tauri = mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("s"), "/x/Budget review"),
      delete_session: () => {
        throw {
          kind: "cannot_move_to_trash",
          path: "/x/Budget review",
          reason: "the volume has no Trash",
        };
      },
      delete_session_permanently: () => {
        entries = [];
        return null;
      },
    });
    const homeWith = (list: SessionEntry[]) => (
      <Home
        entries={list}
        stages={{}}
        recording={null}
        readiness={null}
        chord="⌥⇧L"
        onOpen={() => {}}
        onChanged={() => {}}
        now={now}
      />
    );
    // The list reads the sessions again when Home says they changed.
    const view = renderHome({
      entries,
      onChanged: () => view.rerender(homeWith(entries)),
    });
    const user = userEvent.setup();
    await user.click(
      within(row("Budget review")).getByRole("button", { name: "Actions" }),
    );
    await user.click(screen.getByRole("menuitem", { name: "Delete…" }));
    await user.click(
      within(screen.getByRole("alertdialog")).getByRole("button", {
        name: "Delete",
      }),
    );
    const permanent = await screen.findByRole("alertdialog", {
      name: /can’t be moved to the Trash/,
    });
    await user.click(
      within(permanent).getByRole("button", { name: "Delete Permanently" }),
    );
    await act(async () => {});
    expect(
      tauri.calls
        .filter((c) => c.cmd === "delete_session_permanently")
        .map((c) => c.args),
    ).toEqual([{ sessionId: "s" }]);
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(screen.queryByRole("option", { name: /Budget review/ })).toBeNull();
  });

  test("a delete that fails for another reason shows why, and keeps the row", async () => {
    const tauri = mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("s"), null),
      delete_session: () => {
        throw { kind: "failed", message: "the store is locked" };
      },
    });
    const onChanged = mock(() => {});
    renderHome({
      entries: [entry("s", { title: "Budget review" })],
      onChanged,
    });
    const user = userEvent.setup();
    await user.click(
      within(row("Budget review")).getByRole("button", { name: "Actions" }),
    );
    await user.click(screen.getByRole("menuitem", { name: "Delete…" }));
    await user.click(
      within(screen.getByRole("alertdialog")).getByRole("button", {
        name: "Delete",
      }),
    );
    expect(await screen.findByText("The store is locked.")).toBeTruthy();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(row("Budget review").getAttribute("aria-selected")).toBe("true");
    await waitFor(() =>
      expect(globalThis.document.activeElement).toBe(row("Budget review")),
    );
    expect(
      tauri.calls.some((c) => c.cmd === "delete_session_permanently"),
    ).toBe(false);
    expect(onChanged).toHaveBeenCalled();
  });

  test("Rename commits on Return, and Escape cancels", async () => {
    const tauri = mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("s"), null),
      rename_session: () => ({ kind: "moved", path: "/x/Budget review" }),
    });
    const onChanged = mock(() => {});
    renderHome({ entries: [entry("s", { title: "Old title" })], onChanged });
    const user = userEvent.setup();

    await user.click(
      within(row("Old title")).getByRole("button", { name: "Actions" }),
    );
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    await user.clear(screen.getByRole("textbox", { name: "Title" }));
    await user.keyboard("Discarded{Escape}");
    expect(screen.queryByRole("textbox", { name: "Title" })).toBeNull();
    expect(tauri.calls.some((c) => c.cmd === "rename_session")).toBe(false);

    await user.click(
      within(row("Old title")).getByRole("button", { name: "Actions" }),
    );
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    await user.clear(screen.getByRole("textbox", { name: "Title" }));
    await user.keyboard("Budget review{Enter}");
    await act(async () => {});
    expect(
      tauri.calls.filter((c) => c.cmd === "rename_session").map((c) => c.args),
    ).toEqual([{ sessionId: "s", title: "Budget review" }]);
    expect(onChanged).toHaveBeenCalled();
  });

  // Covers R35.
  test("an empty title clears the title", async () => {
    const tauri = mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("s"), null),
      rename_session: () => ({ kind: "later" }),
    });
    renderHome({ entries: [entry("s", { title: "Old title" })] });
    const user = userEvent.setup();
    await user.click(
      within(row("Old title")).getByRole("button", { name: "Actions" }),
    );
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    await user.clear(screen.getByRole("textbox", { name: "Title" }));
    await user.keyboard("{Enter}");
    await act(async () => {});
    expect(
      tauri.calls.filter((c) => c.cmd === "rename_session").map((c) => c.args),
    ).toEqual([{ sessionId: "s", title: "" }]);
  });

  test("a folder that keeps its old name shows the message", async () => {
    mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("s"), null),
      rename_session: () => ({
        kind: "kept",
        message: "the folder is open in another app",
      }),
    });
    renderHome({ entries: [entry("s", { title: "Old title" })] });
    const user = userEvent.setup();
    await user.click(
      within(row("Old title")).getByRole("button", { name: "Actions" }),
    );
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    await user.clear(screen.getByRole("textbox", { name: "Title" }));
    await user.keyboard("New{Enter}");
    expect(
      await screen.findByText(/The folder is open in another app/),
    ).toBeTruthy();
  });

  // Covers AE6, R16.
  test("a note in an unavailable storage folder stays listed and says so under its title", async () => {
    mockTauri();
    renderHome({
      entries: [
        entry("a", { title: "Planning", storage_unavailable: true }),
        entry("b", { title: "Budget" }),
      ],
    });
    const away = row("Planning");
    expect(within(away).getByText("Storage folder unavailable")).toBeTruthy();
    expect(
      within(row("Budget")).queryByText("Storage folder unavailable"),
    ).toBeNull();

    await userEvent
      .setup()
      .click(within(away).getByRole("button", { name: "Actions" }));
    expect(
      screen
        .getByRole("menuitem", { name: "Delete…" })
        .hasAttribute("disabled"),
    ).toBe(true);
  });
});

const ready = {
  microphone_authorized: true,
  system_audio: "allowed" as const,
  destination: { kind: "ready" as const, path: "/x", free_bytes: null },
  armed: true,
  shortcut: { chord: "Alt+Shift+L", registered: true, error: null },
  missing_models: [],
  endpoint_configured: true, can_start_note: true, note_start_refusal: null,
};
const home = (
  entries: SessionEntry[],
  extra: Partial<React.ComponentProps<typeof Home>> = {},
) => (
  <Home
    entries={entries}
    stages={{}}
    recording={null}
    readiness={null}
    chord={null}
    onOpen={() => {}}
    onChanged={() => {}}
    {...extra}
  />
);
const previewMocks = () =>
  mockTauri({
    session_document: () => document("Summary."),
    session_detail: () => detail(entry("a"), null),
  });

test("selection survives refresh, new notes, and pointer movement without taking focus", async () => {
  previewMocks();
  const old = entry("old", { title: "Older", started: "2026-09-23T12:00:00Z" });
  const recent = entry("recent", { title: "Recent" });
  const view = render(home([old, recent]));
  expect(row("Recent").getAttribute("aria-selected")).toBe("true");
  const user = userEvent.setup();
  await user.click(row("Older"));
  expect(globalThis.document.activeElement).toBe(row("Older"));
  await user.hover(row("Recent"));
  await user.unhover(row("Recent"));
  view.rerender(
    home([
      old,
      recent,
      entry("new", { title: "Newest", started: "2026-09-26T12:00:00Z" }),
    ]),
  );
  expect(row("Older").getAttribute("aria-selected")).toBe("true");
  expect(globalThis.document.activeElement).toBe(row("Older"));
  expect(
    within(screen.getByRole("region", { name: "Preview" })).getByRole(
      "heading",
      { name: "Older" },
    ),
  ).toBeTruthy();
});

test("tied starts choose by ID regardless of input order", async () => {
  previewMocks();
  render(home([entry("b", { title: "Beta" }), entry("a", { title: "Alpha" })]));
  await act(async () => {});
  expect(row("Alpha").getAttribute("aria-selected")).toBe("true");
});

test("loading, initial error and pending capture do not show an empty library", async () => {
  previewMocks();
  const view = render(home([], { loaded: false }));
  expect(screen.getByText("Loading notes…")).toBeTruthy();
  expect(screen.queryByText("No notes yet")).toBeNull();
  view.rerender(
    home([], { loaded: false, loadError: "Could not load notes." }),
  );
  expect(screen.getByText("Could not load notes.")).toBeTruthy();
  expect(screen.queryByText("No notes yet")).toBeNull();
  view.rerender(
    home([], {
      recording: { sessionId: "a", elapsedMs: 0, interrupted: false },
    }),
  );
  expect(screen.queryByText("No notes yet")).toBeNull();
  view.rerender(home([entry("a", { state: "capturing" })]));
  await act(async () => {});
  expect(screen.getByRole("option").getAttribute("aria-selected")).toBe("true");
});

test("arrow keys include interrupted and recording notes after a pointer selection", async () => {
  previewMocks();
  const onOpen = mock(() => {});
  render(
    home(
      [
        entry("a", { title: "Alpha" }),
        entry("b", { state: "interrupted" }),
        entry("c", { state: "capturing" }),
      ],
      { onOpen },
    ),
  );
  const user = userEvent.setup();
  await user.click(row("Alpha"));
  await user.keyboard("{ArrowDown}");
  await waitFor(() =>
    expect(globalThis.document.activeElement).toBe(
      row("Recording interrupted"),
    ),
  );
  await user.keyboard("{ArrowDown}");
  await waitFor(() =>
    expect(
      globalThis.document.activeElement?.getAttribute("data-session"),
    ).toBe("c"),
  );
  await user.keyboard("{Enter}");
  expect(onOpen).toHaveBeenCalledWith("c");
});

test("double-clicks on actions and rename stay inside those controls", async () => {
  previewMocks();
  const onOpen = mock(() => {});
  render(home([entry("a", { title: "Alpha" })], { onOpen }));
  const user = userEvent.setup();
  const actions = within(row("Alpha")).getByRole("button", { name: "Actions" });
  await user.dblClick(actions);
  if (!screen.queryByRole("menuitem", { name: "Rename" }))
    await user.click(actions);
  fireEvent.doubleClick(screen.getByRole("menuitem", { name: "Rename" }));
  await user.click(screen.getByRole("menuitem", { name: "Rename" }));
  await user.dblClick(screen.getByRole("textbox", { name: "Title" }));
  expect(onOpen).not.toHaveBeenCalled();
});

test("successful deletion excludes stale data and restores keyboard focus to the newest remaining row", async () => {
  mockTauri({
    session_document: () => document("Summary."),
    session_detail: () => detail(entry("a"), null),
    delete_session: () => null,
  });
  const onOpen = mock(() => {});
  render(
    home([entry("a", { title: "Alpha" }), entry("b", { title: "Beta" })], {
      onOpen,
    }),
  );
  const user = userEvent.setup();
  await user.click(row("Alpha"));
  await user.click(
    within(row("Alpha")).getByRole("button", { name: "Actions" }),
  );
  await user.click(screen.getByRole("menuitem", { name: "Delete…" }));
  await user.click(
    within(screen.getByRole("alertdialog")).getByRole("button", {
      name: "Delete",
    }),
  );
  expect(screen.queryByRole("option", { name: /Alpha/ })).toBeNull();
  await waitFor(() =>
    expect(globalThis.document.activeElement).toBe(row("Beta")),
  );
  await user.keyboard("{ArrowDown}{Enter}");
  expect(onOpen).toHaveBeenCalledWith("b");
});

test("final deletion removes the sidebar and focuses New Note", async () => {
  mockTauri({
    session_document: () => document("Summary."),
    session_detail: () => detail(entry("a"), null),
    delete_session: () => null,
  });
  render(home([entry("a", { title: "Alpha" })], { readiness: ready }));
  const user = userEvent.setup();
  await user.click(row("Alpha"));
  await user.click(
    within(row("Alpha")).getByRole("button", { name: "Actions" }),
  );
  await user.click(screen.getByRole("menuitem", { name: "Delete…" }));
  await user.click(
    within(screen.getByRole("alertdialog")).getByRole("button", {
      name: "Delete",
    }),
  );
  expect(screen.getByText("No notes yet")).toBeTruthy();
  expect(screen.queryByRole("region", { name: "Preview" })).toBeNull();
  await waitFor(() =>
    expect(globalThis.document.activeElement).toBe(
      screen.getByRole("button", { name: "Record New Note" }),
    ),
  );
});

test("preview revisions refresh saved summaries and late results cannot replace another note", async () => {
  let finishA!: (value: SessionDocument) => void;
  let text = "Beta summary.";
  mockTauri({
    session_document: ({ sessionId }) =>
      sessionId === "a"
        ? new Promise<SessionDocument>((resolve) => {
            finishA = resolve;
          })
        : document(text),
    session_detail: () => detail(entry("a"), null),
  });
  const entries = [
    entry("a", { title: "Alpha" }),
    entry("b", { title: "Beta" }),
  ];
  const view = render(home(entries));
  await userEvent.setup().click(row("Beta"));
  expect(await screen.findByText("Beta summary.")).toBeTruthy();
  await act(async () => finishA(document("Late Alpha summary.")));
  expect(screen.queryByText("Late Alpha summary.")).toBeNull();
  text = "Revised Beta summary.";
  view.rerender(home(entries, { revisions: { b: 1 } }));
  expect(await screen.findByText("Revised Beta summary.")).toBeTruthy();
});

test("missing summaries and read errors have distinct preview messages", async () => {
  let fail = false;
  mockTauri({
    session_document: () => {
      if (fail) throw "unavailable";
      return { ...document(""), summary: null };
    },
    session_detail: () => detail(entry("a"), null),
  });
  const entries = [entry("a")];
  const view = render(home(entries));
  expect(await screen.findByText("No summary yet.")).toBeTruthy();
  fail = true;
  view.rerender(home(entries, { revisions: { a: 1 } }));
  expect(await screen.findByText("Could not read this note.")).toBeTruthy();
  expect(screen.queryByText("No summary yet.")).toBeNull();
});

test("a restored note returns after a successful deletion refresh", async () => {
  mockTauri({
    session_document: () => document("Summary."),
    session_detail: () => detail(entry("a"), null),
    delete_session: () => null,
  });
  const entries = [entry("a", { title: "Alpha" })];
  const view = render(home(entries));
  const user = userEvent.setup();
  await user.click(
    within(row("Alpha")).getByRole("button", { name: "Actions" }),
  );
  await user.click(screen.getByRole("menuitem", { name: "Delete…" }));
  await user.click(
    within(screen.getByRole("alertdialog")).getByRole("button", {
      name: "Delete",
    }),
  );
  view.rerender(home([]));
  view.rerender(home(entries));
  expect(row("Alpha").getAttribute("aria-selected")).toBe("true");
  await act(async () => {});
});

test("a list read error offers Retry", async () => {
  previewMocks();
  const onChanged = mock(() => {});
  render(
    home([], { loaded: false, loadError: "Could not load notes.", onChanged }),
  );
  await userEvent.setup().click(screen.getByRole("button", { name: "Retry" }));
  expect(onChanged).toHaveBeenCalledTimes(1);
});

test("a background refresh does not move focus from New Note", async () => {
  previewMocks();
  const entries = [entry("a", { title: "Alpha" })];
  const view = render(home(entries, { readiness: ready }));
  const button = screen.getByRole("button", { name: "Record New Note" });
  button.focus();
  view.rerender(
    home([...entries, entry("b", { title: "Beta" })], { readiness: ready }),
  );
  await act(async () => {});
  expect(globalThis.document.activeElement).toBe(button);
});

test("arrow navigation starts from the selected note when the page has focus", async () => {
  previewMocks();
  render(home([entry("a", { title: "Alpha" }), entry("b", { title: "Beta" })]));
  const user = userEvent.setup();
  expect(globalThis.document.activeElement).toBe(globalThis.document.body);
  await user.keyboard("{ArrowDown}");
  expect(row("Beta").getAttribute("aria-selected")).toBe("true");
  await waitFor(() =>
    expect(globalThis.document.activeElement).toBe(row("Beta")),
  );
  await user.click(screen.getByRole("heading", { name: "Notes" }));
  expect(globalThis.document.activeElement).toBe(globalThis.document.body);
  await user.keyboard("{ArrowUp}");
  expect(row("Alpha").getAttribute("aria-selected")).toBe("true");
});

test("page arrow navigation leaves controls, modified keys, and hidden Home alone", async () => {
  previewMocks();
  const entries = [
    entry("a", { title: "Alpha" }),
    entry("b", { title: "Beta" }),
  ];
  const view = render(home(entries, { readiness: ready }));
  const user = userEvent.setup();
  screen.getByRole("button", { name: "Record New Note" }).focus();
  await user.keyboard("{ArrowDown}");
  expect(row("Alpha").getAttribute("aria-selected")).toBe("true");
  await user.click(screen.getByRole("heading", { name: "Notes" }));
  await user.keyboard("{Shift>}{ArrowDown}{/Shift}");
  expect(row("Alpha").getAttribute("aria-selected")).toBe("true");
  view.rerender(home(entries, { active: false }));
  await user.keyboard("{ArrowDown}");
  expect(row("Alpha").getAttribute("aria-selected")).toBe("true");
});

test("a restored note returns after a failed deletion refresh without accepting an older read", async () => {
  const note = entry("a", { title: "Alpha" });
  let reads = 0;
  let finishOld!: (value: SessionEntry[]) => void;
  let restored = false;
  const ipc = mockTauri({
    list_sessions: () => {
      reads++;
      if (reads === 1 || restored) return [{ ...note }];
      if (reads === 2)
        return new Promise<SessionEntry[]>((resolve) => {
          finishOld = resolve;
        });
      throw "Read failed";
    },
    session_document: () => document("Summary."),
    session_detail: () => detail(note, null),
    delete_session: () => null,
  });
  function Library() {
    const sessions = useSessions();
    return home(sessions.entries, {
      loaded: sessions.loaded,
      loadError: sessions.loadError,
      listRevision: sessions.listRevision,
      getListRevision: sessions.getListRevision,
      revisions: sessions.version,
      onChanged: () => void sessions.refresh(),
    });
  }
  render(<Library />);
  await screen.findByRole("option", { name: /Alpha/ });
  fireEvent(window, new Event("focus"));
  const user = userEvent.setup();
  await user.click(
    within(row("Alpha")).getByRole("button", { name: "Actions" }),
  );
  await user.click(screen.getByRole("menuitem", { name: "Delete…" }));
  await user.click(
    within(screen.getByRole("alertdialog")).getByRole("button", {
      name: "Delete",
    }),
  );
  await screen.findByText("Could not load notes.");
  await act(async () => finishOld([note]));
  expect(screen.queryByRole("option", { name: /Alpha/ })).toBeNull();
  expect(screen.getByText("Could not load notes.")).toBeTruthy();
  restored = true;
  await act(async () =>
    ipc.emit("library-changed", { changed: [note.id], removed: [] }),
  );
  expect(await screen.findByRole("option", { name: /Alpha/ })).toBeTruthy();
  expect(screen.queryByText("Could not load notes.")).toBeNull();
});

test("a preview stays visible during a same-note refresh and reports a later error", async () => {
  const note = entry("a", { title: "Alpha" });
  let hold = false;
  let fail!: (reason: string) => void;
  mockTauri({
    session_document: () =>
      hold
        ? new Promise<SessionDocument>((_, reject) => {
            fail = reject;
          })
        : document("Current summary."),
    session_detail: () => detail(note, "/notes/Alpha"),
  });
  const view = render(home([note], { revisions: { a: 0 } }));
  await screen.findByText("Current summary.");
  hold = true;
  view.rerender(home([note], { revisions: { a: 1 } }));
  expect(screen.getByText("Current summary.")).toBeTruthy();
  expect(screen.queryByText("Loading summary…")).toBeNull();
  await act(async () => fail("Read failed"));
  expect(screen.getByText("Could not read this note.")).toBeTruthy();
  expect(screen.queryByText("Current summary.")).toBeNull();
});

test("a read accepted during a pending delete cannot end deletion suppression", async () => {
  const note = entry("a", { title: "Alpha" });
  let reads = 0;
  let finishRead!: (value: SessionEntry[]) => void;
  let finishDelete!: (value: null) => void;
  let acceptedRevision!: () => number;
  const ipc = mockTauri({
    list_sessions: () => {
      reads++;
      if (reads === 1 || reads > 3) return [{ ...note }];
      if (reads === 2)
        return new Promise<SessionEntry[]>((resolve) => {
          finishRead = resolve;
        });
      throw "Read failed";
    },
    session_document: () => document("Summary."),
    session_detail: () => detail(note, null),
    delete_session: () =>
      new Promise<null>((resolve) => {
        finishDelete = resolve;
      }),
  });
  function Library() {
    const sessions = useSessions();
    acceptedRevision = sessions.getListRevision;
    return home(sessions.entries, {
      loaded: sessions.loaded,
      loadError: sessions.loadError,
      listRevision: sessions.listRevision,
      getListRevision: sessions.getListRevision,
      onChanged: () => void sessions.refresh(),
    });
  }
  render(<Library />);
  await screen.findByRole("option", { name: /Alpha/ });
  const user = userEvent.setup();
  await user.click(
    within(row("Alpha")).getByRole("button", { name: "Actions" }),
  );
  await user.click(screen.getByRole("menuitem", { name: "Delete…" }));
  await user.click(
    within(screen.getByRole("alertdialog")).getByRole("button", {
      name: "Delete",
    }),
  );
  fireEvent(window, new Event("focus"));
  await act(async () => {
    finishRead([note]);
    // Complete the IPC read while React still batches this operation.
    for (let step = 0; step < 10; step++) await Promise.resolve();
    expect(acceptedRevision()).toBe(2);
    finishDelete(null);
  });
  await screen.findByText("Could not load notes.");
  expect(screen.queryByRole("option", { name: /Alpha/ })).toBeNull();
  await act(async () =>
    ipc.emit("library-changed", { changed: [note.id], removed: [] }),
  );
  expect(await screen.findByRole("option", { name: /Alpha/ })).toBeTruthy();
});

test("a changed note or state never retains the previous summary", async () => {
  const note = entry("a", { title: "Alpha" });
  let hold = false;
  mockTauri({
    session_document: () =>
      hold
        ? new Promise<SessionDocument>(() => {})
        : document("Alpha summary."),
    session_detail: () =>
      hold ? new Promise<SessionDetail>(() => {}) : detail(note, null),
  });
  const view = render(home([note]));
  await screen.findByText("Alpha summary.");
  hold = true;
  view.rerender(home([entry("b", { title: "Beta" })]));
  expect(screen.queryByText("Alpha summary.")).toBeNull();
  expect(screen.getByText("Loading summary…")).toBeTruthy();
  view.rerender(home([{ ...note, state: "processing" }]));
  expect(screen.queryByText("Alpha summary.")).toBeNull();
  expect(screen.queryByText("Loading summary…")).toBeNull();
});

describe("Home: notes with no audio and other computers", () => {
  // R9.
  test("with the default that does not record, New Note opens a note without recording", async () => {
    const tauri = mockTauri({ start_recording: () => "n" });
    const started: string[] = [];
    let changed = 0;
    render(
      home([], {
        readiness: ready,
        newNoteRecords: false,
        onStarted: (id) => started.push(id),
        onChanged: () => changed++,
      }),
    );
    await userEvent.setup().click(screen.getByRole("button", { name: "New Note" }));
    expect(tauri.calls.find((c) => c.cmd === "start_recording")?.args).toEqual({
      record: false,
      config: null,
    });
    expect(started).toEqual(["n"]);
    expect(changed).toBeGreaterThan(0);
  });

  // R7, AE8.
  test("while the storage folder is unavailable, New Note is unavailable and says why", () => {
    mockTauri();
    render(
      home([], {
        readiness: {
          ...ready,
          armed: false,
          can_start_note: false,
          note_start_refusal: "The storage folder is not available.",
        },
      }),
    );
    expect(screen.getByRole("button", { name: "Record New Note" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("button", { name: "New Note Options" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByText("The storage folder is not available.")).toBeTruthy();
    // One banner only, with the way to the fix.
    expect(screen.queryByText(/Not ready to record|To record,/)).toBeNull();
    expect(screen.getAllByRole("button", { name: "Open Settings…" })).toHaveLength(1);
  });

  test("when a note can start but not record, the record banner shows", () => {
    mockTauri();
    render(home([], { readiness: { ...ready, armed: false } }));
    expect(screen.getByText(/Not ready to record|To record,/)).toBeTruthy();
    expect(screen.getAllByRole("button", { name: "Open Settings…" })).toHaveLength(1);
  });

  // R14.
  test("the preview of a note with no audio shows the start of its notes", async () => {
    const tauri = mockTauri({
      load_session_notes: () => ({ markdown: "First thoughts.\n\nMore.", fingerprint: "f" }),
      session_detail: () => detail(entry("a"), null),
    });
    render(home([entry("a", { duration_ms: 0, found_speakers: null })]));
    const preview = screen.getByRole("region", { name: "Preview" });
    expect(await within(preview).findByText("First thoughts.")).toBeTruthy();
    expect(within(preview).getByText("My Notes")).toBeTruthy();
    expect(within(preview).queryByText("Summary")).toBeNull();
    expect(tauri.calls.some((c) => c.cmd === "session_document")).toBe(false);
  });

  test("the preview of a note with no audio and no notes says so", async () => {
    mockTauri({
      load_session_notes: () => ({ markdown: "", fingerprint: "f" }),
      session_detail: () => detail(entry("a"), null),
    });
    render(home([entry("a", { duration_ms: 0, found_speakers: null })]));
    expect(await screen.findByText("No notes yet.")).toBeTruthy();
  });

  // R15.
  test("a row of a note with no audio shows its title or start time and no length", () => {
    mockTauri();
    renderHome({
      entries: [
        entry("a", { duration_ms: 0, found_speakers: null }),
        entry("b", { title: "Agenda", duration_ms: 0, found_speakers: null }),
      ],
    });
    const untitled = row(sessionTitle(entry("a").started, now));
    expect(untitled.textContent).not.toMatch(/minute|min\b|Ready/);
    expect(within(untitled).getByText("Notes only")).toBeTruthy();
    const titled = row("Agenda");
    expect(titled.textContent).not.toMatch(/minute|min\b|Ready/);
    expect(titled.textContent).toContain("Notes only");
  });

  // R20, R21.
  test("a note in use on another computer says so, and its Delete waits", async () => {
    mockTauri();
    renderHome({ entries: [entry("a", { title: "Planning", remote: "in_use" })] });
    expect(within(row("Planning")).getByText("In use on another computer")).toBeTruthy();
    await userEvent.setup().click(within(row("Planning")).getByRole("button", { name: "Actions" }));
    const del = screen.getByRole("menuitem", { name: "Delete…" });
    expect(del.hasAttribute("disabled")).toBe(true);
    expect(del.getAttribute("title")).toBe("Wait until the other computer is done with this note.");
  });

  // R23.
  test("a note that another computer did not finish says so, and Delete asks with the recording named", async () => {
    const tauri = mockTauri({ delete_session: () => null });
    renderHome({ entries: [entry("a", { title: "Planning", remote: "not_finished", duration_ms: 0 })] });
    expect(within(row("Planning")).getByText("Not finished on another computer")).toBeTruthy();
    const user = userEvent.setup();
    await user.click(within(row("Planning")).getByRole("button", { name: "Actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Delete…" }));
    const dialog = screen.getByRole("alertdialog");
    expect(dialog.textContent).toContain("The recording that another computer did not save is lost.");
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));
    expect(tauri.calls.some((c) => c.cmd === "delete_session")).toBe(true);
  });
});
