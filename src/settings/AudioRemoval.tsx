// Settings > Storage: when the audio of a saved note goes (R3, R17, R18).
// A change that removes audio at once asks first: the pane asks Rust how
// many notes the change removes now, and shows a confirmation only when
// that count is not zero. Rust starts the removal after the save (KTD6).

import { useState } from "react";
import { type AudioRemovalSetting, commands, type Settings } from "../bindings";
import { readOnlyReason } from "./shared";
import { CommitField, ErrorText, Group, Row, Switch, notes, useConfirm } from "./ui";

export const MIN_DAYS = 1;
export const MAX_DAYS = 3650;
/// The delay that the switch uses when the pane knows no other delay.
const DEFAULT_DAYS = 30;

/// The whole number of days in `text`, or null when it is not a whole
/// number from MIN_DAYS to MAX_DAYS.
export function parseDays(text: string): number | null {
  const trimmed = text.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const days = Number(trimmed);
  return days >= MIN_DAYS && days <= MAX_DAYS ? days : null;
}

export function AudioRemoval({ settings, onSettings }: { settings: Settings; onSettings: (s: Settings) => void }) {
  // A settings view with no value shows the default.
  const removal: AudioRemovalSetting = settings.audio_removal ?? { kind: "after_days", days: DEFAULT_DAYS };
  const on = removal.kind === "after_days";
  const [lastDays, setLastDays] = useState(on ? removal.days : DEFAULT_DAYS);
  const days = on ? removal.days : lastDays;
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // A new key shows the saved value in the field again after a refused or
  // cancelled change.
  const [fieldKey, setFieldKey] = useState(0);
  const { ask, dialog } = useConfirm();
  const readOnly = readOnlyReason(settings);
  const disabled = readOnly !== null || busy;

  const change = async (next: AudioRemovalSetting) => {
    setBusy(true);
    try {
      const count = await commands.audioRemovalCount(next);
      if (count.status === "error") {
        setError(count.error);
        return;
      }
      if (count.data > 0) {
        const yes = await ask({
          title: `Remove the audio of ${notes(count.data)} now?`,
          message:
            "The transcript, summary and notes stay. You can’t undo this. You can’t transcribe these notes again or resume their recordings.",
          action: "Remove Audio",
        });
        if (!yes) return;
      }
      const saved = await commands.setAudioRemoval(next);
      if (saved.status === "error") {
        setError(saved.error);
        return;
      }
      setError(null);
      if (next.kind === "after_days") setLastDays(next.days);
      else if (on) setLastDays(removal.days);
      onSettings(saved.data);
    } finally {
      setFieldKey((k) => k + 1);
      setBusy(false);
    }
  };

  const commitDays = (text: string) => {
    const parsed = parseDays(text);
    if (parsed === null) {
      setError(`Enter a number of days from ${MIN_DAYS} to ${MAX_DAYS}.`);
      setFieldKey((k) => k + 1);
      return;
    }
    if (parsed === days) {
      setError(null);
      setFieldKey((k) => k + 1);
      return;
    }
    void change({ kind: "after_days", days: parsed });
  };

  return (
    <Group
      header="Audio"
      footer={
        readOnly
          ? `${readOnly} This setting can’t be changed now.`
          : "This setting applies on every computer that uses this storage folder."
      }
    >
      <Row label="Remove old audio" detail="The transcript, summary and notes stay. A note without audio can’t be transcribed again or resumed.">
        <Switch
          label="Remove old audio"
          checked={on}
          disabled={disabled}
          onChange={(next) => void change(next ? { kind: "after_days", days } : { kind: "off" })}
        />
      </Row>
      {on && (
        <Row label="Remove audio after">
          <CommitField
            key={fieldKey}
            label="Days before the audio is removed"
            value={String(days)}
            inputMode="numeric"
            width="w-16 text-right tabular-nums"
            disabled={disabled}
            onCommit={commitDays}
          />
          <span className="text-label-secondary">{days === 1 ? "day" : "days"}</span>
        </Row>
      )}
      {error && <ErrorText>{error}</ErrorText>}
      {dialog}
    </Group>
  );
}
