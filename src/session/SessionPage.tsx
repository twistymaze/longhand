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
  type ModelStatus,
  type Readiness,
  type SessionDetail,
  type SessionDocument,
} from "../bindings";
import { rowTitle } from "../home/group";
import { formatDuration, sentence, sessionTitle, startedAt } from "../processing/describe";
import type { Capture } from "../recording/useCapture";
import { Button } from "../settings/ui";
import { useDeleteSession } from "./DeleteSession";
import { LevelBars } from "./LevelBars";
import { NotesTab } from "./NotesTab";
import { ProcessingStatus } from "./ProcessingStatus";
import { RecordingBar } from "./RecordingBar";
import { copyText, tabMarkdown } from "./copy";
import { canDelete, type MenuItem, SessionMenu } from "./SessionMenu";
import { SummaryTab } from "./SummaryTab";
import { type Notice, TitleField, type TitleFieldHandle } from "./TitleField";
import { TranscriptTab } from "./TranscriptTab";
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
              ? "bg-white text-label shadow-[0_0.5px_1.5px_rgb(0_0_0/0.18),0_0_0_0.5px_rgb(0_0_0/0.04)] dark:bg-white/[0.2] dark:shadow-[0_0.5px_1px_rgb(0_0_0/0.4)]"
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
  models,
  onBack,
}: {
  id: string;
  capture: Capture;
  sessions: Sessions;
  readiness: Readiness | null;
  models: ModelStatus[];
  onBack: () => void;
}) {
  const entry = sessions.entries.find((e) => e.id === id);
  // The list can lag a recording that just started, so the capture status
  // decides whether this session records.
  const open = capture.status?.session_id === id;
  const phase: Phase = open ? "recording" : !entry ? "missing" : entry.state === "saved" ? "saved" : "processing";

  const [tab, setTab] = useState<Tab>(phase === "recording" ? "notes" : "summary");
  const [detail, setDetail] = useState<SessionDetail | null>(null);
  const [doc, setDoc] = useState<SessionDocument | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [regenerating, setRegenerating] = useState(false);
  const [speakers, setSpeakers] = useState(false);
  const [notes, setNotes] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [scrolled, setScrolled] = useState(false);
  const title = useRef<TitleFieldHandle>(null);
  const notesPanel = useRef<HTMLDivElement>(null);

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

  const regenerate = () => {
    setRegenerating(true);
    setNotice(null);
    void commands.regenerateSummary(id).then(async (r) => {
      if (r.status === "error") setNotice({ tone: "negative", text: sentence(r.error) });
      await loadDocument();
      setRegenerating(false);
      // Regenerate can propose a new title (R31).
      void sessions.refresh();
    });
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

  const items: MenuItem[] = [];
  if (phase === "recording" && capture.status?.recording) {
    items.push({ label: "Number of Speakers…", onSelect: () => setSpeakers(true) });
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
  }
  items.push({ label: "Rename", onSelect: () => title.current?.edit() });
  if (entry && !open && canDelete(entry.state)) {
    items.push({ label: "Delete…", onSelect: () => deletion.ask(entry) });
  }

  const live = sessions.live[id];
  const progress = regenerating && live?.stage === "summarize" && live.total > 1 ? ` part ${live.done} of ${live.total}` : "";
  const regenerateControl = (
    <Button prominent={!doc?.summary || doc.summary.stub} disabled={regenerating} onClick={regenerate}>
      {regenerating ? `Summarizing…${progress}` : "Regenerate Summary"}
    </Button>
  );

  const asr = models.find((m) => m.role === "recognition" && m.required);
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
      ) : (
        <AfterStop>
          {asr?.streaming
            ? "The transcript is ready after you stop recording. A live transcript is possible with this model, but this version does not show one."
            : "The transcript is ready after you stop recording."}
        </AfterStop>
      );
    }
    if (phase === "processing") return processing;
    if (error) return <p className="text-negative">{error}</p>;
    if (!doc) return null;
    return which === "summary" ? (
      <SummaryTab summary={doc.summary} regenerate={regenerateControl} />
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
        className={`grid shrink-0 grid-cols-[1fr_auto_1fr] items-center gap-3 border-b px-3 py-2 select-none ${
          scrolled ? "border-separator" : "border-transparent"
        }`}
      >
        <div>
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
        <div className="flex items-center justify-end gap-1">
          <button
            type="button"
            disabled={copyable === null}
            title={copyable === null ? undefined : copyLabel}
            onClick={() => void copy()}
            className="rounded-md px-2 py-[3px] text-label enabled:hover:bg-fill enabled:active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent disabled:text-label-tertiary"
          >
            {copied ? "Copied" : "Copy"}
          </button>
          <SessionMenu items={items} />
        </div>
      </header>
      <main
        className="min-h-0 flex-1 overflow-y-auto select-text"
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
    </div>
  );
}
