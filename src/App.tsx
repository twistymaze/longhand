// The page has six windows (KTD3). The Settings window loads it with
// `?window=settings`, the licences window with `?window=licences`, the
// Recent changes window with `?window=changelog`, the update
// window with `?window=update`, the recording widget with `?window=widget`,
// and the main window with no query.

import { useEffect, useRef, useState } from "react";
import { useInterfaceZoom } from "./lib/useInterfaceZoom";
import { commands, events } from "./bindings";
import { Home } from "./home/Home";
import { RecentChangesWindow } from "./changelog/RecentChangesWindow";
import { LicencesWindow } from "./licences/LicencesWindow";
import { ScopeTonesProvider } from "./lib/Labels";
import { notesSaved } from "./lib/notesSaves";
import { formatChord } from "./lib/timeline";
import { useCapture } from "./recording/useCapture";
import { SessionPage } from "./session/SessionPage";
import { useSessions } from "./session/useSessions";
import { SettingsWindow } from "./settings/SettingsWindow";
import { UpdateWindow } from "./update/UpdateWindow";
import { RecordingWidget } from "./widget/RecordingWidget";
import { useAppSettings } from "./settings/useAppSettings";

const isMac = navigator.userAgent.includes("Mac");

/// What the main window shows: Home, or the page of one session (KTD2).
/// `started` is true for a note that the user just started: its notes
/// editor takes the focus.
export type View = { kind: "home" } | { kind: "session"; id: string; started?: boolean };

function MainWindow() {
  const [view, setView] = useState<View>({ kind: "home" });
  const capture = useCapture();
  const sessions = useSessions();
  const { settings, readiness } = useAppSettings();

  // A recording that starts or resumes, from the shortcut or anywhere, opens
  // its session's page (KTD2), from any view. So does the tray's "Open
  // Longhand" while a session records (R3). A pause, a device switch or a
  // restart opens no segment and does not move the view. The page opens
  // when the capture status or the list knows the session, so it never
  // shows a session it cannot find.
  const [requested, setRequested] = useState<string | null>(null);
  // The requested note that the user just started: New Note on Home, or
  // the shortcut. The shortcut and the tray send `showSession`. The tray
  // sends it only while a note records, and that page takes the focus too.
  const started = useRef<string | null>(null);
  const requestStarted = useRef((id: string) => {
    started.current = id;
    setRequested(id);
  }).current;
  // The reason of a failed shortcut press, for the view that the window
  // showed at the press. The next view change takes it away.
  const [shortcutNotice, setShortcutNotice] = useState<{ text: string; view: View } | null>(null);
  const shownView = useRef(view);
  shownView.current = view;
  const pressNotice = shortcutNotice?.view === view ? shortcutNotice : null;
  useEffect(() => {
    const unlisteners = [
      events.recordingChanged.listen(({ payload }) => {
        if (payload.note_id && payload.opened) setRequested(payload.note_id);
      }),
      events.showSession.listen(({ payload }) => requestStarted(payload.note_id)),
      events.shortcutFailed.listen(({ payload }) =>
        setShortcutNotice({ text: payload.text, view: shownView.current }),
      ),
    ];
    return () => {
      for (const u of unlisteners) void u.then((f) => f());
    };
  }, []);
  // The backend keeps the shown note for the recording shortcut (KTD7). The
  // note that the window stops showing is left: an empty note goes (R18).
  // The leave waits for the notes saves of the note. The backend keeps a
  // note that the window shows again, so each report goes before the leave.
  const shown = view.kind === "session" ? view.id : null;
  const reported = useRef<string | null>(null);
  // Leave the note `id`. A note that goes while the window shows it again
  // gives Home in its place.
  const leave = useRef(async (id: string) => {
    await notesSaved(id);
    const left = await commands.leaveNote(id);
    if (left.status === "ok" && left.data.kind === "removed" && reported.current === id) {
      reported.current = null;
      setView({ kind: "home" });
    }
  }).current;
  useEffect(() => {
    const previous = reported.current;
    reported.current = shown;
    const report = commands.setShownNote(shown);
    if (previous !== null && previous !== shown) void report.then(() => leave(previous));
  }, [shown, leave]);
  // A hide or close of the main window leaves the shown note too. The
  // window that comes back reports its note again.
  useEffect(() => {
    const unlisten = events.mainWindowHidden.listen(async () => {
      await commands.setShownNote(null);
      const id = reported.current;
      if (id !== null) await leave(id);
    });
    const report = () => void commands.setShownNote(reported.current);
    window.addEventListener("focus", report);
    return () => {
      void unlisten.then((f) => f());
      window.removeEventListener("focus", report);
    };
  }, [leave]);

  const known =
    requested !== null &&
    (capture.status?.note_id === requested || sessions.entries.some((e) => e.id === requested));
  useEffect(() => {
    if (requested === null || !known) return;
    setView({ kind: "session", id: requested, started: started.current === requested });
    started.current = null;
    setRequested(null);
  }, [requested, known]);

  const open = capture.status?.note_id ?? null;
  const recording = open
    ? {
        noteId: open,
        elapsedMs: capture.recordedMs,
        interrupted: capture.status?.interruption != null,
        paused: capture.status?.paused ?? false,
      }
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
          started={view.started ?? false}
          shortcutNotice={pressNotice}
          onBack={back}
          onOpenNote={(id) => {
            setRequested(id);
            void sessions.refresh();
          }}
        />
      )}
      <div hidden={view.kind === "session"} className="h-full">
        <Home
          active={view.kind === "home"}
          entries={sessions.entries}
          loaded={sessions.loaded}
          loadError={sessions.loadError}
          listRevision={sessions.listRevision}
          getListRevision={sessions.getListRevision}
          revisions={sessions.version}
          stages={sessions.stages}
          recording={recording}
          readiness={readiness}
          chord={
            !settings ? "the shortcut" : settings.shortcut ? formatChord(settings.shortcut, isMac) : null
          }
          newNoteRecords={settings?.new_note_records ?? true}
          damagedFile={settings?.shared.damaged_file ?? null}
          shortcutNotice={view.kind === "home" ? pressNotice : null}
          onOpen={(id) => setView({ kind: "session", id })}
          // A note that does not record opens when the list knows it.
          onStarted={requestStarted}
          onChanged={() => void sessions.refresh()}
        />
      </div>
      </div>
    </div>
  );
}

export default function App() {
  const which = new URLSearchParams(window.location.search).get("window");
  // The update window and the recording widget have a fixed size, so the
  // interface size does not apply to them.
  const zoom = useInterfaceZoom({ enabled: which !== "update" && which !== "widget" });
  if (which === "widget") return <RecordingWidget />;
  return <>
    {which === "settings" ? (
      // The Settings window and the main window show tag labels, with the
      // same scope colours.
      <ScopeTonesProvider>
        <SettingsWindow />
      </ScopeTonesProvider>
    ) : which === "licences" ? (
      <LicencesWindow />
    ) : which === "changelog" ? (
      <RecentChangesWindow />
    ) : which === "update" ? (
      <UpdateWindow />
    ) : (
      <ScopeTonesProvider>
        <MainWindow />
      </ScopeTonesProvider>
    )}
    {zoom.error && <div role="alert" className="fixed inset-x-3 bottom-3 z-50 rounded-lg bg-group p-3 text-label shadow-lg">
      <p>Could not change the interface size: {zoom.error}</p>
      <button onClick={zoom.dismiss} className="mt-2 text-accent">Dismiss</button>
    </div>}
  </>;
}
