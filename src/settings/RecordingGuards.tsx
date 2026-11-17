// Settings > Recording: the silence check, the time limit, the countdown of
// the "Keep recording?" prompt and the long-silence cut (R10, R11, KTD13).
// The four settings are shared settings, and a change applies to the next
// note. Each check and the cut has a switch and a time; the countdown has
// only a time.

import { useRef, useState } from "react";
import { type CheckTimeSetting, commands, type Result, type Settings } from "../bindings";
import { plural, sentence } from "../processing/describe";
import { readOnlyReason } from "./shared";
import { type CommitHow, CommitField, ErrorText, Group, Row, Switch } from "./ui";

/// The range and unit of one time field.
type Range = { min: number; max: number; unit: "minute" | "hour" };

export const SILENCE_CHECK_RANGE: Range = { min: 5, max: 240, unit: "minute" };
export const TIME_LIMIT_RANGE: Range = { min: 1, max: 24, unit: "hour" };
export const LONG_SILENCE_CUT_RANGE: Range = { min: 5, max: 240, unit: "minute" };
export const COUNTDOWN_RANGE: Range = { min: 1, max: 10, unit: "minute" };

/// The whole number in `text`, or null when it is not a whole number in
/// the range.
export function parseTime(text: string, range: Range): number | null {
  const trimmed = text.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const time = Number(trimmed);
  return time >= range.min && time <= range.max ? time : null;
}

const rangeError = (range: Range) => `Enter a number of ${range.unit}s from ${range.min} to ${range.max}.`;

/// One time field with its unit. It commits through `save`, and shows a
/// range error for a value outside the range. An unattended commit (a
/// window close or an unmount) with a bad value saves nothing and shows
/// nothing.
function TimeRow({
  label,
  fieldLabel,
  detail,
  time,
  range,
  disabled,
  save,
  latestOn = () => true,
}: {
  label: string;
  fieldLabel: string;
  detail?: string;
  time: number;
  range: Range;
  disabled: boolean;
  save: (time: number, how: CommitHow) => Promise<boolean>;
  /// False when the setting of the last render is off. An unattended
  /// commit then does not turn the setting on again. Without it, the
  /// setting is always on.
  latestOn?: () => boolean;
}) {
  const [error, setError] = useState<string | null>(null);
  // A new key shows the saved value in the field again after a refused
  // change.
  const [fieldKey, setFieldKey] = useState(0);

  const commit = async (text: string, how: CommitHow): Promise<boolean> => {
    if (how === "unattended" && !latestOn()) return false;
    const parsed = parseTime(text, range);
    if (parsed === null) {
      if (how === "unattended") return false;
      setError(rangeError(range));
      setFieldKey((k) => k + 1);
      return false;
    }
    if (parsed === time) {
      setError(null);
      setFieldKey((k) => k + 1);
      return true;
    }
    const saved = await save(parsed, how);
    if (saved) setError(null);
    else setFieldKey((k) => k + 1);
    return saved;
  };

  return (
    <>
      <Row label={label} detail={detail}>
        <CommitField
          key={fieldKey}
          label={fieldLabel}
          value={String(time)}
          inputMode="numeric"
          width="w-16 text-right tabular-nums"
          disabled={disabled}
          onCommit={commit}
        />
        <span className="text-label-secondary">{plural(time, range.unit)}</span>
      </Row>
      {error && <ErrorText>{error}</ErrorText>}
    </>
  );
}

/// The save of one setting, with its busy state and its error text. `save`
/// calls `call`. On success, it clears the error, calls `onSaved` and then
/// `onSettings`. On an error, it shows the error only for a user commit.
function useSave(onSettings: (s: Settings) => void) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const save = async (
    call: () => Promise<Result<Settings, string>>,
    how: CommitHow,
    onSaved?: () => void,
  ): Promise<boolean> => {
    setBusy(true);
    try {
      const result = await call();
      if (result.status === "error") {
        if (how === "user") setError(sentence(result.error));
        return false;
      }
      setError(null);
      onSaved?.();
      onSettings(result.data);
      return true;
    } finally {
      setBusy(false);
    }
  };

  return { error, busy, save };
}

