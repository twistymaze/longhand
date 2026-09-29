---
title: System audio capture
description: "Record the computer's audio with the microphone; built in PR #12 and hidden. Includes mixer apps and the per-channel processing design"
area: capture
added: 2026-09-21
source: "Deferred from the walking skeleton (docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md, Scope Boundaries and How This Work Fits Together)"
related: [live-transcript-view, faster-diarization]
---

# System audio capture

## Idea
A second capture path for the computer's own audio, alongside the microphone, with its permission surface. The same work lets the user choose the microphone instead of only the system default.

## Why
Needed for meetings held on calls. Irrelevant to an in-person room, which is why the skeleton left it out.

## Why parked
The capture works, but processing a stereo note is not reliable. The mono mix makes a fault in one channel a fault in the whole note, and two Teams test calls failed Verify. It waits for a new processing design ([per-channel processing](#process-each-channel-on-its-own)). Faster diarization, which that design needed first, shipped in PR #18.

## Current state
- Planned in `docs/plans/2026-09-26-0827-feat-system-audio-and-microphone-choice-plan.md` and built in PR #12:
  - a Computer audio switch for each note;
  - a stereo recording of the raw microphone and the raw system audio;
  - a live mix with an echo gate, and echo removal after Stop;
  - a mono mix for the existing pipeline.
- The microphone choice ships. Computer audio is **hidden and turned off** (`SYSTEM_AUDIO_AVAILABLE` in `src-tauri/src/managers/capture.rs`, `features.systemAudio` in `src/lib/features.ts`). The capture code, the processing code and their tests stay in the tree.
- Why it is hidden: on 2026-09-26 two Teams test calls in the signed bundle failed Verify, each for a different reason. The mono mix makes a fault in one channel a fault in the whole note:
  - A mixer app put a late copy of the microphone and extra copies of the call on the system channel (see [Mixer apps and copies of the microphone](#mixer-apps-and-copies-of-the-microphone) below).
  - A speaker echo gave false hits in the copy check.
- Next: design the processing of a stereo note again. The likely design is [per-channel processing](#process-each-channel-on-its-own). It needed [faster diarization](../done/faster-diarization.md) first, which shipped in PR #18.
- Windows follows one output device (anarlog's rule). When the followed device is silent while another plays, the recording page warns.

## Mixer apps and copies of the microphone
Keep the system channel free of the audio that a mixer app plays again, and warn when the system channel still holds a copy of the microphone.

### Why
A global process tap records each app's own output on every device. A mixer app, such as Elgato Wave Link, reads the output of other apps and plays it again to several devices. It also plays the microphone into its virtual mixes (Chat Mix, Stream Mix). On 2026-09-26 the tap recorded the call about four times and the user's voice twice, 136 and 142 ms late. The mono mix was not recognizable, and Verify failed.

### Current state (PR #12, hidden with Computer audio)
- **Exclusion:** the macOS tap excludes the known mixer apps: `com.elgato.WaveLink3`, `com.elgato.WaveLink`, `com.obsproject.obs-studio`, and every `com.rogueamoeba.` app, including `arkaudiod`. It reads the list again at each device check (`crates/audio-system`, `is_mixer`). A test call then gave digital silence on the system channel while the user spoke.
- **Copy check:** a check on each stereo segment sets a flag, and the recording page warns (`crates/audio/src/mic_copy.rs`).
  - Its correlation test gave false hits on faint speaker echo and on tones.
  - A GCC-PHAT rule works on both test notes. The rule is: a peak ratio of 30 or more, a delay of 80 to 400 ms, and at least 8 hits in 30 s that agree on the delay within ±10 ms. It gave 1 hit on the note with no mixer and 69 on the Wave Link note.
  - This rule is tested in Python only. Put it into the Rust check if the new processing design keeps the check.

### Notes
- **Research (2026-09-26):**
  - No meeting recorder that we found excludes mixer apps.
  - Voicemod's official fix for a doubled voice in OBS is to capture "all audio except VoicemodDesktop.exe".
  - Elgato documents the same doubling for Discord screen share with Wave Link 3.
  - Per-app capture (a list of call apps) has its own failures: browser calls come from helper processes, and new Teams gives silence to process loopback on Windows.
- **Windows:** Wave Link makes one virtual output for each channel. Loopback of the default output then gets only the apps routed to it, and a call app routed to another channel is missed. The other-output warning covers that. This is not tested on Windows.
- **Unknown mixers:** a mixer that is not on the list still causes the problem. The copy check is the backup.

## Process each channel on its own
For a note with Computer audio, recognize and diarize the microphone channel and the system channel separately, then join the two transcripts by time. Each speaker keeps its channel ("Room: Speaker 1", "Call: Speaker A"). This replaces the mono mix of PR #12 (KD4).

### Why
- **The mix is fragile.** The mono mix works only when four things are right: clean echo removal, aligned channels, a level balance that fits both sources, and no copy of the microphone on the system channel. When one of them fails, the words are not recognizable and Verify fails the note.
- **Separate channels degrade gradually.** With separate channels, a fault gives duplicate lines or a wrong label, not a failed note.
- **The labels are more useful.** The transcript can show which side of the call a speaker is on.

### Notes
- **anarlog's assumption:** anarlog does this with one assumption that Longhand must not make. It treats the microphone as one person ("You") and diarizes only the system channel. Longhand's main case is a room with several people plus a call, so both channels need diarization. The people on the two channels are different, except through echo.
- **Echo removal:** the microphone still needs echo removal after Stop (the echo spans and the DTLN-aec pass exist), but only to prevent duplicate lines.
- **Cost:** about twice the recognition time, and twice the diarization time, for a note with Computer audio. A note with only the microphone does not change. Depends on [faster diarization](../done/faster-diarization.md).
- **Stages that change:** Transcribe runs once per channel, Diarize runs once per channel, Join merges by time, and Verify checks each channel.
- **Data to test with:** the stereo recordings of the test notes of 2026-09-26 are kept, so a new design can process them again.
- **anarlog reference:** `crates/listener-core`, `crates/owhisper-client/src/live.rs`, `crates/transcript/src/label.rs` and `crates/listener2-core/src/batch/diarize.rs` in the anarlog clone.
- **Shorthand reference (reviewed 2026-09-29 at `90b8d40`):** Shorthand keeps the microphone and the system audio in separate lanes. Each lane has its own VAD and its own streaming recognizer. A merger puts the committed text in order of arrival and labels it "Me" or "Them" (`src-tauri/src/managers/transcription.rs:85-287`). This supports per-channel processing. Do not take its capture method: it taps only the default output device that was set at open, excludes no process, has no echo or mixer handling, polls the TCC preflight in the same process (the stale pattern in [the TCC solution](../../solutions/integration-issues/tcc-preflight-stale-after-tap-prompt-grant.md)), and was not tested in a signed bundle. Longhand's parked capture is equal or better in each of these areas.
- **Smallest first step (no capture code):** split the kept stereo recordings of 2026-09-26 into the microphone channel and the system channel. Run Transcribe and Diarize on each channel, join by sample time, and count the duplicate lines from echo and from the Wave Link copy of the microphone. If Verify passes, turn on `SYSTEM_AUDIO_AVAILABLE` behind the new processing.
