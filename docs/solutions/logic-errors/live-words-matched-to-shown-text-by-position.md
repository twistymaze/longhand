---
title: Live speaker headings go on the wrong word when timed words match the shown text by position
date: 2026-09-29
category: logic-errors
module: Live transcript speaker rows
problem_type: logic_error
component: frontend
symptoms:
  - "In the open part, a Speaker N heading shows one or more words before or after the real speaker change."
  - "The wrong heading position continues until the model hypothesis agrees with the committed text again, or until the part saves."
  - "Without the segment offset, the headings of later capture segments compare words against turns from an earlier time."
  - "After a cut at the 8-minute limit, the 2 s replay words show two times."
root_cause: data_integrity
resolution_type: code_fix
severity: medium
related_components: ["service_layer"]
tags: ["live-transcript", "speaker-turns", "word-alignment", "word-timing", "sample-timeline", "replay-floor"]
---

# Live speaker headings go on the wrong word when timed words match the shown text by position

## Problem

The live transcript joins live words to live speaker turns in the UI (`src/session/liveRows.ts`). The first version of the plan matched the timed words of the open part to the shown text by position. The two lists are not the same list:

- The timed words of the open part come from the model's current raw hypothesis. `LiveStream::open_words` (`crates/asr/src/stream.rs:76`) is implemented by `open_words(&self.stream.snapshot())` (`crates/asr/src/engine.rs:371-372`). This reads the snapshot tokens (`crates/asr/src/engine.rs:306-313`).
- The shown text is the append-only committed text plus the tentative text (`OpenPart.committed` and `OpenPart.tentative`, `src/session/liveRows.ts:20-27`).

After the model revises a committed word, the hypothesis and the shown text have a different word count or a different word order. A match by position then gives a shown word the time of a different word. The speaker heading goes on the wrong word, and it stays there while the two lists disagree.

A second condition applies to all times. The two sources use different units and different zero points:

- Words are in milliseconds from the start of their capture segment (`TimedWord`, `src/session/liveRows.ts:12-13`).
- Speaker turns are 16 kHz samples on the note timeline (`SpeakerTurn`, `src/session/liveRows.ts:39-45`).

## Symptoms

- In the open part, a "Speaker N" heading shows one or more words before or after the real speaker change.
- The wrong heading position continues until the model's hypothesis agrees with the committed text again, or until the part saves.
- Without the segment offset, every word after the first capture segment compares against turns from an earlier time, so the headings of later segments are wrong.
- After a cut at the 8-minute limit, the next part replays the last 2 s of audio (`PartPolicy::default`, `crates/asr/src/stream.rs:165-173`). Without a filter, the words in that 2 s show two times and get two speaker checks.

## What Didn't Work

- **Match by position.** The first plan version assumed that the timed words and the shown words are the same list. The plan review showed that the open hypothesis and the shown text can differ (plan, `docs/plans/2026-09-29-1143-feat-live-speaker-rows-plan.md:93`). The plan changed to requirement R3a (line 57) and KTD8 (line 121): match by text.
- **Estimate word times in the UI.** Rejected (plan line 43). An estimate from text length gives no real relation to the audio, so a heading can go on the wrong word in the same way.
- **Join words to turns in the backend.** Rejected (plan line 43). It changes the processing pipeline. The user accepts more data sent to the UI, but no change to the pipeline approach.

## Solution

### 1. Align timed words to shown words by text

`openUnits` splits both lists into tokens, normalizes each token to lowercase with no punctuation, and aligns them (`src/session/liveRows.ts:219-241`):

```ts
const pairs = (part.words ?? []).flatMap((word) => tokens(word.text).map((text) => ({ text, word })));
const timed = pairs.map((pair) => pair.word);
const match = align(shown.map((s) => normal(s.text)), pairs.map((pair) => normal(pair.text)));
```

`normal` removes case and punctuation (`src/session/liveRows.ts:148`). `tokens` splits each character of an unspaced script into its own token (`src/session/liveRows.ts:131-147`), so Japanese and Chinese text aligns by character.

`align` (`src/session/liveRows.ts:151-188`) matches the equal head and the equal tail first. It then runs a longest common subsequence on the middle words. A size guard stops the LCS table when it is too large:

```ts
if (!a.length || !b.length || a.length * b.length > 1_000_000) return match;
```

Above the guard, only the head and tail matches stay, and the other shown words have no time.

### 2. A shown word with no match takes the speaker before it

A unit with no match gets `start: null` and `end: null` (`src/session/liveRows.ts:234-235`). `turnFor` returns null for such a unit (`src/session/liveRows.ts:322`). `buildLiveRows` then uses the speaker of the word before it (`src/session/liveRows.ts:385-388`):

