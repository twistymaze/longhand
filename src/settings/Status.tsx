import { useEffect, useState } from "react";
import { commands, type Readiness, type Result, type Settings, type SettingsPane, type ShortcutStatus } from "../bindings";
import { sentence } from "../processing/describe";
import { features } from "../lib/features";
import { describeDestination } from "./DestinationPicker";
import { ShortcutField } from "./ShortcutField";
import { readOnlyReason } from "./shared";
import { LoginItem } from "./LoginItem";
import { isMac, MicrophoneAllowButton, SystemAudioButton, systemAudioDetail } from "./Permissions";
import { outstandingItems, type SetupItem } from "./setup";
import { Button, ErrorText, Group, Pane, Row, Status as StatusText, Switch } from "./ui";

const SHOW_SHORTCUT_TEXT = "This shortcut brings the window back from any app, and hides it again when it is in front.";

/// The Window group footer: what a window close does on this platform
/// with this Hide from Dock value (R5).
function closeBehaviour(hideFromDock: boolean): string {
  if (!isMac()) return "Closing the window keeps Longhand running in the system tray.";
  return hideFromDock
    ? "Closing the window keeps Longhand running only in the menu bar."
    : "Closing the window keeps Longhand in the Dock and the menu bar.";
}

/// A pane of the Settings window. `open_settings` takes the same values.
export type PaneId = SettingsPane;

function GoTo({ pane, onNavigate, children }: { pane: PaneId; onNavigate: (p: PaneId) => void; children: string }) {
  return <Button onClick={() => onNavigate(pane)}>{children}</Button>;
}

/// One outstanding item of the setup list (R3, R4): what is missing, and
/// one button that fixes it in place or opens its pane.
function SetupRow({
  item,
  readiness,
  settings,
  onRefresh,
  onNavigate,
}: {
  item: SetupItem;
  readiness: Readiness;
  settings: Settings;
  onRefresh: () => Promise<void>;
  onNavigate: (p: PaneId) => void;
}) {
  switch (item.id) {
    case "microphone":
      return (
        <Row
          label="Microphone access"
          detail={<StatusText tone="caution">Longhand needs permission to hear the room</StatusText>}
        >
          <MicrophoneAllowButton onRefresh={onRefresh} />
        </Row>
      );
    case "shortcut":
      return (
        <Row
          label="Recording shortcut"
          detail={<StatusText tone="negative">{readiness.shortcut.error ?? "Not registered"}</StatusText>}
        >
          <GoTo pane="recording" onNavigate={onNavigate}>
            Fix…
          </GoTo>
        </Row>
      );
    case "storage": {
      const detail =
        readiness.destination.kind !== "ready"
          ? describeDestination(readiness.destination)
          : { tone: "caution" as const, text: readOnlyReason(settings) ?? "" };
      return (
        <Row label="Storage" detail={<StatusText tone={detail.tone}>{detail.text}</StatusText>}>
          <GoTo pane="destination" onNavigate={onNavigate}>
            Fix…
          </GoTo>
        </Row>
      );
    }
    case "system_audio":
      return (
        <Row label="Computer audio access" detail={systemAudioDetail(readiness.system_audio)}>
          <SystemAudioButton permission={readiness.system_audio} onRefresh={onRefresh} />
        </Row>
      );
    case "models":
      return (
        <Row label="Models" detail={`${readiness.missing_models.length} still to download`}>
          <GoTo pane="recognition" onNavigate={onNavigate}>
            Download…
          </GoTo>
        </Row>
      );
    case "provider":
      return (
        <Row label="AI provider" detail="Not set up yet. Notes are saved without a summary.">
          <GoTo pane="provider" onNavigate={onNavigate}>
            Set Up…
          </GoTo>
        </Row>
      );
  }
}

/// The setup group (R2, R3): one readiness row, then only the items that
/// are still outstanding.
function SetupGroup({
  readiness,
  settings,
  onRefresh,
  onNavigate,
}: {
  readiness: Readiness;
  settings: Settings;
  onRefresh: () => Promise<void>;
  onNavigate: (p: PaneId) => void;
}) {
  const items = outstandingItems(readiness, settings, { systemAudio: features.systemAudio });
  const detail = !readiness.armed
    ? "Set up the items below to record"
    : items.length > 0
      ? "The items below do not stop a recording"
      : undefined;
  return (
    <Group
      header="Setup"
      footer={items.length > 0 ? "A note recorded before these are ready is processed once they are." : undefined}
    >
      <Row
        label={
          <StatusText tone={readiness.armed ? "positive" : "caution"}>
            {readiness.armed ? "Ready to record" : "Not ready to record"}
          </StatusText>
        }
        detail={detail}
      />
      {items.map((item) => (
        <SetupRow
          key={item.id}
          item={item}
          readiness={readiness}
          settings={settings}
          onRefresh={onRefresh}
          onNavigate={onNavigate}
        />
      ))}
    </Group>
  );
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
  // Readiness holds the recording shortcut; only the show shortcut is read here.
  const [showWindow, setShowWindow] = useState<ShortcutStatus>({ chord: null, registered: false, error: null });
  useEffect(() => {
    void commands.shortcutStatus().then((s) => setShowWindow(s.show_window));
  }, []);

  // Only the Updates group, which shows on macOS only, uses it.
  const readOnly = isMac() ? readOnlyReason(settings) : null;

  return (
    <Pane title="General">
      <SetupGroup readiness={readiness} settings={settings} onRefresh={onRefresh} onNavigate={onNavigate} />

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
          onChanged={(next) => {
            setShowWindow(next.show_window);
            // A capture pauses both shortcuts, and the resume can change whether
            // the recording shortcut is registered (R13 of the shortcut work).
            void onRefresh();
          }}
        />
      </Group>

      {isMac() && (
        <Group
          header="Updates"
          footer={
            readOnly
              ? `${readOnly} This setting can’t be changed now.`
              : "Longhand asks before it installs a new version."
          }
        >
          <AutomaticUpdates settings={settings} onSettings={onSettings} />
        </Group>
      )}
    </Pane>
  );
}

/// A switch for one Boolean setting. A failed change keeps the previous
/// value.
export function SettingSwitch({
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
