import { useEffect, useState } from "react";
import { requestMicrophonePermission, checkMicrophonePermission } from "tauri-plugin-macos-permissions-api";
import { DEFAULT_SHORTCUT, commands, type SystemAudioPermission, type Readiness, type Result, type Settings, type ShortcutStatus, type ShortcutStatuses } from "../bindings";
import { sentence } from "../processing/describe";
import { features } from "../lib/features";
import { describeDestination } from "./DestinationPicker";
import { ShortcutField } from "./ShortcutField";
import { readOnlyReason } from "./shared";
import { LoginItem } from "./LoginItem";
import { Button, ErrorText, Group, Pane, Row, Status as StatusText, Switch } from "./ui";

// Read the platform on each render, so that tests can set the user agent.
const isMac = () => navigator.userAgent.includes("Mac");

const SHOW_SHORTCUT_TEXT = "This shortcut brings the window back from any app, and hides it again when it is in front.";

/// The Window group footer: what a window close does on this platform
/// with this Hide from Dock value (R5).
function closeBehaviour(hideFromDock: boolean): string {
  if (!isMac()) return "Closing the window keeps Longhand running in the system tray.";
  return hideFromDock
    ? "Closing the window keeps Longhand running only in the menu bar."
    : "Closing the window keeps Longhand in the Dock and the menu bar.";
}

export type PaneId = "status" | "recording" | "destination" | "summarization" | "recognition" | "about";

function GoTo({ pane, onNavigate, children }: { pane: PaneId; onNavigate: (p: PaneId) => void; children: string }) {
  return <Button onClick={() => onNavigate(pane)}>{children}</Button>;
}

export function StatusPane({
  readiness,
  settings,
  onSettings,
  onRefresh,
  onNavigate,
}: {
  readiness: Readiness;
  settings: Settings;
  onSettings: (s: Settings) => void;
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
      <Group header="Window" footer={`${closeBehaviour(settings.hide_from_dock)} ${SHOW_SHORTCUT_TEXT}`}>
        <LoginItem />
        {isMac() && <HideFromDock settings={settings} onSettings={onSettings} />}
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
            isMac() && (
              <Button prominent disabled={asking} onClick={askForMicrophone}>
                {asking ? "Waiting…" : "Allow…"}
              </Button>
            )
          )}
        </Row>
        {features.systemAudio && <SystemAudioRow permission={readiness.system_audio} onRefresh={onRefresh} />}
        <Row label="Storage"detail={<StatusText tone={destination.tone}>{destination.text}</StatusText>}>
          {readiness.destination.kind !== "ready" && (
            <GoTo pane="destination" onNavigate={onNavigate}>
              Fix…
            </GoTo>
          )}
        </Row>
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

      {isMac() && (
        <Group
          header="Updates"
          footer={
            readOnlyReason(settings)
              ? `${readOnlyReason(settings)} This setting can’t be changed now.`
              : "Longhand asks before it installs a new version."
          }
        >
          <AutomaticUpdates settings={settings} onSettings={onSettings} />
        </Group>
      )}

      <AlertSound />
    </Pane>
  );
}

/// A switch for one Boolean setting. A failed change keeps the previous
/// value.
function SettingSwitch({
  label,
  detail,
  checked,
  set,
  onSettings,
  disabled = false,
}: {
  label: string;
  detail: string;
  checked: boolean;
  set: (enabled: boolean) => Promise<Result<Settings, string>>;
  onSettings: (s: Settings) => void;
  /// The setting can't be changed now, for example a shared setting while
  /// the storage folder is not available.
  disabled?: boolean;
}) {
  const [error, setError] = useState<string | null>(null);
  // The switch is off while a change is in progress, so a second click
  // cannot send a second request.
  const [pending, setPending] = useState(false);
  const change = async (enabled: boolean) => {
    setPending(true);
    try {
      const result = await set(enabled);
      if (result.status === "error") setError(sentence(result.error));
      else {
        setError(null);
        onSettings(result.data);
      }
    } finally {
      setPending(false);
    }
  };
  return (
    <>
      <Row label={label} detail={detail}>
        <Switch label={label} checked={checked} disabled={pending || disabled} onChange={(v) => void change(v)} />
      </Row>
      {error && <ErrorText>{error}</ErrorText>}
    </>
  );
}

/// The Hide from Dock switch (R2, KTD3). It shows on macOS only, because
/// the setter returns an error on other platforms.
function HideFromDock({ settings, onSettings }: { settings: Settings; onSettings: (s: Settings) => void }) {
  return (
    <SettingSwitch
      label="Hide from Dock"
      detail="Show Longhand in the Dock only while a window is open"
      checked={settings.hide_from_dock}
      set={commands.setHideFromDock}
      onSettings={onSettings}
    />
  );
}

/// The switch for automatic update checks (R10, KD7). Updates are macOS
/// only, so the group shows on macOS only.
function AutomaticUpdates({ settings, onSettings }: { settings: Settings; onSettings: (s: Settings) => void }) {
  return (
    <SettingSwitch
      label="Check for updates automatically"
      detail="Checks when Longhand opens and every hour"
      checked={settings.check_updates_automatically}
      set={commands.setCheckUpdatesAutomatically}
      onSettings={onSettings}
      disabled={readOnlyReason(settings) !== null}
    />
  );
}

/// The system audio permission, in the "Computer audio" row (R9, KTD12,
/// KTD15). It follows the Microphone row: a status and at most one button.
function SystemAudioRow({
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

  const openSettings = () => void commands.openSystemAudioSettings();

  const detail =
    permission === "allowed" ? (
      "Records what this computer plays while Computer audio is on"
    ) : permission === "not_asked" ? (
      <StatusText tone="neutral">Not allowed yet</StatusText>
    ) : permission === "denied" ? (
      <StatusText tone="negative">Not allowed</StatusText>
    ) : (
      <StatusText tone="caution">Cannot check</StatusText>
    );

  return (
    <Row label="Computer audio" detail={detail}>
      {permission === "allowed" ? (
        <StatusText tone="positive">Allowed</StatusText>
      ) : permission === "not_asked" ? (
        <Button prominent disabled={asking} onClick={() => void ask()}>
          {asking ? "Waiting…" : "Allow…"}
        </Button>
      ) : (
        <Button onClick={openSettings}>Open System Settings</Button>
      )}
    </Row>
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
    <Group header="Alerts">
      <Row
        label="Alert sound"
        detail={error ?? "Plays when recording stops and cannot restart"}
      >
        <Button onClick={() => void play()}>Play Sound</Button>
      </Row>
    </Group>
  );
}
