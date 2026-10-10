// Four small bars next to the Transcript tab label while a note records.
// They show that the microphone hears the room: each bar is the peak of
// one of the latest ticks, the newest on the right, and the bars go flat
// on silence. The bars are the only motion on the recording page, and
// they are small, so a glance is enough.
//
// With reduced motion the bars hold one shape. They are bright while
// sound arrives and dim on silence or when no audio arrives.

import { useSyncExternalStore } from "react";
import { levelFraction } from "../lib/timeline";
import type { InputStatus } from "../recording/inputStatus";
import { QUIET_PEAK } from "../recording/inputStatus";

const BARS = 4;

/// The smallest bar, as a fraction of the full height: a flat bar still
/// shows as a dot.
const FLAT = 0.2;

/// The shape the bars hold under reduced motion.
const STILL = [0.45, 1, 0.7, 0.4];

/// Ticks that decide whether sound arrives under reduced motion: 2 s.
const SIGNAL_TICKS = 8;

const reducedMotion = "(prefers-reduced-motion: reduce)";

function subscribe(onChange: () => void) {
  const query = window.matchMedia?.(reducedMotion);
  query?.addEventListener?.("change", onChange);
  return () => query?.removeEventListener?.("change", onChange);
}

export function useReducedMotion(): boolean {
  return useSyncExternalStore(subscribe, () => window.matchMedia?.(reducedMotion).matches ?? false);
}

/// The height of one bar for a tick's peak. The power curve keeps room
/// noise near flat and lets speech reach the top.
export function barScale(peak: number): number {
  const scale = FLAT + (1 - FLAT) * levelFraction(peak) ** 1.5;
  return Math.round(scale * 1000) / 1000;
}

/// `large` draws 3 px bars, 12 px high, for the recording widget. Both sizes
/// use whole pixels, so each bar has the same width on the screen.
export function LevelBars({ levels, input, large = false }: { levels: number[]; input: InputStatus; large?: boolean }) {
  const reduce = useReducedMotion();
  const live = input === "capturing" || input === "quiet";
  const recent = levels.slice(-BARS);
  const peaks = Array.from({ length: BARS - recent.length }, () => 0).concat(recent);
  const signal = live && levels.slice(-SIGNAL_TICKS).some((p) => p >= QUIET_PEAK);
  const scales = reduce ? STILL : peaks.map((p) => (live ? barScale(p) : FLAT));
  const tone = input === "stalled" || input === "reconnecting" || input === "lost" ? "text-caution" : "text-negative";

  return (
    <span
      aria-hidden
      data-meter
      data-signal={signal}
      className={`inline-flex items-center gap-[2px] ${large ? "h-[12px]" : "h-[10px]"} ${tone} ${
        reduce && !signal ? "opacity-40" : ""
      }`}
    >
      {scales.map((scale, i) => (
        <span
          key={i}
          data-bar
          className={`h-full ${large ? "w-[3px]" : "w-[2px]"} rounded-full bg-current motion-safe:transition-transform motion-safe:duration-200 motion-safe:ease-[cubic-bezier(0.23,1,0.32,1)]`}
          style={{ transform: `scaleY(${scale})` }}
        />
      ))}
    </span>
  );
}
