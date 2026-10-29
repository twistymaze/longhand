// The scenes of the website screenshots (R12-R14 of the landing page plan):
// the real SessionPage and Home with invented notes. All people, companies,
// topics, notes, transcripts and summaries here are invented. Each scene has
// the command results that its components ask for.

import type { ReactNode } from "react";
import type {
  LibraryNotices,
  Readiness,
  RecordingStatus,
  SessionDetail,
  SessionDocument,
  SessionEntry,
  TagEntry,
} from "../bindings";
import { Home } from "../home/Home";
import type { Capture } from "../recording/useCapture";
import { SessionPage } from "../session/SessionPage";
import type { Sessions } from "../session/useSessions";
import type { CommandHandler } from "../test/tauri";

export type SceneName = "recording" | "summary" | "library";

export type Scene = {
  /// The command results for mockTauri.
  handlers: Record<string, CommandHandler>;
  view: ReactNode;
};

/// All dates are relative to this day (Wednesday 28 October 2026), so the
/// screenshots do not change with the day they are made. The library shows
/// the list at 16:00, after the team meeting of the other scenes.
export const NOW = new Date(2026, 9, 28, 16, 0);

const at = (dayOffset: number, hour: number, minute: number) =>
  new Date(2026, 9, 28 + dayOffset, hour, minute).toISOString();

const readiness: Readiness = {
  microphone_authorized: true,
  system_audio: "allowed",
  destination: { kind: "ready", path: "/notes", free_bytes: null },
  armed: true,
  shortcut: { chord: "Alt+Shift+L", registered: true, error: null },
  missing_models: [],
  endpoint_configured: true,
  can_start_note: true,
  note_start_refusal: null,
};

const idle: RecordingStatus = {
  note_id: null,
  state: null,
  recording: false,
  reconnecting: false,
  interruption: null,
  working_free_bytes: null,
  session_demand_bytes: null,
  processing_bytes_per_second: 0,
  reserve_bytes: 0,
  microphone: null,
  microphone_fallback: false,
  note_apps: [],
  app_audio: null,
  microphone_choice: null,
  paused: false,
  recorded_ms: 0,
};

const entry = (id: string, over: Partial<SessionEntry>): SessionEntry => ({
  id,
  started: at(0, 9, 0),
  duration_ms: 30 * 60_000,
  state: "saved",
  stage: null,
  title: null,
  found_speakers: null,
  storage_unavailable: false,
  problem: null,
  remote: null,
  note_type: null,
  tags: [],
  waits_for: null,
  ...over,
});

