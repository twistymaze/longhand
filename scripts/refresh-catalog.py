# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Make Longhand's recognition-model entries from Handy's model catalog (KTD1).

    uv run scripts/refresh-catalog.py [path/to/handy/catalog.json]
    uv run scripts/refresh-catalog.py --languages whisper-large-v3-turbo

The first form prints, as JSON, the eight `transcribe-cpp` entries that
`crates/models/catalog.json` takes from Handy, in Longhand's schema. Copy
them into that file. The script does not write the Whisper entry: that entry
keeps its Q5_K_M file, so existing installs do not download it again. The
second form prints the language codes of one Handy model.

Each entry uses Handy's default quant, pinned revision, file name, size and
SHA-256. The download URL is the Hugging Face URL at the pinned revision;
Handy's mirror is not used. The script reads a local file only. It does not
use the network.
"""
import argparse
import json
import sys
from pathlib import Path

DEFAULT_SOURCE = "reference-source/Handy/src-tauri/src/catalog/catalog.json"
ORG = "handy-computer"

# Handy slug -> the name the picker shows (R1). The order is the order in
# `catalog.json`. Whisper is not in this list; see the module text.
MODELS = {
    "parakeet-unified-en-0.6b": "Parakeet Unified EN 0.6B",
    "nemotron-speech-streaming-en-0.6b": "Nemotron Speech Streaming EN 0.6B",
    "nemotron-3.5-asr-streaming-0.6b": "Nemotron 3.5 ASR Streaming 0.6B",
    "multitalker-parakeet-streaming-0.6b-v1": "Multitalker Parakeet Streaming 0.6B",
    "parakeet-tdt-0.6b-v2": "Parakeet TDT 0.6B v2",
    "parakeet-tdt-0.6b-v3": "Parakeet TDT 0.6B v3",
    "parakeet-tdt-1.1b": "Parakeet TDT 1.1B",
    "moss-transcribe-diarize": "MOSS Transcribe Diarize",
}

# The model a new install selects (KD8). Only this entry is recommended.
RECOMMENDED = "parakeet-unified-en-0.6b"

# Handy licence id -> (name, url). Handy writes "other" for the NVIDIA
# models. Their cards name the NVIDIA Open Model License, except the slugs
# in LICENCE_OVERRIDES (checked against the Hugging Face cards on 2026-09-25).
LICENCES = {
    "cc-by-4.0": ("CC-BY-4.0", "https://creativecommons.org/licenses/by/4.0/"),
    "apache-2.0": ("Apache-2.0", "https://www.apache.org/licenses/LICENSE-2.0"),
    "other": (
        "NVIDIA Open Model License",
        "https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/",
    ),
}

# Slug -> (name, url) when the card names a licence other than the one
# that LICENCES gives for Handy's licence id.
LICENCE_OVERRIDES = {
    "nemotron-3.5-asr-streaming-0.6b": ("OpenMDW-1.1", "https://openmdw.ai/license/1-1/"),
}

# Hugging Face organization of the base model -> the publisher name.
PUBLISHERS = {"nvidia": "NVIDIA", "OpenMOSS-Team": "OpenMOSS Team"}

TIMESTAMP_LEVELS = {"token", "word", "segment"}


def fail(message):
    sys.exit(f"refresh-catalog: {message}")


def quant_suffix(quant):
    """Q8_0 -> q80 and Q5_K_M -> q5km, as in `whisper-large-v3-turbo-q5km`."""
    return quant.lower().replace("_", "")


def describe(languages, timestamps, streaming):
    count = "English" if languages == ["en"] else f"{len(languages)} languages"
    stream = "streaming" if streaming else "not streaming"
    return f"{count}, {timestamps}-level timestamps, {stream}."


def entry(handy, name):
    slug = handy["slug"]
    caps = handy["capabilities"]
    quant = handy["default_quant"]
    # Files under `bundle/` include a second model (the Multitalker
    # diarizer). Longhand takes the plain file only.
    files = [
        f for f in handy["files"] if f["quant"] == quant and "/" not in f["filename"]
    ]
    if len(files) != 1:
        fail(f"{slug}: expected one plain {quant} file, found {len(files)}")
    file = files[0]

    languages = handy["languages"]
    if handy["language_count"] != len(languages):
        fail(f"{slug}: language_count does not agree with languages")
    timestamps = caps["timestamps"]
    if timestamps not in TIMESTAMP_LEVELS:
        fail(f"{slug}: timestamp level {timestamps!r} (the speaker join needs one)")
    if handy["license"] not in LICENCES:
        fail(f"{slug}: unknown licence {handy['license']!r}")
    licence_name, licence_url = LICENCE_OVERRIDES.get(slug, LICENCES[handy["license"]])

    base = handy["base_model"]
    publisher = PUBLISHERS.get(base.split("/")[0])
    if publisher is None:
        fail(f"{slug}: unknown publisher of {base}")
    revision = handy["revision"]
    if len(revision) != 40:
        fail(f"{slug}: revision {revision!r} is not a commit")
    repository = f"https://huggingface.co/{ORG}/{slug}-gguf"
    if handy["id"] != f"{ORG}/{slug}-gguf":
        fail(f"{slug}: repository {handy['id']} is not {ORG}/{slug}-gguf")

    return {
        "id": f"{slug}-{quant_suffix(quant)}",
        "kind": "asr",
        "name": name,
        "description": describe(languages, timestamps, caps["streaming"]),
        "streaming": caps["streaming"],
        "timestamps": timestamps,
        "languages": languages,
        "recommended": slug == RECOMMENDED,
        "repository": repository,
        "revision": revision,
        "filename": file["filename"],
        "url": f"{repository}/resolve/{revision}/{file['filename']}",
        "size_bytes": file["size_bytes"],
        "sha256": file["sha256"],
        "licence": {
            "name": licence_name,
            "url": licence_url,
            "attribution": f"GGUF conversion by {ORG} of {base} ({publisher}, {licence_name}).",
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("source", nargs="?", default=DEFAULT_SOURCE, type=Path)
    parser.add_argument("--languages", metavar="SLUG")
    args = parser.parse_args()

    catalog = json.loads(args.source.read_text())
    by_slug = {m["slug"]: m for m in catalog["models"]}

    if args.languages:
        if args.languages not in by_slug:
            fail(f"{args.languages} is not in {args.source}")
        print(json.dumps(by_slug[args.languages]["languages"]))
        return

    missing = [s for s in MODELS if s not in by_slug]
    if missing:
        fail(f"not in {args.source}: {', '.join(missing)}")
    entries = [entry(by_slug[slug], name) for slug, name in MODELS.items()]
    print(json.dumps(entries, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
