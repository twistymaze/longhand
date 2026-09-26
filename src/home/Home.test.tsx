import { describe, expect, mock, test } from "bun:test";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { SessionDetail, SessionDocument, SessionEntry } from "../bindings";
import { sessionTitle } from "../processing/describe";
import { mockTauri } from "../test/tauri";
import { Home, type RecordingNow } from "./Home";

const now = new Date(2026, 8, 24, 18, 0);

const entry = (id: string, over: Partial<SessionEntry> = {}): SessionEntry => ({
  id,
  started: new Date(2026, 8, 24, 14, 5).toISOString(),
  duration_ms: 42 * 60_000,
  state: "saved",
  stage: null,
  title: null,
  found_speakers: 3,
  ...over,
});

const detail = (e: SessionEntry, home: string | null): SessionDetail => ({
  entry: e,
  done: [],
  current: null,
  windows_done: null,
  windows_total: null,
  failed_stage: null,
  message: null,
  summary_problem: null,
  resumable: false,
  home_path: home,
  endpoint: null,
  audio_kept: true,
  transcript_kept: true,
  asr_model: null,
  live_transcript: false,
  transcribe_again: false,
  transcribe_again_error: null,
  summary_not_updated: null,
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

const row = (name: string) => screen.getByRole("option", { name: new RegExp(name) });

describe("Home", () => {
  test("with no sessions, shows the line that names the shortcut and says it starts a new note", () => {
    mockTauri();
    renderHome();
    expect(screen.getByRole("heading", { name: "Notes" })).toBeTruthy();
    expect(screen.getByText("No notes yet")).toBeTruthy();
    expect(
      screen.getByText((_, el) => el?.tagName === "P" && /Press ⌥⇧L .*to start a new note/.test(el.textContent ?? "")),
    ).toBeTruthy();
    expect(screen.queryByRole("option")).toBeNull();
  });

  test("with no sessions and no shortcut, the line names New Note and shows no key cap", () => {
    mockTauri();
    const { container } = renderHome({ chord: null });
    expect(
      screen.getByText((_, el) => el?.tagName === "P" && /New Note/.test(el.textContent ?? "")),
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
          endpoint_configured: true,
        }}
        chord="⌥⇧L"
        onOpen={() => {}}
        onChanged={() => {}}
        now={now}
      />,
    );
    expect(screen.queryByRole("button", { name: "Record" })).toBeNull();
    await userEvent.setup().click(screen.getByRole("button", { name: "New Note" }));
    expect(tauri.calls.map((c) => c.cmd)).toContain("start_recording");
  });

  test("a session with no title shows its start time, and a titled session shows its title", () => {
    mockTauri();
    renderHome({ entries: [entry("a"), entry("b", { title: "Budget review" })] });
    expect(row(sessionTitle(entry("a").started, now))).toBeTruthy();
    expect(row("Budget review")).toBeTruthy();
  });

  test("with no row hovered or selected, there is no preview panel", () => {
    mockTauri();
    renderHome({ entries: [entry("a")] });
    expect(screen.queryByRole("region", { name: "Preview" })).toBeNull();
  });

  test("hovering a row shows its overview in the preview panel, and leaving the list removes it", async () => {
    const e = entry("a", { title: "Budget review" });
    const tauri = mockTauri({
      session_document: () => document("## Overview\n\nWe agreed the **budget** for next year.\n\n## Decisions\n\n- More"),
      session_detail: () => detail(e, "/Users/me/Longhand/2026-09-24 1405 Budget review"),
    });
    const onOpen = mock((_id: string) => {});
    renderHome({ entries: [e], onOpen });
    expect(tauri.calls.filter((c) => c.cmd === "session_document")).toEqual([]);
    const user = userEvent.setup();
    await user.hover(row("Budget review"));
    const preview = await screen.findByRole("region", { name: "Preview" });
    expect(await within(preview).findByText("We agreed the budget for next year.")).toBeTruthy();
    expect(within(preview).getByText("2026-09-24 1405 Budget review")).toBeTruthy();
    expect(within(preview).getByText("3")).toBeTruthy();
    expect(onOpen).not.toHaveBeenCalled();
    await user.unhover(row("Budget review"));
    // The panel stays for a short time after the pointer goes off the row.
    await act(() => new Promise((done) => setTimeout(done, 300)));
    expect(screen.queryByRole("region", { name: "Preview" })).toBeNull();
  });

  test("the arrow keys select a row and show its preview", async () => {
    mockTauri({
      session_document: () => document("An overview."),
      session_detail: () => detail(entry("a"), null),
    });
    renderHome({ entries: [entry("a", { title: "Budget review" }), entry("b", { title: "Standup" })] });
    row("Budget review").focus();
    await userEvent.setup().keyboard("{ArrowDown}");
    expect(row("Budget review").getAttribute("aria-selected")).toBe("true");
    const preview = await screen.findByRole("region", { name: "Preview" });
    expect(within(preview).getByRole("heading", { name: "Budget review" })).toBeTruthy();
  });

  test("a single click and Return open the note", async () => {
    mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("a"), null),
    });
    const onOpen = mock((_id: string) => {});
    renderHome({ entries: [entry("a", { title: "Budget review" })], onOpen });
    const user = userEvent.setup();
    await user.click(row("Budget review"));
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
    await userEvent.setup().click(within(row("Budget review")).getByRole("button", { name: "Actions" }));
    expect(screen.getByRole("menuitem", { name: "Rename" })).toBeTruthy();
    expect(onOpen).not.toHaveBeenCalled();
  });

  // Covers AE6.
  test("with a recording in progress, shows the recording row with the elapsed time, and selecting it opens the page", async () => {
    mockTauri();
    const onOpen = mock((_id: string) => {});
    renderHome({
      entries: [entry("rec", { state: "capturing" })],
      recording: { sessionId: "rec", elapsedMs: 5 * 60_000 + 7_000, interrupted: false },
      onOpen,
    });
    const recording = screen.getByRole("button", { name: /Recording/ });
    expect(recording.textContent).toContain("00:05:07");
    // The open session is the recording row, not a row in the list.
    expect(screen.queryByRole("option")).toBeNull();
    await userEvent.setup().click(recording);
    expect(onOpen).toHaveBeenCalledWith("rec");
  });

  // Covers AE5.
  test("the menu of a processing session has Delete disabled", async () => {
    mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("p"), null),
    });
    renderHome({ entries: [entry("p", { state: "processing", title: "Budget review" })] });
    const user = userEvent.setup();
    await user.click(within(row("Budget review")).getByRole("button", { name: "Actions" }));
    const del = screen.getByRole("menuitem", { name: "Delete…" });
    expect(del.hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("menuitem", { name: "Rename" }).hasAttribute("disabled")).toBe(false);
  });

  // Covers AE10.
  test("a Trash failure offers Delete Permanently, and Cancel leaves the row", async () => {
    const tauri = mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("s"), "/x/Budget review"),
      delete_session: () => {
        throw { kind: "cannot_move_to_trash", path: "/x/Budget review", reason: "the volume has no Trash" };
      },
      delete_session_permanently: () => null,
    });
    const onChanged = mock(() => {});
    renderHome({ entries: [entry("s", { title: "Budget review" })], onChanged });
    const user = userEvent.setup();
    await user.click(within(row("Budget review")).getByRole("button", { name: "Actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Delete…" }));
    const confirm = screen.getByRole("alertdialog");
    await user.click(within(confirm).getByRole("button", { name: "Delete" }));
    const permanent = await screen.findByRole("alertdialog", { name: /can’t be moved to the Trash/ });
    expect(within(permanent).getByRole("button", { name: "Delete Permanently" })).toBeTruthy();
    await user.click(within(permanent).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(row("Budget review")).toBeTruthy();
    expect(tauri.calls.some((c) => c.cmd === "delete_session_permanently")).toBe(false);
    expect(onChanged).not.toHaveBeenCalled();
  });

  // Covers AE10: the second question deletes the session for good.
  test("a Trash failure offers Delete Permanently, which deletes the session and its row", async () => {
    let entries = [entry("s", { title: "Budget review" })];
    const tauri = mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("s"), "/x/Budget review"),
      delete_session: () => {
        throw { kind: "cannot_move_to_trash", path: "/x/Budget review", reason: "the volume has no Trash" };
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
    const view = renderHome({ entries, onChanged: () => view.rerender(homeWith(entries)) });
    const user = userEvent.setup();
    await user.click(within(row("Budget review")).getByRole("button", { name: "Actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Delete…" }));
    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Delete" }));
    const permanent = await screen.findByRole("alertdialog", { name: /can’t be moved to the Trash/ });
    await user.click(within(permanent).getByRole("button", { name: "Delete Permanently" }));
    await act(async () => {});
    expect(tauri.calls.filter((c) => c.cmd === "delete_session_permanently").map((c) => c.args)).toEqual([
      { sessionId: "s" },
    ]);
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
    renderHome({ entries: [entry("s", { title: "Budget review" })], onChanged });
    const user = userEvent.setup();
    await user.click(within(row("Budget review")).getByRole("button", { name: "Actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Delete…" }));
    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Delete" }));
    expect(await screen.findByText("The store is locked.")).toBeTruthy();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(row("Budget review")).toBeTruthy();
    expect(tauri.calls.some((c) => c.cmd === "delete_session_permanently")).toBe(false);
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

    await user.click(within(row("Old title")).getByRole("button", { name: "Actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    await user.clear(screen.getByRole("textbox", { name: "Title" }));
    await user.keyboard("Discarded{Escape}");
    expect(screen.queryByRole("textbox", { name: "Title" })).toBeNull();
    expect(tauri.calls.some((c) => c.cmd === "rename_session")).toBe(false);

    await user.click(within(row("Old title")).getByRole("button", { name: "Actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    await user.clear(screen.getByRole("textbox", { name: "Title" }));
    await user.keyboard("Budget review{Enter}");
    await act(async () => {});
    expect(tauri.calls.filter((c) => c.cmd === "rename_session").map((c) => c.args)).toEqual([
      { sessionId: "s", title: "Budget review" },
    ]);
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
    await user.click(within(row("Old title")).getByRole("button", { name: "Actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    await user.clear(screen.getByRole("textbox", { name: "Title" }));
    await user.keyboard("{Enter}");
    await act(async () => {});
    expect(tauri.calls.filter((c) => c.cmd === "rename_session").map((c) => c.args)).toEqual([
      { sessionId: "s", title: "" },
    ]);
  });

  test("a folder that keeps its old name shows the message", async () => {
    mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("s"), null),
      rename_session: () => ({ kind: "kept", message: "the folder is open in another app" }),
    });
    renderHome({ entries: [entry("s", { title: "Old title" })] });
    const user = userEvent.setup();
    await user.click(within(row("Old title")).getByRole("button", { name: "Actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    await user.clear(screen.getByRole("textbox", { name: "Title" }));
    await user.keyboard("New{Enter}");
    expect(await screen.findByText(/The folder is open in another app/)).toBeTruthy();
  });
});