```ts
const assigned: Assigned | null =
  unit.fixed ?? (turn ? { scope: turn.scope, speaker: turn.speaker, final: turn.speaker_final } : previous);
```

The row of an inherited word is marked provisional (`src/session/liveRows.ts:426`). When the part saves, its saved words have real times and split by speaker.

### 3. Convert word times to note samples

The backend sends `offset_ms`, the start of the capture segment in milliseconds from the start of the note, with each open-part event and each saved part (`LiveTranscriptEvent.offset_ms`, `src-tauri/src/managers/live.rs:108-110`; `LiveCurrent.offset_ms`, `src-tauri/src/managers/live.rs:210-212`). The UI adds it to each word time (`src/session/liveRows.ts:212-213`, `:234-235`), and `turnFor` multiplies by 16 samples per millisecond (`src/session/liveRows.ts:10`, `:323-325`):

```ts
const start = unit.start * SAMPLES_PER_MS;
const end = Math.max(unit.end * SAMPLES_PER_MS, start + 1);
```

The result is `note_sample = (offset_ms + word_ms) * 16`.

### 4. Remove the replay words in the backend

After a cut at the limit, `Parts::finish` keeps the words that start in the first half of the replay and sets a floor for the next part (`crates/asr/src/stream.rs:341-363`). `in_part` removes each word whose midpoint is before the floor (`crates/asr/src/stream.rs:388-402`):

```rust
if let Some(floor) = floor_ms {
    words.retain(|w| (w.start_ms + w.end_ms) / 2 >= floor);
}
```

`Parts::open_words` applies the same shift, clamp and floor to the open-part words (`crates/asr/src/stream.rs:266-269`). The live worker calls it before each text event (`crates/pipeline/src/live.rs:813-823`). The UI never gets the replay words.

## Why This Works

- The committed text is append-only, and the hypothesis changes only where the model revises words. Most words are equal in the two lists, so the head, tail and LCS match give almost every shown word the time of the same word in the hypothesis. A revised word has no match, so it gets no wrong time.
- A word with no time takes the speaker before it. This is the same rule as a word that no turn covers yet (R6), so the reader sees one consistent behaviour. The provisional mark tells the UI that the speaker can still change.
- The size guard limits the LCS table to 1,000,000 cells (4 MB as `Uint32Array`). The equal head and tail usually leave a small middle, so the guard rarely applies.
- `offset_ms` and the 16 kHz rate put words and turns on one timeline, so overlap in samples is a correct comparison.
- The floor filter uses the same rule for open-part words and for saved-part words. Thus the words that the UI shows for an open part match the words of the saved part that replaces it.

## Prevention

- Do not match two word lists by position when they come from different sources. The committed text and the stream snapshot are different sources, although they describe the same audio.
- Keep the unit and the zero point in each type comment (`TimedWord` and `SpeakerTurn`, `src/session/liveRows.ts:12-13`, `:39-45`). Convert at one place.
- Apply one filter function to open-part words and saved-part words (`in_part`), so the two cannot differ.

Tests in `src/session/liveRows.test.ts`:

- `a displayed word that the timed words do not have goes under the speaker before it (R3a)` (line 301): the shown text "We should go now." against the timed words "we should now". "go" takes Speaker 1, and "now." takes Speaker 2.
- `unspaced open-part text aligns by character` (line 319).
- `open-part text with no words shows under the last speaker, and the saved part splits into speaker rows (AE4)` (line 276).
- `an open part that a saved part replaces shows only once` (line 310).
- `a second epoch at minute 20 and a new segment at minute 25 each add one divider and restart at Speaker 1 (AE2)` (line 133): uses a segment offset.
- `each word of a long note takes the turn it overlaps` (line 202).

Tests in `crates/pipeline/tests/live.rs`:

- `after_a_cut_at_the_limit_the_open_part_words_leave_out_the_replay` (line 505): the second part sends no word before the floor, and each saved part keeps the words that the text calls sent.
- `each_text_call_sends_only_the_new_open_part_words` (line 462).
- `a_revised_word_sends_the_words_from_its_index` (line 488).
- `a_stream_without_timed_words_sends_the_text_with_no_words` (line 559).
- `live_word_times_match_the_assembled_file_before_and_after_a_dropped_second` (line 336).

## Related Issues

- [Live speaker rows plan](../../plans/2026-09-29-1143-feat-live-speaker-rows-plan.md): R3a, KTD2 and KTD8 record the design decisions.
- [Speaker labels in the live transcript](../../ideas/done/live-speaker-labels.md): the idea that PR #26 shipped.
- [Granite word timing was treated as absent](granite-word-timing-misclassified-as-absent.md): read the returned timing collection, not the capability flag. `open_words` follows this rule.
