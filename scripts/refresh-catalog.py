# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Make Longhand's recognition-model entries from Handy's model catalog (KTD1).

    uv run scripts/refresh-catalog.py [path/to/handy/catalog.json]
    uv run scripts/refresh-catalog.py --languages nemotron-3.5-asr-streaming-0.6b
    uv run scripts/refresh-catalog.py --assessment whisper-large-v3-turbo

The first form prints, as JSON, the four `transcribe-cpp` entries that
`crates/models/catalog.json` takes from Handy, in Longhand's schema. Copy
them into that file. The script does not write the Whisper entry: that entry
keeps its Q5_K_M file, so existing installs do not download it again. The
second form prints the language codes of one Handy model. Do not copy it into
the Whisper entry: that entry keeps the language order of Handy 8f9cf53. The
third form prints the assessment block of one Handy model, for the Whisper
entry.

Handy's catalog is a copy of the `@handy-computer/models` package, schema 2.
Each entry uses the package's default quant, pinned revision, file name, size
and SHA-256. The download URL is the Hugging Face URL at the pinned revision;
Handy's mirror is not used. Region tags such as `en-US` become language codes
such as `en`. The script reads a local file only. It does not use the network.

The 0-100 assessment scores use the formulas of Handy's model list, on the
package's benchmark results: speed = 100 * (1 - e^(-rtf/8)) from the GPU
real-time factor (the CPU one when there is no GPU result), and accuracy =
100 * e^(-error/15) from the error percent for English (the model's first
language when there is no English result). A model with no result is not
rated.

