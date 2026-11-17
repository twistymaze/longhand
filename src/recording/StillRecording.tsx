// The "Still recording?" prompt of the note that records (R5, R6, R7, R9).
// The recording widget and the main window show it. Both read it from the
// recording status and from the `RecordingPrompt` event, and both answer
// with the generation of the prompt, so an answer to a prompt that closed
// does nothing. The prompt plays no sound, takes no focus and has no
// default button: it must not pull the attention of the people at the
// table to the screen (U7 of the walking skeleton). The countdown is text
// that changes once a second, with no motion.

import { useCallback, useEffect, useRef, useState } from "react";
import { commands, events, type Prompt } from "../bindings";
import { count } from "../processing/describe";
import { Button } from "../settings/ui";

/// The time to `deadlineMs` as `m:ss`, rounded up to the next whole
/// second. It stays at 0:00 after the deadline, until the prompt closes.
export function formatCountdown(deadlineMs: number, nowMs: number): string {
  const seconds = Math.max(0, Math.ceil((deadlineMs - nowMs) / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/// Each check in the prompt, as one sentence for each check.
export function promptReason(prompt: Prompt): string {
  const parts: string[] = [];
  if (prompt.silence_check_minutes != null) parts.push(`No speech for ${count(prompt.silence_check_minutes, "minute")}.`);
  if (prompt.time_limit_hours != null) parts.push(`Recording for ${count(prompt.time_limit_hours, "hour")}.`);
  return parts.join(" ");
}

/// The countdown text to `deadlineMs`. It changes once a second, at the
/// moment that a whole second passes, from the UTC clock. A monotonic
/// clock is not used, because the deadline is UTC wall-clock time.
export function useCountdown(deadlineMs: number): string {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    let timer: number | undefined;
    const plan = (at: number) => {
      const left = deadlineMs - at;
      if (left <= 0) return;
      timer = window.setTimeout(tick, left % 1000 || 1000);
    };
    const tick = () => {
      const at = Date.now();
      setNow(at);
      plan(at);
    };
    plan(Date.now());
    return () => window.clearTimeout(timer);
  }, [deadlineMs]);
  return formatCountdown(deadlineMs, now);
}

export type PromptState = {
  /// The open prompt, or null.
  prompt: Prompt | null;
  /// Call before a read of the recording status. Give the returned
  /// function the `prompt` of that status. A prompt event that arrives
  /// during the read is newer than the status, so the status then changes
  /// nothing.
  read: () => (prompt: Prompt | null) => void;
};

/// The open prompt, from the recording status and the `RecordingPrompt`
/// event (R9). An event of an older generation than the newest known
/// prompt changes nothing, and a closed generation does not open again.
/// `CaptureEnded` closes the prompt.
export function usePrompt(): PromptState {
  const [prompt, setPrompt] = useState<Prompt | null>(null);
  /// The newest generation that this window knows.
  const generation = useRef(0);
  /// The newest generation that closed.
  const closed = useRef(0);
  /// The number of prompt changes from events.
  const changes = useRef(0);

  const read = useCallback(() => {
    const at = changes.current;
    return (next: Prompt | null) => {
      if (changes.current !== at) return;
      if (next && (next.generation < generation.current || next.generation <= closed.current)) return;
      if (next) generation.current = next.generation;
      setPrompt(next);
    };
  }, []);

  useEffect(() => {
    const unlisteners = [
      events.recordingPrompt.listen(({ payload }) => {
        if (payload.generation < generation.current) return;
        if (payload.prompt && payload.generation <= closed.current) return;
        generation.current = payload.generation;
        if (!payload.prompt) closed.current = Math.max(closed.current, payload.generation);
        changes.current += 1;
        setPrompt(payload.prompt);
      }),
      events.captureEnded.listen(() => {
        // The note stopped, so its prompt is closed, also for a status
        // read that started before the end.
        closed.current = generation.current;
        changes.current += 1;
        setPrompt(null);
      }),
    ];
    return () => {
      for (const u of unlisteners) void u.then((f) => f());
    };
  }, []);

  return { prompt, read };
}

/// Answer the prompt `generation`: Keep Recording or Stop. Returns the
/// error text, or null. A prompt that closed before the answer gives no
/// error: the answer does nothing.
export async function answerPrompt(answer: "keep" | "stop", generation: number): Promise<string | null> {
  const result = answer === "keep" ? await commands.keepRecording(generation) : await commands.stopFromPrompt(generation);
  return result.status === "error" && result.error ? result.error : null;
}

/// The prompt in the main window (KTD12): a row above Home and the note
/// page, in the shape of a warning of the recording bar. The live region
/// holds the question, the checks and the countdown length. The `m:ss`
/// text is outside it, so a screen reader does not read it each second.
export function StillRecordingBar({ prompt, onAnswered }: { prompt: Prompt; onAnswered?: () => void }) {
  const countdown = useCountdown(prompt.deadline_ms);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const answer = async (choice: "keep" | "stop") => {
    setBusy(true);
    try {
      setError(await answerPrompt(choice, prompt.generation));
    } finally {
      setBusy(false);
    }
    onAnswered?.();
  };

  return (
    <section aria-label="Still recording?" className="shrink-0 border-b border-separator bg-group select-none">
      <div className="flex items-center gap-2.5 px-4 py-2">
        <div role="status" className="flex min-w-0 flex-1 items-baseline gap-2">
          <span className="shrink-0 font-medium text-caution">Still recording?</span>
          <span className="min-w-0 truncate text-label-secondary">{promptReason(prompt)}</span>
          <span className="sr-only">The note stops in {count(prompt.countdown_minutes, "minute")}.</span>
        </div>
        <span aria-hidden="true" className="shrink-0 text-label-secondary">
          Stops in <span className="tabular-nums">{countdown}</span>
        </span>
        <Button disabled={busy} onClick={() => void answer("keep")}>
          Keep Recording
        </Button>
        <Button disabled={busy} onClick={() => void answer("stop")}>
          Stop
        </Button>
      </div>
      {error && <p className="px-4 pb-2 text-[0.85rem] text-negative">{error}</p>}
    </section>
  );
}
