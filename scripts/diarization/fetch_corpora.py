# /// script
# requires-python = ">=3.10"
# dependencies = ["datasets>=4.0", "soundfile", "numpy"]
# ///
"""Materialise public diarization corpora as `<stem>.wav` + `<stem>.rttm` pairs.

    uv run scripts/diarization/fetch_corpora.py --out fixtures/diar-test \
        --dataset diarizers-community/ami:test --subset sdm --limit 8

Replaces anarlog's `eval/fetch_datasets.py`, which no longer runs: current
`datasets` releases decode audio through `torchcodec`, which drags in torch and
then fails to load its native library against a system FFmpeg it does not match.
This reads the audio column undecoded and decodes the raw bytes with soundfile,
so nothing beyond numpy and soundfile is needed.

AMI needs a subset: `sdm` is the single distant microphone, which is the
condition Longhand targets; `ihm` is individual headset mics and is a much
easier problem than this product faces. Licences: AMI and VoxConverse are
CC BY 4.0. Evaluation data only -- never shipped, never committed.
"""

from __future__ import annotations

import argparse
import io
from pathlib import Path

import numpy as np
import soundfile as sf
from datasets import Audio, load_dataset

SAMPLE_RATE = 16_000


def write_rttm(path: Path, stem: str, starts, ends, speakers) -> int:
    lines = []
    for start, end, speaker in zip(starts, ends, speakers):
        duration = float(end) - float(start)
        if duration <= 0:
            continue
        lines.append(
            f"SPEAKER {stem} 1 {float(start):.3f} {duration:.3f} <NA> <NA> {speaker} <NA> <NA>"
        )
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return len(lines)


def decode(entry) -> tuple[np.ndarray, int]:
    """Decode an undecoded audio cell to mono float32 and its sample rate."""
    raw = entry["bytes"] if entry.get("bytes") is not None else Path(entry["path"]).read_bytes()
    audio, rate = sf.read(io.BytesIO(raw), dtype="float32", always_2d=True)
    return audio.mean(axis=1), rate


def resample(audio: np.ndarray, source: int, target: int) -> np.ndarray:
    if source == target:
        return audio
    count = int(round(len(audio) * target / source))
    return np.interp(
        np.linspace(0.0, len(audio) - 1, count),
        np.arange(len(audio)),
        audio,
    ).astype(np.float32)


def materialise(spec: str, out: Path, limit: int | None, subset: str | None) -> int:
    name, _, split = spec.partition(":")
    split = split or "test"
    dataset = load_dataset(name, subset, split=split) if subset else load_dataset(name, split=split)
    dataset = dataset.cast_column("audio", Audio(decode=False))

    prefix = f"{name.split('/')[-1]}_{subset + '_' if subset else ''}{split}"
    written = 0
    for index, row in enumerate(dataset):
        if limit is not None and written >= limit:
            break
        stem = f"{prefix}_{index:04d}"
        audio, rate = decode(row["audio"])
        audio = resample(audio, rate, SAMPLE_RATE)
        sf.write(out / f"{stem}.wav", audio, SAMPLE_RATE, subtype="PCM_16")
        turns = write_rttm(
            out / f"{stem}.rttm", stem,
            row["timestamps_start"], row["timestamps_end"], row["speakers"],
        )
        speakers = len(set(row["speakers"]))
        print(f"  {stem}: {len(audio) / SAMPLE_RATE / 60:.1f} min, {turns} turns, {speakers} speakers")
        written += 1
    return written


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--dataset", action="append", required=True,
                        help="Hub id with optional :split, e.g. diarizers-community/ami:test")
    parser.add_argument("--subset", default=None, help="config name, e.g. sdm for AMI")
    parser.add_argument("--limit", type=int, default=None, help="max recordings per dataset")
    args = parser.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    for spec in args.dataset:
        print(f"{spec}{' [' + args.subset + ']' if args.subset else ''}:")
        count = materialise(spec, args.out, args.limit, args.subset)
        print(f"  wrote {count} recordings to {args.out}")


if __name__ == "__main__":
    main()
