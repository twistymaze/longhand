// One session's page in the main window (R10, KD3). The same page serves
// the three states of a session:
// - recording: My Notes, with the recording bar below (R11, R12);
// - processing: the stages on Summary and Transcript (R14);
// - saved: the summary, the transcript and the notes (R15).
// The toolbar has Back, the tabs, Copy for the open tab (R17) and the
// "…" menu. The tabs change at
// once, with no motion: they are used many times in a session.

import { type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import {
  commands,
  type MicrophoneChoice,
  type MicrophoneList,
  type Readiness,
  type SessionDetail,
  type SessionDocument,
} from "../bindings";
import { rowTitle } from "../home/group";
import { features } from "../lib/features";
import { formatDuration, sentence, sessionTitle, startedAt } from "../processing/describe";
import type { Capture } from "../recording/useCapture";
import { Button } from "../settings/ui";
import { useDeleteSession } from "./DeleteSession";
import { LevelBars } from "./LevelBars";
import { NotesTab } from "./NotesTab";
import { ProcessingStatus, RegenerateStatus } from "./ProcessingStatus";
import { RecordingBar } from "./RecordingBar";
import { copyText, tabMarkdown } from "./copy";
import { canDelete, type MenuItem, SessionMenu } from "./SessionMenu";
import { SummaryTab } from "./SummaryTab";
import { type Notice, TitleField, type TitleFieldHandle } from "./TitleField";
import { TranscriptTab } from "./TranscriptTab";
import { useTranscribeAgain } from "./TranscribeAgain";
import { LiveTranscript, type LiveMode, READY_AFTER_STOP } from "./LiveTranscript";
import type { Sessions } from "./useSessions";

type Tab = "summary" | "transcript" | "notes";

// The tabs follow the order things happen: notes during the recording,
// then the transcript, then the summary.
const tabs: { value: Tab; label: string }[] = [
  { value: "notes", label: "My Notes" },
  { value: "transcript", label: "Transcript" },
  { value: "summary", label: "Summary" },
];

type Phase = "recording" | "processing" | "saved" | "missing";

/// The states after Stop in which Transcript keeps the live text, and how
/// it reads in each. The other states need an action on the stages.
const afterLive: Record<string, LiveMode> = {
  interrupted: "interrupted",
  queued: "processing",
  processing: "processing",
};

/// A segmented control. The segments have one width, so the control keeps
/// its shape when the level bars show on Transcript.
function Tabs({ tab, onTab, meter }: { tab: Tab; onTab: (t: Tab) => void; meter: ReactNode }) {
  return (
    <div
      role="tablist"
      aria-label="Show"
      className="inline-grid auto-cols-fr grid-flow-col gap-0.5 rounded-[7px] bg-fill p-[2px]"
      onKeyDown={(e) => {
        if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
        e.preventDefault();
        const at = tabs.findIndex((t) => t.value === tab);
        const next = tabs[(at + (e.key === "ArrowRight" ? 1 : tabs.length - 1)) % tabs.length];
        onTab(next.value);
        e.currentTarget.querySelector<HTMLElement>(`[data-tab="${next.value}"]`)?.focus();
      }}
    >
      {tabs.map((t) => (
        <button
          key={t.value}
          type="button"
          role="tab"
          data-tab={t.value}
          aria-selected={tab === t.value}
          tabIndex={tab === t.value ? 0 : -1}
          onClick={() => onTab(t.value)}
          className={`flex items-center justify-center gap-1.5 rounded-[5px] px-4 py-[3px] whitespace-nowrap ${
            tab === t.value
              ? "bg-group text-label shadow-[0_0.5px_1.5px_rgb(0_0_0/0.18),0_0_0_0.5px_rgb(0_0_0/0.04)] dark:bg-white/[0.2] dark:shadow-[0_0.5px_1px_rgb(0_0_0/0.4)]"
              : "text-label-secondary hover:text-label active:bg-fill-pressed"
          } outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent`}
        >
          {t.label}
          {t.value === "transcript" && meter}
        </button>
      ))}
    </div>
  );
}

/// What Summary and Transcript say while the note records (R13).
function AfterStop({ children }: { children: string }) {
  return (
    <p className="mx-auto max-w-[24rem] pt-16 text-center leading-relaxed text-label-secondary">{children}</p>
  );
}

/// The "Microphone" submenu: "System default", then the connected inputs,
/// with a check mark on the note's choice (R2, KTD14). The backend finds a
/// choice by its ID first, then by the exact name. A chosen microphone
/// that is not connected shows last, checked and disabled.
export function microphoneItems(
  list: MicrophoneList | null,
  choice: MicrophoneChoice | null,
  choose: (next: MicrophoneChoice | null) => void,
): MenuItem[] {
  const inputs = list?.inputs ?? [];
  const current = inputs.find((i) => i.id === list?.default_id);
  const match = choice && (inputs.find((i) => i.id === choice.id) ?? inputs.find((i) => i.name === choice.label));
  const items: MenuItem[] = [
    {
      key: "default",
      label: current ? `System default (${current.name})` : "System default",
      checked: choice === null,
      radio: true,
      onSelect: () => choose(null),
    },
    ...inputs.map((input) => ({
      key: `input:${input.id}`,
      label: input.name,
      checked: match === input,
      radio: true,
      onSelect: () => choose({ id: input.id, label: input.name }),
    })),
  ];
  if (choice && !match) {
    items.push({
      key: "missing",
      label: `${choice.label} (not connected)`,
      checked: true,
      radio: true,
      disabled: true,
      reason: "This microphone is not connected.",
      onSelect: () => {},
    });
  }
  return items;
}

function BackChevron() {
  return (
    <svg aria-hidden viewBox="0 0 10 16" className="h-[13px] w-[8px]">
      <path d="M8 1.5L2 8l6 6.5" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function SessionPage({
  id,
  capture,
  sessions,
  readiness,
  onBack,
}: {
  id: string;
  capture: Capture;
  sessions: Sessions;
  readiness: Readiness | null;
  onBack: () => void;
}) {
  const entry = sessions.entries.find((e) => e.id === id);
  // A recording that just started can be known first to the capture status
  // or first to the list. Either one makes the page open as a recording, so
  // it never opens on Summary and then moves to My Notes.
  const open = capture.status?.session_id === id || entry?.state === "capturing";
  const phase: Phase = open ? "recording" : !entry ? "missing" : entry.state === "saved" ? "saved" : "processing";

  const [tab, setTab] = useState<Tab>(phase === "recording" ? "notes" : "summary");
  const [detail, setDetail] = useState<SessionDetail | null>(null);
  const [doc, setDoc] = useState<SessionDocument | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [regenerating, setRegenerating] = useState(false);
  const [speakers, setSpeakers] = useState(false);
  const [microphones, setMicrophones] = useState<MicrophoneList | null>(null);
  const [notes, setNotes] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [scrolled, setScrolled] = useState(false);
  const title = useRef<TitleFieldHandle>(null);
  const notesPanel = useRef<HTMLDivElement>(null);
  const scroller = useRef<HTMLElement>(null);

  // A recording shows My Notes (R11). Stop shows Summary, where the
  // processing stages show. A session that completes shows Summary (F2),
  // unless the user is typing notes.
  const previous = useRef(phase);
  useEffect(() => {
    const before = previous.current;
    previous.current = phase;
    if (before === phase) return;
    if (phase === "recording") setTab("notes");
    else if (before === "recording") setTab("summary");
    else if (phase === "saved" && before === "processing" && !notesPanel.current?.contains(document.activeElement)) {
      setTab("summary");
    }
  }, [phase]);

  const version = sessions.version[id] ?? 0;
  const state = entry?.state;
  // Each read has a number. The page ignores a response from an older read,
  // so a late response never replaces a newer one.
  const detailRead = useRef(0);
  const loadDetail = useCallback(async () => {
    const read = ++detailRead.current;
    const result = await commands.sessionDetail(id);
    if (read !== detailRead.current) return;
    if (result.status === "ok") setDetail(result.data);
  }, [id]);
  useEffect(() => {
    if (state === undefined) return;
    void loadDetail();
  }, [loadDetail, version, state]);

  const documentRead = useRef(0);
  const loadDocument = useCallback(async () => {
    const read = ++documentRead.current;
    const result = await commands.sessionDocument(id);
    if (read !== documentRead.current) return;
    if (result.status === "ok") {
      setDoc(result.data);
      setError(null);
    } else setError(sentence(result.error));
  }, [id]);
  useEffect(() => {
    if (phase === "saved") void loadDocument();
  }, [phase, loadDocument]);

  const changed = () => {
    void loadDetail();
    void sessions.refresh();
  };

  const resume = () =>
    void commands.resumeRecording(id).then((r) => {
      setNotice(r.status === "error" ? { tone: "negative", text: sentence(r.error) } : null);
      void capture.refresh();
      changed();
    });

  // The progress event of an earlier run stays in `sessions.live`. Only an
  // event that arrives after this run starts shows as its progress.
  const liveAtStart = useRef(sessions.live[id]);
  const regenerate = () => {
    liveAtStart.current = sessions.live[id];
    setRegenerating(true);
    setNotice(null);
    // The status shows on Summary, so the page moves there, as it does
    // after Stop.
    setTab("summary");
    // A rejected call must also end the status, or the tab shows it with
    // no end.
    void (async () => {
      try {
        const r = await commands.regenerateSummary(id);
        if (r.status === "error") setNotice({ tone: "negative", text: sentence(r.error) });
        await loadDocument();
      } catch (e) {
        console.error("Regenerate failed:", e);
        setNotice({ tone: "negative", text: "The summary could not be updated." });
      } finally {
        setRegenerating(false);
      }
      // Regenerate can propose a new title (R31).
      void sessions.refresh();
    })();
  };

  const deletion = useDeleteSession({
    titleOf: (e) => rowTitle(e),
    onDeleted: () => {
      void sessions.refresh();
      onBack();
    },
    onFailed: (text) => {
      setNotice({ tone: "negative", text });
      void sessions.refresh();
    },
  });

  const again = useTranscribeAgain({
    sessionId: id,
    noteModel: detail?.asr_model ?? null,
    onQueued: () => {
      setNotice(null);
      changed();
    },
    onFailed: (text) => setNotice({ tone: "negative", text }),
  });

  // The inputs are read each time the menu opens, because a microphone can
  // be connected or removed during the note.
  const readMicrophones = async () => {
    const result = await commands.listMicrophones();
    if (result.status === "ok") setMicrophones(result.data);
  };

  // Each change is for this note only. Settings does not change (R2, R8).
  const changeInput = async (change: () => ReturnType<typeof commands.setNoteMicrophone>) => {
    const result = await change();
    setNotice(result.status === "error" ? { tone: "negative", text: sentence(result.error) } : null);
    await capture.refresh();
  };

  const items: MenuItem[] = [];
  if (phase === "recording" && capture.status?.recording) {
    items.push({ label: "Number of Speakers…", onSelect: () => setSpeakers(true) });
  }
  if (phase === "recording" && capture.status?.session_id === id) {
    const choice = capture.status.microphone_choice;
    items.push({
      label: "Microphone",
      onSelect: () => {},
      submenu: microphoneItems(microphones, choice, (next) =>
        void changeInput(() => commands.setNoteMicrophone(id, next)),
      ),
    });
    if (features.systemAudio) {
      const on = capture.status.system_audio;
      items.push({
        label: "Record Computer Audio",
        checked: on,
        onSelect: () => void changeInput(() => commands.setNoteSystemAudio(id, !on)),
      });
    }
  }
  if (phase === "processing" || phase === "saved") {
    items.push({
      label: "Resume Recording",
      disabled: !detail?.resumable,
      reason: "This note can’t be resumed.",
      onSelect: resume,
    });
  }
  if (phase === "saved") {
    items.push({ label: "Regenerate Summary", disabled: regenerating, onSelect: regenerate });
    items.push({
      label: "Transcribe Again…",
      disabled: regenerating,
      reason: "Wait until the summary is ready.",
      onSelect: again.ask,
    });
  }
  items.push({ label: "Rename", onSelect: () => title.current?.edit() });
  if (entry && !open && canDelete(entry.state)) {
    items.push({ label: "Delete…", onSelect: () => deletion.ask(entry) });
  }

  // What the last Transcribe Again left for the user to know (R19).
  const againFailed = phase === "saved" && detail?.transcribe_again_error != null;
  const summaryKept = phase === "saved" && detail?.summary_not_updated != null;

  const live = sessions.live[id];
  const regenerateControl = (
    <Button prominent={!doc?.summary || doc.summary.stub || summaryKept} onClick={regenerate}>
      Regenerate Summary
    </Button>
  );

  // The note's own live mode decides, not the model selected now (R9). A
  // Transcribe Again run has no live text.
  const liveMode = (detail?.live_transcript ?? false) && !detail?.transcribe_again;
  const liveView = (mode: LiveMode) => <LiveTranscript sessionId={id} mode={mode} scroller={scroller} />;
  const processing =
    detail && phase === "processing" ? (
      <ProcessingStatus
        detail={detail}
        live={live}
        waitsForRecording={capture.status?.recording ?? false}
        onChanged={changed}
      />
    ) : null;

  const content = (which: "summary" | "transcript") => {
    if (phase === "missing") return <AfterStop>This note is no longer in the list.</AfterStop>;
    if (phase === "recording") {
      return which === "summary" ? (
        <AfterStop>The summary is written after you stop recording.</AfterStop>
      ) : !detail ? null : liveMode ? (
        liveView(capture.status?.recording ? "recording" : "interrupted")
      ) : (
        <AfterStop>{READY_AFTER_STOP}</AfterStop>
      );
    }
    if (phase === "processing") {
      // The live text stays until the saved transcript is ready. A note
      // that needs an action shows the stages, where the action is.
      if (which === "transcript" && liveMode && state && afterLive[state]) return liveView(afterLive[state]);
      return processing;
    }
    if (error) return <p className="text-negative">{error}</p>;
    // While the summary is written again, Summary shows the summarize stage
    // in place of the previous summary. A failed run shows it again.
    if (which === "summary" && regenerating && detail) {
      return <RegenerateStatus detail={detail} live={live === liveAtStart.current ? undefined : live} />;
    }
    if (!doc) return which === "transcript" && liveMode ? liveView("loading") : null;
    return which === "summary" ? (
      <>
        {summaryKept && (
          <p role="status" className="mb-4 text-[0.85rem] text-caution">
            The summary could not be updated. It is from the previous transcript.
          </p>
        )}
        <SummaryTab summary={doc.summary} regenerate={regenerateControl} />
      </>
    ) : (
      <TranscriptTab
        lines={doc.lines}
        hasSpeakers={doc.has_speakers}
        foundSpeakers={entry?.found_speakers ?? null}
        durationMs={entry?.duration_ms ?? 0}
      />
    );
  };

  // The summary and the transcript exist only once the session is saved.
  const copyable = tabMarkdown(tab, phase === "saved" ? doc : null, notes);
  const copyTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(copyTimer.current), []);
  useEffect(() => setCopied(false), [tab]);
  const copy = async () => {
    if (copyable === null) return;
    const done = await copyText(copyable);
    setNotice(done ? null : { tone: "negative", text: "The text could not be copied to the clipboard." });
    setCopied(done);
    clearTimeout(copyTimer.current);
    // "Copied" stays long enough to read, then the button is Copy again.
    if (done) copyTimer.current = setTimeout(() => setCopied(false), 1500);
  };

  const subtitle = entry
    ? [startedAt(entry.started), !open && entry.duration_ms > 0 ? formatDuration(entry.duration_ms) : null]
        .filter(Boolean)
        .join(" · ")
    : "";

  const copyLabel = { notes: "Copy My Notes", transcript: "Copy the transcript", summary: "Copy the summary" }[tab];

  return (
    <div className="flex h-full min-h-0 flex-col bg-window">
      <header
        data-tauri-drag-region
        className={`grid shrink-0 grid-cols-[1fr_auto_1fr] items-center gap-3 border-b px-[16px] pt-0.5 pb-2.5 select-none ${
          scrolled ? "border-separator" : "border-transparent"
        }`}
      >
        <div data-tauri-drag-region>
          <button
            type="button"
            onClick={onBack}
            className="flex items-center gap-1 rounded-md py-[3px] pr-2 pl-1.5 text-accent hover:bg-fill active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent"
          >
            <BackChevron />
            Back
          </button>
        </div>
        <Tabs
          tab={tab}
          onTab={setTab}
          meter={open && capture.status?.recording ? <LevelBars levels={capture.levels} input={capture.input} /> : null}
        />
        <div data-tauri-drag-region className="flex items-center justify-end gap-1">
          <button
            type="button"
            disabled={copyable === null}
            title={copyable === null ? undefined : copyLabel}
            onClick={() => void copy()}
            className="rounded-md px-2 py-[3px] text-label enabled:hover:bg-fill enabled:active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent disabled:text-label-tertiary"
          >
            {copied ? "Copied" : "Copy"}
          </button>
          <SessionMenu items={items} onOpen={phase === "recording" ? () => void readMicrophones() : undefined} />
        </div>
      </header>
      <main
        ref={scroller}
        className="min-h-0 flex-1 overflow-y-auto bg-pane select-text"
        // The header shows its bottom line only while content is under it.
        onScroll={(e) => setScrolled(e.currentTarget.scrollTop > 0)}
      >
        <div className="mx-auto max-w-[44rem] px-8 pt-6 pb-1">
          <TitleField
            ref={title}
            sessionId={id}
            title={entry?.title ?? null}
            placeholder={entry ? sessionTitle(entry.started) : "Recording"}
            onRenamed={(n) => {
              setNotice(n);
              void sessions.refresh();
            }}
          />
          <p className="mt-1 text-[0.85rem] text-label-secondary tabular-nums">{subtitle}</p>
          {notice && (
            <p
              role="status"
              className={`mt-2 text-[0.85rem] ${notice.tone === "caution" ? "text-caution" : "text-negative"}`}
            >
              {notice.text}
            </p>
          )}
          {!notice && againFailed && (
            <p role="status" className="mt-2 text-[0.85rem] text-negative" title={detail?.transcribe_again_error ?? undefined}>
              The new transcript failed. The previous transcript is unchanged.
            </p>
          )}
        </div>
        {tab !== "notes" && (
          <div role="tabpanel" className="mx-auto max-w-[44rem] px-8 pt-5 pb-12">
            {content(tab)}
          </div>
        )}
        {/* My Notes stays mounted, so the editor keeps its document, cursor
            and undo history while another tab shows. */}
        <div ref={notesPanel} role="tabpanel" hidden={tab !== "notes"}>
          <NotesTab
            key={id}
            sessionId={id}
            autoFocus={tab === "notes" && phase === "recording"}
            onMarkdown={setNotes}
          />
        </div>
      </main>
      {open && (
        <RecordingBar
          capture={capture}
          readiness={readiness}
          speakers={speakers}
          onSpeakersDone={() => setSpeakers(false)}
        />
      )}
      {deletion.dialog}
      {again.dialog}
    </div>
  );
}
