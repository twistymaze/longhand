// The microphone and Computer audio permissions (R4, R6, R7, KTD5). The
// General setup list and the Recording pane use the same rows and buttons,
// so each permission request has one implementation.

import { useState } from "react";
import { checkMicrophonePermission, requestMicrophonePermission } from "tauri-plugin-macos-permissions-api";
import { commands, type SystemAudioPermission } from "../bindings";
import { Button, Row, Status as StatusText } from "./ui";

/// Read the platform on each render, so that tests can set the user agent.
export const isMac = () => navigator.userAgent.includes("Mac");

/// The microphone Allow button. It shows on macOS only: on other platforms
/// Longhand cannot show the system prompt (R4).
export function MicrophoneAllowButton({ onRefresh }: { onRefresh: () => Promise<void> }) {
  const [asking, setAsking] = useState(false);

  const ask = async () => {
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

  if (!isMac()) return null;
  return (
    <Button prominent disabled={asking} onClick={() => void ask()}>
      {asking ? "Waiting…" : "Allow…"}
    </Button>
  );
}

/// The microphone access row: a status, or the Allow button.
export function MicrophoneAccessRow({
  authorized,
  onRefresh,
}: {
  authorized: boolean;
  onRefresh: () => Promise<void>;
}) {
  return (
    <Row
      label="Microphone access"
      detail={authorized ? "Access granted" : "Longhand needs permission to hear the room"}
    >
      {authorized ? <StatusText tone="positive">Allowed</StatusText> : <MicrophoneAllowButton onRefresh={onRefresh} />}
    </Row>
  );
}

/// The one button for a Computer audio permission that is not allowed:
/// Allow when the user was not asked yet, else Open System Settings.
export function SystemAudioButton({
  permission,
  onRefresh,
}: {
  permission: SystemAudioPermission;
  onRefresh: () => Promise<void>;
}) {
  const [asking, setAsking] = useState(false);

  const ask = async () => {
    setAsking(true);
    // The short tap shows the macOS prompt, and the prompt does not wait
    // for the tap. Poll until the user answers.
    let state = await commands.requestSystemAudioPermission();
    for (let i = 0; i < 120 && state.status === "ok" && state.data === "not_asked"; i++) {
      await new Promise((r) => setTimeout(r, 500));
      state = await commands.systemAudioPermission();
    }
    setAsking(false);
    await onRefresh();
  };

  if (permission === "allowed") return null;
  if (permission === "not_asked") {
    return (
      <Button prominent disabled={asking} onClick={() => void ask()}>
        {asking ? "Waiting…" : "Allow…"}
      </Button>
    );
  }
  return <Button onClick={() => void commands.openSystemAudioSettings()}>Open System Settings</Button>;
}

/// The status text for a Computer audio permission.
export function systemAudioDetail(permission: SystemAudioPermission) {
  return permission === "allowed" ? (
    "Records what this computer plays while Computer audio is on"
  ) : permission === "not_asked" ? (
    <StatusText tone="neutral">Not allowed yet</StatusText>
  ) : permission === "denied" ? (
    <StatusText tone="negative">Not allowed</StatusText>
  ) : (
    <StatusText tone="caution">Cannot check</StatusText>
  );
}

/// The Computer audio access row (R9, KTD12, KTD15): a status and at most
/// one button.
export function SystemAudioAccessRow({
  permission,
  onRefresh,
}: {
  permission: SystemAudioPermission;
  onRefresh: () => Promise<void>;
}) {
  return (
    <Row label="Computer audio access" detail={systemAudioDetail(permission)}>
      {permission === "allowed" ? (
        <StatusText tone="positive">Allowed</StatusText>
      ) : (
        <SystemAudioButton permission={permission} onRefresh={onRefresh} />
      )}
    </Row>
  );
}
