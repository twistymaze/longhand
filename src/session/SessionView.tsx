// A completed session (F4): notes on the left, as while processing, beside a
// pane that switches between the summary and the transcript. Resume records
// more into it while its folder holds its audio (R6d).

import { useCallback, useEffect, useState } from "react";
import { commands, type SessionDocument, type SessionEntry } from "../bindings";
import { formatDuration, sessionTitle } from "../processing/describe";
import { Button } from "../settings/ui";
import { NotesPane } from "./NotesPane";
import { SummaryPane } from "./SummaryPane";
import { TranscriptPane } from "./TranscriptPane";
import type { Live } from "./useSessions";

type Mode = "summary" | "transcript";

function Segmented({ mode, onMode }: { mode: Mode; onMode: (m: Mode) => void }) {
  const option = (value: Mode, label: string) => (
    <button
      type="button"
      role="tab"
      aria-selected={mode === value}
      onClick={() => onMode(value)}
      className={`rounded-[5px] px-3 py-[2px] ${
        mode === value ? "bg-control shadow-[0_0.5px_1.5px_rgb(0_0_0/0.2)]" : "text-label active:bg-fill-pressed"
      } outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent`}
    >
      {label}
    </button>
  );
  return (
    <div role="tablist" aria-label="Show" className="inline-flex gap-0.5 rounded-md bg-fill p-0.5">
      {option("summary", "Summary")}
      {option("transcript", "Transcript")}
    </div>
  );
}

export function SessionView({ entry, live }: { entry: SessionEntry; live: Live | undefined }) {
  const [doc, setDoc] = useState<SessionDocument | null>(null);
  const [path, setPath] = useState<string | null>(null);
  const [resumable, setResumable] = useState(false);
  const [resumeError, setResumeError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>("summary");
  const [regenerating, setRegenerating] = useState(false);
  const [regenerateError, setRegenerateError] = useState<string | null>(null);

  const load = useCallback(async () => {
    const [document, detail] = await Promise.all([
      commands.sessionDocument(entry.id),
      commands.sessionDetail(entry.id),
    ]);
    if (document.status === "ok") {
      setDoc(document.data);
      setError(null);
    } else setError(document.error);
    if (detail.status === "ok") {
      setPath(detail.data.home_path);
      setResumable(detail.data.resumable);
    }
  }, [entry.id]);

  useEffect(() => {
    void load();
  }, [load]);

  const resume = () =>
    void commands.resumeRecording(entry.id).then((r) => {
      setResumeError(r.status === "error" ? r.error : null);
      if (r.status === "error") void load();
    });

  const regenerate = () => {
    setRegenerating(true);
    setRegenerateError(null);
    void commands.regenerateSummary(entry.id).then(async (r) => {
      if (r.status === "error") setRegenerateError(r.error);
      await load();
      setRegenerating(false);
    });
  };

  const progress = regenerating && live?.stage === "summarize" && live.total > 1 ? ` part ${live.done} of ${live.total}` : "";
  const regenerateControl = (
    <span className="inline-flex items-center gap-2">
      {regenerateError && <span className="text-negative">{regenerateError}</span>}
      <Button prominent={!doc?.summary || doc.summary.stub} disabled={regenerating} onClick={regenerate}>
        {regenerating ? `Summarizing…${progress}` : "Regenerate Summary"}
      </Button>
    </span>
  );

  return (
    <div className="flex h-full min-h-0">
      <aside className="flex w-60 shrink-0 flex-col border-r border-separator">
        <NotesPane key={entry.id} sessionId={entry.id} />
      </aside>
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex items-center gap-3 border-b border-separator px-6 pt-4 pb-3">
          <div className="min-w-0 flex-1">
            <h1 className="truncate text-[1.1rem] leading-tight font-semibold">{sessionTitle(entry.started)}</h1>
            <p className="truncate text-[0.85rem] text-label-secondary" title={path ?? undefined}>
              {formatDuration(entry.duration_ms)}
              {path && ` · ${path}`}
            </p>
          </div>
          {resumable && <Button onClick={resume}>Resume Recording</Button>}
          <Segmented mode={mode} onMode={setMode} />
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto select-text">
          {resumeError && <p className="px-6 pt-4 text-negative">{resumeError}</p>}
          {error && <p className="px-6 py-4 text-negative">{error}</p>}
          {doc && mode === "summary" && <SummaryPane summary={doc.summary} regenerate={regenerateControl} />}
          {doc && mode === "transcript" && (
            <TranscriptPane lines={doc.lines} hasSpeakers={doc.has_speakers} target={null} />
          )}
        </div>
      </div>
    </div>
  );
}
