// The Recording pane (R1, R6, KTD15): the usual microphone and the app list.
// A new note starts with both. The pane also holds the recording and Pause
// shortcuts, the two permissions and the alert sound (R6–R9, R16).

import { type KeyboardEvent, useCallback, useEffect, useRef, useState } from "react";
import {
  DEFAULT_SHORTCUT,
  commands,
  type MicrophoneChoice,
  type MicrophoneList,
  type Readiness,
  type RunningApp,
  type Settings,
  type ShortcutStatus,
} from "../bindings";
import { sentence } from "../processing/describe";
import { Menu, type MenuEntry } from "../session/SessionMenu";
import { CheckIcon, ChevronIcon } from "./icons";
import { isMac, isWindows, MicrophoneAccessRow, SystemAudioAccessRow } from "./Permissions";
import { ShortcutField } from "./ShortcutField";
import { readOnlyReason } from "./shared";
import { SettingSwitch } from "./Status";
import { Button, ErrorText, Group, Pane, Row, Status as StatusText } from "./ui";

/// One row of the Microphone group. `choice` is null for "System default".
export type MicrophoneOption = {
  key: string;
  label: string;
  detail: string | null;
  choice: MicrophoneChoice | null;
};

/// The rows of the Microphone group: "System default" first, then the
/// connected inputs in the order of the list. Inputs with the same name get a
/// number after the name, in the order of the device list. A chosen input
/// that is not connected is added after the inputs as "Not connected".
/// `selected` is the key of the row that holds the choice.
export function microphoneOptions(
  list: MicrophoneList,
  chosen: MicrophoneChoice | null,
): { options: MicrophoneOption[]; selected: string } {
  const count = new Map<string, number>();
  for (const input of list.inputs) count.set(input.name, (count.get(input.name) ?? 0) + 1);
  const seen = new Map<string, number>();
  const inputs: MicrophoneOption[] = list.inputs.map((input) => {
    const n = (seen.get(input.name) ?? 0) + 1;
    seen.set(input.name, n);
    return {
      key: `input:${input.id}`,
      label: (count.get(input.name) ?? 0) > 1 ? `${input.name} ${n}` : input.name,
      detail: null,
      // The label is the device name, so the backend can find the input by name.
      choice: { id: input.id, label: input.name },
    };
  });

  const current = list.inputs.find((i) => i.id === list.default_id);
  const options: MicrophoneOption[] = [
    { key: "default", label: "System default", detail: current ? `Currently ${current.name}` : null, choice: null },
    ...inputs,
  ];
  if (!chosen) return { options, selected: "default" };

  // The backend finds the input by its ID first, then by the exact name.
  const match =
    inputs.find((o) => o.choice?.id === chosen.id) ?? inputs.find((o) => o.choice?.label === chosen.label);
  if (match) return { options, selected: match.key };
  const missing = { key: "missing", label: chosen.label, detail: "Not connected", choice: chosen };
  return { options: [...options, missing], selected: missing.key };
}

type PaneProps = {
  settings: Settings;
  onSettings: (s: Settings) => void;
  readiness: Readiness;
  onRefresh: () => Promise<void>;
};

