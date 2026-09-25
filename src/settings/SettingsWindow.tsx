// The Settings window (R2, KTD3): a toolbar of panes above the selected
// pane, as in a standard macOS Settings window. It holds settings only; the
// sessions are in the main window.

import { useCallback, useEffect, useState } from "react";
import { commands, events, type DownloadUpdate, type Readiness, type Settings } from "../bindings";
import { DestinationPicker } from "./DestinationPicker";
import { EndpointForm } from "./EndpointForm";
import { Licences } from "./Licences";
import { Models } from "./Models";
import { type PaneId, StatusPane } from "./Status";
import { ErrorText, Status, type Tone } from "./ui";
import { useAppSettings } from "./useAppSettings";

const panes: { id: PaneId; label: string }[] = [
  { id: "status", label: "Status" },
  { id: "destination", label: "Destination" },
  { id: "summarization", label: "Summarization" },
  { id: "recognition", label: "Recognition" },
  { id: "licences", label: "Licences" },
];

function paneTone(id: PaneId, readiness: Readiness): Tone | null {
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

export function SettingsWindow() {
  const [pane, setPane] = useState<PaneId>("status");
  const { settings, readiness, models, error, refresh, setSettings } = useAppSettings();
  const [live, setLive] = useState<Record<string, DownloadUpdate>>({});

  useEffect(() => {
    const unlisten = events.modelDownloadEvent.listen(({ payload }) => {
      setLive((l) => ({ ...l, [payload.model_id]: payload.update }));
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, []);

  const onSettings = useCallback(
    (next: Settings) => {
      setSettings(next);
      void refresh();
    },
    [setSettings, refresh],
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

  if (!settings || !readiness) {
    return <div className="h-screen bg-window pt-3">{error && <ErrorText>{error}</ErrorText>}</div>;
  }

  return (
    <div className="flex h-screen flex-col bg-window">
      <nav
        aria-label="Settings"
        role="tablist"
        className="flex shrink-0 justify-center gap-1 border-b border-separator bg-sidebar px-3 py-2 select-none"
      >
        {panes.map((p) => {
          const tone = paneTone(p.id, readiness);
          const selected = pane === p.id;
          return (
            <button
              key={p.id}
              type="button"
              role="tab"
              aria-selected={selected}
              onClick={() => setPane(p.id)}
              className={`inline-flex items-center gap-1.5 rounded-md px-2.5 py-1 ${
                selected ? "bg-fill-pressed text-label" : "text-label-secondary active:bg-fill-pressed"
              } outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent`}
            >
              {p.label}
              {tone && (
                <Status tone={tone}>
                  <span className="sr-only">needs attention</span>
                </Status>
              )}
            </button>
          );
        })}
      </nav>
      {error && (
        <div className="shrink-0 pt-2">
          <ErrorText>{error}</ErrorText>
        </div>
      )}
      <main role="tabpanel" className="min-h-0 flex-1 overflow-y-auto">
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
