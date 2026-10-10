// The recording widget: a small pill at the bottom of the screen while a
// note is open (R6–R10). It reminds the user that a note records or is
// paused when the main window is not in front.
// - Recording: a red dot, the level meter, the time, Pause and Stop.
// - Paused: "Paused", the time, Continue and Stop.
// - Interrupted: "Interrupted", the time and Stop.
// A click on the pill outside its controls shows the note (R10). A drag on
// the pill outside its controls moves the window. The × hides
// the pill until the note stops (R12); it shows while the pointer is on the
// pill. On macOS the panel is never the key window, so WebKit updates CSS
// hover late. There the widget thread sends `WidgetPointer` from the panel's
// tracking area, and the page sets the hover of each control itself.
//
// The window is built once and hidden, and the page stays loaded. The page
// reads the status again each time the pill shows (`WidgetShown`), after
// each control, and when the recording changes (KTD10). Nothing in the pill
// moves except the meter, so the pill draws little attention from the
// participants.

import { type MouseEvent, type PointerEvent, type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { commands, events, type RecordingStatus } from "../bindings";
import { LevelBars } from "../session/LevelBars";
import { formatPosition } from "../lib/timeline";

type Mode = "recording" | "paused" | "interrupted";

function modeOf(status: RecordingStatus | null): Mode | null {
  if (!status?.session_id) return null;
  if (status.interruption) return "interrupted";
  if (status.paused) return "paused";
  return status.recording ? "recording" : null;
}

/// The meter shows the latest peaks; four bars need four ticks.
const LEVELS = 4;

const isMac = navigator.userAgent.includes("Mac");

/// The controls of the pill. `data-control` on each button holds its name,
/// so the hover does not depend on the label text.
type Control = "pause" | "continue" | "stop" | "hide";

/// The control at a point of the page, or null.
function controlAt(x: number, y: number): Control | null {
  const control = document.elementFromPoint(x, y)?.closest("button")?.dataset.control;
  return (control as Control | undefined) ?? null;
}

/// A press that moves this far, in page pixels, drags the pill. A shorter
/// press is a click, which shows the note.
const DRAG_DISTANCE = 4;

/// The hover classes: from the pointer events on macOS, from CSS elsewhere.
const hoverFill = isMac ? "data-[hover=true]:bg-fill" : "hover:bg-fill";

export function RecordingWidget() {
  const [status, setStatus] = useState<RecordingStatus | null>(null);
  const [recordedMs, setRecordedMs] = useState(0);
  const [levels, setLevels] = useState<number[]>([]);
  const [inside, setInside] = useState(false);
  const [hovered, setHovered] = useState<Control | null>(null);
  // The press on the pill body: where it started, and whether it dragged.
  const press = useRef<{ x: number; y: number; dragged: boolean } | null>(null);

  // Reads of the status can overlap (an event and a control's own read). Only
  // the newest read sets the state, so an older status never replaces it.
  const reads = useRef(0);
  const refresh = useCallback(async () => {
    const read = ++reads.current;
    const result = await commands.recordingStatus();
    if (read !== reads.current || result.status !== "ok") return;
    setStatus(result.data);
    // While a segment records, the status does not hold that segment, so
    // the ticks give the time. In the other modes no tick arrives.
    if (result.data.paused || result.data.interruption || !result.data.recording) {
      setRecordedMs(result.data.recorded_ms);
      setLevels([]);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const unlisteners = [
      events.widgetShown.listen(() => {
        setInside(false);
        setHovered(null);
        void refresh();
      }),
      events.widgetPointer.listen(({ payload }) => {
        setInside(payload.inside);
        setHovered(payload.inside ? controlAt(payload.x, payload.y) : null);
      }),
      events.captureTick.listen(({ payload }) => {
        setRecordedMs(payload.recorded_ms);
        setLevels((l) => [...l.slice(-(LEVELS - 1)), payload.peak]);
      }),
      events.recordingChanged.listen(() => void refresh()),
      events.captureEnded.listen(() => void refresh()),
    ];
    return () => {
      for (const u of unlisteners) void u.then((f) => f());
    };
  }, [refresh]);

  const mode = modeOf(status);
  if (!mode) return null;

  // A control that races a Stop from another place fails because the note
  // already stopped. The refetch shows the true mode, so no error shows.
  const control = (run: () => Promise<unknown>) => async (event: MouseEvent) => {
    event.stopPropagation();
    await run();
    await refresh();
  };

  // A press on the pill body that moves drags the window (the user moves
  // the pill out of the way). The controls do not start a drag.
  const pressStart = (event: PointerEvent) => {
    if (event.button !== 0 || (event.target as Element).closest("button")) return;
    press.current = { x: event.clientX, y: event.clientY, dragged: false };
  };
  const pressMove = (event: PointerEvent) => {
    const p = press.current;
    if (!p || p.dragged || (event.buttons & 1) === 0) return;
    if (Math.hypot(event.clientX - p.x, event.clientY - p.y) < DRAG_DISTANCE) return;
    p.dragged = true;
    void getCurrentWindow().startDragging();
  };
  const click = () => {
    const dragged = press.current?.dragged ?? false;
    press.current = null;
    if (!dragged) void commands.widgetOpenNote();
  };

  return (
    <div className="fixed inset-0 flex items-end justify-center pb-1 select-none">
      <div
        onPointerDown={pressStart}
        onPointerMove={pressMove}
        onClick={click}
        className="group flex h-12 cursor-default items-center gap-2.5 rounded-full border border-control-border bg-group pr-2 pl-4 text-label shadow-[0_1px_3px_rgb(0_0_0/0.12)]"
      >
        {mode === "recording" ? (
          <>
            <span data-dot aria-hidden className="size-2 shrink-0 rounded-full bg-negative" />
            <LevelBars levels={levels} input="capturing" large />
          </>
        ) : (
          <span className="text-[14px] font-medium text-caution">{mode === "paused" ? "Paused" : "Interrupted"}</span>
        )}
        <span className="text-[14px] text-label-secondary tabular-nums">{formatPosition(recordedMs)}</span>
        <span className="flex items-center gap-[2px]">
          {mode === "recording" && (
            <IconButton control="pause" label="Pause" hovered={hovered} onClick={control(() => commands.pauseRecording())}>
              <PauseIcon />
            </IconButton>
          )}
          {mode === "paused" && (
            <IconButton control="continue" label="Continue" hovered={hovered} onClick={control(() => commands.continueRecording())}>
              <ContinueIcon />
            </IconButton>
          )}
          <IconButton control="stop" label="Stop" hovered={hovered} onClick={control(() => commands.stopRecording())}>
            <span aria-hidden className="size-[11px] rounded-[2px] bg-current" />
          </IconButton>
          <button
            type="button"
            aria-label="Hide"
            title="Hide until the note stops"
            data-visible={inside}
            data-control="hide"
            data-hover={hovered === "hide"}
            onClick={(event) => {
              event.stopPropagation();
              if (status?.session_id) void commands.widgetHide(status.session_id);
            }}
            className={`ml-[2px] flex size-6 items-center justify-center rounded-full bg-fill text-label-secondary opacity-0 data-[visible=true]:opacity-100 ${
              isMac
                ? "data-[hover=true]:bg-fill-pressed data-[hover=true]:text-label"
                : "group-hover:opacity-100 hover:bg-fill-pressed hover:text-label"
            }`}
          >
            <svg aria-hidden viewBox="0 0 10 10" className="size-[10px]" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round">
              <path d="M2 2l6 6M8 2l-6 6" />
            </svg>
          </button>
        </span>
      </div>
    </div>
  );
}

function IconButton({
  control,
  label,
  hovered,
  onClick,
  children,
}: {
  control: Control;
  label: string;
  hovered: Control | null;
  onClick: (event: MouseEvent) => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      data-control={control}
      data-hover={hovered === control}
      onClick={onClick}
      className={`flex size-[34px] items-center justify-center rounded-full text-label active:bg-fill-pressed ${hoverFill}`}
    >
      {children}
    </button>
  );
}

function PauseIcon() {
  return (
    <svg aria-hidden viewBox="0 0 10 10" className="size-3" fill="currentColor">
      <rect x="1.5" y="1" width="2.5" height="8" rx="0.6" />
      <rect x="6" y="1" width="2.5" height="8" rx="0.6" />
    </svg>
  );
}

function ContinueIcon() {
  return (
    <svg aria-hidden viewBox="0 0 10 10" className="size-3" fill="currentColor">
      <path d="M2.5 1.3v7.4a.5.5 0 0 0 .76.43l6-3.7a.5.5 0 0 0 0-.86l-6-3.7a.5.5 0 0 0-.76.43z" />
    </svg>
  );
}
