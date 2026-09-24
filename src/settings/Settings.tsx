import { useCallback, useEffect, useState } from "react";
import {
  commands,
  events,
  type DownloadUpdate,
  type ModelStatus,
  type Readiness,
  type Settings as SettingsData,
} from "../bindings";
import { DestinationPicker } from "./DestinationPicker";
import { EndpointForm } from "./EndpointForm";
import { Licences } from "./Licences";
import { Models } from "./Models";
import { type PaneId, StatusPane } from "./Status";
import { RecordingPane } from "../recording/RecordingPane";
import { useCapture } from "../recording/useCapture";
import { ProcessingView } from "../processing/ProcessingView";
import { entryStatus, entryTone, isOpen, sessionTitle } from "../processing/describe";
import { SessionView } from "../session/SessionView";
import { useSessions } from "../session/useSessions";
import { Status, type Tone } from "./ui";

const panes: { id: PaneId; label: string }[] = [
  { id: "status", label: "Status" },
  { id: "destination", label: "Destination" },
  { id: "summarization", label: "Summarization" },
  { id: "recognition", label: "Recognition" },
  { id: "licences", label: "Licences" },
];

function paneTone(id: PaneId, readiness: Readiness | null): Tone | null {
  if (!readiness) return null;
  switch (id) {
    case "status":
      return readiness.armed ? null : "caution";
    case "destination":
      return readiness.destination.kind === "ready" ? null : "caution";
    case "summarization":
      return readiness.endpoint_configured ? null : "neutral";
    case "recognition":
      return readiness.missing_models.length === 0 ? null : "neutral";
    case "licences":
    case "scratchpad":
      return null;
  }
}

/// What the window shows: one settings pane, or one session (U12).
type Selection = { pane: PaneId } | { session: string };

const itemClass = (selected: boolean) =>
  `rounded-md px-2.5 py-1 text-left ${
    selected ? "bg-accent text-white" : "text-label active:bg-fill-pressed"
  } outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent`;

function SectionHeader({ children }: { children: string }) {
  return <h2 className="mt-3 mb-0.5 px-2.5 text-[0.8rem] font-semibold text-label-tertiary">{children}</h2>;
}