function MicrophoneGroup({ settings, onSettings, readiness, onRefresh }: PaneProps) {
  const [list, setList] = useState<MicrophoneList | null>(null);
  const [error, setError] = useState<string | null>(null);
  const radios = useRef(new Map<string, HTMLButtonElement>());

  const read = useCallback(async () => {
    const result = await commands.listMicrophones();
    if (result.status === "error") setError(sentence(result.error));
    else {
      setError(null);
      setList(result.data);
    }
  }, []);

  // A microphone can be connected or removed while the window is away.
  useEffect(() => {
    void read();
    window.addEventListener("focus", read);
    return () => window.removeEventListener("focus", read);
  }, [read]);

  const { options, selected } = microphoneOptions(list ?? { default_id: null, inputs: [] }, settings.microphone);

  const choose = async (option: MicrophoneOption) => {
    // WKWebView does not focus a clicked button, and the arrow keys go to the
    // focused element.
    radios.current.get(option.key)?.focus();
    if (option.key === selected) return;
    const result = await commands.setMicrophone(option.choice);
    if (result.status === "error") setError(sentence(result.error));
    else {
      setError(null);
      onSettings(result.data);
    }
  };

  // The up and down arrows move the choice, as in a radio group.
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const at = options.findIndex((o) => o.key === selected);
    const next = options[Math.min(options.length - 1, Math.max(0, at + (e.key === "ArrowDown" ? 1 : -1)))];
    void choose(next);
  };

  return (
    <Group
      header="Microphone"
      footer="Each new note records from this microphone. If it is not connected, the note uses the system default."
    >
      <MicrophoneAccessRow authorized={readiness.microphone_authorized} onRefresh={onRefresh} />
      <div role="radiogroup" aria-label="Microphone" onKeyDown={onKey}>
        {options.map((o) => {
          const on = o.key === selected;
          return (
            <div
              key={o.key}
              onClick={() => void choose(o)}
              className="settings-row flex cursor-default items-stretch pl-2 hover:bg-row-hover active:bg-fill"
            >
              <span aria-hidden className="flex w-6 shrink-0 items-center justify-center text-accent">
                {on && <CheckIcon className="size-[14px]" />}
              </span>
              <div className="flex min-h-[2.75rem] min-w-0 flex-1 items-center py-2 pr-4 pl-1">
                <button
                  ref={(el) => {
                    if (el) radios.current.set(o.key, el);
                    else radios.current.delete(o.key);
                  }}
                  type="button"
                  role="radio"
                  aria-label={o.label}
                  aria-checked={on}
                  tabIndex={on ? 0 : -1}
                  className="min-w-0 flex-1 text-left outline-offset-2 focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-accent"
                >
                  <span className="block truncate font-medium">{o.label}</span>
                  {o.detail && (
                    <span className="mt-0.5 block text-[0.85rem] leading-[1.4] text-label-secondary">{o.detail}</span>
                  )}
                </button>
              </div>
            </div>
          );
        })}
      </div>
      {error && <ErrorText>{error}</ErrorText>}
    </Group>
  );
}

/// Why the app list cannot change on Windows before Windows 11 (R17).
export const APP_AUDIO_UNAVAILABLE = "App audio needs Windows 11";

/// The secondary line of the Teams row on Windows (R20, KD12).
export const WINDOWS_TEAMS_NOTE =
  "Windows can’t record Teams by itself, so Longhand records all computer sound while Teams plays.";

/// The app ID of Teams on Windows (`WINDOWS_TEAMS_ID` in the backend).
const WINDOWS_TEAMS_ID = "ms-teams";

/// The running apps, read again at each call of `read`. A reply that
/// arrives after the reply of a newer read is dropped, so an old list
/// does not replace a newer one.
export function useRunningApps(all: boolean) {
  const [apps, setApps] = useState<RunningApp[] | null>(null);
  const reads = useRef(0);
  const read = useCallback(async () => {
    const n = ++reads.current;
    const result = await commands.listRunningApps(all);
    if (n !== reads.current) return;
    if (result.status === "ok") setApps(result.data);
  }, [all]);
  return { apps, read };
}

/// The icon of an app on the list, or a generic icon when the app has no
/// icon that Longhand can read (R5). The icon comes from the app's path,
/// so it shows also when the app does not run.
function ListedAppIcon({ path }: { path: string }) {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    let current = true;
    void commands.appIcon(path).then((result) => {
      if (current) setSrc(result.status === "ok" ? result.data : null);
    });
    return () => {
      current = false;
    };
  }, [path]);
  if (src) return <img src={src} alt="" className="size-6 shrink-0" />;
  return (
    <svg aria-hidden data-generic-icon viewBox="0 0 24 24" className="size-6 shrink-0 text-label-tertiary">
      <rect x="3" y="3" width="18" height="18" rx="5" fill="none" stroke="currentColor" strokeWidth={1.5} />
      <path d="M8 9.5h8M8 14.5h5" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" />
    </svg>
  );
}

