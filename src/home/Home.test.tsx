import { describe, expect, test, vi } from "vitest";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type {
  NoteTypeList,
  Readiness,
  SessionDetail,
  SessionDocument,
  SessionEntry,
  TagEntry,
} from "../bindings";
import { tagKey, tagName } from "../lib/Labels";
import { sessionTitle } from "../processing/describe";
import { type CommandHandler, mockTauri } from "../test/tauri";
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
  problem: null,
  remote: null,
  note_type: null,
  tags: [],
  waits_for: null,
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
  low_coverage: false,
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
  regenerating: false,
  transcribe_again_error: null,
  retry_starts_over: false,
  summary_not_updated: null,
  waiting: [],
  conflict_files: [],
  not_saved_yet: null,
});

const document = (body: string): SessionDocument => ({
  has_speakers: true,
  lines: [],
  transcript_markdown: "",
  summary: { body, stub: false, host: null, model: null, prompt: null, prompt_id: null },
});

function renderHome({
  entries = [] as SessionEntry[],
  recording = null as RecordingNow | null,
  chord = "⌥⇧L" as string | null,
  onOpen = (_id: string) => {},
  onChanged = () => {},
} = {}) {
  return render(
    <Home
      entries={entries}
      stages={{}}
      recording={recording}
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
      noteType: null,
    });
    // The saved default configuration applies; no override is set.
    expect(tauri.calls.some((c) => c.cmd === "set_recognition_override")).toBe(false);
  });

  test("a failed start shows its error, and a start that a Stop cancelled shows nothing", async () => {
    let error = "microphone: the microphone did not start within 5s";
    mockTauri({
      start_recording: () => {
        throw error;
      },
    });
    const { container } = render(
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
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Record New Note" }));
    expect(
      await screen.findByText(/The microphone did not start within 5s/i),
    ).toBeTruthy();

    error = "";
    await user.click(screen.getByRole("button", { name: "Record New Note" }));
    await waitFor(() =>
      expect(screen.queryByText(/did not start within 5s/i)).toBeNull(),
    );
    expect(container.querySelector("p[role=status].text-negative")).toBeNull();
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
    const onOpen = vi.fn((_id: string) => {});
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
    const onOpen = vi.fn((_id: string) => {});
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
    const onOpen = vi.fn((_id: string) => {});
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
    const onOpen = vi.fn((_id: string) => {});
    renderHome({
      entries: [entry("rec", { state: "capturing" })],
      recording: {
        noteId: "rec",
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

  // Covers R9, R11.
  test("a paused note stays the open-note row with Paused, its time and a grey icon, and New Note stays unavailable", () => {
    mockTauri();
    render(
      <Home
        entries={[entry("rec", { state: "capturing" })]}
        stages={{}}
        recording={{ noteId: "rec", elapsedMs: 5 * 60_000 + 12_000, interrupted: false, paused: true }}
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
    const paused = row("Paused");
    expect(paused.getAttribute("aria-selected")).toBe("true");
    expect(paused.textContent).toContain("Open to continue or stop");
    expect(paused.textContent).toContain("00:05:12");
    expect(paused.textContent).not.toMatch(/Recording|In progress/);
    const icon = paused.querySelector("[aria-hidden]")!;
    expect(icon.className).toContain("text-label-secondary");
    expect(icon.className).not.toMatch(/negative|caution/);
    expect(screen.queryByRole("button", { name: /New Note/ })).toBeNull();
  });

  test("an interrupted note still shows Recording interrupted with the caution colour", () => {
    mockTauri();
    renderHome({
      entries: [entry("rec", { state: "interrupted" })],
      recording: { noteId: "rec", elapsedMs: 60_000, interrupted: true, paused: false },
    });
    const interrupted = row("Recording interrupted");
    expect(interrupted.textContent).toContain("Open to resume or stop");
    expect(interrupted.querySelector("[aria-hidden]")!.className).toContain("text-caution");
  });

  test("while processing waits for a recording, a processing row and its preview say so", () => {
    mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("p"), null),
    });
    renderHome({
      entries: [entry("p", { state: "processing", stage: "assemble", title: "Budget review", waits_for: "recording" })],
    });
    expect(within(row("Budget review")).getByText("Waiting for the recording to end")).toBeTruthy();
    expect(screen.queryByText("Preparing the audio")).toBeNull();
  });

  test("a row says what its note waits for, with the texts of the note page", () => {
    mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("p"), null),
    });
    renderHome({
      entries: [
        entry("a", { state: "processing", stage: "assemble", title: "Claimed", waits_for: "another_note" }),
        entry("q", { state: "queued", title: "Queued", waits_for: "another_note" }),
        // A recording is active, but the note waits for its summary turn.
        entry("s", { state: "processing", stage: "summarize", title: "Summary", waits_for: "summary_turn" }),
        entry("m", { state: "queued", title: "Model", waits_for: "model" }),
      ],
    });
    expect(within(row("Claimed")).getByText("Waiting for another note to finish")).toBeTruthy();
    expect(within(row("Queued")).getByText("Waiting for another note to finish")).toBeTruthy();
    expect(within(row("Summary")).getByText("Waiting to summarize")).toBeTruthy();
    expect(within(row("Model")).getByText("Waiting for the model to download")).toBeTruthy();
    expect(screen.queryByText("Preparing the audio")).toBeNull();
    expect(screen.queryByText("Summarizing")).toBeNull();
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
    const onChanged = vi.fn(() => {});
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
    ).toEqual([{ noteId: "s" }]);
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
    const onChanged = vi.fn(() => {});
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
    const onChanged = vi.fn(() => {});
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
    ).toEqual([{ noteId: "s", title: "Budget review" }]);
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
    ).toEqual([{ noteId: "s", title: "" }]);
  });

  // The app renames the folder later without help: the user gets no message.
  test("a folder that keeps its old name for now shows no message", async () => {
    const tauri = mockTauri({
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
    await waitFor(() =>
      expect(tauri.calls.some((c) => c.cmd === "rename_session")).toBe(true),
    );
    await act(async () => {});
    expect(screen.queryByText(/open in another app/)).toBeNull();
    expect(screen.queryByText(/kept its old name/)).toBeNull();
  });

  test("a rename that fails shows the error", async () => {
    mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("s"), null),
      rename_session: () => {
        throw "the note was not found";
      },
    });
    renderHome({ entries: [entry("s", { title: "Old title" })] });
    const user = userEvent.setup();
    await user.click(
      within(row("Old title")).getByRole("button", { name: "Actions" }),
    );
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    await user.clear(screen.getByRole("textbox", { name: "Title" }));
    await user.keyboard("New{Enter}");
    expect(await screen.findByText("The note was not found.")).toBeTruthy();
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

  // Covers R6: a note whose file does not parse stays listed. Rename waits,
  // and Delete stays available.
  test("a note whose session.md does not parse cannot be renamed and can be deleted", async () => {
    mockTauri({ library_notices: () => ({ conversion: null, problems: [] }) });
    const reason = "session.md has an error on line 3. Correct the file to change this note.";
    renderHome({
      entries: [
        entry("a", {
          title: "Planning",
          problem: { kind: "user_file", file: "session.md", line: 3, reason },
        }),
      ],
    });
    await userEvent.setup().click(within(row("Planning")).getByRole("button", { name: "Actions" }));
    expect(screen.getByRole("menuitem", { name: "Rename" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("menuitem", { name: "Delete…" }).hasAttribute("disabled")).toBe(false);
  });

  // Covers AE3, R6, KTD14: one notice counts the notes whose files do not
  // parse and lists each with what to do.
  test("notes whose files do not parse show in one notice", async () => {
    mockTauri({
      library_notices: () => ({
        conversion: null,
        problems: [
          {
            name: "Planning",
            problem: {
              kind: "user_file",
              file: "session.md",
              line: 3,
              reason: "session.md has an error on line 3. Correct the file to change this note.",
            },
          },
          {
            name: "2026-09-24 1405 Budget",
            problem: {
              kind: "record",
              file: "longhand.json",
              line: null,
              reason: "This note waits for longhand.json to sync. If it does not sync, update Longhand on the other computer.",
            },
          },
        ],
      }),
    });
    renderHome({ entries: [entry("a", { title: "Planning" })] });
    const notice = await screen.findByRole("region", { name: "Notes that Longhand can’t read" });
    expect(within(notice).getByText("Longhand can’t read 2 notes.")).toBeTruthy();
    expect(within(notice).getByText(/^session\.md has an error on line 3/)).toBeTruthy();
    expect(within(notice).getByText("2026-09-24 1405 Budget")).toBeTruthy();
    expect(within(notice).getByText(/^This note waits for longhand\.json/)).toBeTruthy();
  });

  // Covers R6: the normal path shows nothing.
  test("a library with no problem shows no library notice", async () => {
    const tauri = mockTauri({ library_notices: () => ({ conversion: null, problems: [] }) });
    renderHome({ entries: [entry("a", { title: "Planning" })] });
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "library_notices")).toBe(true));
    expect(screen.queryByRole("region", { name: "Notes that Longhand can’t read" })).toBeNull();
    // Covers AE4: with no old notes, no conversion control shows.
    expect(screen.queryByRole("region", { name: "Notes in an older format" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Convert" })).toBeNull();
  });

  // Covers F3, R11, R12: the Convert notice counts the old notes, and only
  // the user starts the conversion. While it runs, the button waits.
  test("the convert notice converts when the user selects Convert", async () => {
    let finish = () => {};
    let old = 2;
    const tauri = mockTauri({
      library_notices: () => ({
        conversion: old > 0 ? { old_notes: old, running: false, skipped: 0, failed: [] } : null,
        problems: [],
      }),
      convert_old_notes: () =>
        new Promise<null>((resolve) => {
          finish = () => {
            old = 0;
            resolve(null);
          };
        }),
    });
    renderHome({ entries: [entry("a", { title: "Planning" })] });
    const notice = await screen.findByRole("region", { name: "Notes in an older format" });
    expect(within(notice).getByText(/^2 notes are in an older format\./)).toBeTruthy();
    expect(tauri.calls.some((c) => c.cmd === "convert_old_notes")).toBe(false);

    await userEvent.setup().click(within(notice).getByRole("button", { name: "Convert" }));
    const busy = await within(notice).findByRole("button", { name: "Converting…" });
    expect(busy.hasAttribute("disabled")).toBe(true);
    await act(async () => finish());
    // All notes converted: the notice goes, with no success message.
    await waitFor(() => expect(screen.queryByRole("region", { name: "Notes in an older format" })).toBeNull());
  });

  // Covers R10: skipped notes and failures stay in the notice, after the
  // count, and Convert stays available.
  test("the convert notice says which notes are in use and which did not convert", async () => {
    mockTauri({
      library_notices: () => ({
        conversion: {
          old_notes: 3,
          running: false,
          skipped: 1,
          failed: [{ name: "Budget", reason: "the frontmatter cannot change" }],
        },
        problems: [],
      }),
    });
    renderHome({ entries: [] });
    const notice = await screen.findByRole("region", { name: "Notes in an older format" });
    expect(within(notice).getByText(/1 note could not be converted yet because it was in use\. Select Convert again later\./)).toBeTruthy();
    expect(within(notice).getByText(/These notes could not be converted: Budget\./)).toBeTruthy();
    expect(within(notice).getByRole("button", { name: "Convert" }).hasAttribute("disabled")).toBe(false);
  });

  // Covers R10: a conversion that fails as a whole says so, and Convert
  // stays available. The error does not escape the notice.
  for (const [kind, fail] of [
    ["an error", () => Promise.reject(new Error("the call failed"))],
    ["an error result", () => Promise.reject("the run stopped")],
  ] as const) {
    test(`the convert notice shows a failure when the conversion returns ${kind}`, async () => {
      const unhandled: unknown[] = [];
      const onUnhandled = (event: PromiseRejectionEvent) => {
        unhandled.push(event.reason);
        event.preventDefault();
      };
      window.addEventListener("unhandledrejection", onUnhandled);
      try {
        mockTauri({
          library_notices: () => ({
            conversion: { old_notes: 2, running: false, skipped: 0, failed: [] },
            problems: [],
          }),
          convert_old_notes: fail,
        });
        renderHome({ entries: [] });
        const notice = await screen.findByRole("region", { name: "Notes in an older format" });
        await userEvent.setup().click(within(notice).getByRole("button", { name: "Convert" }));
        expect(await within(notice).findByText("The conversion did not finish. Select Convert to try again.")).toBeTruthy();
        expect(within(notice).getByRole("button", { name: "Convert" }).hasAttribute("disabled")).toBe(false);
        expect(unhandled).toEqual([]);
      } finally {
        window.removeEventListener("unhandledrejection", onUnhandled);
      }
    });
  }
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
      recording: { noteId: "a", elapsedMs: 0, interrupted: false },
    }),
  );
  expect(screen.queryByText("No notes yet")).toBeNull();
  view.rerender(home([entry("a", { state: "capturing" })]));
  await act(async () => {});
  expect(screen.getByRole("option").getAttribute("aria-selected")).toBe("true");
});

