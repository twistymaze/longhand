---
title: Process each channel of a stereo note on its own
status: open
area: processing
added: 2026-09-26
source: "PR #12 signed-bundle tests and the discussion of anarlog's two-stream design"
related: [system-audio-capture, faster-diarization, mixer-apps-and-microphone-copies]
---

# Process each channel of a stereo note on its own

## Idea
For a note with Computer audio, recognize and diarize the microphone channel and the system channel separately, then join the two transcripts by time. Each speaker keeps its channel ("Room: Speaker 1", "Call: Speaker A"). This replaces the mono mix of PR #12 (KD4).

## Why
- **The mix is fragile.** The mono mix works only when four things are right: clean echo removal, aligned channels, a level balance that fits both sources, and no copy of the microphone on the system channel. When one of them fails, the words are not recognizable and Verify fails the note.
- **Separate channels degrade gradually.** With separate channels, a fault gives duplicate lines or a wrong label, not a failed note.
- **The labels are more useful.** The transcript can show which side of the call a speaker is on.

## Notes
- **anarlog's assumption:** anarlog does this with one assumption that Longhand must not make. It treats the microphone as one person ("You") and diarizes only the system channel. Longhand's main case is a room with several people plus a call, so both channels need diarization. The people on the two channels are different, except through echo.
- **Echo removal:** the microphone still needs echo removal after Stop (the echo spans and the DTLN-aec pass exist), but only to prevent duplicate lines.
- **Cost:** about twice the recognition time, and twice the diarization time, for a note with Computer audio. A note with only the microphone does not change. Depends on [faster diarization](faster-diarization.md).
- **Stages that change:** Transcribe runs once per channel, Diarize runs once per channel, Join merges by time, and Verify checks each channel.
- **Data to test with:** the stereo recordings of the test notes of 2026-09-26 are kept, so a new design can process them again.
- **anarlog reference:** `crates/listener-core`, `crates/owhisper-client/src/live.rs`, `crates/transcript/src/label.rs` and `crates/listener2-core/src/batch/diarize.rs` in the anarlog clone.
