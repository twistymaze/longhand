// The live transcript on the Transcript tab (R8, R10, U8). The row builder
// (`liveRows.ts`) groups the live text into speaker rows and paragraphs,
// with a divider where the speaker labels restart. This view draws the rows
// in the layout of the saved transcript, and shows one notice for
// provisional results. The view follows new text while it is at the bottom.
// A scroll up stops the follow and shows "Scroll to bottom". While the
// reader is scrolled up, a change above keeps the first visible paragraph in
// place (KTD14).
//
// Nothing here moves or blinks. The tab does not open by default during a
// recording, and the text must not pull the user's attention back to the
// screen (ux-redesign KD2, U7).

import { type RefObject, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useLiveTranscript } from "./useLiveTranscript";
import { recognitionRows, speakerBlocks, type RecognitionView } from "./liveRecognition";
import {
  buildLiveRows,
  paragraphText,
  type LiveRowItem,
  type LiveSpeakerRow,
  type NativeRow,
  type NumberMap,
  type OpenPart,
  type SpeakerBlock,
} from "./liveRows";
import { Marker } from "./Marker";

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

const isSpeakerRow = (item: LiveRowItem): item is LiveSpeakerRow => item.kind === "speaker";
const rowText = (row: LiveSpeakerRow) => row.paragraphs.map(paragraphText).join("\n");

/**
 * True when a change moved, split or joined a speaker row that had a
 * heading, or changed its number. New text at the end of a row, and a first
 * heading on text that had none, are not changes of the labels.
 */
function labelsChanged(before: LiveRowItem[], after: LiveRowItem[]): boolean {
  const next = new Map(after.filter(isSpeakerRow).map((row) => [row.key, row]));
  return before.filter(isSpeakerRow).some((row) => {
    if (row.number === null) return false;
    const now = next.get(row.key);
    return !now || now.number !== row.number || !rowText(now).startsWith(rowText(row));
  });
}

/**
 * The numbers of the collapsed speaker list. Each window counts from 1 in
 * order of first speech, and a speaker keeps a number that a heading
 * already showed. The list does not add numbers to the note's map.
 */
