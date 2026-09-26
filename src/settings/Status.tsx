import { useEffect, useState } from "react";
import { requestMicrophonePermission, checkMicrophonePermission } from "tauri-plugin-macos-permissions-api";
import { DEFAULT_SHORTCUT, commands, type Readiness, type ShortcutStatus, type ShortcutStatuses } from "../bindings";
import { describeDestination } from "./DestinationPicker";
import { ShortcutField } from "./ShortcutField";
import { Button, Group, Pane, Row, Status as StatusText } from "./ui";

const isMac = navigator.userAgent.includes("Mac");

export type PaneId = "status" | "destination" | "summarization" | "recognition" | "licences";

function GoTo({ pane, onNavigate, children }: { pane: PaneId; onNavigate: (p: PaneId) => void; children: string }) {
  return <Button onClick={() => onNavigate(pane)}>{children}</Button>;
}

export function StatusPane({
  readiness,
  onRefresh,
  onNavigate,
}: {
  readiness: Readiness;
  onRefresh: () => Promise<void>;
  onNavigate: (p: PaneId) => void;
}) {
  const [asking, setAsking] = useState(false);
  const destination = describeDestination(readiness.destination);
  // Readiness holds the recording shortcut; only the show shortcut is read here.
  const recording = readiness.shortcut;
  const [showWindow, setShowWindow] = useState<ShortcutStatus>({ chord: null, registered: false, error: null });
  useEffect(() => {
    void commands.shortcutStatus().then((s) => setShowWindow(s.show_window));
  }, []);
  const onShortcutsChanged = (next: ShortcutStatuses) => {
    setShowWindow(next.show_window);
    // Ready to record depends on the recording shortcut (R13).
    void onRefresh();
  };

  const askForMicrophone = async () => {
    setAsking(true);
    // The plugin passes a nil completion handler to AVCaptureDevice, so this
    // resolves before the user answers the system dialog. Poll instead.
    await requestMicrophonePermission();
    for (let i = 0; i < 120; i++) {
      if (await checkMicrophonePermission()) break;
      await new Promise((r) => setTimeout(r, 500));
    }
    setAsking(false);
    await onRefresh();
  };

  const missing = readiness.missing_models.length;

  return (
    <Pane title="General">
      <Group
        header="Recording"
        footer={
          !readiness.armed
            ? "Set these up ahead of time, so a new note never stops to ask a question."
            : recording.registered
              ? "The shortcut starts recording immediately, with nothing to answer first."
              : "New Note starts recording immediately, with nothing to answer first."
        }
      >
        <Row
          label="Ready to record"
          detail={readiness.armed ? "Everything below is set up" : "Not yet — finish the steps below"}
        >
          <StatusText tone={readiness.armed ? "positive" : "caution"}>
            {readiness.armed ? "Ready" : "Not ready"}
          </StatusText>
        </Row>
        <ShortcutField
          kind="recording"
          label="Recording shortcut"
          status={recording}
          resetTo={DEFAULT_SHORTCUT}
          detail={
            !recording.chord ? (
              <StatusText tone="neutral">Off — New Note still starts a recording</StatusText>
            ) : recording.registered ? (
              "Starts and stops a recording from any app"
            ) : (
              <StatusText tone="negative">{recording.error ?? "Not registered"}</StatusText>
            )
          }
          onChanged={onShortcutsChanged}
        />
        <Row
          label="Microphone"
          detail={readiness.microphone_authorized ? "Access granted" : "Longhand needs permission to hear the room"}
        >
          {readiness.microphone_authorized ? (
            <StatusText tone="positive">Allowed</StatusText>
          ) : (
            isMac && (
              <Button prominent disabled={asking} onClick={askForMicrophone}>
                {asking ? "Waiting…" : "Allow…"}
              </Button>
            )
          )}
        </Row>
        <Row label="Storage" detail={<StatusText tone={destination.tone}>{destination.text}</StatusText>}>
          {readiness.destination.kind !== "ready" && (
            <GoTo pane="destination" onNavigate={onNavigate}>
              {readiness.destination.kind === "unset" ? "Choose…" : "Fix…"}
            </GoTo>
          )}
        </Row>
      </Group>

      <Group
        header="Window"
        footer="Closing the window keeps Longhand running in the menu bar. This shortcut brings the window back from any app, and hides it again when it is in front."
      >
        <ShortcutField
          kind="show_window"
          label="Show Longhand"
          status={showWindow}
          detail={
            !showWindow.chord ? (
              <StatusText tone="neutral">Off</StatusText>
            ) : showWindow.registered ? (
              "Shows or hides the Longhand window from any app"
            ) : (
              <StatusText tone="negative">{showWindow.error ?? "Not registered"}</StatusText>
            )
          }
          onChanged={onShortcutsChanged}
        />
      </Group>

      <Group
        header="Processing"
        footer="Recording never waits on these. A note recorded before they are ready is processed once they are."
      >
        <Row
          label="Models"
          detail={missing === 0 ? "All downloaded" : `${missing} still to download`}
        >
          {missing === 0 ? (
            <StatusText tone="positive">Ready</StatusText>
          ) : (
            <GoTo pane="recognition" onNavigate={onNavigate}>
              Download…
            </GoTo>
          )}
        </Row>
        <Row
          label="Summaries"
          detail={readiness.endpoint_configured ? "Endpoint configured" : "No endpoint yet — summaries wait until one is set up"}
        >
          {readiness.endpoint_configured ? (
            <StatusText tone="positive">Ready</StatusText>
          ) : (
            <GoTo pane="summarization" onNavigate={onNavigate}>
              Set Up…
            </GoTo>
          )}
        </Row>
      </Group>

      <AlertSound />
    </Pane>
  );
}

/// The failed-capture sound (R18, KTD16). The user can play it, and no test
/// is asked for.
function AlertSound() {
  const [error, setError] = useState<string | null>(null);

  const play = async () => {
    const result = await commands.playAlertSound();
    setError(result.status === "error" ? result.error : null);
  };

  return (
    <Group
      header="Alerts"
      footer="If recording stops and cannot restart, Longhand plays this sound three times over 90 seconds and posts a notification. Focus and Do Not Disturb can silence the notification, and Longhand cannot tell when they do."
    >
      <Row
        label="Alert sound"
        detail={error ?? "Plays when recording stops and cannot restart"}
      >
        <Button onClick={() => void play()}>Play Sound</Button>
      </Row>
    </Group>
  );
}