/// The app list (R1–R5, R15, R17, R20): the apps whose sound each new note
/// records with the microphone. The running apps are read when the pane
/// opens, when the window gets the focus, after each change, and each time
/// the add menu opens (R2).
function AppsGroup({ settings, onSettings, readiness, onRefresh }: PaneProps) {
  const [available, setAvailable] = useState<boolean | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  const running = useRunningApps(false);
  const { read } = running;
  const listed = settings.apps;

  useEffect(() => {
    void commands.appAudioAvailable().then((result) => setAvailable(result.status === "ok" && result.data));
  }, []);

  // The list of apps to add changes when the app list changes.
  const listedIds = listed.map((a) => a.id).join("\n");
  useEffect(() => {
    void read();
  }, [read, listedIds]);
  useEffect(() => {
    window.addEventListener("focus", read);
    return () => window.removeEventListener("focus", read);
  }, [read]);

  const change = async (action: () => ReturnType<typeof commands.removeApp>) => {
    setBusy(true);
    const result = await action();
    setBusy(false);
    if (result.status === "error") setError(sentence(result.error));
    else {
      setError(null);
      onSettings(result.data);
    }
    // On macOS, the first add starts the permission request.
    await onRefresh();
  };

  const toAdd = running.apps ?? [];
  const unavailable = available === false;
  const addReason = unavailable ? APP_AUDIO_UNAVAILABLE : toAdd.length === 0 ? "No other apps are running" : null;
  const callReason = addReason ?? (toAdd.some((a) => a.call_app) ? null : "No other call apps are running");

  const closeMenu = () => {
    setMenuOpen(false);
    addButton.current?.focus();
  };
  const menuItems: MenuEntry[] =
    toAdd.length > 0
      ? toAdd.map((app) => ({
          key: app.id,
          label: app.name,
          onSelect: () => {
            setMenuOpen(false);
            void change(() => commands.addApp({ id: app.id, name: app.name, path: app.path }));
          },
        }))
      : [{ key: "none", label: "No other apps are running", disabled: true, onSelect: () => {} }];

  return (
    <Group
      header="App Audio"
      footer="Each new note records the sound of these apps with the microphone, such as the other people on a call. A change applies from the next note."
    >
      {/* The order of the steps: access, Add apps, then the list, so the Add
          buttons keep their place when the list grows. The access row shows
          also before the first add, so the user can allow access first. */}
      {isMac() && <SystemAudioAccessRow permission={readiness.system_audio} onRefresh={onRefresh} />}
      <Row label="Add apps" detail={addReason ?? callReason ?? "Choose from the apps that are running now"}>
        <span className="relative inline-flex" title={addReason ?? undefined}>
          <Button
            ref={addButton}
            expanded={menuOpen}
            disabled={busy || addReason !== null}
            onClick={(e) => {
              // WKWebView does not focus a clicked button. The focus must be
              // here, so that Escape in the menu gives it back.
              e.currentTarget.focus();
              if (!menuOpen) void read();
              setMenuOpen((o) => !o);
            }}
          >
            <span className="inline-flex items-center gap-1">
              Add App
              <ChevronIcon className="size-3 rotate-90" />
            </span>
          </Button>
          {menuOpen && (
            <Menu onClose={closeMenu} items={menuItems} floating={addButton} width="w-max min-w-52 max-w-80" />
          )}
        </span>
        <span className="inline-flex" title={callReason ?? undefined}>
          <Button disabled={busy || callReason !== null} onClick={() => void change(() => commands.addCallApps())}>
            Add Call Apps
          </Button>
        </span>
      </Row>
      {listed.map((app) => (
        <div key={app.id} className="settings-row px-4">
          <div className="settings-row-content flex min-h-[2.75rem] items-center gap-3 py-2">
            <ListedAppIcon path={app.path} />
            <div className="min-w-0 flex-1">
              <div className="truncate font-medium">{app.name}</div>
              {isWindows() && app.id === WINDOWS_TEAMS_ID && (
                <div className="mt-0.5 text-[0.85rem] leading-[1.4] text-label-secondary">{WINDOWS_TEAMS_NOTE}</div>
              )}
            </div>
            <Button label={`Remove ${app.name}`} disabled={busy || unavailable} onClick={() => void change(() => commands.removeApp(app.id))}>
              Remove
            </Button>
          </div>
        </div>
      ))}
      {error && <ErrorText>{error}</ErrorText>}
    </Group>
  );
}

/// The detail of the switch of a shared setting: `normal`, or why the setting
/// can't be changed now.
function sharedDetail(settings: Settings, normal: string): string {
  const readOnly = readOnlyReason(settings);
  return readOnly ? `${readOnly} This setting can’t be changed now.` : normal;
}

function NewNote({ settings, onSettings, readiness, onRefresh }: PaneProps) {
  return (
    <Group footer="When off, New Note opens a note without recording. The recording shortcut then records into the open note, or opens a new note if that note can’t record.">
      <SettingSwitch
        label="New Note starts recording"
        detail={sharedDetail(settings, "Start recording as soon as a new note opens")}
        checked={settings.new_note_records}
        set={commands.setNewNoteRecords}
        onSettings={onSettings}
        disabled={readOnlyReason(settings) !== null}
      />
      <RecordingShortcut
        readiness={readiness}
        newNoteRecords={settings.new_note_records}
        onRefresh={onRefresh}
      />
      <OptionalShortcut
        kind="pause"
        label="Pause shortcut"
        does="Pauses and continues a recording from any app"
        onRefresh={onRefresh}
      />
    </Group>
  );
}