Schema 2 does not give the timestamp level, the recommendation or the speaker
preferences. The script copies these values from the entry with the same
artifact ID in Longhand's checked-in catalog, so a new artifact ID requires an
entry in that catalog before refresh can print it. The source must be a
committed Handy file, or you must give `--source-revision`. Record that Handy
commit in `crates/models/PROVENANCE.md`.
"""
import argparse
import json
import math
import subprocess
import sys
from pathlib import Path

DEFAULT_SOURCE = "reference-source/Handy/src-tauri/src/catalog/catalog.json"
CHECKED_IN_CATALOG = Path(__file__).resolve().parents[1] / "crates/models/catalog.json"
SCHEMA = 2
ORG = "handy-computer"

# Handy id -> (the name the picker shows (R1), base model). The order is the
# order in `catalog.json`. Whisper is not in this list; see the module text.
MODELS = {
    "parakeet-unified-en-0.6b": ("Parakeet Unified EN 0.6B", "nvidia/parakeet-unified-en-0.6b"),
    "nemotron-speech-streaming-en-0.6b": (
        "Nemotron Speech Streaming EN 0.6B",
        "nvidia/nemotron-speech-streaming-en-0.6b",
    ),
    "nemotron-3.5-asr-streaming-0.6b": (
        "Nemotron 3.5 ASR Streaming 0.6B",
        "nvidia/nemotron-3.5-asr-streaming-0.6b",
    ),
    "parakeet-tdt-0.6b-v3": ("Parakeet TDT 0.6B v3", "nvidia/parakeet-tdt-0.6b-v3"),
}

# Handy licence id -> (name, url). Handy writes "other" for most NVIDIA
# models. Their cards name the NVIDIA Open Model License, except the ids in
# LICENCE_OVERRIDES.
LICENCES = {
    "cc-by-4.0": ("CC-BY-4.0", "https://creativecommons.org/licenses/by/4.0/"),
    "apache-2.0": ("Apache-2.0", "https://www.apache.org/licenses/LICENSE-2.0"),
    "other": (
        "NVIDIA Open Model License",
        "https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/",
    ),
}

# Id -> (name, url) when the Hugging Face card names a licence other than the
# one that LICENCES gives for Handy's licence id. Checked against the cards on
# 2026-09-25 and 2026-11-15.
LICENCE_OVERRIDES = {
    "parakeet-unified-en-0.6b": LICENCES["other"],
    "nemotron-3.5-asr-streaming-0.6b": ("OpenMDW-1.1", "https://openmdw.ai/license/1-1/"),
}

# Hugging Face organization of the base model -> the publisher name.
PUBLISHERS = {"nvidia": "NVIDIA"}

TIMESTAMP_LEVELS = {"token", "word", "segment"}

# The formulas of Handy's model list (`src-tauri/src/catalog/mod.rs`).
SPEED_SCALE = 8.0
ACCURACY_SCALE = 15.0


def fail(message):
    sys.exit(f"refresh-catalog: {message}")


def quant_suffix(quant):
    """Q8_0 -> q80 and Q5_K_M -> q5km, as in `whisper-large-v3-turbo-q5km`."""
    return quant.lower().replace("_", "")


def language_codes(tags):
    """en-US, en-GB, es-ES -> en, es: primary subtags, first seen order."""
    codes = []
    for tag in tags:
        code = tag.split("-")[0].lower()
        if code not in codes:
            codes.append(code)
    return codes


def describe(languages, timestamps, streaming):
    count = "English" if languages == ["en"] else f"{len(languages)} languages"
    stream = "streaming" if streaming else "not streaming"
    return f"{count}, {timestamps}-level timestamps, {stream}."


def checked_in_values(artifact_id, checked_in_entries):
    matches = [e for e in checked_in_entries if e["id"] == artifact_id]
    if len(matches) != 1:
        fail(f"{artifact_id}: expected one checked-in entry, found {len(matches)}")
    checked_in = matches[0]
    if checked_in.get("timestamps") not in TIMESTAMP_LEVELS:
        fail(f"{artifact_id}: checked-in timestamp level {checked_in.get('timestamps')!r} (the speaker join needs one)")
    if not isinstance(checked_in.get("recommended"), bool):
        fail(f"{artifact_id}: missing recommended policy")
    preferences = checked_in.get("speaker_preferences")
    if not isinstance(preferences, dict) or any(
        not isinstance(preferences.get(mode), list) or not preferences[mode]
        for mode in ("after_stop", "during_recording")
    ):
        fail(f"{artifact_id}: missing speaker_preferences policy")
    return checked_in


def assessment(results, benchmarks, languages, source_revision):
    """The 0-100 scores from one model's benchmark results (see the module text)."""
    results = results or {}
    performance = results.get("performance") or {}
    speed_kind = next((k for k in ("accelerated", "cpu") if performance.get(k)), None)
    accuracy = results.get("accuracy") or {}
    language = next((code for code in ["en", *language_codes(languages)[:1]] if code in accuracy), None)

    speed_score = None
    accuracy_score = None
    context = []
    if language is not None:
        error = accuracy[language]["error"]
        accuracy_score = round(100 * math.exp(-error / ACCURACY_SCALE))
        data = benchmarks["accuracy"]
        context.append(f"{data['dataset']} {data['split']} error for {language}")
    if speed_kind is not None:
        speed_score = round(100 * (1 - math.exp(-performance[speed_kind] / SPEED_SCALE)))
        machine = benchmarks["speed"][speed_kind]
        context.append(f"real-time factor on {machine['machine']} ({machine['backend']})")
    if context:
        context.append(f"{benchmarks['quant']} files")
    return {
        "source": f"https://github.com/cjpais/Handy/blob/{source_revision}/src-tauri/src/catalog/catalog.json",
        "source_revision": source_revision,
        "speed_score": speed_score,
        "asr_accuracy_score": accuracy_score,
        "speaker_accuracy_score": None,
        "benchmark_context": "Handy benchmarks: " + "; ".join(context) + "." if context else None,
    }