const detail = (e: SessionEntry): SessionDetail => ({
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
  home_path: null,
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

// The weekly team meeting after it ended. The recording scene shows
// it at 15:30, while it records; the summary and library scenes show it here.
const teamMeeting = entry("team-now", {
  started: at(0, 15, 12),
  duration_ms: 41 * 60_000,
  found_speakers: 5,
  title: "Team Meeting: Card update estimate, retry fix and Thanksgiving on-call",
  note_type: "Internal Meeting",
  tags: ["Topic::Team Meeting"],
});

// The library, newest first. The invented world: the user leads two
// engineering teams at a company that sells delivery and dispatch software.
// The library also has a personal note and a role-playing
// game session, as a real library does.
const notes: SessionEntry[] = [
  teamMeeting,
  entry("carver", {
    started: at(0, 13, 0),
    duration_ms: 48 * 60_000,
    found_speakers: 5,
    title: "Customer Meeting: Carver Logistics rollout, month-end reports and driver training",
    note_type: "Customer Meeting",
    tags: ["Customer::Carver Logistics"],
  }),
  entry("priya", {
    started: at(0, 10, 30),
    duration_ms: 28 * 60_000,
    title: "1:1 Meeting: Hiring for the open role, on-call load and a conference talk",
    note_type: "One-on-One",
    tags: ["Person::Priya Nair"],
  }),
  entry("architecture-review", {
    started: at(0, 9, 0),
    duration_ms: 52 * 60_000,
    found_speakers: 6,
    title: "Internal Meeting: Architecture review of the event pipeline and its rollout plan",
    note_type: "Internal Meeting",
    tags: ["Topic::Architecture Review"],
  }),
  entry("pta", {
    started: at(-1, 18, 30),
    duration_ms: 44 * 60_000,
    title: "Planning Meeting: School fundraiser auction, volunteers and a venue",
    note_type: "Personal",
    tags: ["Topic::PTA"],
  }),
  entry("stratus", {
    started: at(-1, 15, 0),
    duration_ms: 31 * 60_000,
    title: "Vendor Meeting: Cloud bill increase, reserved capacity and support plan",
    note_type: "Vendor Meeting",
    tags: ["Vendor::Stratus Cloud"],
  }),
  entry("marcus", {
    started: at(-1, 11, 0),
    duration_ms: 26 * 60_000,
    title: "1:1 Meeting: Card update estimate, code review backlog and holiday time off",
    note_type: "One-on-One",
    tags: ["Person::Marcus Bell"],
  }),
  entry("planning", {
    started: at(-2, 14, 0),
    duration_ms: 85 * 60_000,
    title: "Internal Meeting: First-half planning, headcount and the platform budget",
    note_type: "Internal Meeting",
    tags: ["Topic::Planning"],
  }),
  entry("jordan", {
    started: at(-2, 9, 30),
    duration_ms: 30 * 60_000,
    title: "1:1 Meeting: Mid-year review, team growth and the planning doc",
    note_type: "One-on-One",
    tags: ["Person::Jordan Reyes"],
  }),
  entry("rpg", {
    started: at(-3, 18, 30),
    duration_ms: 176 * 60_000,
    found_speakers: 6,
    title: "The Lantern Keeper's Bargain",
    note_type: "RPG Session",
    tags: ["Campaign::Ashen Coast"],
  }),
  entry("team-last", {
    started: at(-7, 15, 10),
    duration_ms: 35 * 60_000,
    found_speakers: 5,
    title: "Team Meeting: Payment retry bug, card update flow and holiday on-call",
    note_type: "Internal Meeting",
    tags: ["Topic::Team Meeting"],
  }),
];

const noteTypes = {
  types: ["Customer Meeting", "Internal Meeting", "One-on-One", "Personal", "RPG Session", "Vendor Meeting"].map((name) => ({
    name,
    valid: true,
    shared: true,
    notes: notes.filter((n) => n.note_type === name).length,
    general: false,
  })),
  default_type: null,
};

const tags = (): TagEntry[] => {
  const keys = new Map<string, TagEntry>();
  for (const n of notes)
    for (const text of n.tags) {
      const key = text.toLowerCase();
      const found = keys.get(key);
      if (found) found.count += 1;
      else keys.set(key, { text, key, count: 1 });
    }
  return [...keys.values()];
};

type Line = SessionDocument["lines"][number];

const turn = (position_ms: number, speaker: number, text: string): Line => ({ kind: "turn", position_ms, speaker, text });

const document = (body: string, lines: Line[]): SessionDocument => ({
  has_speakers: true,
  lines,
  transcript_markdown: "",
  summary: { body, stub: false, host: "api.example.com", model: "model", prompt: null, prompt_id: null },
});

const sessionsOf = (entries: SessionEntry[]): Sessions => ({
  entries,
  stages: {},
  starts: {},
  live: {},
  speakers: {},
  version: {},
  synced: {},
  removed: {},
  refresh: async () => {},
});

const common = (notesMarkdown: string): Record<string, CommandHandler> => ({
  note_types: () => noteTypes,
  list_tags: () => tags(),
  app_audio_available: () => true,
  list_running_apps: () => [],
  load_session_notes: () => ({ markdown: notesMarkdown, fingerprint: "f0" }),
  save_session_notes: () => ({ kind: "saved", fingerprint: "f1" }),
});

// The level bars of a voice at a normal level.
const levels = Array.from({ length: 48 }, (_, i) => 0.25 + 0.2 * Math.abs(Math.sin(i * 0.7)) + 0.1 * Math.abs(Math.sin(i * 2.3)));

// The weekly team meeting, one week after "team-last".
const recordingNotes = `Weekly team meeting

- Ben out today, Marcus running the demo
- retry bug is fixed, shipped tues.  no new reports since
- card update flow
  - Marcus thinks ~3 wks, 2 if we skip the email reminder
  - do we need the email for v1?  ask support
- thanksgiving on-call, Dana and Sam swapping
- `;

function recording(): Scene {
  const e = entry("team-now", {
    started: at(0, 15, 12),
    duration_ms: 0,
    state: "capturing",
    note_type: "Internal Meeting",
    tags: ["Topic::Team Meeting"],
  });
  const capture: Capture = {
    status: { ...idle, note_id: e.id, state: "capturing", recording: true, recorded_ms: 18 * 60_000 + 24_000 },
    input: "capturing",
    levels,
    recordedMs: 18 * 60_000 + 24_000,
    lostReason: null,
    refresh: async () => {},
  };
  return {
    handlers: {
      ...common(recordingNotes),
      session_detail: () => detail(e),
      session_document: () =>
        document("", [
          turn(0, 1, "Okay, let's start. Ben's out today, so Marcus is doing the demo."),
          turn(5_000, 2, "Sure. First the retry fix, it went out on Tuesday."),
        ]),
    },
    view: <SessionPage id={e.id} capture={capture} sessions={sessionsOf([e, ...notes.slice(1)])} readiness={readiness} onBack={() => {}} />,
  };
}

const teamSummary = `The team met without Ben. The payment retry fix went out on Tuesday, and support has had no new reports since. Most of the meeting was about the card update flow: Marcus estimates three weeks, or two weeks without the email reminder. The team also agreed the Thanksgiving on-call change.

## Payment retry fix

- **Status:** The fix went out on Tuesday. Support has had no new reports of double charges since.
- **Check:** Marcus showed the retry log for one customer. Each failed payment now retries once, after 10 minutes.

## Card update flow

- **Estimate:** About three weeks with the email reminder, about two weeks without it.
- **Email reminder:** It tells the customer that the card expires soon. The team does not know if customers need it in the first version.
- **Demo:** Marcus showed the new card form. Dana asked for a clearer message when the bank declines the card.

## Thanksgiving on-call

- **Change:** Dana and Sam swap their weeks, so Sam is on call for Thanksgiving week and Dana for the week after.

## Open questions

- Is the email reminder necessary for the first version of the card update flow?

## Follow-ups

- **Me:** Ask support if customers need the email reminder, before the next team meeting.
- **Marcus:** Start the card update flow without the reminder, and keep the email as a separate task.
- **Dana:** Update the on-call calendar with the swap.`;

const teamDocument = () =>
  document(teamSummary, [
    turn(0, 1, "Okay, let's start. Ben's out today, so Marcus is doing the demo."),
    turn(5_000, 2, "Sure. First the retry fix, it went out on Tuesday."),
  ]);

function summary(): Scene {
  const e = teamMeeting;
  const capture: Capture = { status: idle, input: "idle", levels: [], recordedMs: 0, lostReason: null, refresh: async () => {} };
  return {
    handlers: {
      ...common(recordingNotes),
      session_detail: () => detail(e),
      session_document: teamDocument,
    },
    view: <SessionPage id={e.id} capture={capture} sessions={sessionsOf(notes)} readiness={readiness} onBack={() => {}} />,
  };
}

// The library selects the newest note, the team meeting of the other
// scenes, and its preview shows the summary's first paragraph.
function library(): Scene {
  const selected = teamMeeting;
  return {
    handlers: {
      ...common(recordingNotes),
      library_notices: (): LibraryNotices => ({
        conversion: null,
        problems: [],
        imports: { watched_folder: null, waiting_for_space: [], failed: [] },
      }),
      session_detail: () => detail(selected),
      session_document: teamDocument,
    },
    view: (
      <Home
        entries={notes}
        stages={{}}
        recording={null}
        readiness={readiness}
        chord="⌥⇧L"
        onOpen={() => {}}
        onChanged={() => {}}
        now={NOW}
      />
    ),
  };
}

export const scenes: Record<SceneName, () => Scene> = { recording, summary, library };
