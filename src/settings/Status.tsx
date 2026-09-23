import { useEffect, useState } from "react";
import { requestMicrophonePermission, checkMicrophonePermission } from "tauri-plugin-macos-permissions-api";
import { commands, events, type ProbeStatus, type Readiness } from "../bindings";
import { describeDestination } from "./DestinationPicker";
import { Button, Group, Pane, Row, Status as StatusText } from "./ui";

const isMac = navigator.userAgent.includes("Mac");

export type PaneId = "status" | "destination" | "summarization" | "recognition" | "licences";

function GoTo({ pane, onNavigate, children }: { pane: PaneId; onNavigate: (p: PaneId) => void; children: string }) {
  return <Button onClick={() => onNavigate(pane)}>{children}</Button>;
}

// Phase 2a probe, kept until U7 lands the real shortcut.
function ShortcutProbe() {
  const [status, setStatus] = useState<ProbeStatus | null>(null);
  useEffect(() => {
    void commands.probeStatus().then(setStatus);
    const unlisten = events.probeShortcutFired.listen((event) => {
      setStatus((s) => (s ? { ...s, fired: event.payload.count } : s));
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, []);
  if (!status) return null;
  return (
    <Group
      header="Shortcut"
      footer="Recording from the shortcut arrives in a later build; for now it only counts presses."
    >
      <Row
        label={<code>{status.shortcut}</code>}
        detail={status.registered ? `Pressed ${status.fired} time${status.fired === 1 ? "" : "s"} this launch` : status.registration_error}
      >
        <StatusText tone={status.registered ? "positive" : "negative"}>
          {status.registered ? "Registered" : "Not registered"}
        </StatusText>
      </Row>
    </Group>
  );
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
    <Pane title="Status">
      <Group
        header="Recording"
        footer={
          readiness.armed
            ? "The shortcut starts recording immediately, with nothing to answer first."
            : "Both of these are settled here, ahead of time, so the first press of a session never asks a question."
        }
      >
        <Row
          label="Shortcut"
          detail={readiness.armed ? "Ready to record" : "Not ready — finish the steps below"}
        >
          <StatusText tone={readiness.armed ? "positive" : "caution"}>
            {readiness.armed ? "Armed" : "Not armed"}
          </StatusText>
        </Row>
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
        <Row label="Destination" detail={<StatusText tone={destination.tone}>{destination.text}</StatusText>}>
          {readiness.destination.kind !== "ready" && (
            <GoTo pane="destination" onNavigate={onNavigate}>
              {readiness.destination.kind === "unset" ? "Choose…" : "Fix…"}
            </GoTo>
          )}
        </Row>
      </Group>

      <Group
        header="Processing"
        footer="Recording never waits on these. A session recorded before they are ready is processed once they are."
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
          label="Summarization"
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

      <ShortcutProbe />
    </Pane>
  );
}