export function Settings() {
  const [selection, setSelection] = useState<Selection>({ pane: "scratchpad" });
  const pane = "pane" in selection ? selection.pane : null;
  const setPane = useCallback((id: PaneId) => setSelection({ pane: id }), []);
  const capture = useCapture();
  const sessions = useSessions();
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [readiness, setReadiness] = useState<Readiness | null>(null);
  const [models, setModels] = useState<ModelStatus[]>([]);
  const [live, setLive] = useState<Record<string, DownloadUpdate>>({});

  const refresh = useCallback(async () => {
    const [s, r, m] = await Promise.all([
      commands.getSettings(),
      commands.readiness(),
      commands.modelStatuses(),
    ]);
    setSettings(s);
    if (r.status === "ok") setReadiness(r.data);
    setModels(m);
  }, []);

  // A folder can vanish or the microphone be revoked while the window is
  // away; coming back to it is the moment to look again.
  useEffect(() => {
    void refresh();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [refresh]);

  useEffect(() => {
    const unlisten = events.modelDownloadEvent.listen(({ payload }) => {
      setLive((l) => ({ ...l, [payload.model_id]: payload.update }));
      const kind = payload.update.kind;
      if (kind === "completed" || kind === "cancelled" || kind === "failed") void refresh();
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, [refresh]);

  // A recording that starts, from the shortcut or anywhere, brings its
  // scratchpad forward. Nothing moves the window itself.
  const recordingSession = capture.status?.recording ? capture.status.session_id : null;
  const [lastRecorded, setLastRecorded] = useState<string | null>(null);
  useEffect(() => {
    if (recordingSession) {
      setPane("scratchpad");
      setLastRecorded(recordingSession);
    }
  }, [recordingSession, setPane]);

  // Stopping shows the session just recorded on its way through processing.
  // Anything later, completion included, updates its entry and moves nothing.
  useEffect(() => {
    if (recordingSession || !lastRecorded) return;
    const stopped = sessions.entries.find((e) => e.id === lastRecorded);
    if (stopped && !isOpen(stopped.state)) {
      setSelection((s) => ("pane" in s && s.pane === "scratchpad" ? { session: stopped.id } : s));
      setLastRecorded(null);
    }
  }, [recordingSession, lastRecorded, sessions.entries]);

  const listed = sessions.entries.filter((e) => !isOpen(e.state));
  const selectedId = "session" in selection ? selection.session : null;
  const shown = listed.find((e) => e.id === selectedId);

  const onSettings = useCallback(
    (next: SettingsData) => {
      setSettings(next);
      void refresh();
    },
    [refresh],
  );

  const actions = {
    download: (ids: string[]) => {
      setLive((l) => {
        const next = { ...l };
        for (const id of ids) delete next[id];
        return next;
      });
      void commands.downloadModels(ids).then(refresh);
    },
    cancel: () => void commands.cancelModelDownloads(),
  };

  if (!settings || !readiness) return <div className="h-screen bg-window" />;

  return (
    <div className="flex h-screen bg-window select-none">
      <nav aria-label="Longhand" className="flex w-52 shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-separator bg-sidebar p-2 pt-4">
        <button
          type="button"
          aria-current={pane === "scratchpad" ? "page" : undefined}
          onClick={() => setPane("scratchpad")}
          className={`flex items-center justify-between ${itemClass(pane === "scratchpad")}`}
        >
          Scratchpad
          {capture.status?.recording && (
            <span aria-label="recording" className="size-2 rounded-full bg-negative" />
          )}
        </button>
        <SectionHeader>Settings</SectionHeader>
        {panes.map((p) => {
          const tone = paneTone(p.id, readiness);
          const selected = pane === p.id;
          return (
            <button
              key={p.id}
              type="button"
              aria-current={selected ? "page" : undefined}
              onClick={() => setPane(p.id)}
              className={`flex items-center justify-between ${itemClass(selected)}`}
            >
              {p.label}
              {tone && !selected && (
                <Status tone={tone}>
                  <span className="sr-only">needs attention</span>
                </Status>
              )}
            </button>
          );
        })}
        {listed.length > 0 && <SectionHeader>Sessions</SectionHeader>}
        {listed.map((entry) => {
          const selected = shown?.id === entry.id;
          const tone = entryTone(entry.state);
          return (
            <button
              key={entry.id}
              type="button"
              aria-current={selected ? "page" : undefined}
              onClick={() => setSelection({ session: entry.id })}
              className={`flex items-center gap-2 ${itemClass(selected)}`}
            >
              <span className="min-w-0 flex-1">
                <span className="block truncate">{sessionTitle(entry.started)}</span>
                <span className={`block truncate text-[0.8rem] ${selected ? "text-white/80" : "text-label-secondary"}`}>
                  {entryStatus(entry, sessions.stages[entry.id] ?? null)}
                </span>
              </span>
              {tone && !selected && (
                <Status tone={tone}>
                  <span className="sr-only">needs attention</span>
                </Status>
              )}
            </button>
          );
        })}
      </nav>
      <main
        className={`min-w-0 flex-1 select-text ${shown ? "overflow-hidden" : "overflow-y-auto"}`}
      >
        {shown?.state === "landed" && <SessionView key={shown.id} entry={shown} live={sessions.live[shown.id]} />}
        {shown && shown.state !== "landed" && (
          <ProcessingView
            key={shown.id}
            entry={shown}
            version={sessions.version[shown.id] ?? 0}
            live={sessions.live[shown.id]}
            recording={capture.status?.recording ?? false}
            onChanged={() => void sessions.refresh()}
          />
        )}
        {"session" in selection && !shown && (
          <p className="px-6 pt-5 text-label-secondary">This session is no longer listed.</p>
        )}
        {pane === "scratchpad" && (
          <RecordingPane capture={capture} readiness={readiness} models={models} settings={settings} />
        )}
        {pane === "status" && <StatusPane readiness={readiness} onRefresh={refresh} onNavigate={setPane} />}
        {pane === "destination" && (
          <DestinationPicker settings={settings} status={readiness.destination} onSettings={onSettings} />
        )}
        {pane === "summarization" && <EndpointForm settings={settings} onSettings={onSettings} />}
        {pane === "recognition" && (
          <Models models={models} live={live} actions={actions} settings={settings} onSettings={onSettings} />
        )}
        {pane === "licences" && <Licences models={models} />}
      </main>
    </div>
  );
}