/// A check or the cut: a switch, and while it is on, its time. The switch
/// turns the setting on with the last time that the pane knows.
function CheckTime({
  label,
  detail,
  timeLabel,
  fieldLabel,
  setting,
  fallback,
  range,
  readOnly,
  set,
  onSettings,
}: {
  label: string;
  detail: string;
  timeLabel: string;
  fieldLabel: string;
  setting: CheckTimeSetting;
  fallback: number;
  range: Range;
  readOnly: boolean;
  set: (next: CheckTimeSetting) => Promise<Result<Settings, string>>;
  onSettings: (s: Settings) => void;
}) {
  const on = setting.kind === "after";
  const latest = useRef(setting);
  latest.current = setting;
  const [lastTime, setLastTime] = useState(on ? setting.time : fallback);
  const time = on ? setting.time : lastTime;
  const { error, busy, save } = useSave(onSettings);
  const disabled = readOnly || busy;

  const change = (next: CheckTimeSetting, how: CommitHow = "user"): Promise<boolean> =>
    save(() => set(next), how, () => setLastTime(next.kind === "after" ? next.time : time));

  return (
    <>
      <Row label={label} detail={detail}>
        <Switch
          label={label}
          checked={on}
          disabled={disabled}
          onChange={(next) => void change(next ? { kind: "after", time } : { kind: "off" })}
        />
      </Row>
      {error && <ErrorText>{error}</ErrorText>}
      {on && (
        <TimeRow
          label={timeLabel}
          fieldLabel={fieldLabel}
          time={time}
          range={range}
          disabled={disabled}
          save={(t, how) => change({ kind: "after", time: t }, how)}
          latestOn={() => latest.current.kind === "after"}
        />
      )}
    </>
  );
}

/// The countdown of the prompt. It has no off state.
function Countdown({
  settings,
  readOnly,
  onSettings,
}: {
  settings: Settings;
  readOnly: boolean;
  onSettings: (s: Settings) => void;
}) {
  const { error, busy, save } = useSave(onSettings);

  return (
    <>
      <TimeRow
        label="Time to answer"
        fieldLabel="Minutes to answer before the note stops"
        detail="If nobody answers, the note stops"
        time={settings.stop_countdown_minutes}
        range={COUNTDOWN_RANGE}
        disabled={readOnly || busy}
        save={(minutes, how) => save(() => commands.setStopCountdownMinutes(minutes), how)}
      />
      {error && <ErrorText>{error}</ErrorText>}
    </>
  );
}

export function RecordingGuards({ settings, onSettings }: { settings: Settings; onSettings: (s: Settings) => void }) {
  const reason = readOnlyReason(settings);
  const readOnly = reason !== null;
  return (
    <Group
      header="Long Recordings"
      footer={
        reason
          ? `${reason} These settings can’t be changed now.`
          : "These settings apply on every computer that uses this storage folder. A change applies from the next note."
      }
    >
      <CheckTime
        label="Ask after silence"
        detail="Asks “Keep recording?” when nobody speaks"
        timeLabel="Silence before asking"
        fieldLabel="Minutes with no speech before the question"
        setting={settings.silence_check_minutes}
        fallback={30}
        range={SILENCE_CHECK_RANGE}
        readOnly={readOnly}
        set={commands.setSilenceCheckMinutes}
        onSettings={onSettings}
      />
      <CheckTime
        label="Ask after a time limit"
        detail="Asks “Keep recording?” after a long recording"
        timeLabel="Recording time before asking"
        fieldLabel="Hours of recording before the question"
        setting={settings.recording_time_limit_hours}
        fallback={6}
        range={TIME_LIMIT_RANGE}
        readOnly={readOnly}
        set={commands.setRecordingTimeLimitHours}
        onSettings={onSettings}
      />
      <Countdown settings={settings} readOnly={readOnly} onSettings={onSettings} />
      <CheckTime
        label="Remove long silence"
        detail="When a note stops, removes long silent parts of the audio. 1 to 2 minutes stays next to speech."
        timeLabel="Shortest silence to remove"
        fieldLabel="Shortest silence to remove, in minutes"
        setting={settings.long_silence_cut_minutes}
        fallback={10}
        range={LONG_SILENCE_CUT_RANGE}
        readOnly={readOnly}
        set={commands.setLongSilenceCutMinutes}
        onSettings={onSettings}
      />
    </Group>
  );
}
