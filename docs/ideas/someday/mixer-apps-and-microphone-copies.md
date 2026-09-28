---
title: Mixer apps and copies of the microphone in system audio
description: "Keep mixer apps such as Wave Link out of the system channel, and detect copies of the microphone"
area: capture
added: 2026-09-26
source: "PR #12 signed-bundle tests with Elgato Wave Link"
related: [system-audio-capture, per-channel-processing]
---

# Mixer apps and copies of the microphone in system audio

## Idea
Keep the system channel free of the audio that a mixer app plays again, and warn when the system channel still holds a copy of the microphone.

## Why
A global process tap records each app's own output on every device. A mixer app, such as Elgato Wave Link, reads the output of other apps and plays it again to several devices. It also plays the microphone into its virtual mixes (Chat Mix, Stream Mix). On 2026-09-26 the tap recorded the call about four times and the user's voice twice, 136 and 142 ms late. The mono mix was not recognizable, and Verify failed.

## Current state (PR #12, hidden with Computer audio)
- **Exclusion:** the macOS tap excludes the known mixer apps: `com.elgato.WaveLink3`, `com.elgato.WaveLink`, `com.obsproject.obs-studio`, and every `com.rogueamoeba.` app, including `arkaudiod`. It reads the list again at each device check (`crates/audio-system`, `is_mixer`). A test call then gave digital silence on the system channel while the user spoke.
- **Copy check:** a check on each stereo segment sets a flag, and the recording page warns (`crates/audio/src/mic_copy.rs`).
  - Its correlation test gave false hits on faint speaker echo and on tones.
  - A GCC-PHAT rule works on both test notes. The rule is: a peak ratio of 30 or more, a delay of 80 to 400 ms, and at least 8 hits in 30 s that agree on the delay within ±10 ms. It gave 1 hit on the note with no mixer and 69 on the Wave Link note.
  - This rule is tested in Python only. Put it into the Rust check if the new processing design keeps the check.

## Notes
- **Research (2026-09-26):**
  - No meeting recorder that we found excludes mixer apps.
  - Voicemod's official fix for a doubled voice in OBS is to capture "all audio except VoicemodDesktop.exe".
  - Elgato documents the same doubling for Discord screen share with Wave Link 3.
  - Per-app capture (a list of call apps) has its own failures: browser calls come from helper processes, and new Teams gives silence to process loopback on Windows.
- **Windows:** Wave Link makes one virtual output for each channel. Loopback of the default output then gets only the apps routed to it, and a call app routed to another channel is missed. The other-output warning covers that. This is not tested on Windows.
- **Unknown mixers:** a mixer that is not on the list still causes the problem. The copy check is the backup.
