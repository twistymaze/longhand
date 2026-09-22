# /// script
# requires-python = ">=3.10"
# dependencies = ["numpy>=1.26", "scipy>=1.11"]
# ///
"""Score a diarization hypothesis on conversational structure rather than attribution.

    uv run scripts/diarization/structure_check.py fixtures/sample.rttm hyp/sample.rttm

The gate is collapse: a stack that flattens a multi-party session into one
speaker leaves a summarizer unable to tell an exchange took place, which is the
failure this product exists to prevent. Splitting one person across two labels
does not hurt that consumer, so label accuracy is reported as a diagnostic and
never gates.

Windowed DER is computed with the optimal label mapping recomputed inside each
window, so a speaker relabelled between windows costs nothing while two people
merged inside one window costs full price.
"""

from __future__ import annotations

import argparse
import sys
from collections import defaultdict
from pathlib import Path

import numpy as np
from scipy.optimize import linear_sum_assignment

FRAME = 0.01


def load_rttm(path: Path) -> list[tuple[float, float, str]]:
    turns = []
    for line in path.read_text(encoding="utf-8").splitlines():
        parts = line.split()
        if len(parts) < 8 or parts[0] != "SPEAKER":
            continue
        start, duration, speaker = float(parts[3]), float(parts[4]), parts[7]
        if duration > 0:
            turns.append((start, start + duration, speaker))
    return sorted(turns)


def span(turns) -> float:
    return max((end for _, end, _ in turns), default=0.0)


def speakers_in(turns, start: float, stop: float) -> set[str]:
    return {s for a, b, s in turns if a < stop and b > start}


def windows(total: float, size: float, stride: float | None = None):
    stride = stride or size
    edge = 0.0
    while edge < total:
        yield edge, min(edge + size, total)
        edge += stride


def boundaries(turns) -> list[float]:
    """Turn-change times, label-free: the start of every turn after a different speaker."""
    marks = []
    previous = None
    for start, _, speaker in turns:
        if previous is not None and speaker != previous:
            marks.append(start)
        previous = speaker
    return marks


def boundary_agreement(ref, hyp, tolerance: float):
    ref_marks, hyp_marks = boundaries(ref), boundaries(hyp)
    if not ref_marks or not hyp_marks:
        return 0.0, 0.0, 0.0
    hyp_sorted = np.array(sorted(hyp_marks))
    ref_sorted = np.array(sorted(ref_marks))
    matched_ref = sum(
        1 for m in ref_sorted
        if len(hyp_sorted) and np.min(np.abs(hyp_sorted - m)) <= tolerance
    )
    matched_hyp = sum(
        1 for m in hyp_sorted
        if len(ref_sorted) and np.min(np.abs(ref_sorted - m)) <= tolerance
    )
    recall = matched_ref / len(ref_sorted)
    precision = matched_hyp / len(hyp_sorted)
    f1 = 0.0 if precision + recall == 0 else 2 * precision * recall / (precision + recall)
    return precision, recall, f1


def frame_labels(turns, start: float, stop: float):
    count = int(round((stop - start) / FRAME))
    grid = [set() for _ in range(count)]
    for a, b, speaker in turns:
        if b <= start or a >= stop:
            continue
        lo = max(0, int(round((a - start) / FRAME)))
        hi = min(count, int(round((b - start) / FRAME)))
        for i in range(lo, hi):
            grid[i].add(speaker)
    return grid


def collar_mask(turns, start: float, stop: float, collar: float):
    count = int(round((stop - start) / FRAME))
    mask = np.ones(count, dtype=bool)
    pad = int(round(collar / FRAME))
    for a, b, _ in turns:
        for edge in (a, b):
            if edge <= start or edge >= stop:
                continue
            centre = int(round((edge - start) / FRAME))
            mask[max(0, centre - pad):min(count, centre + pad)] = False
    return mask