test("arrow keys include interrupted and recording notes after a pointer selection", async () => {
  previewMocks();
  const onOpen = vi.fn(() => {});
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
  const onOpen = vi.fn(() => {});
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
  const onOpen = vi.fn(() => {});
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
    session_document: ({ noteId }) =>
      noteId === "a"
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
  const onChanged = vi.fn(() => {});
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
      noteType: null,
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

  // Safe agent edits U6, R22: a settings file that did not parse for about
  // one minute names the file and the line. The normal path shows nothing.
  test("a damaged shared settings file shows a notice with the file and the line", () => {
    mockTauri();
    const view = render(home([], { readiness: ready }));
    expect(screen.queryByText(/can’t read the settings file/)).toBeNull();
    view.rerender(home([], { readiness: ready, damagedFile: { path: ".longhand/settings.json", line: 3 } }));
    const notice = screen.getByText(/can’t read the settings file/);
    expect(notice.textContent).toContain("(.longhand/settings.json, line 3)");
    expect(notice.textContent).toContain("Fix the file, or restore it from another computer.");
    expect(notice.textContent).toContain("the recording widget and New Note");
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

describe("Home: filter by type and tags", () => {
  const typeList = (...names: string[]): NoteTypeList => ({
    types: [
      { name: "General", valid: true, shared: false, notes: 1, general: true, prompt_choice: null, prompt_name: null, summary_prompt_id: "topic-summary" },
      ...names.map((name) => ({ name, valid: true, shared: true, notes: 1, general: false, prompt_choice: null, prompt_name: null, summary_prompt_id: "topic-summary" })),
    ],
    default_type: null,
  });
  const tagList = (...texts: string[]): TagEntry[] =>
    texts.map((text) => ({ text, key: tagKey(text), count: 1 }));
  const ready: Readiness = {
    armed: true,
    microphone_authorized: true,
    destination: { kind: "ready", path: "/notes", free_bytes: null },
    shortcut: { chord: null, registered: false, error: null },
    system_audio: "allowed",
    missing_models: [],
    endpoint_configured: true,
    can_start_note: true,
    note_start_refusal: null,
  };
  const notes = [
    entry("a", { title: "1:1 with Nathan", note_type: "One on One", tags: ["Person::Nathan"] }),
    entry("b", { title: "1:1 with Sarah", note_type: "One on One", tags: ["Person :: Sarah"] }),
    entry("c", { title: "Session 12", note_type: "Pathfinder", tags: ["person::nathan", "Topic::Loot"] }),
    entry("d", { title: "Standup", note_type: null, tags: [] }),
  ];
  // The rows of the notes list. An open Tags menu has options too.
  const titles = () => {
    const list = screen.queryByRole("listbox", { name: "Notes" });
    return list ? within(list).queryAllByRole("option").map((o) => o.querySelector(".font-medium")?.textContent ?? "") : [];
  };

  function setup({
    types = typeList("Pathfinder", "One on One"),
    tags = tagList("Person::Nathan", "Person::Sarah", "Topic::Loot"),
    entries = notes,
    handlers = {} as Record<string, CommandHandler>,
  } = {}) {
    const state = { types, tags };
    const tauri = mockTauri({
      note_types: () => state.types,
      list_tags: () => state.tags,
      ...handlers,
    });
    const props = {
      entries,
      stages: {},
      recording: null,
      readiness: ready,
      chord: null,
      onOpen: () => {},
      onChanged: () => {},
      now,
    };
    const view = render(<Home {...props} />);
    return { tauri, state, view, props };
  }

  type User = ReturnType<typeof userEvent.setup>;
  const chooseType = async (user: User, name: string) => user.click(await screen.findByRole("radio", { name }));
  const addTag = async (user: User, text: string) => {
    // The Tags button shows after the tag list is read (R5). The menu
    // stays open after a choice (R9).
    const button = await screen.findByRole("button", { name: "Tags" });
    if (button.getAttribute("aria-expanded") !== "true") await user.click(button);
    const menu = await screen.findByRole("listbox", { name: "Tags" });
    await user.click(within(menu).getByRole("option", { name: tagName(text) }));
  };

  // F3, R19, R20.
  test("a type and a tag show only the notes with both, and removing the tag label shows the type again", async () => {
    setup();
    const user = userEvent.setup();
    await chooseType(user, "One on One");
    expect(titles()).toEqual(["1:1 with Nathan", "1:1 with Sarah"]);
    await addTag(user, "Person::Nathan");
    expect(titles()).toEqual(["1:1 with Nathan"]);
    const chosen = screen.getByRole("list", { name: "Chosen tags" });
    await user.click(within(chosen).getByRole("button", { name: "Remove Person: Nathan" }));
    expect(titles()).toEqual(["1:1 with Nathan", "1:1 with Sarah"]);
    expect(screen.queryByRole("list", { name: "Chosen tags" })).toBeNull();
    // The focus stays in the bar. WKWebView does not move it by itself.
    expect(globalThis.document.activeElement).toBe(screen.getByRole("button", { name: "Tags" }));
  });

  test("a tag matches without regard to case and spaces around ::, and every chosen tag must match", async () => {
    setup();
    const user = userEvent.setup();
    await addTag(user, "Person::Nathan");
    expect(titles()).toEqual(["1:1 with Nathan", "Session 12"]);
    await addTag(user, "Topic::Loot");
    expect(titles()).toEqual(["Session 12"]);
  });

  test("General shows only the notes at the top of the storage folder", async () => {
    setup();
    await chooseType(userEvent.setup(), "General");
    expect(titles()).toEqual(["Standup"]);
  });

  // R2: General is a choice when types are in use, also with no note in it.
  test("with no note in General, General is still a choice", async () => {
    setup({ entries: notes.filter((n) => n.note_type !== null) });
    await screen.findByRole("radio", { name: "Pathfinder" });
    expect(screen.getByRole("radio", { name: "General" })).toBeTruthy();
    expect(screen.getByRole("radio", { name: "All" })).toBeTruthy();
  });

  // R3: the type filter goes when the last other type goes, and so does its choice.
  test("a chosen General drops out of the filter when no other type is left", async () => {
    const { tauri, state } = setup();
    await chooseType(userEvent.setup(), "General");
    expect(titles()).toEqual(["Standup"]);
    state.types = typeList();
    await act(() => tauri.emit("settings-changed", { settings: {} }));
    await waitFor(() => expect(screen.queryByRole("radiogroup", { name: "Note Type" })).toBeNull());
    expect(titles()).toHaveLength(4);
    expect(screen.queryByRole("button", { name: "Clear Filters" })).toBeNull();
  });

  // AE1, R1, R5.
  test("with only General and no tags, Home shows no filter bar", async () => {
    const { tauri } = setup({ types: typeList(), tags: [], entries: [entry("d", { title: "Standup" })] });
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "list_tags")).toBe(true));
    await act(() => Promise.resolve());
    expect(screen.queryByRole("radiogroup", { name: "Note Type" })).toBeNull();
    expect(screen.queryByRole("button", { name: /Note Type/ })).toBeNull();
    expect(screen.queryByRole("button", { name: "Tags" })).toBeNull();
    expect(screen.queryByRole("group", { name: "Filter" })).toBeNull();
  });

  // AE2, R1, R2.
  test("a type made in Settings shows the type filter with All, General and the type", async () => {
    const { tauri, state } = setup({ types: typeList(), tags: [], entries: [entry("d", { title: "Standup" })] });
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "note_types")).toBe(true));
    state.types = { ...typeList("Pathfinder"), types: typeList("Pathfinder").types.map((t) => ({ ...t, notes: t.general ? 1 : 0 })) };
    await act(() => tauri.emit("settings-changed", { settings: {} }));
    await screen.findByRole("radio", { name: "Pathfinder" });
    expect(screen.getAllByRole("radio").map((r) => r.textContent)).toEqual(["All", "General", "Pathfinder"]);
    expect(screen.queryByRole("button", { name: "Tags" })).toBeNull();
  });

  // AE4, R3, R5.
  test("when the last tag goes, the Tags menu and the chosen tag go", async () => {
    const { tauri, state } = setup({
      tags: tagList("follow-up"),
      entries: [entry("a", { title: "Planning", note_type: "Pathfinder", tags: ["follow-up"] }), entry("d", { title: "Standup" })],
    });
    await addTag(userEvent.setup(), "follow-up");
    expect(titles()).toEqual(["Planning"]);
    state.tags = [];
    await act(() => tauri.emit("library-changed", { changed: [], removed: [] }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Tags" })).toBeNull());
    expect(screen.queryByRole("list", { name: "Chosen tags" })).toBeNull();
    expect(titles()).toHaveLength(2);
  });

  // AE7, R12.
  test("Clear Filters in the filter bar shows every note and goes away", async () => {
    setup();
    const user = userEvent.setup();
    const bar = await screen.findByRole("group", { name: "Filter" });
    expect(within(bar).queryByRole("button", { name: "Clear Filters" })).toBeNull();
    await chooseType(user, "One on One");
    await addTag(user, "Person::Nathan");
    expect(titles()).toEqual(["1:1 with Nathan"]);
    await user.click(within(bar).getByRole("button", { name: "Clear Filters" }));
    expect(titles()).toHaveLength(4);
    expect(screen.getByRole("radio", { name: "All" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.queryByRole("list", { name: "Chosen tags" })).toBeNull();
    expect(within(bar).queryByRole("button", { name: "Clear Filters" })).toBeNull();
  });

  // AE5, R21.
  test("a recording note of another type stays visible", async () => {
    setup({
      entries: [...notes, entry("r", { state: "capturing", note_type: "Internal Meeting", duration_ms: 1000 })],
      types: typeList("Pathfinder", "One on One", "Internal Meeting"),
    });
    await chooseType(userEvent.setup(), "Pathfinder");
    expect(screen.getByRole("option", { name: /Recording/ })).toBeTruthy();
    expect(screen.getByRole("option", { name: /Session 12/ })).toBeTruthy();
    expect(screen.queryByRole("option", { name: /Standup/ })).toBeNull();
  });

  // R32.
  test("with no match, Home says so, and Clear Filters shows all notes again", async () => {
    setup();
    const user = userEvent.setup();
    await chooseType(user, "General");
    await addTag(user, "Topic::Loot");
    expect(titles()).toHaveLength(0);
    expect(screen.getByText("No notes match the filter.")).toBeTruthy();
    expect(screen.queryByText("No notes yet")).toBeNull();
    // The filter bar has its own Clear Filters button too (R12).
    const bar = screen.getByRole("group", { name: "Filter" });
    expect(within(bar).getByRole("button", { name: "Clear Filters" })).toBeTruthy();
    const empty = screen.getByText("No notes match the filter.").parentElement as HTMLElement;
    await user.click(within(empty).getByRole("button", { name: "Clear Filters" }));
    expect(titles()).toHaveLength(4);
    expect(screen.getByRole("radio", { name: "All" }).getAttribute("aria-checked")).toBe("true");
  });

  // R33.
  test("the filter stays when the user opens a note and comes back", async () => {
    const { view, props } = setup();
    await chooseType(userEvent.setup(), "Pathfinder");
    view.rerender(<Home {...props} active={false} />);
    view.rerender(<Home {...props} active />);
    await waitFor(() => expect(titles()).toEqual(["Session 12"]));
    expect(screen.getByRole("radio", { name: "Pathfinder" }).getAttribute("aria-checked")).toBe("true");
  });

  test("a type that Settings deletes drops out of the filter, and the chosen tags stay", async () => {
    const { tauri, state } = setup();
    const user = userEvent.setup();
    await chooseType(user, "Pathfinder");
    await addTag(user, "Person::Nathan");
    expect(titles()).toEqual(["Session 12"]);
    state.types = typeList("One on One");
    await act(() => tauri.emit("settings-changed", { settings: {} }));
    await waitFor(() => expect(screen.queryByRole("radio", { name: "Pathfinder" })).toBeNull());
    expect(screen.getByRole("radio", { name: "All" }).getAttribute("aria-checked")).toBe("true");
    expect(titles()).toEqual(["1:1 with Nathan", "Session 12"]);
  });

  test("a tag that is on no note any more drops out of the filter", async () => {
    const { tauri, state } = setup();
    const user = userEvent.setup();
    await addTag(user, "Topic::Loot");
    expect(titles()).toEqual(["Session 12"]);
    state.tags = tagList("Person::Nathan", "Person::Sarah");
    await act(() => tauri.emit("library-changed", { changed: [], removed: [] }));
    await waitFor(() => expect(titles()).toHaveLength(4));
    expect(screen.queryByRole("list", { name: "Chosen tags" })).toBeNull();
  });

  // R19.
  test("with many types, one menu replaces the segmented control", async () => {
    setup({ types: typeList("A", "B", "C", "D", "E", "F", "G") });
    const user = userEvent.setup();
    const menuButton = await screen.findByRole("button", { name: "Note Type: All Types" });
    expect(screen.queryByRole("radiogroup", { name: "Note Type" })).toBeNull();
    await user.click(menuButton);
    const items = screen.getAllByRole("menuitemradio").map((i) => i.textContent);
    expect(items).toEqual(["All Types", "General", "A", "B", "C", "D", "E", "F", "G"]);
    await user.click(screen.getByRole("menuitemradio", { name: "General" }));
    expect(titles()).toEqual(["Standup"]);
    expect(screen.getByRole("button", { name: "Note Type: General" })).toBeTruthy();
  });

  test("after a press on the type menu button that ends off the button, a key opens the menu", async () => {
    setup({ types: typeList("A", "B", "C", "D", "E", "F", "G") });
    const user = userEvent.setup();
    const button = await screen.findByRole("button", { name: "Note Type: All Types" });
    await user.click(button);
    expect(screen.getAllByRole("menuitemradio").length).toBeGreaterThan(0);
    // The press closes the menu. It ends off the button, so no click follows.
    fireEvent.pointerDown(button);
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    // Space or Return on the button gives a click with no pointer press.
    fireEvent.click(button, { detail: 0 });
    expect(screen.getAllByRole("menuitemradio").length).toBeGreaterThan(0);
  });

  // R22, KTD3.
  test("a row shows the type and the tags", () => {
    setup({
      entries: [
        entry("p", { title: "Planning", note_type: "One on One", tags: ["Person::Nathan", "follow-up"] }),
      ],
    });
    const planning = row("Planning");
    expect(within(planning).getByText("One on One").closest('[data-label="type"]')).toBeTruthy();
    expect(within(planning).getByText("Person: Nathan")).toBeTruthy();
    expect(within(planning).getByText("follow-up")).toBeTruthy();
  });

  // AE3, R4: when types are in use, a row of General shows a General label.
  test("with other types, a row of General shows a General type label", async () => {
    setup({ entries: [entry("g", { title: "Standup", note_type: null, tags: ["follow-up"] })] });
    const standup = row("Standup");
    const label = await within(standup).findByText("General");
    expect(label.closest('[data-label="type"]')).toBeTruthy();
    expect(within(standup).getByText("follow-up")).toBeTruthy();
  });

  // R4: with only General, a row of General shows no type label.
  test("with only General, a row of General shows no type label", async () => {
    const { tauri } = setup({ types: typeList(), entries: [entry("g", { title: "Standup", note_type: null, tags: ["follow-up"] })] });
    await waitFor(() => expect(tauri.calls.some((c) => c.cmd === "note_types")).toBe(true));
    await act(() => Promise.resolve());
    const standup = row("Standup");
    expect(standup.querySelector('[data-label="type"]')).toBeNull();
    expect(within(standup).getByText("follow-up")).toBeTruthy();
  });

  // R4: a recording of a General note shows the General label too.
  test("with other types, a recording of a General note shows a General label", async () => {
    setup({ entries: [entry("r", { state: "capturing", note_type: null, duration_ms: 1000 })] });
    const recording = screen.getByRole("option", { name: /Recording/ });
    const label = await within(recording).findByText("General");
    expect(label.closest('[data-label="type"]')).toBeTruthy();
  });

  // R22.
  test("a recording row shows the type and the tags", () => {
    setup({
      entries: [entry("r", { state: "capturing", note_type: "Internal Meeting", tags: ["follow-up"], duration_ms: 1000 })],
    });
    const recording = screen.getByRole("option", { name: /Recording/ });
    expect(within(recording).getByText("Internal Meeting").closest('[data-label="type"]')).toBeTruthy();
    expect(within(recording).getByText("follow-up")).toBeTruthy();
  });

  // F1, R24.
  test("choosing a type in the New Note menu starts a note of that type", async () => {
    const { tauri } = setup({ handlers: { start_recording: () => "s" } });
    const user = userEvent.setup();
    await screen.findByRole("radio", { name: "Pathfinder" });
    await user.click(screen.getByRole("button", { name: "New Note Options" }));
    await user.click(screen.getByRole("menuitem", { name: "Pathfinder" }));
    expect(tauri.calls.find((c) => c.cmd === "start_recording")?.args).toEqual({
      record: true,
      config: null,
      noteType: { kind: "named", name: "Pathfinder" },
    });
  });
});
