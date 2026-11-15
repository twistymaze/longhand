---
title: Tell the user when the microphone sent only silence
description: "When every sample of a recording is exactly zero (for example a muted microphone), say so on the note instead of showing an empty transcript"
area: capture
added: 2026-10-31
source: "upkeep review 2026-10-31: Handy 0e5e9ca"
related: []
---

# Tell the user when the microphone sent only silence

## Idea
When audio arrived but every sample was exactly zero, mark the recording as silent input and tell the user on the note. Do not show an empty transcript with no reason.

## Why
A microphone that the operating system or a hardware switch mutes sends frames of exact zeros. Longhand records them and makes a note with no words and no message. The user must do something (unmute and record again), so the message rule permits a message. Handy added this check in 0e5e9ca (`input_silent`).

## Notes / Current state
- The recording rail already shows "quiet" after 20 s of near-silent frames (`src/recording/inputStatus.ts`, R12). That status is live only, and it does not separate a quiet room from a muted microphone.
- Exact zeros are a stronger signal than a low level: a real room always has some noise. The check can run in the recorder and also make the live rail say "muted" instead of "quiet".
- Added 2026-11-15 (upkeep review, Handy AirPods mode): while an iPhone plays audio through AirPods, macOS can open the AirPods microphone and deliver only zero samples. Handy adds an opt-in, experimental macOS setting that uses handy-recorder's `take_headset` option to move the headset to the Mac during a recording. `crates/audio-cpal/src/config.rs` already swaps a Bluetooth default input for a wired or built-in one when there is one, but a user who picks the AirPods microphone, or has no other input, can get the same silent recording. The silent-input message helps here first; the headset move needs capture code and a signed-build test.