function listNumbers(blocks: SpeakerBlock[], numbers: NumberMap): Map<string, number> {
  const out = new Map<string, number>();
  for (const block of blocks) {
    const prefix = `${block.generation}:${block.epoch}:`;
    let next = 1;
    for (const [id, number] of numbers) if (id.startsWith(prefix)) next = Math.max(next, number + 1);
    for (const turn of [...block.turns].sort((a, b) => a.start_sample - b.start_sample)) {
      if (turn.speaker === null || out.has(turn.speaker)) continue;
      out.set(turn.speaker, numbers.get(turn.speaker) ?? next++);
    }
  }
  return out;
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
  textAfterStop = false,
}: {
  sessionId: string;
  mode: LiveMode;
  /// The element that scrolls the tab.
  scroller: RefObject<HTMLElement | null>;
  textAfterStop?: boolean;
}) {
  const live = useLiveTranscript(sessionId);
  const following = useRef(true);
  const [away, setAway] = useState(false);
  const rows = useRef<HTMLDivElement>(null);
  /// The first visible paragraph and its offset from the top of the scroller.
  const anchor = useRef<{ key: string; top: number } | null>(null);

  const afterStop = mode === "processing" || mode === "loading";
  const { blocks, native, items } = useMemo(() => {
    const blocks = speakerBlocks(live.recognition);
    const hasLiveText =
      live.parts.some((p) => p.text.trim() !== "") || live.open.some((p) => (p.committed + p.tentative).trim() !== "");
    // The original live stream stays complete while post-Stop recognition
    // publishes partial replacements. The saved transcript replaces that
    // stream after processing; showing both sources here would repeat the
    // same speech.
    const native: NativeRow[] = hasLiveText
      ? []
      : recognitionRows(live.recognition).map((row) => ({
          generation: row.generation,
          epoch: row.epoch,
          start_sample: row.start_sample,
          end_sample: row.end_sample,
          text: row.text ?? "",
          speaker: row.speaker,
          text_final: row.text_final,
          speaker_final: row.speaker_final,
        }));
    // A saved part with no word times shows its text under the speaker before it.
    const untimed: OpenPart[] = live.parts
      .filter((p) => p.words.length === 0 && p.text.trim() !== "")
      .map((p) => ({ segment: p.segment, part: p.part, offset_ms: p.offset_ms, words: null, committed: p.text, tentative: "" }));
    const items = buildLiveRows({
      parts: live.parts.filter((p) => p.words.length > 0),
      open: [...untimed, ...live.open],
      native,
      speakers: blocks,
      numbers: live.numbers,
      afterStop,
    });
    return { blocks, native, items };
  }, [live, afterStop]);

  // The screen-reader message plays when a speaker revision changes the rows
  // that show (R7: a shown number does not change, so an unchanged row plays
  // nothing).
  const [announced, setAnnounced] = useState(0);
  const shown = useRef<{ sessionId: string; recognition: RecognitionView; items: LiveRowItem[] } | null>(null);
  useEffect(() => {
    const before = shown.current;
    shown.current = { sessionId, recognition: live.recognition, items };
    if (!before || before.sessionId !== sessionId) {
      setAnnounced(0);
      return;
    }
    if (before.recognition !== live.recognition && labelsChanged(before.items, items)) setAnnounced((n) => n + 1);
  }, [sessionId, live.recognition, items]);

  /// Record the first visible paragraph (KTD14).
  const record = useCallback(() => {
    const el = scroller.current;
    const box = rows.current;
    anchor.current = null;
    if (!el || !box) return;
    const top = el.getBoundingClientRect().top;
    for (const p of box.querySelectorAll<HTMLElement>("[data-key]")) {
      const rect = p.getBoundingClientRect();
      if (rect.bottom > top) {
        anchor.current = { key: p.dataset.key!, top: rect.top - top };
        return;
      }
    }
  }, [scroller]);

  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const onScroll = () => {
      const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < BOTTOM_SLACK_PX;
      following.current = atBottom;
      setAway(!atBottom);
      record();
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, [scroller, record]);

  const toBottom = useCallback(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [scroller]);

  // Before paint, so the new text never shows above the bottom first, and a
  // change above the visible area never moves the text that the reader sees.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (following.current) toBottom();
    else if (el && anchor.current && rows.current) {
      const { key, top } = anchor.current;
      const p = rows.current.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"]`);
      if (p) {
        const shift = p.getBoundingClientRect().top - el.getBoundingClientRect().top - top;
        if (shift !== 0) el.scrollTop += shift;
      }
    }
    // The anchor is read only while the reader is scrolled up. The scroll
    // handler records it when the reader leaves the bottom.
    if (!following.current) record();
  }, [live, items, toBottom, record, scroller]);

  if (!live.loaded) return null;

  const turns = blocks.some((b) => b.turns.length > 0);
  const provisionalText = native.some((row) => !row.text_final);
  const provisionalSpeakers = mode !== "loading" && items.some((item) => isSpeakerRow(item) && item.provisional);
  const hasText = items.length > 0 || turns;
  const status = textAfterStop && mode === "recording" ? READY_AFTER_STOP : line(mode, live.state, hasText);

  if (!hasText) {
    return (
      <>
      {live.recognition.error && <p role="alert">{live.recognition.error}</p>}
      <p role="status" className="mx-auto max-w-[24rem] pt-16 text-center leading-relaxed text-label-secondary">
        {status}
      </p>
      </>
    );
  }

  const header = afterStop;
  const statusLine = status && (
    <p role="status" className={`text-[0.85rem] text-label-secondary ${header ? "mb-4" : "mt-4 pl-[4.5rem]"}`}>
      {status}
    </p>
  );
  const numbers = items.length === 0 && turns ? listNumbers([...blocks].sort((a, b) => a.epoch - b.epoch), live.numbers) : null;

  return (
    <div ref={rows}>
      {live.recognition.error && <p role="alert" className="text-negative">{live.recognition.error}</p>}
      {announced > 0 && <p role="status" className="sr-only"><span key={announced}>Speaker labels updated.</span></p>}
      {numbers && (
        <details className="mb-4 text-[0.85rem] text-label-secondary">
          <summary>Speaker analysis so far</summary>
          <p>Text and speaker labels can become ready at different times.</p>
          {[...blocks].sort((a, b) => a.epoch - b.epoch).map((block, i) => (
            <div key={`${block.generation}:${block.epoch}`}>
              {i > 0 && <p>Speaker labels restart</p>}
              {[...block.turns].sort((a, b) => a.start_sample - b.start_sample).map((turn, j) => (
                <p key={j}>
                  Speaker {turn.speaker ? numbers.get(turn.speaker) : "unknown"} · {Math.floor(turn.start_sample / 16000)}–{Math.ceil(turn.end_sample / 16000)} seconds{!turn.speaker_final && " · Provisional label"}
                </p>
              ))}
            </div>
          ))}
        </details>
      )}
      {(provisionalText || provisionalSpeakers) && <p className="mb-3 text-[0.85rem] text-label-secondary">{[provisionalText && "Provisional text", provisionalSpeakers && "Speaker labels may change"].filter(Boolean).join(" · ")}</p>}
      {header && statusLine}
      {items.map((item, i) =>
        item.kind === "divider" ? (
          <Marker key={item.key} label="Speaker labels restart" />
        ) : (
          <section key={item.key} className={i > 0 ? "mt-5" : undefined}>
            {item.number !== null && (
              <h3 className="mb-1 pl-[4.5rem] text-[0.85rem] font-semibold text-label-secondary">Speaker {item.number}</h3>
            )}
            {item.paragraphs.map((paragraph) => (
              <p key={paragraph.key} data-key={paragraph.key} data-testid="live-part" className="py-1 pl-[4.5rem] leading-[1.55]">
                {paragraph.pieces.map((piece, j) =>
                  piece.tentative ? (
                    <span key={j} data-tentative="true" className="text-label-secondary">{piece.text}</span>
                  ) : (
                    <span key={j}>{piece.text}</span>
                  ),
                )}
              </p>
            ))}
          </section>
        ),
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
