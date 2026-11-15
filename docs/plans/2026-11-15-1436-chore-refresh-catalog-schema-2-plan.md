---
title: Refresh catalog from Handy schema 2 - Plan
type: chore
date: 2026-11-15
topic: refresh-catalog-schema-2
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# Refresh catalog from Handy schema 2 - Plan

## Goal Capsule

- **Objective:** A maintainer can run `scripts/refresh-catalog.py` on Handy's current model catalog and get the four borrowed recognition entries of `crates/models/catalog.json` again, with no change to what users download.
- **Means:** Rewrite the script to read Handy catalog schema 2. Longhand's checked-in catalog keeps the values that schema 2 does not supply.
- **Authority:** The checked-in `crates/models/catalog.json` wins over Handy's catalog for timestamp levels, recommendation and speaker preferences. The script's own tables give picker names, base models and licence overrides.
- **Stop conditions:** Stop and ask when the script output for one of the four entries is different from the checked-in entry (other than key order), or when Handy's pinned revision, Q8_0 file name, size or SHA-256 changed.
- **Execution profile:** `ce-work` in a worktree from `origin/main`, one commit, one PR titled `chore(models): …` (no app release).
- **Open blockers:** None.

---

## Product Contract

### Summary

Handy now copies its catalog from the npm package `@handy-computer/models` (Handy commit `93c9ee05`, package 0.2.2; Handy `aca8985` has package 0.2.3) and deleted its generator. The catalog has a new shape (schema 2), so `scripts/refresh-catalog.py` cannot read it. This change makes the script read schema 2 and gives the same output as before.

### Problem Frame

The script reads schema 1 fields that schema 2 removed: `slug`, `default_quant`, `language_count`, `base_model`, `speed_score` and `accuracy_score`. In schema 2, `id` is the package id (the Hugging Face repository name without `-gguf`), `quant` names the default file, `files` is a map by quant with `size`, `capabilities.timestamps` is a boolean, and benchmark results are in a top-level `results` block (FLEURS error percent per language and real-time factors). Nemotron 3.5 ASR Streaming lists 32 region tags (`en-US`, `en-GB`, …) in place of 28 language codes. Handy also changed the licence id of Parakeet Unified EN 0.6B from `other` to `cc-by-4.0`, but its Hugging Face card names the NVIDIA Open Model License (read 2026-11-15).

The pinned revision, Q8_0 file name, size and SHA-256 of all four entries, and the Q5_K_M file of Whisper Large v3 Turbo, are unchanged upstream.

### Requirements

- R1. The script reads a schema 2 catalog and fails with a clear message on any other schema.
- R2. On Handy's current catalog, the script prints the four entries that `crates/models/catalog.json` has now, equal as JSON values.
- R3. `--languages <id>` prints the language codes of one Handy model, reduced as in KTD3.
- R4. `crates/models/PROVENANCE.md` names the Handy revision and package version that the script reads, and `scripts/check-provenance.sh` passes.
- R5. The script tests cover the schema 2 reading and the existing checks against the checked-in entries.

### Scope Boundaries

