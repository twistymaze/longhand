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
});

const document = (body: string): SessionDocument => ({
  has_speakers: true,
  lines: [],
  transcript_markdown: "",
  summary: { body, stub: false, host: null, model: null },
});

function renderHome({
  entries = [] as SessionEntry[],
  recording = null as RecordingNow | null,
  onOpen = (_id: string) => {},
  onChanged = () => {},
} = {}) {
  return render(
    <Home
      entries={entries}
      stages={{}}
      recording={recording}
      readiness={null}
      chord="⌥⇧L"
      onOpen={onOpen}
      onChanged={onChanged}
      now={now}
    />,
  );
}

const row = (name: string) => screen.getByRole("option", { name: new RegExp(name) });

describe("Home", () => {
  test("with no sessions, shows the line that names the shortcut", () => {
    mockTauri();
    renderHome();
    expect(screen.getByText(/Press ⌥⇧L to record/)).toBeTruthy();
    expect(screen.queryByRole("option")).toBeNull();
  });

  test("a session with no title shows its start time, and a titled session shows its title", () => {
    mockTauri();
    renderHome({ entries: [entry("a"), entry("b", { title: "Budget review" })] });
    expect(row(sessionTitle(entry("a").started, now))).toBeTruthy();
    expect(row("Budget review")).toBeTruthy();
  });

  test("with no row selected, the preview panel shows nothing", () => {
    mockTauri();
    renderHome({ entries: [entry("a")] });
    expect(screen.getByRole("region", { name: "Preview" }).textContent).toBe("");
  });

  test("selecting a row shows its overview in the preview panel", async () => {
    const e = entry("a", { title: "Budget review" });
    const tauri = mockTauri({
      session_document: () => document("## Overview\n\nWe agreed the **budget** for next year.\n\n## Decisions\n\n- More"),
      session_detail: () => detail(e, "/Users/me/Longhand/2026-09-24 1405 Budget review"),
    });
    renderHome({ entries: [e] });
    expect(tauri.calls.filter((c) => c.cmd === "session_document")).toEqual([]);
    await userEvent.setup().click(row("Budget review"));
    const preview = screen.getByRole("region", { name: "Preview" });
    expect(await within(preview).findByText("We agreed the budget for next year.")).toBeTruthy();
    expect(within(preview).getByText("2026-09-24 1405 Budget review")).toBeTruthy();
    expect(within(preview).getByText("3")).toBeTruthy();
  });

  test("double-click and Return open the session", async () => {
    mockTauri({
      session_document: () => document("Text."),
      session_detail: () => detail(entry("a"), null),
    });
    const onOpen = mock((_id: string) => {});
    renderHome({ entries: [entry("a", { title: "Budget review" })], onOpen });
    const user = userEvent.setup();
    await user.dblClick(row("Budget review"));
    expect(onOpen).toHaveBeenCalledWith("a");
    onOpen.mockClear();
    row("Budget review").focus();
    await user.keyboard("{Enter}");
    expect(onOpen).toHaveBeenCalledWith("a");
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
