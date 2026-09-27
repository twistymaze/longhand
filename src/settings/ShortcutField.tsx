// A settings row whose control is set by pressing keys (R1, R11, KTD2).
// After anarlog's dictation-shortcut control: the shortcuts are paused
// before any key is accepted, the first complete chord is sent to the
// backend, and the row keeps its capture state until the backend answers,
// so a refused chord never looks accepted. Escape cancels; Tab or a click
// away ends capture with no change. See PROVENANCE.md.

import { type KeyboardEvent, type ReactNode, useEffect, useRef, useState } from "react";
import { commands, type ShortcutKind, type ShortcutStatus, type ShortcutStatuses } from "../bindings";
import { formatChord } from "../lib/timeline";
import { chordFromKey } from "./chord";
import { Button, Row } from "./ui";

const isMac = navigator.userAgent.includes("Mac");

type Mode = "rest" | "preparing" | "capturing" | "pending";

// Both rows share one pause. The first row that captures pauses the
// shortcuts and the last row that finishes resumes them, so one row's finish
// never resumes them while the other row captures. The calls go through one
// chain, so a resume never overtakes the pause it follows.
let holders = 0;
let calls: Promise<unknown> = Promise.resolve();

function holdPause(): Promise<unknown> {
  holders += 1;
  if (holders === 1) calls = calls.then(() => commands.suspendShortcuts());
  return calls;
}

function releasePause(): Promise<unknown> {
  holders -= 1;
  if (holders === 0) calls = calls.then(() => commands.resumeShortcuts());
  return calls;
}

// Only one row waits for keys at a time. A row that starts to wait ends the
// wait of the other row.
let waiting: { current: () => void } | null = null;

export function ShortcutField({
  kind,
  label,
  status,
  detail,
  resetTo,
  onChanged,
}: {
  kind: ShortcutKind;
  label: string;
  status: ShortcutStatus;
  /// What the row says at rest, from the status.
  detail: ReactNode;
  /// The default chord, when the shortcut has one to reset to.
  resetTo?: string;
  onChanged: (statuses: ShortcutStatuses) => void;
}) {
  const [mode, setMode] = useState<Mode>("rest");
  const [refusal, setRefusal] = useState<string | null>(null);
  // Each capture has a number, so a pause that returns after the capture
  // ended does not start capturing again.
  const attempt = useRef(0);
  // True while this row holds the shared pause.
  const holding = useRef(false);
  // A row that closes during capture gives its hold back.
  useEffect(
    () => () => {
      if (waiting === endWait) waiting = null;
      if (holding.current) {
        holding.current = false;
        void releasePause();
      }
    },
    [],
  );

  /// Send the chord. Returns the statuses when the backend accepts it.
  const save = async (chord: string | null) => {
    setMode("pending");
    const result = await commands.setShortcut(kind, chord);
    if (result.status === "ok") {
      setRefusal(null);
      return result.data;
    }
    setRefusal(result.error);
    return null;
  };

  const button = useRef<HTMLButtonElement>(null);
  // Ends this row's wait; the other row calls it when it starts to wait.
  const endWait = useRef<() => void>(() => {});

  const stopWaiting = () => {
    if (waiting === endWait) waiting = null;
  };

  const finish = async () => {
    stopWaiting();
    attempt.current += 1;
    setMode("rest");
    if (!holding.current) return;
    holding.current = false;
    await releasePause();
    // Read the statuses after the resume, so the pane does not show the
    // paused shortcuts as not registered.
    const statuses = await commands.shortcutStatus();
    onChanged(statuses);
  };

  const start = async () => {
    if (mode !== "rest") return;
    const mine = ++attempt.current;
    setRefusal(null);
    setMode("preparing");
    holding.current = true;
    const paused = holdPause();
    // Take the pause before the other row ends its wait, so its finish does
    // not resume the shortcuts.
    const other = waiting;
    waiting = endWait;
    other?.current();
    // WebKit does not focus a button on a click, and the keys go to the
    // focused element.
    button.current?.focus();
    await paused;
    if (attempt.current !== mine) return; // The capture ended during the pause.
    setMode("capturing");
  };
  endWait.current = () => void finish();

  const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (mode !== "capturing") return;
    e.stopPropagation();
    const result = chordFromKey(e.nativeEvent);
    if (result.kind === "end") {
      void finish();
      return;
    }
    e.preventDefault();
    if (result.kind === "cancel") void finish();
    if (result.kind === "chord") {
      // The row no longer waits for keys while the backend answers.
      stopWaiting();
      void (async () => {
        await save(result.chord);
        await finish();
      })();
    }
  };

  const onBlur = () => {
    if (mode === "capturing" || mode === "preparing") void finish();
  };

  const act = async (chord: string | null) => {
    const statuses = await save(chord);
    setMode("rest");
    if (statuses) onChanged(statuses);
  };

  const listening = mode !== "rest";
  const shown = listening ? (
    "Press the new shortcut"
  ) : refusal ? (
    <span className="text-negative">{refusal}</span>
  ) : (
    detail
  );

  return (
    <Row label={label} detail={<span aria-live="polite">{shown}</span>}>
      <button
        ref={button}
        data-shortcut-capture={listening ? "true" : undefined}
        type="button"
        aria-label={status.chord ? `${label}: ${formatChord(status.chord, isMac)}. Change` : `Set ${label}`}
        className={`min-w-28 rounded-md border px-2.5 py-[3px] text-center font-medium tracking-[0.04em] select-none outline-offset-0 focus-visible:outline-3 focus-visible:outline-accent/50 ${
          listening
            ? "border-accent bg-control text-label-secondary outline-3 outline-accent/50"
            : "border-control-border bg-control text-label active:bg-fill-pressed"
        }`}
        onClick={() => void start()}
        onKeyDown={onKeyDown}
        onBlur={onBlur}
      >
        {listening ? "Press keys…" : status.chord ? formatChord(status.chord, isMac) : "Set Shortcut"}
      </button>
      {resetTo && status.chord !== resetTo && (
        <Button disabled={listening} onClick={() => void act(resetTo)}>
          Reset
        </Button>
      )}
      {status.chord && (
        <Button disabled={listening} onClick={() => void act(null)}>
          Clear
        </Button>
      )}
    </Row>
  );
}