def window_der(ref, hyp, start: float, stop: float, collar: float):
    ref_grid = frame_labels(ref, start, stop)
    hyp_grid = frame_labels(hyp, start, stop)
    mask = collar_mask(ref, start, stop, collar)

    ref_ids = sorted({s for f in ref_grid for s in f})
    hyp_ids = sorted({s for f in hyp_grid for s in f})
    total = sum(len(f) for f, keep in zip(ref_grid, mask) if keep)
    if total == 0:
        return None

    if ref_ids and hyp_ids:
        overlap = np.zeros((len(ref_ids), len(hyp_ids)))
        r_index = {s: i for i, s in enumerate(ref_ids)}
        h_index = {s: i for i, s in enumerate(hyp_ids)}
        for r_set, h_set, keep in zip(ref_grid, hyp_grid, mask):
            if not keep:
                continue
            for r in r_set:
                for h in h_set:
                    overlap[r_index[r], h_index[h]] += 1
        rows, cols = linear_sum_assignment(-overlap)
        mapping = {hyp_ids[c]: ref_ids[r] for r, c in zip(rows, cols)}
    else:
        mapping = {}

    miss = false_alarm = confusion = 0
    for r_set, h_set, keep in zip(ref_grid, hyp_grid, mask):
        if not keep:
            continue
        mapped = {mapping.get(h) for h in h_set}
        correct = len(r_set & mapped)
        miss += max(0, len(r_set) - len(h_set))
        false_alarm += max(0, len(h_set) - len(r_set))
        confusion += min(len(r_set), len(h_set)) - correct
    return (miss + false_alarm + confusion) / total, miss / total, false_alarm / total, confusion / total


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("hypothesis", type=Path)
    parser.add_argument("--window", type=float, default=300.0, help="gate window, seconds (default 300)")
    parser.add_argument("--der-windows", type=float, nargs="*", default=[300.0, 600.0])
    parser.add_argument("--collar", type=float, default=0.25)
    parser.add_argument("--tolerance", type=float, default=0.5, help="boundary match tolerance, seconds")
    parser.add_argument("--min-multi-window-fraction", type=float, default=0.90)
    parser.add_argument("--turn-ratio-range", type=float, nargs=2, default=[0.5, 2.0])
    parser.add_argument("--skip-der", action="store_true")
    args = parser.parse_args()

    ref, hyp = load_rttm(args.reference), load_rttm(args.hypothesis)
    if not ref:
        sys.exit(f"no turns in {args.reference}")
    if not hyp:
        print("GATE: FAIL -- hypothesis contains no turns at all")
        sys.exit(1)
    total = max(span(ref), span(hyp))

    # --- Gate 1: multi-speaker windows preserved -----------------------------
    considered = kept = 0
    for start, stop in windows(total, args.window):
        if len(speakers_in(ref, start, stop)) >= 2:
            considered += 1
            if len(speakers_in(hyp, start, stop)) >= 2:
                kept += 1
    multi_fraction = kept / considered if considered else 1.0

    # --- Gate 2: turn count in range ----------------------------------------
    ratio = len(hyp) / len(ref)
    low, high = args.turn_ratio_range

    gate_multi = multi_fraction >= args.min_multi_window_fraction
    gate_turns = low <= ratio <= high
    passed = gate_multi and gate_turns

    print(f"session span            {total / 3600:.2f} h")
    print(f"reference turns         {len(ref)}  ({len({s for _, _, s in ref})} labels)")
    print(f"hypothesis turns        {len(hyp)}  ({len({s for _, _, s in hyp})} labels)")
    print()
    print("GATE (collapse)")
    print(f"  multi-speaker {args.window / 60:.0f}-min windows kept   "
          f"{kept}/{considered} = {multi_fraction:.3f}   "
          f"{'PASS' if gate_multi else 'FAIL'} (>= {args.min_multi_window_fraction})")
    print(f"  turn count ratio hyp/ref             {ratio:.3f}   "
          f"{'PASS' if gate_turns else 'FAIL'} (in {low}-{high})")
    print(f"  VERDICT: {'PASS' if passed else 'FAIL'}")
    print()

    precision, recall, f1 = boundary_agreement(ref, hyp, args.tolerance)
    print("DIAGNOSTICS (not gating)")
    print(f"  boundary agreement +/-{args.tolerance}s   "
          f"P={precision:.3f} R={recall:.3f} F1={f1:.3f}")

    for size in (300.0, 600.0):
        ref_counts = [len(speakers_in(ref, a, b)) for a, b in windows(total, size)]
        hyp_counts = [len(speakers_in(hyp, a, b)) for a, b in windows(total, size)]
        print(f"  speakers per {size / 60:.0f}-min window     "
              f"ref median {int(np.median(ref_counts))} max {max(ref_counts)}   "
              f"hyp median {int(np.median(hyp_counts))} max {max(hyp_counts)}")

    mass = defaultdict(float)
    for a, b, speaker in hyp:
        mass[speaker] += b - a
    ranked = sorted(mass.values(), reverse=True)
    whole = sum(ranked) or 1.0
    for top in (6, 16):
        if len(ranked) > top:
            print(f"  hyp speech mass in top {top:<2} labels  "
                  f"{sum(ranked[:top]) / whole:.1%} of {len(ranked)} labels")

    dense = sum(
        1 for a, b in windows(total, 10.0, 2.0)
        if len(speakers_in(ref, a, b)) >= 4
    )
    dense_total = sum(1 for _ in windows(total, 10.0, 2.0))
    print(f"  ref 10s windows with >=4 speakers   {dense}/{dense_total} = {dense / dense_total:.3f}")

    if not args.skip_der:
        for size in args.der_windows:
            scores = [
                window_der(ref, hyp, a, b, args.collar)
                for a, b in windows(total, size)
            ]
            scores = [s for s in scores if s is not None]
            if not scores:
                continue
            der, miss, fa, conf = (float(np.mean([s[i] for s in scores])) for i in range(4))
            print(f"  windowed DER {size / 60:.0f} min           "
                  f"{der:.3f}  (miss {miss:.3f}  FA {fa:.3f}  conf {conf:.3f})")

    sys.exit(0 if passed else 1)


if __name__ == "__main__":
    main()
