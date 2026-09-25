// One session's page in the main window (R1): "← Back" and the title above
// the view for the session's state. This is a temporary page that shows the
// current views; the session page with its tabs replaces it (U13).

import type { ModelStatus, Readiness, Settings } from "../bindings";
import { rowTitle } from "../home/group";
import { isOpen } from "../processing/describe";
import { ProcessingView } from "../processing/ProcessingView";
import { RecordingPane } from "../recording/RecordingPane";
import type { Capture } from "../recording/useCapture";
import { SessionView } from "./SessionView";
import type { Sessions } from "./useSessions";

export function SessionPage({
  id,
  capture,
  sessions,
  readiness,
  models,
  settings,
  onBack,
}: {
  id: string;
  capture: Capture;
  sessions: Sessions;
  readiness: Readiness | null;
  models: ModelStatus[];
  settings: Settings | null;
  onBack: () => void;
}) {
  const entry = sessions.entries.find((e) => e.id === id);
  // The list can lag a recording that just started, so the capture status
  // also counts.
  const open = capture.status?.session_id === id || (entry !== undefined && isOpen(entry.state));

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 items-center gap-3 border-b border-separator px-3 py-2 select-none">
        <button
          type="button"
          onClick={onBack}
          className="rounded-md px-2 py-[3px] text-accent active:bg-fill-pressed outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent"
        >
          ← Back
        </button>
        <h1 className="min-w-0 flex-1 truncate font-semibold">{entry ? rowTitle(entry) : open ? "Recording" : ""}</h1>
      </header>
      <main className="min-h-0 flex-1 overflow-hidden select-text">
        {open && readiness && settings && (
          <div className="h-full overflow-y-auto">
            <RecordingPane capture={capture} readiness={readiness} models={models} settings={settings} />
          </div>
        )}
        {!open && entry?.state === "saved" && (
          <SessionView key={entry.id} entry={entry} live={sessions.live[entry.id]} />
        )}
        {!open && entry && entry.state !== "saved" && (
          <div className="h-full overflow-y-auto">
            <ProcessingView
              key={entry.id}
              entry={entry}
              version={sessions.version[entry.id] ?? 0}
              live={sessions.live[entry.id]}
              recording={capture.status?.recording ?? false}
              onChanged={() => void sessions.refresh()}
            />
          </div>
        )}
        {!open && !entry && <p className="px-6 pt-5 text-label-secondary">This session is no longer listed.</p>}
      </main>
    </div>
  );
}
