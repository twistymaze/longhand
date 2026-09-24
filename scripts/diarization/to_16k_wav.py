# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Convert any audio file to the mono 16 kHz WAV `diarization-eval` accepts.

    uv run scripts/diarization/to_16k_wav.py sample.m4a fixtures/sample.wav

The evaluation binary rejects anything that is not mono at 16 kHz and accepts
either integer or float samples, so this writes signed 16-bit PCM to match what
`eval/fetch_datasets.py` produces for the public corpora. Requires ffmpeg.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

SAMPLE_RATE = 16_000


def probe_duration(path: Path) -> float:
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration",
         "-of", "default=noprint_wrappers=1:nokey=1", str(path)],
        capture_output=True, text=True, check=True,
    )
    return float(out.stdout.strip())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("dest", type=Path)
    parser.add_argument("--overwrite", action="store_true")
    args = parser.parse_args()

    if not args.source.is_file():
        sys.exit(f"no such file: {args.source}")
    if args.dest.exists() and not args.overwrite:
        sys.exit(f"{args.dest} exists; pass --overwrite to replace it")
    for tool in ("ffmpeg", "ffprobe"):
        if shutil.which(tool) is None:
            sys.exit(f"{tool} not found on PATH")

    args.dest.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y",
         "-i", str(args.source),
         "-ac", "1", "-ar", str(SAMPLE_RATE), "-c:a", "pcm_s16le",
         str(args.dest)],
        check=True,
    )
    duration = probe_duration(args.dest)
    print(f"{args.source} -> {args.dest}")
    print(f"  {duration:.2f} s ({duration / 3600:.2f} h), mono, {SAMPLE_RATE} Hz, pcm_s16le")


if __name__ == "__main__":
    main()
