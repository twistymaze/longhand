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
      return null;
  }
}

export function Settings() {
  const [pane, setPane] = useState<PaneId>("status");
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
      <nav aria-label="Settings" className="flex w-44 shrink-0 flex-col gap-0.5 border-r border-separator bg-sidebar p-2 pt-4">
        {panes.map((p) => {
          const tone = paneTone(p.id, readiness);
          const selected = pane === p.id;
          return (
            <button
              key={p.id}
              type="button"
              aria-current={selected ? "page" : undefined}
              onClick={() => setPane(p.id)}
              className={`flex items-center justify-between rounded-md px-2.5 py-1 text-left ${
                selected ? "bg-accent text-white" : "text-label active:bg-fill-pressed"
              } outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent`}
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
      </nav>
      <main className="min-w-0 flex-1 overflow-y-auto select-text">
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
