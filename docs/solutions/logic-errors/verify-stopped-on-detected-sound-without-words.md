---
title: Verify stopped a note on detected sound without words
date: 2026-10-05
last_updated: 2026-10-23
category: logic-errors
module: pipeline verify stage (transcript coverage)
problem_type: logic_error
component: service_layer
symptoms:
  - "A note stops with \"Stopped while checking the transcript\" and \"Segment 0 has speech but no text.\""
  - "A note stops with \"segment 0 text covers 23.4% of detected speech\"."
  - "A note with app audio stops with \"segment 1 text covers 60.1% of detected speech\" after a Teams call starts."
  - "Retry gives the same error every time."
root_cause: logic_error
resolution_type: code_fix
severity: high
tags: ["verify", "coverage", "vad", "gap-fill", "live-transcript", "retry", "app-audio", "stereo"]
---

# Verify stopped a note on detected sound without words

## Problem

The Verify stage stopped a note when the speech detector (VAD) marked sound as speech and the model found no words in it. Retry runs the same model on the same audio, so the note could never finish. Archived PR 39 changed the coverage rule. PR #63 found a second path to the same failure and changed the policy: Verify no longer stops a note.

## Symptoms

Three real notes failed:

- A 40-minute note. A microphone change 5.6 s after the start closed part 0. The VAD marked 1.1 s in part 0 as speech (two short sounds). The live part had no words. The gap fill for 0–2.3 s returned empty text. Error: "segment 0 has speech but no text".
- A 1-minute note recorded during a Teams meeting. Live words stopped at 4.9 s. The VAD marked speech until 19 s. The gap fill over 1.1–19.4 s kept no words. Error: "text covers 23.4% of detected speech".
- A 2-minute note with app audio (PR #63, before the merge). Segment 1 was the first two-channel segment (microphone left, app audio right). It started with the Teams connect tune. Error: "segment 1 text covers 60.1% of detected speech". No line "live text covers all but … s of speech" was in the log for segment 1, so no gap fill ran there.

## What Didn't Work

- **Retry.** The model output is the same for the same audio, so a retry always gives the same empty result.
- **Reading the saved fill result as the model's full output.** A saved gap-fill window holds only the words that `merge` keeps: words inside the gaps. An empty saved result does not mean that the model heard nothing in the window. In the 1-minute note, the model found 32 tokens in 1.5–4.8 s, and `merge` dropped them because the live text already had them.
- **A minimum amount of speech.** A threshold such as "ignore less than 3 s" fixes the 1 s case only. The 1-minute note had 13 s of sound without words.
- **Treating the app audio note as a mix defect.** The two-channel recording was correct: both channels were saved, in sync, and the processing file was the plain mix with no clipping. The failure was in which text Verify judged, not in the audio.

## Solution

### PR 39: wordless sound after a gap fill counts as covered

Voiced time inside a gap-fill window counts as covered, with or without words:

- `verify::refilled_spans` (`crates/pipeline/src/verify.rs:200`) returns the spans of the non-live windows. It does this only for a segment that has a live part, because a gap fill runs only in such a segment.
- `verify::judge` (`crates/pipeline/src/verify.rs:220`) measures coverage with these spans counted as exact. `SLACK_MS` widens text spans only. The window spans stay as they are, because voiced time outside a window was not sent to the model.
- Both verify paths use `judge`: `verify::run` (`crates/pipeline/src/verify.rs:127`) and the recognition route's `verify` (`crates/pipeline/src/recognition/stages.rs:705`).
- `verify::wordless_ms` writes the seconds of sound without words to the log.

### PR #63: a two-channel segment keeps its live text

The app audio note failed because its two-channel segment had no live text. `live_source_matches` (`crates/pipeline/src/recognition/durable.rs:189`) reused live results only for a mono segment. That rule came from the echo canceller: the processing audio of a stereo segment then differed from the live mix, so live text could not be trusted. With no live text, the segment got a full batch pass, `refilled_spans` gave no exemption, and the tune counted as missing text.

PR #63 removed the echo canceller. The live archive reader and Assemble now give the same plain mix (`audio::live_mix::mix`, `crates/audio/src/live_mix.rs:12`) to recognition. So a stereo segment keeps its verified live text, as a mono segment does (`crates/pipeline/src/recognition/durable.rs:211`). The gap fill then hears the tune, and the PR 39 rule applies.

### PR #63: Verify never stops a note

By the user's decision, a note with a part that has no words is better than a note that does not finish. Low coverage no longer fails the Verify stage on either route:

- `verify::run` and the recognition route's `verify` log the shortfall at warn level and call `mark_low_coverage` (`crates/pipeline/src/verify.rs:40`). It writes the `LOW_COVERAGE` checkpoint of the Verify stage (`crates/pipeline/src/verify.rs:37`), or removes it when coverage is good.
- The note's details carry `low_coverage` (`src-tauri/src/commands/session.rs:489`). The Transcript tab shows one quiet `role="status"` line in the caution colour: "Some speech in this note may have no text. Transcribe Again can try again." (`src/session/SessionPage.tsx:816`). The note is saved, and the user can choose Transcribe Again.
- A missing window plan is still an error: that is a defect, not low coverage.
- `VerifyFailure` and the `FAILED` checkpoint stay, so Retry still works on a note that an older build stopped at Verify.

## Why This Works

The coverage check exists to find text that is missing because recognition did not run or lost output (AE11). A gap fill is the second model pass over voiced time that the live text missed. When it also finds no words, the model has given its answer for that audio. A failure there is not missing text, and Retry cannot change it.

A rule that protects reuse of live text must name the condition that makes the text untrustworthy. "Stereo" stood for "the processing audio differs from the live audio". When the echo canceller went, the condition went, but the stereo rule stayed and quietly turned off the gap-fill exemption for every two-channel segment.

Even with both rules, some notes have real gaps: a span where the microphone hears the speakers holds two delayed copies of the call, and the model can miss words there. Stopping the note gave the user nothing; saving it with a short warning gives the user the text and a choice.

## Prevention

- An empty model result for voiced audio is a normal outcome, not a temporary fault. Real model faults appear as errors, for example `OutputTruncated` (`crates/asr/src/recovery.rs`). In real notes, about half of all gap-fill windows return no words.
- When you remove a processing step that made two inputs differ, search for the rules that protected against the difference. For the echo canceller these were the stereo condition in `live_source_matches` and its invalidation reason.
- Call apps play sounds without words: connect tunes, ringing, hold music, join chimes. App audio makes these common. Test a note that starts with one.
- Tests for the rules:
  - `a_gap_fill_window_that_decodes_nothing_passes_verify` (`crates/pipeline/tests/live.rs:1138`).
  - `low_coverage_does_not_stop_the_note_and_marks_it` (`crates/pipeline/tests/live.rs:1152`).
  - `a_stereo_segment_keeps_its_verified_live_text` (`crates/pipeline/tests/recognition_stages.rs:1191`).
  - `voiced_sound_without_words_after_a_gap_fill_passes_verify` and `voiced_sound_without_words_and_without_a_gap_fill_continues_with_low_coverage` (`crates/pipeline/tests/recognition_stages.rs:2249`, `:2261`).
  - `a_window_that_missed_speech_is_saved_with_a_low_coverage_mark` (`crates/pipeline/tests/runner.rs:1581`).
- Tests of Retry and Start Over use a failing transcription window (`fail_on`) now, not a Verify failure.

### Diagnose low coverage from a user's note

1. Get the app log (`~/Library/Logs/net.ewal.longhand/Longhand*.log`) and a backup of `longhand.db` (`sqlite3 … ".backup …"`). Keep them in `manual-tests/`.
2. Find the warning "text covers N% of detected speech" in the log. Look for the line "live text covers all but … s of speech" for the same segment. When it is absent, the segment had no live text, so no gap fill ran.
3. In the `checkpoints` table, read the `transcribe` rows for the segment with the text removed: `seg-NNNN-plan` (VAD `speech` spans), `seg-NNNN-live-*` (live words), `seg-NNNN-win-*` (window results), `live-invalidation-reasons`, and the `verify` row `seg-NNNN`. Replace `"text":"…"` values before you print them. Print word times only.
4. For a two-channel note, measure each second of audio: the RMS and peak of each channel of the stereo files in the note's `audio-recovery/` folder, and of `proc/segment-NN.wav` (16 kHz mono f32). This shows whether a span with no words has app sound only, microphone sound only, or both at a similar level (the microphone hears the speakers). Use `uv run --no-project python` with the standard `wave` and `array` modules.
5. To see what the model finds in a span, cut it from `proc/segment-NN.wav` to a 16 kHz mono PCM16 WAV in a temporary folder. Run the ignored test `recognition_covers_each_independent_window` in `crates/asr/tests/native_attribution.rs` with `LONGHAND_ASR_MODEL`, `LONGHAND_ASR_WAV` and `LONGHAND_ASR_SECONDS`. It prints counts and times only, never transcript text. "produced no text" means the model finds no words in that audio. To check for music or another language, run it again with the Whisper model.
6. Ask the user what played in the span. Do not play or upload the audio yourself.