- `crates/models/catalog.json` does not change.
- The Whisper entry stays written by hand, as before.
- Changed during review of the PR: the scores now come from the schema 2 `results` block (KTD6), so `crates/models/catalog.json` changes its five assessment blocks and the PR is a `fix`.
- No change to app code.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **Schema 2 input.** Select models by `id`. Take the file from `files[quant]` and require a plain file name (no `/`). Require `role` to be `asr`, `repository` to be `handy-computer/<id>-gguf`, and `revision` to be a 40-character commit. Build the download URL from the Hugging Face repository at the pinned revision, as before.
- KTD2. **Timestamp level from the checked-in entry.** Schema 2 only says whether a model has timestamps. Longhand's speaker join needs the level, so the script copies `timestamps` from the checked-in entry, as it does for `recommended`. It fails when the checked-in level is missing or is not `token`, `word` or `segment`, and when Handy's `capabilities.timestamps` is false. A table by Handy `family` would guess: all four models have family `parakeet`, and a later model in that family can have another level.
- KTD3. **Language codes from tags.** Reduce each tag to its lower-case primary subtag and drop repeats, in Handy's order. Nemotron 3.5's 32 tags give the 28 codes in the checked-in entry.
- KTD4. **Base model in the script.** Schema 2 has no `base_model`, so the script's model table gives each entry its picker name and base model (`nvidia/<id>`). The publisher comes from the base model's organization, as before.
- KTD5. **Licence.** Keep the map from Handy's licence id and the overrides for each id. Add Parakeet Unified EN 0.6B → NVIDIA Open Model License (see Problem Frame).
- KTD6. **Assessment from the schema 2 results** (changed in review; the first version copied the frozen scores). Handy's model list computes its scores at runtime with the old formulas on the package's results: speed = 100·(1 − e^(−rtf/8)) from the GPU real-time factor (CPU when there is no GPU result), accuracy = 100·e^(−error/15) from the FLEURS error for English (first language otherwise). The old scores mixed LibriSpeech test-clean and FLEURS results, so they could not be compared between models. The script uses Handy's formulas, writes a `benchmark_context`, and gives no score when a model has no result. `--assessment <id>` prints the Whisper entry's block.
- KTD7. **Handy commit check kept.** The assessment links to the Handy commit that the script reads, so the script keeps the Git check of the source file and `--source-revision`.

### Risks

- A later package version can change a pinned value. The script prints what the catalog says; the stop condition and R2's comparison catch a change before a maintainer copies it.

---

## Implementation Units

### U1. Read schema 2

- **Goal:** The script prints the four borrowed entries from a schema 2 catalog.
- **Requirements:** R1, R2, R3, R4, R5
- **Dependencies:** None
- **Files:** `scripts/refresh-catalog.py`, `scripts/tests/test_refresh_catalog.py`, `crates/models/PROVENANCE.md`
- **Approach:** Apply KTD1–KTD7 in `entry()` and `main()`. Update the module text to describe schema 2 and the values that come from the checked-in catalog. In PROVENANCE, the `catalog.json` record gets the full Handy commit `aca8985190c3ad7f361d8a80cb30f179513ed690` (package 0.2.3) as its revision. Its Source paths list the schema 2 fields that the script reads: repository, pinned revision, default quant file name, size and SHA-256, streaming, the `timestamps` flag, language tags and licence id. Its Status says that the timestamp level, the assessment scores of all five entries and the Whisper entry's values (language order, licence) come from Handy `8f9cf53` through the checked-in catalog. The `src/catalog.rs` record does not change.
- **Test scenarios:**
  - A schema 2 model with a checked-in entry gives the checked-in fields and Handy's pinned revision, file name, size and SHA-256.
  - Region tags reduce to primary codes with repeats removed, in order.
  - Handy `timestamps: false` fails, and a checked-in entry with no valid timestamp level fails.
  - A missing `files[quant]` and a file name with `/` each fail.
  - A repository that is not `handy-computer/<id>-gguf` fails.
  - The assessment block comes from the checked-in entry; a missing block fails.
  - Parakeet Unified EN gets the NVIDIA Open Model License although Handy says `cc-by-4.0`.
  - The existing checked-in entry tests (entry for a different artifact ID, missing speaker preferences) still pass.
  - A catalog with another `schema` value fails.
- **Verification:** The tests pass. The script output on Handy `aca8985` and on `93c9ee05` equals the four checked-in entries.

---

## Verification Contract

| Check | Proves |
|---|---|
| `uv run --no-project python -m unittest discover -s scripts/tests -t scripts` | R5 |
| `uv run scripts/refresh-catalog.py reference-source/Handy/src-tauri/src/catalog/catalog.json`, compared as JSON with the four entries in `crates/models/catalog.json`, at Handy `aca8985` and `93c9ee05` | R1, R2 |
| `uv run scripts/refresh-catalog.py --languages nemotron-3.5-asr-streaming-0.6b` equals the checked-in codes | R3 |
| `scripts/check-provenance.sh` | R4 |
| `git diff --exit-code crates/models/catalog.json` | Scope |

---

## Definition of Done

- U1 is committed on one branch, and the `chore` PR is open and green.
- `crates/models/catalog.json` has no change.
