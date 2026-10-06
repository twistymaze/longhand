---
title: Verify stopped a note on detected sound without words
date: 2026-10-05
category: logic-errors
module: pipeline verify stage (transcript coverage)
problem_type: logic_error
component: service_layer
symptoms:
  - "A note stops with \"Stopped while checking the transcript\" and \"Segment 0 has speech but no text.\""
  - "A note stops with \"segment 0 text covers 23.4% of detected speech\"."
  - "Retry gives the same error every time."
root_cause: logic_error
resolution_type: code_fix
severity: high
tags: ["verify", "coverage", "vad", "gap-fill", "live-transcript", "retry"]
---

# Verify stopped a note on detected sound without words

## Problem

The Verify stage stopped a note when the speech detector (VAD) marked sound as speech and the model found no words in it. Retry runs the same model on the same audio, so the note could never finish. [PR #39](https://github.com/twistymaze/longhand/pull/39) changed the coverage rule.

## Symptoms

Two real notes failed:

- A 40-minute note. A microphone change 5.6 s after the start closed part 0. The VAD marked 1.1 s in part 0 as speech (two short sounds). The live part had no words. The gap fill for 0–2.3 s returned empty text. Error: "segment 0 has speech but no text" (`crates/pipeline/src/recognition/stages.rs:684`).
- A 1-minute note recorded during a Teams meeting. Live words stopped at 4.9 s. The VAD marked speech until 19 s. The gap fill over 1.1–19.4 s kept no words. Error: "text covers 23.4% of detected speech".

## What Didn't Work

- **Retry.** The model output is the same for the same audio, so a retry always gives the same empty result.
- **Reading the saved fill result as the model's full output.** A saved gap-fill window holds only the words that `merge` keeps: words inside the gaps (`crates/pipeline/src/transcribe.rs:410`). An empty saved result does not mean that the model heard nothing in the window. In the 1-minute note, the model found 32 tokens in 1.5–4.8 s, and `merge` dropped them because the live text already had them.
- **A minimum amount of speech.** A threshold such as "ignore less than 3 s" fixes the 1 s case only. The 1-minute note had 13 s of sound without words.

## Solution

Voiced time inside a gap-fill window counts as covered, with or without words:

- `verify::refilled_spans` (`crates/pipeline/src/verify.rs:193`) returns the spans of the non-live windows. It does this only for a segment that has a live part, because a gap fill runs only in such a segment.
- `verify::judge` (`crates/pipeline/src/verify.rs:213`) measures coverage with these spans counted as exact. `SLACK_MS` widens text spans only. The window spans stay as they are, because voiced time outside a window was not sent to the model. The independent Codex review found this case.
- Both verify paths decide pass or fail from `judge`: `verify::run` (`crates/pipeline/src/verify.rs:105`) and the recognition route's `verify` (`crates/pipeline/src/recognition/stages.rs:709`).
- Voiced time that the model never received still fails (AE11). A segment without live text (a full pass) keeps the old rule.
- `verify::wordless_ms` writes the seconds of sound without words to the log. The note shows nothing; the user does not want a notice for short stretches.

## Why This Works

The coverage check exists to find text that is missing because recognition did not run or lost output (AE11). A gap fill is the second model pass over voiced time that the live text missed. When it also finds no words, the model has given its answer for that audio. A failure there is not missing text, and Retry cannot change it.

## Prevention

- An empty model result for voiced audio is a normal outcome, not a temporary fault. Real model faults appear as errors, for example `OutputTruncated` (`crates/asr/src/recovery.rs`). The old test `a_verify_failure_and_retry_keep_the_live_rows_and_redo_only_the_flagged_window` used an empty gap-fill result as its failure. It now uses a lost window result instead (`crates/pipeline/tests/live.rs:1105`). `a_gap_fill_window_that_decodes_nothing_passes_verify` (`crates/pipeline/tests/live.rs:1091`) holds the new rule.
- Stage tests with a model that returns no words: `voiced_sound_without_words_after_a_gap_fill_passes_verify` and `voiced_sound_without_words_and_without_a_gap_fill_fails_verify` (`crates/pipeline/tests/recognition_stages.rs:1997`, `:2008`).
- In real notes, about half of all gap-fill windows return no words. Before you treat an empty window as a defect, compare it with other notes.

### Diagnose a Verify failure from a user's note

1. Get the app log (`~/Library/Logs/net.ewal.longhand/Longhand*.log`) and a backup of `longhand.db` (`sqlite3 … ".backup …"`). Keep them in `manual-tests/`.
2. In the `checkpoints` table, read the `transcribe` rows for the segment with the text removed: `seg-NNNN-plan` (VAD `speech` spans), `seg-NNNN-live-*` (live words), `seg-NNNN-win-*` (gap-fill results), and the `verify` rows `failed` and `seg-NNNN`. Replace `"text":"…"` values before you print them.
3. To see the full model output that `merge` drops, run the recognizer on the note's processing file for the window span. That file is in the note's working folder in app data, at proc/segment-NN.wav (16 kHz mono f32), not in the repo. Add a temporary example binary to the `crates/asr` crate (an examples folder, which the repo does not keep) that calls `Engine::load` and `transcribe_tokens` and prints only token times. Delete it after the test.
