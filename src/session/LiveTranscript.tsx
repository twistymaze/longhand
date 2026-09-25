// The live transcript on the Transcript tab (R8, R10, U8). Saved parts show
// as paragraphs, and the open part's tentative text shows in a lighter
// style. The text has no speaker labels: processing after Stop adds them.
// The view follows new text while it is at the bottom. A scroll up stops
// the follow and shows "Scroll to bottom".
//
// Nothing here moves or blinks. The tab does not open by default during a
// recording, and the text must not pull the user's attention back to the
// screen (ux-redesign KD2).

import { type RefObject, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useLiveTranscript } from "./useLiveTranscript";

/// Where the note is:
/// - recording: the live worker can add text;
/// - interrupted: the capture stopped before Stop;
/// - processing: after Stop, until the saved transcript is ready;
/// - loading: the note is saved, and the page reads the saved transcript.
export type LiveMode = "recording" | "interrupted" | "processing" | "loading";

/// The view is at the bottom when less than this is below it.
const BOTTOM_SLACK_PX = 32;

export const READY_AFTER_STOP = "The transcript is ready after you stop recording.";

/// One plain line for the state, or null when the text says enough.
function line(mode: LiveMode, state: string | null, hasText: boolean): string | null {
  if (mode === "processing") return "The final transcript, with speakers, shows here when processing finishes.";
  if (mode === "loading") return "Loading the final transcript…";
  if (mode === "interrupted") return hasText ? "Recording interrupted. The text so far stays." : READY_AFTER_STOP;
  if (state === "paused") return "Live transcript paused. The missing part is transcribed after you stop.";
  if (state === "unavailable" || state === "ended") {
    return hasText ? "The live transcript stopped. The rest is transcribed after you stop." : READY_AFTER_STOP;
  }
  if (hasText) return null;
  if (state === "running") return "The transcript shows here as people speak.";
  return "Starting the live transcript…";
}

function DownArrow() {
  return (
    <svg aria-hidden viewBox="0 0 12 12" className="size-[11px]">
      <path d="M6 1.5v8.5M2 6.5L6 10.5l4-4" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function LiveTranscript({
  sessionId,
  mode,
  scroller,
}: {
  sessionId: string;
  mode: LiveMode;
  /// The element that scrolls the tab.
  scroller: RefObject<HTMLElement | null>;
}) {
  const live = useLiveTranscript(sessionId);
  const following = useRef(true);
  const [away, setAway] = useState(false);

  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const onScroll = () => {
      const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < BOTTOM_SLACK_PX;
      following.current = atBottom;
      setAway(!atBottom);
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, [scroller]);

  const toBottom = useCallback(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [scroller]);

  // Before paint, so the new text never shows above the bottom first.
  useLayoutEffect(() => {
    if (following.current) toBottom();
  }, [live, toBottom]);

  if (!live.loaded) return null;

  const parts = live.parts.filter((p) => p.text.trim() !== "");
  const current = live.current && (live.current.committed + live.current.tentative).trim() !== "" ? live.current : null;
  const hasText = parts.length > 0 || current !== null;
  const status = line(mode, live.state, hasText);

  if (!hasText) {
    return (
      <p role="status" className="mx-auto max-w-[24rem] pt-16 text-center leading-relaxed text-label-secondary">
        {status}
      </p>
    );
  }

  const header = mode === "processing" || mode === "loading";
  const statusLine = status && (
    <p role="status" className={`text-[0.85rem] text-label-secondary ${header ? "mb-4" : "mt-4 pl-[4.5rem]"}`}>
      {status}
    </p>
  );

  return (
    <div>
      {header && statusLine}
      {parts.map((p) => (
        <p key={`${p.segment}-${p.part}`} data-testid="live-part" className="py-1.5 pl-[4.5rem] leading-[1.55]">
          {p.text}
        </p>
      ))}
      {current && (
        <p data-testid="live-part" className="py-1.5 pl-[4.5rem] leading-[1.55]">
          <span>{current.committed}</span>
          {current.tentative && (
            <span data-tentative="true" className="text-label-secondary">
              {current.tentative}
            </span>
          )}
        </p>
      )}
      {!header && statusLine}
      {away && (
        // No layout height: the button floats over the text at the bottom
        // of the view.
        <div className="pointer-events-none sticky bottom-4 flex h-0 items-end justify-center">
          <button
            type="button"
            onClick={() => {
              following.current = true;
              setAway(false);
              toBottom();
            }}
            className="pointer-events-auto flex items-center gap-1.5 rounded-full bg-group px-3 py-1 text-[0.85rem] text-label shadow-[0_0_0_0.5px_var(--separator),0_2px_8px_rgb(0_0_0/0.12)] hover:bg-[linear-gradient(var(--row-hover),var(--row-hover))] active:bg-[linear-gradient(var(--fill-pressed),var(--fill-pressed))] outline-offset-1 focus-visible:outline-2 focus-visible:outline-accent dark:shadow-[0_0_0_0.5px_var(--separator),0_2px_8px_rgb(0_0_0/0.4)]"
          >
            <DownArrow />
            Scroll to bottom
          </button>
        </div>
      )}
    </div>
  );
}
