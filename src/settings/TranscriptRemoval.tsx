// Settings > Storage: when the transcript of a saved note goes (R3, R4, R5,
// R19, R28). The transcript delay is never shorter than the audio delay, and
// the setting is unavailable while audio removal is off (R4). A change that
// removes transcripts at once, or that skips notes whose summary failed,
// asks first. Rust starts the removal after the save.

import { useRef, useState } from "react";
import { type Settings, type TranscriptCount, type TranscriptRemovalSetting, commands } from "../bindings";
import { AudioRemoval, MAX_DAYS, MIN_DAYS, audioRemovalOf, parseDays } from "./AudioRemoval";
import { readOnlyReason } from "./shared";
import { type CommitHow, CommitField, ErrorText, Group, Row, Switch, notes, useConfirm } from "./ui";

/// The delay that the switch uses when the pane knows no other delay. It
/// is never shorter than the audio delay.
const DEFAULT_DAYS = 90;

/// The audio and transcript settings. A save of one can change the other
/// (R5). The Settings window reads the settings again after a save, so
/// until new props come, the pane shows the two values that the save
/// returned.
export function OldNoteRemoval({ settings, onSettings }: { settings: Settings; onSettings: (s: Settings) => void }) {
  const [own, setOwn] = useState<{ props: Settings; saved: Settings } | null>(null);
  // The props of the last render. A save that ends after new props came
  // applies to those props.
  const props = useRef(settings);
  props.current = settings;
  const shown =
    own && own.props === settings
      ? { ...settings, audio_removal: own.saved.audio_removal, transcript_removal: own.saved.transcript_removal }
      : settings;
  const saved = (s: Settings) => {
    setOwn({ props: props.current, saved: s });
    onSettings(s);
  };
  return (
    <>
      <AudioRemoval settings={shown} onSettings={saved} />
      <TranscriptRemoval settings={shown} onSettings={saved} />
    </>
  );
}

/// The question for a change with this count, or null when the change
/// needs no question.
function question(count: TranscriptCount) {
  if (count.remove === 0 && count.skipped === 0) return null;
  const skipped =
    count.skipped === 0
      ? ""
      : count.skipped === 1
        ? "1 note keeps its transcript because its summary failed."
        : `${notes(count.skipped)} keep their transcript because their summary failed.`;
  if (count.remove === 0) {
    return { title: "Remove old transcripts?", message: skipped, action: "Save" };
  }
  return {
    title: `Remove the transcripts of ${notes(count.remove)} now?`,
    message: [
      "The summary and notes stay. You can’t undo this. These notes can’t use Regenerate Summary, Transcribe Again or Resume.",
      skipped,
    ]
      .filter(Boolean)
      .join(" "),
    action: "Remove Transcripts",
  };
}

export function TranscriptRemoval({ settings, onSettings }: { settings: Settings; onSettings: (s: Settings) => void }) {
  const audio = audioRemovalOf(settings);
  const audioDays = audio.kind === "after_days" ? audio.days : null;
  // A settings view with no value shows off.
  const removal: TranscriptRemovalSetting = settings.transcript_removal ?? { kind: "off" };
  // The setting is off while audio removal is off, and its delay is never
  // shorter than the audio delay (KTD7).
  const on = audioDays !== null && removal.kind === "after_days";
  const minDays = audioDays ?? MIN_DAYS;
  // The setting of the last render.
  const latest = useRef({ on, minDays });
  latest.current = { on, minDays };
  const [lastDays, setLastDays] = useState(removal.kind === "after_days" ? removal.days : DEFAULT_DAYS);
  const days = Math.max(on && removal.kind === "after_days" ? removal.days : lastDays, minDays);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // A new key shows the saved value in the field again after a refused or
  // cancelled change.
  const [fieldKey, setFieldKey] = useState(0);
  const { ask, dialog } = useConfirm();
  const readOnly = readOnlyReason(settings);
  const disabled = readOnly !== null || busy;

  // Resolves to true when the change was saved. An unattended change (a
  // window close or a pane change) asks nothing, so it does not save a
  // change that needs a question.
  const change = async (next: TranscriptRemovalSetting, how: CommitHow = "user"): Promise<boolean> => {
    setBusy(true);
    try {
      if (next.kind === "after_days") {
        const count = await commands.transcriptRemovalCount(next);
        if (count.status === "error") {
          if (how === "user") setError(count.error);
          return false;
        }
        const q = question(count.data);
        if (q) {
          if (how === "unattended") return false;
          if (!(await ask(q))) return false;
        }
      }
      const saved = await commands.setTranscriptRemoval(next);
      if (saved.status === "error") {
        setError(saved.error);
        return false;
      }
      setError(null);
      if (next.kind === "after_days") setLastDays(next.days);
      else if (on) setLastDays(days);
      onSettings(saved.data);
      return true;
    } finally {
      setFieldKey((k) => k + 1);
      setBusy(false);
    }
  };

  const commitDays = async (text: string, how: CommitHow): Promise<boolean> => {
    // An unattended commit runs at the unmount of the field, after a render
    // with a newer setting: for example after the switch turned removal
    // off. Typed days then do not turn removal on again.
    if (how === "unattended" && !latest.current.on) return false;
    const parsed = parseDays(text);
    const refuse = (message: string) => {
      if (how === "user") {
        setError(message);
        setFieldKey((k) => k + 1);
      }
      return false;
    };
    if (parsed === null) return refuse(`Enter a number of days from ${MIN_DAYS} to ${MAX_DAYS}.`);
    if (parsed < latest.current.minDays) {
      return refuse(`Enter at least ${latest.current.minDays} ${latest.current.minDays === 1 ? "day" : "days"}, the audio delay.`);
    }
    if (parsed === days) {
      setError(null);
      setFieldKey((k) => k + 1);
      return true;
    }
    return change({ kind: "after_days", days: parsed }, how);
  };

  return (
    <Group
      header="Transcripts"
      footer={
        readOnly
          ? `${readOnly} This setting can’t be changed now.`
          : "This setting applies on every computer that uses this storage folder."
      }
    >
      <Row
        label="Remove old transcripts"
        detail={
          audioDays === null
            ? "Turn on Remove old audio first."
            : "The summary and notes stay. A note without a transcript can’t use Regenerate Summary, Transcribe Again or Resume."
        }
      >
        <Switch
          label="Remove old transcripts"
          checked={on}
          disabled={disabled || audioDays === null}
          onChange={(next) => void change(next ? { kind: "after_days", days } : { kind: "off" })}
        />
      </Row>
      {on && (
        <Row label="Remove transcripts after">
          <CommitField
            key={fieldKey}
            label="Days before the transcript is removed"
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
