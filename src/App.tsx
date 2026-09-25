// The page has two windows (KTD3). The Settings window loads it with
// `?window=settings`; the main window loads it with no query.

import { useEffect, useState } from "react";
import { events } from "./bindings";
import { Home } from "./home/Home";
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
  const { settings, readiness, models } = useAppSettings();

  // A recording that starts or resumes, from the shortcut or anywhere, opens
  // its session's page (KTD2). The page opens when the capture status or the
  // list knows the session, so it never shows a session it cannot find.
  const [started, setStarted] = useState<string | null>(null);
  useEffect(() => {
    const unlisten = events.recordingChanged.listen(({ payload }) => {
      if (payload.session_id) setStarted(payload.session_id);
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, []);
  const known =
    started !== null &&
    (capture.status?.session_id === started || sessions.entries.some((e) => e.id === started));
  useEffect(() => {
    if (started === null || !known) return;
    setView({ kind: "session", id: started });
    setStarted(null);
  }, [started, known]);

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
  // selection and scroll position.
  return (
    <div className="h-screen bg-window">
      {view.kind === "session" && (
        <SessionPage
          id={view.id}
          capture={capture}
          sessions={sessions}
          readiness={readiness}
          models={models}
          settings={settings}
          onBack={back}
        />
      )}
      <div hidden={view.kind === "session"} className="h-full">
        <Home
          entries={sessions.entries}
          stages={sessions.stages}
          recording={recording}
          readiness={readiness}
          chord={settings ? formatChord(settings.shortcut, isMac) : "the shortcut"}
          onOpen={(id) => setView({ kind: "session", id })}
          onChanged={() => void sessions.refresh()}
        />
      </div>
    </div>
  );
}

export default function App() {
  const settingsWindow = new URLSearchParams(window.location.search).get("window") === "settings";
  return settingsWindow ? <SettingsWindow /> : <MainWindow />;
}