/// The recording shortcut (R8). Readiness holds its status, and ready to
/// record depends on it (R13 of the shortcut work), so a change refreshes
/// readiness. With no chord, the row says that New Note still records only
/// when `newNoteRecords` is true.
function RecordingShortcut({
  readiness,
  newNoteRecords,
  onRefresh,
}: {
  readiness: Readiness;
  newNoteRecords: boolean;
  onRefresh: () => Promise<void>;
}) {
  const recording = readiness.shortcut;
  return (
    <ShortcutField
      kind="recording"
      label="Recording shortcut"
      status={recording}
      resetTo={DEFAULT_SHORTCUT}
      detail={
        !recording.chord ? (
          <StatusText tone="neutral">
            {newNoteRecords ? "Off — New Note still starts a recording" : "Off"}
          </StatusText>
        ) : recording.registered ? (
          "Starts and stops a recording from any app"
        ) : (
          <StatusText tone="negative">{recording.error ?? "Not registered"}</StatusText>
        )
      }
      onChanged={() => void onRefresh()}
    />
  );
}

/// A shortcut with no default chord: the Pause shortcut (R16) and the
/// widget shortcut (R13). Readiness holds only the recording shortcut, so
/// this row reads its own status. `does` says what a registered chord does.
function OptionalShortcut({
  kind,
  label,
  does,
  onRefresh,
}: {
  kind: "pause" | "widget";
  label: string;
  does: string;
  onRefresh: () => Promise<void>;
}) {
  const [status, setStatus] = useState<ShortcutStatus>({ chord: null, registered: false, error: null });
  useEffect(() => {
    void commands.shortcutStatus().then((s) => setStatus(s[kind]));
  }, [kind]);
  return (
    <ShortcutField
      kind={kind}
      label={label}
      status={status}
      detail={
        !status.chord ? (
          <StatusText tone="neutral">Off</StatusText>
        ) : status.registered ? (
          does
        ) : (
          <StatusText tone="negative">{status.error ?? "Not registered"}</StatusText>
        )
      }
      onChanged={(next) => {
        setStatus(next[kind]);
        // A change pauses all shortcuts, and the resume can change whether
        // the recording shortcut is registered.
        void onRefresh();
      }}
    />
  );
}

/// The recording widget (R11, R13): the switch applies from the next note,
/// and the shortcut shows or hides the widget for the open note.
function WidgetGroup({ settings, onSettings, onRefresh }: PaneProps) {
  return (
    <Group
      header="Recording Widget"
      footer="A small bar at the bottom of the screen shows the time and the Pause and Stop buttons while a note is recording, paused or interrupted."
    >
      <SettingSwitch
        label="Show the recording widget"
        detail={sharedDetail(settings, "A change applies from the next note")}
        checked={settings.show_recording_widget}
        set={commands.setShowRecordingWidget}
        onSettings={onSettings}
        disabled={readOnlyReason(settings) !== null}
      />
      <OptionalShortcut
        kind="widget"
        label="Widget shortcut"
        does="Shows or hides the widget for the open note"
        onRefresh={onRefresh}
      />
    </Group>
  );
}

/// The failed-recording sound (R9, KTD1, KTD2). The switch applies from the
/// next alert. Test Sound plays the sound also when the switch is off.
function AlertGroup({ settings, onSettings }: { settings: Settings; onSettings: (s: Settings) => void }) {
  const [error, setError] = useState<string | null>(null);

  const play = async () => {
    const result = await commands.playAlertSound();
    setError(result.status === "error" ? result.error : null);
  };

  return (
    <Group header="Alert">
      <SettingSwitch
        label="Play a sound if recording stops"
        detail="Plays when recording stops and cannot restart"
        checked={settings.alert_sound}
        set={commands.setAlertSound}
        onSettings={onSettings}
      />
      <Row label="Alert sound" detail={error ?? undefined}>
        <Button onClick={() => void play()}>Test Sound</Button>
      </Row>
    </Group>
  );
}

export function RecordingPane(props: PaneProps) {
  return (
    <Pane title="Recording">
      <NewNote {...props} />
      <WidgetGroup {...props} />
      <MicrophoneGroup {...props} />
      <AppsGroup {...props} />
      <AlertGroup settings={props.settings} onSettings={props.onSettings} />
    </Pane>
  );
}
