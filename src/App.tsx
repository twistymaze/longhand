// The page has three windows (KTD3). The Settings window loads it with
// `?window=settings`, the licences window with `?window=licences`, and the
// main window with no query.

import { useEffect, useState } from "react";
import { events } from "./bindings";
import { Home } from "./home/Home";
import { LicencesWindow } from "./licences/LicencesWindow";
import { formatChord } from "./lib/timeline";
import { useCapture } from "./recording/useCapture";
import { SessionPage } from "./session/SessionPage";
import { useSessions } from "./session/useSessions";
import { SettingsWindow } from "./settings/SettingsWindow";
import { useAppSettings } from "./settings/useAppSettings";

const isMac = navigator.userAgent.includes("Mac");

/// What the main window shows: Home, or the page of one session (KTD2).
export type View = { kind: "home" } | { kind: "session"; id: string };

function MainWindow() {
  const [view, setView] = useState<View>({ kind: "home" });
  const capture = useCapture();
  const sessions = useSessions();
  const { settings, readiness } = useAppSettings();

  // A recording that starts or resumes, from the shortcut or anywhere, opens
  // its session's page (KTD2). So does the tray's "Open Longhand" while a
  // session records (R3). The page opens when the capture status or the
  // list knows the session, so it never shows a session it cannot find.
  const [requested, setRequested] = useState<string | null>(null);
  useEffect(() => {
    const unlisteners = [
      events.recordingChanged.listen(({ payload }) => {
        if (payload.session_id) setRequested(payload.session_id);
      }),
      events.showSession.listen(({ payload }) => setRequested(payload.session_id)),
    ];
    return () => {
      for (const u of unlisteners) void u.then((f) => f());
    };
  }, []);
  const known =
    requested !== null &&
    (capture.status?.session_id === requested || sessions.entries.some((e) => e.id === requested));
  useEffect(() => {
    if (requested === null || !known) return;
    setView({ kind: "session", id: requested });
    setRequested(null);
  }, [requested, known]);

  const open = capture.status?.session_id ?? null;
  const recording = open
    ? { sessionId: open, elapsedMs: capture.recordedMs, interrupted: !capture.status?.recording }
    : null;

  // The keyboard focus goes back to the row that Tab moves to in the list.
  const back = () => {
    setView({ kind: "home" });
    requestAnimationFrame(() => document.querySelector<HTMLElement>('[role="option"][tabindex="0"]')?.focus());
  };

  // Home stays mounted under a session page, so "← Back" returns to the same
  // selection and scroll position. The key gives each session a new page:
  // a title edit, a notice or an open tab of one session never goes to the
  // session that opens in its place.
  return (
    <div className="flex h-screen flex-col bg-window">
      <div data-tauri-drag-region className="title-strip" />
      <div className="min-h-0 flex-1">
      {view.kind === "session" && (
        <SessionPage
          key={view.id}
          id={view.id}
          capture={capture}
          sessions={sessions}
          readiness={readiness}
          onBack={back}
        />
      )}
      <div hidden={view.kind === "session"} className="h-full">
        <Home
          entries={sessions.entries}
          stages={sessions.stages}
          recording={recording}
          readiness={readiness}
          chord={
            !settings ? "the shortcut" : settings.shortcut ? formatChord(settings.shortcut, isMac) : null
          }
          onOpen={(id) => setView({ kind: "session", id })}
          onChanged={() => void sessions.refresh()}
        />
      </div>
      </div>
    </div>
  );
}

export default function App() {
  const which = new URLSearchParams(window.location.search).get("window");
  if (which === "settings") return <SettingsWindow />;
  if (which === "licences") return <LicencesWindow />;
  return <MainWindow />;
}