def entry(handy, checked_in_entries, assessment_block):
    model_id = handy["id"]
    name, base = MODELS[model_id]
    caps = handy["capabilities"]
    quant = handy["quant"]
    artifact_id = f"{model_id}-{quant_suffix(quant)}"
    checked_in = checked_in_values(artifact_id, checked_in_entries)

    if handy.get("role") != "asr":
        fail(f"{model_id}: role {handy.get('role')!r} is not asr")
    file = handy["files"].get(quant)
    if file is None:
        fail(f"{model_id}: no {quant} file")
    if "/" in file["filename"]:
        fail(f"{model_id}: {file['filename']} is not a plain file name")
    if caps.get("timestamps") is not True:
        fail(f"{model_id}: Handy gives no timestamps (the speaker join needs them)")
    timestamps = checked_in["timestamps"]
    languages = language_codes(handy["languages"])
    if handy["license"] not in LICENCES:
        fail(f"{model_id}: unknown licence {handy['license']!r}")
    licence_name, licence_url = LICENCE_OVERRIDES.get(model_id, LICENCES[handy["license"]])

    publisher = PUBLISHERS.get(base.split("/")[0])
    if publisher is None:
        fail(f"{model_id}: unknown publisher of {base}")
    revision = handy["revision"]
    if len(revision) != 40:
        fail(f"{model_id}: revision {revision!r} is not a commit")
    if handy["repository"] != f"{ORG}/{model_id}-gguf":
        fail(f"{model_id}: repository {handy['repository']} is not {ORG}/{model_id}-gguf")
    repository = f"https://huggingface.co/{handy['repository']}"

    return {
        "id": artifact_id,
        "kind": "asr",
        "name": name,
        "description": describe(languages, timestamps, caps["streaming"]),
        "streaming": caps["streaming"],
        "timestamps": timestamps,
        "languages": languages,
        "recommended": checked_in["recommended"],
        "speaker_preferences": checked_in["speaker_preferences"],
        "repository": repository,
        "revision": revision,
        "filename": file["filename"],
        "url": f"{repository}/resolve/{revision}/{file['filename']}",
        "size_bytes": file["size"],
        "sha256": file["sha256"],
        "assessment": assessment_block,
        "licence": {
            "name": licence_name,
            "url": licence_url,
            "attribution": f"GGUF conversion by {ORG} of {base} ({publisher}, {licence_name}).",
        },
    }


def models_by_id(catalog, source):
    if catalog.get("schema") != SCHEMA:
        fail(f"{source} has schema {catalog.get('schema')!r}; this script reads schema {SCHEMA}")
    return {m["id"]: m for m in catalog["models"]}


def source_revision(source, supplied):
    """The Handy commit of the source file, which must have no local changes."""
    if supplied:
        if len(supplied) != 40 or any(c not in "0123456789abcdef" for c in supplied):
            fail("--source-revision must be a lowercase 40-character commit")
        return supplied
    source = source.resolve()

    def git(*args):
        return subprocess.check_output(["git", "-C", str(source.parent), *args], stderr=subprocess.DEVNULL)

    try:
        root = Path(git("rev-parse", "--show-toplevel").decode().strip())
        revision = git("rev-parse", "HEAD").decode().strip()
        relative = source.relative_to(root).as_posix()
        if git("show", f"{revision}:{relative}") != source.read_bytes():
            fail("the source has local changes; use a committed Handy catalog")
        return revision
    except (subprocess.CalledProcessError, ValueError):
        fail("cannot read the source revision; give --source-revision for a catalog outside Git")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("source", nargs="?", default=DEFAULT_SOURCE, type=Path)
    parser.add_argument("--languages", metavar="ID")
    parser.add_argument("--assessment", metavar="ID")
    parser.add_argument("--source-revision", help="Handy commit of a catalog outside Git")
    args = parser.parse_args()

    catalog = json.loads(args.source.read_text())
    by_id = models_by_id(catalog, args.source)

    if args.languages:
        if args.languages not in by_id:
            fail(f"{args.languages} is not in {args.source}")
        print(json.dumps(language_codes(by_id[args.languages]["languages"])))
        return

    revision = source_revision(args.source, args.source_revision)
    results = catalog.get("results") or {}

    def assessment_of(model_id):
        return assessment(results.get(model_id), catalog["benchmarks"], by_id[model_id]["languages"], revision)

    if args.assessment:
        if args.assessment not in by_id:
            fail(f"{args.assessment} is not in {args.source}")
        print(json.dumps(assessment_of(args.assessment), indent=2, ensure_ascii=False))
        return

    missing = [i for i in MODELS if i not in by_id]
    if missing:
        fail(f"not in {args.source}: {', '.join(missing)}")
    checked_in_entries = json.loads(CHECKED_IN_CATALOG.read_text())["models"]
    entries = [entry(by_id[i], checked_in_entries, assessment_of(i)) for i in MODELS]
    print(json.dumps(entries, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
