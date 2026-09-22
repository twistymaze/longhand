# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Derive a reference RTTM from a third-party transcript's turn timestamps.

    uv run scripts/diarization/monologue_to_rttm.py sample.txt fixtures/sample.rttm \
        --audio fixtures/sample.wav

Input lines look like `[Speaker 3 - 180:28]`, where the time is MM:SS and the
minute field is unbounded (180:28 is three hours in). Such a transcript carries
only turn starts, so each turn is closed at the next turn's start and the last
at the audio duration. The result is authoritative for WHERE turns change and
not for WHO is speaking: these transcripts over-split speakers, so every
comparison against this reference must be label-free.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

TURN = re.compile(r"^\[Speaker\s+(\d+)\s*-\s*(\d+):(\d{2})\]\s*$")


def probe_duration(path: Path) -> float:
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration",
         "-of", "default=noprint_wrappers=1:nokey=1", str(path)],
        capture_output=True, text=True, check=True,
    )
    return float(out.stdout.strip())


def parse(path: Path) -> list[tuple[float, str]]:
    turns: list[tuple[float, str]] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        match = TURN.match(line)
        if match is None:
            continue
        speaker, minutes, seconds = match.groups()
        turns.append((int(minutes) * 60 + int(seconds), f"spk{speaker}"))
    return turns


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("transcript", type=Path)
    parser.add_argument("dest", type=Path)
    parser.add_argument("--audio", type=Path, help="probe this file for the final turn's end")
    parser.add_argument("--duration", type=float, help="audio duration in seconds")
    parser.add_argument("--min-duration", type=float, default=0.1,
                        help="drop turns shorter than this after closing (default 0.1 s)")
    parser.add_argument("--collapse-labels", type=int, metavar="N",
                        help="rewrite the labels onto N of them, keeping boundaries identical. "
                             "`diarization-eval --exact-speakers` takes its speaker bound from the "
                             "count of distinct labels in the reference and offers no flag of its "
                             "own, so this exists solely to set that bound to the number of people "
                             "actually present. It asserts nothing about who spoke when.")
    args = parser.parse_args()

    turns = parse(args.transcript)
    if not turns:
        sys.exit(f"no `[Speaker N - MM:SS]` lines found in {args.transcript}")

    if args.duration is not None:
        end = args.duration
    elif args.audio is not None:
        end = probe_duration(args.audio)
    else:
        sys.exit("pass --audio or --duration so the final turn can be closed")
    if end <= turns[-1][0]:
        sys.exit(f"duration {end:.2f}s is not after the last turn at {turns[-1][0]:.2f}s")

    if args.collapse_labels is not None:
        if args.collapse_labels < 1:
            sys.exit("--collapse-labels needs a positive count")
        ranked = sorted(
            {s for _, s in turns},
            key=lambda s: sum(1 for _, t in turns if t == s),
            reverse=True,
        )
        keep = ranked[: args.collapse_labels - 1]
        merged = f"spk{args.collapse_labels}"
        remap = {s: (s if s in keep else merged) for s in ranked}
        turns = [(start, remap[speaker]) for start, speaker in turns]

    stem = args.dest.stem
    lines, dropped = [], 0
    for index, (start, speaker) in enumerate(turns):
        stop = turns[index + 1][0] if index + 1 < len(turns) else end
        duration = stop - start
        if duration < args.min_duration:
            dropped += 1
            continue
        lines.append(
            f"SPEAKER {stem} 1 {start:.3f} {duration:.3f} <NA> <NA> {speaker} <NA> <NA>"
        )

    args.dest.parent.mkdir(parents=True, exist_ok=True)
    args.dest.write_text("\n".join(lines) + "\n", encoding="utf-8")

    speakers = {s for _, s in turns}
    print(f"{args.transcript} -> {args.dest}")
    print(f"  {len(lines)} turns written ({dropped} dropped under {args.min_duration}s)")
    print(f"  {len(speakers)} distinct labels, spanning {end / 3600:.2f} h")
    print("  labels are NOT reliable; use this reference for boundaries only")
    if args.collapse_labels is not None:
        print(f"  labels collapsed to {args.collapse_labels} to set the eval binary's speaker bound")


if __name__ == "__main__":
    main()
