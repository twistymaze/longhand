// The Settings window (R2, KTD3): a sidebar of panes on the left and the
// selected pane on the right, as in a standard macOS settings window. It
// holds settings only; the notes are in the main window.

import { type KeyboardEvent, type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { commands, events, type DownloadUpdate, type Readiness, type Settings } from "../bindings";
import { DestinationPicker } from "./DestinationPicker";
import { EndpointForm } from "./EndpointForm";
import { AppIcon, FolderIcon, InfoIcon, MicrophoneIcon, SlidersIcon, SparkleIcon, TranscriptIcon } from "./icons";
import { About } from "./About";
import { type Actions, Models } from "./Models";
import { RecordingPane } from "./Recording";
import { type PaneId, StatusPane } from "./Status";
import { ErrorText, type Tone } from "./ui";
import { useAppSettings } from "./useAppSettings";

type PaneEntry = { id: PaneId; label: string; icon: ReactNode };

const sections: { label: string; panes: PaneEntry[] }[] = [
  {
    label: "Settings",
    panes: [
      { id: "status", label: "General", icon: <SlidersIcon /> },
      { id: "recording", label: "Recording", icon: <MicrophoneIcon /> },
      { id: "destination", label: "Storage", icon: <FolderIcon /> },
      // The order of processing: a note is saved, then transcribed, then
      // summarized. The General pane lists its rows in the same order.
      { id: "recognition", label: "Transcription", icon: <TranscriptIcon /> },
      { id: "summarization", label: "Summaries", icon: <SparkleIcon /> },
    ],
  },
  {
    label: "About",
    panes: [{ id: "about", label: "About", icon: <InfoIcon /> }],
  },
];

const order = sections.flatMap((s) => s.panes.map((p) => p.id));

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
    case "recording":
    case "about":
      return null;
  }
}

const badgeClass: Record<Tone, string> = {
  positive: "bg-positive",
  caution: "bg-caution",
  negative: "bg-negative",
  neutral: "bg-label-tertiary",
};

export function SettingsWindow() {
  const [pane, setPane] = useState<PaneId>("status");
  const { settings, readiness, models, error, refresh, setSettings } = useAppSettings();
  const [live, setLive] = useState<Record<string, DownloadUpdate>>({});
  const nav = useRef<HTMLDivElement>(null);
  const main = useRef<HTMLElement>(null);

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

  // A new pane starts at its top.
  const show = (id: PaneId) => {
    setPane(id);
    main.current?.scrollTo({ top: 0 });
  };

  const actions: Actions = {
    cancel: () => void commands.cancelModelDownloads(),
    refresh,
  };

  // The up and down arrows move through the panes, as in a sidebar.
  const onNavKey = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const at = order.indexOf(pane);
    const next = order[Math.min(order.length - 1, Math.max(0, at + (e.key === "ArrowDown" ? 1 : -1)))];
    show(next);
    requestAnimationFrame(() => nav.current?.querySelector<HTMLElement>(`[data-pane="${next}"]`)?.focus());
  };

  if (!settings || !readiness) {
    return <div className="h-screen bg-pane pt-3">{error && <ErrorText>{error}</ErrorText>}</div>;
  }

  return (
    <div className="flex h-screen bg-pane">
      <div className="settings-navigation flex min-h-0 w-[30%] max-w-[13.5rem] min-w-0 shrink-0 flex-col border-r border-separator bg-sidebar select-none">
        <div className="settings-brand flex shrink-0 flex-wrap items-center gap-2 px-4 pt-4 pb-3">
          <AppIcon className="size-7 shrink-0" />
          <span className="min-w-0 [overflow-wrap:anywhere] text-[0.95rem] font-semibold tracking-[-0.01em]">Longhand</span>
        </div>
        <div
          ref={nav}
          role="tablist"
          aria-label="Settings"
          aria-orientation="vertical"
          onKeyDown={onNavKey}
          className="min-h-0 flex-1 overflow-y-auto px-2.5 pb-3"
        >
          {sections.map((section) => (
            <div key={section.label} role="presentation" className="pt-2">
              <div
                role="presentation"
                className="px-2 pt-1.5 pb-1 text-[0.7rem] font-semibold tracking-[0.07em] text-label-tertiary uppercase"
              >
                {section.label}
              </div>
              {section.panes.map((p) => {
                const tone = paneTone(p.id, readiness);
                const selected = pane === p.id;
                return (
                  <button
                    key={p.id}
                    type="button"
                    role="tab"
                    data-pane={p.id}
                    aria-selected={selected}
                    tabIndex={selected ? 0 : -1}
                    onClick={() => show(p.id)}
                    className={`mb-px flex w-full items-center gap-2.5 rounded-[7px] px-2 py-[5px] text-left ${
                      selected ? "bg-sidebar-selected font-medium text-label" : "text-label hover:bg-row-hover"
                    } outline-offset-0 focus-visible:outline-2 focus-visible:outline-accent`}
                  >
                    <span className={selected ? "text-accent" : "text-label-secondary"}>{p.icon}</span>
                    <span className="settings-tab-label min-w-0 flex-1 truncate">{p.label}</span>
                    {tone && (
                      <span aria-hidden className={`size-[7px] shrink-0 rounded-full ${badgeClass[tone]}`} />
                    )}
                    {tone && <span className="sr-only">needs attention</span>}
                  </button>
                );
              })}
            </div>
          ))}
        </div>
      </div>

      <main ref={main} role="tabpanel" className="min-h-0 min-w-0 flex-1 overflow-y-auto">
        {error && (
          <div className="pt-3">
            <ErrorText>{error}</ErrorText>
          </div>
        )}
        <div>
          {pane === "status" && <StatusPane readiness={readiness} onRefresh={refresh} onNavigate={show} />}
          {pane === "recording" && <RecordingPane settings={settings} onSettings={onSettings} />}
          {pane === "destination" && (
            <DestinationPicker settings={settings} status={readiness.destination} onSettings={onSettings} />
          )}
          {pane === "summarization" && <EndpointForm settings={settings} onSettings={onSettings} />}
          {pane === "recognition" && (
            <Models models={models} live={live} actions={actions} settings={settings} onSettings={onSettings} />
          )}
          {pane === "about" && <About models={models} />}
        </div>
      </main>
    </div>
  );
}
