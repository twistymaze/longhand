---
title: System Audio and Microphone Choice - Plan
type: feat
date: 2026-09-26
topic: system-audio-and-microphone-choice
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# System Audio and Microphone Choice - Plan

## Goal Capsule

- **Objective:** A note records a meeting on a call as completely as a meeting in a room. The remote people, the people in the room and the user are all in the transcript, whether the user wears headphones or uses the speakers, and whichever call app or browser tab the call uses. The user also records from the microphone of their choice, not only the system default.
- **Product authority:** This plan covers the microphone choice, the Computer audio switch, the stereo recording, echo removal and the mono mix for processing. It is area D of `docs/plans/2026-09-24-1956-feat-ux-redesign-plan.md` (How This Work Fits Together). Room/Call marks on transcript turns and per-channel diarization are not active scope. The Product Contract wins on behavior.
- **Open blockers:** None.

---

## Product Contract

### Summary

Settings gets a Recording pane. It holds the usual microphone and the default of a new Computer audio switch. The recording page shows the microphone in use, can change it for one note, and has the Computer audio switch, which records everything the computer plays. Longhand saves the raw microphone and the raw system audio as the two channels of one recording. The live transcript reads a live mono mix. After Stop, processing removes speaker echo from the microphone, mixes the two channels to mono, and transcribes again only the spans where it found echo.

### Problem Frame

Longhand records one microphone, the system default (R2 of `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`). `STRATEGY.md` names sessions "in a room or on a call", but on a call the note gets the remote people only as sound from the laptop speakers that the microphone hears. With headphones the note does not get them at all. The user's calls use Teams, Zoom, Discord and Google Meet in a browser. The user sometimes wears headphones and sometimes does not, and sometimes people are also in the room.

The user also cannot choose a microphone. The capture code can already open an input by name (`crates/audio-cpal/src/config.rs`), but every call site passes no name, and Settings has no microphone field (`src-tauri/src/managers/settings.rs`). A user with a USB conference microphone or a headset must change the system default before each meeting.

Since the recognition models plan (`docs/plans/2026-09-25-1121-feat-recognition-models-and-live-transcript-plan.md`), a streaming model's live text is the final transcript, and processing after Stop fills only the gaps. Any new audio source must reach the live transcript, or the fast finish loses it.

### Key Decisions

- KD1. **Computer audio records all system audio, not one app.** (session-settled: user-directed — chosen over a choice of apps and over an automatic start when a call app runs: calls use several apps and browser tabs, and an app list fails for Meet in a browser.) anarlog does the same. Governs R7.
- KD2. **The user turns Computer audio on or off for each note. Settings holds the default.** (session-settled: user-directed — chosen over always on: a room meeting does not pick up alert sounds, videos or music.) Governs R6, R8.
- KD3. **Computer audio can change at any time during a recording.** (session-settled: user-directed — chosen over a fixed choice at the start: a remote person can join a room meeting late.) Governs R8.
- KD4. **Longhand saves the raw microphone and the raw system audio as two channels, and processing uses a mono mix.** (session-settled: user-directed — chosen over per-channel recognition and diarization, as anarlog does: the existing pipeline stays unchanged. The stereo recording keeps per-channel processing possible later.) Governs R11, R14.
- KD5. **Echo removal runs after Stop, on the saved raw audio, not live.** (session-settled: user-directed — chosen over live echo removal as in anarlog, and over a muted microphone while the call speaks: recording stays as light as today, the raw audio is always kept, and a better canceller can process old notes again. Muting loses room speech that overlaps the call.) Governs R12, R13.
- KD6. **The live transcript reads a live mono mix. After Stop, only the spans with echo are transcribed again.** (session-settled: user-directed — chosen over live echo removal for the live feed and over no live transcript when Computer audio is on: with headphones the live text stays final. With speakers, echo spans can cover most of a call, so the extra cost after Stop is up to one recognition pass of the call.) Governs R15, R16.
- KD7. **The live mix uses anarlog's cheap echo gate on the microphone.** (session-settled: user-approved — proposed at the scope confirmation after the anarlog comparison: it keeps the call's voices from reaching the live transcript twice, at almost no cost.) Governs R15.
- KD8. **The usual microphone is chosen in Settings, and the recording page can change it for one note.** (session-settled: user-directed — chosen over Settings only, as in Handy, and over the recording page only: the shortcut still starts at once with the usual microphone.) Governs R1, R2, R3.
- KD9. **A chosen microphone that is missing is replaced by the system default, and the choice is kept.** (session-settled: user-directed — chosen over Handy's reset of the setting to Default and over a refusal to start: a USB microphone left at the office must not erase the choice or delay the note.) Governs R4.
- KD10. **A chosen microphone that disconnects during a note is replaced by the system default at once, and Longhand moves back when it returns.** (session-settled: user-directed — chosen over staying on the default and over today's pause: nothing is lost, and the note ends on the better microphone.) Governs R5.
- KD11. **The mix balances the levels of the two channels.** (session-settled: user-approved — proposed at the scope confirmation: a quiet call or a loud room must not make the other source too quiet to recognize. anarlog applies no gain, but it never mixes.) Governs R14, R15.
- KD12. **Windows is in scope, through WASAPI loopback.** (session-settled: user-approved — proposed at the scope confirmation, following "cross-platform correctness from day one" in `STRATEGY.md`.) Governs R7.
- KD13. **The Computer audio default in Settings is off.** A room meeting is the main case (`STRATEGY.md`), and off keeps today's behavior for an existing install. Governs R6.

---

### Requirements

**Microphone choice**

- R1. The Recording pane in Settings lists the connected inputs and "System default", and stores the user's choice. "System default" is the choice on a new install and after the update.
- R2. The recording page shows the microphone in use. Before or during a recording the user can change it for this note, and the change does not change the choice in Settings.
- R3. The recording shortcut and New Note start at once with the microphone from Settings. The choice adds no step and no noticeable delay to the start of a recording.
- R4. When the chosen microphone is missing at the start of a note, the note records from the system default, and the recording page shows which microphone it uses. The choice in Settings stays. The next note uses the chosen microphone again when it is connected.
- R5. When the chosen microphone disconnects during a note, recording continues on the system default at once, and the recording page shows the change. When the chosen microphone returns during the note, recording moves back to it. When the lost input was the system default and no input remains, today's behavior applies: the note is interrupted and waits for an input to return.

**Computer audio**

- R6. The Recording pane holds the default of the Computer audio switch. Each new note starts with that default.
- R7. When Computer audio is on, the note records all audio that the computer plays, from any app, on macOS and on Windows. On Windows this includes the default communications device when it differs from the default output device, because call apps often play the call there.
- R8. The recording page shows the Computer audio switch. The user can turn it on or off at any time during a recording, and the note records system audio only while it is on.
- R9. The first time Computer audio turns on, macOS asks for the "Screen & System Audio Recording" permission. The General pane shows this permission next to the microphone permission and lets the user grant it in advance.
- R10. When the permission is denied, or system audio fails during a note, the microphone recording continues. The recording page says that computer audio is not recorded, and for a denied permission it tells the user how to allow it. While the macOS permission prompt waits for an answer, the recording page shows the same message.

**Recording**

- R11. A note that had Computer audio on saves the raw microphone and the raw system audio as two channels of one recording: the microphone on the left, system audio on the right. Spans with Computer audio off hold silence on the right. This recording is the note's saved audio in its home folder. Its compression keeps the two channels independent, each at today's mono quality, so neither channel holds audio from the other. A note that never had Computer audio on saves its audio as today.

**Processing**

- R12. After Stop, processing removes the echo of the system audio from the microphone channel where it finds echo. Where it finds no echo, for example because the user wore headphones, it leaves the microphone unchanged.
- R13. The saved recording keeps the raw channels. Echo removal changes only the audio that processing uses.
- R14. Processing mixes the two channels to mono with balanced levels, so that neither source is too quiet to recognize next to the other. Recognition and speaker detection run on the mix as they run on the microphone today.
- R15. With a streaming model and the Live transcript switch on, the live transcript reads a live mono mix of the microphone and system audio, with levels balanced as in R14. A microphone chunk that is mostly echo of the system audio is left out of the live mix. The gate never changes the saved recording.
- R16. After Stop, each span where processing found echo gets recognition again on the cleaned mix, and that text replaces the live text for the span. Spans without echo keep the live text, as the fast finish requires (R11 of the recognition models plan).
- R17. Transcribe Again on a note with both channels uses the cleaned mix.

---

### Acceptance Examples

- AE1. **Covers R7, R12, R16.** Given a Teams call with headphones and Computer audio on, when the user stops, processing finds no echo, the live text stays, and the transcript holds the remote people and the user.
- AE2. **Covers R12, R14, R15, R16.** Given a Zoom call on the laptop speakers with Computer audio on, the live transcript does not show the remote people's words twice. After Stop, the spans with echo are transcribed again from the cleaned mix, and each remote sentence appears once.
- AE3. **Covers R8, R11, R14.** Given a room meeting with Computer audio off, when a remote person joins after 20 minutes and the user turns Computer audio on, the transcript holds the room for the whole note and the remote person from minute 20.
- AE4. **Covers R4.** Given a USB microphone chosen in Settings and not connected, when the user presses the shortcut, recording starts at once on the system default, and the recording page names the default microphone. After the user connects the USB microphone, the next note uses it.
- AE5. **Covers R5.** Given a note recording on a chosen headset, when the headset goes out of range, recording continues on the built-in microphone with no lost audio beyond the switch. When the headset returns, recording moves back to it.
- AE6. **Covers R9, R10.** Given the permission was denied, when the user turns Computer audio on during a note, the microphone recording continues, and the recording page says computer audio is not recorded and how to allow it.
- AE7. **Covers R11.** Given a note with Computer audio off from start to Stop, its saved audio and its processing are the same as today.
- AE8. **Covers R2.** Given a note recording on the usual microphone, when the user chooses another microphone on the recording page, recording continues on it, and the choice in Settings does not change.

---

### Scope Boundaries

- Room/Call marks on transcript turns, and speaker detection for each channel. The stereo recording keeps these possible later.
- Capture from one app only, and a choice of output device.
- Live echo removal. The live mix uses only the gate in R15.
- A microphone default for each folder.
- Echo removal and a new mix for notes recorded before this work. Those notes have one channel.

---

### Dependencies / Assumptions

- Longhand has no system audio capture yet. `open_speaker_capture` in `crates/audio-cpal` fails on every call, and a test asserts it. The macOS backend follows anarlog's Core Audio process tap (`reference-source/anarlog/crates/audio-actual/src/speaker/macos.rs`), and the Windows backend follows its WASAPI loopback (`speaker/windows.rs`).
- Core Audio process taps need macOS 14.2 or later, and `src-tauri/tauri.conf.json` sets the minimum to 13.0. On an older macOS, Computer audio is not available (unverified; planning confirms the version and the message).
- The recorder writes only mono today: the recovery chunks and the Opus archive are both one channel (`crates/recorder/src/wav.rs`, `crates/pipeline/src/assemble.rs`). R11 needs new recorder and archive code, not only a mix step.
- The app declares only the microphone permission (`src-tauri/Info.plist`). R9 needs a new usage description for system audio capture. Like the microphone, it must be tested from the signed bundle.
- The system audio permission is not the Screen Recording permission, and macOS has no public API to check it. The permissions plugin that Longhand uses checks only Screen Recording. A tap without the permission records silence and returns no error. So R9 and R10 follow anarlog: a private preflight check of the permission (`reference-source/anarlog/crates/tcc`) and a short test tap to ask for it. This borrowed code needs a `PROVENANCE.md` record.
- anarlog's echo canceller is MIT-licensed (`LICENSES/anarlog-MIT.txt`), and Longhand has an ONNX crate (`crates/onnx`). Borrowed code and model files need a `PROVENANCE.md` record. The licence of the model weights needs a check.
- The live worker reads the microphone frames that the recorder copies to it (`src-tauri/src/managers/capture.rs`, KTD3 of the recognition models plan). R15 changes what it reads to the live mix.

---

### Outstanding Questions

**Deferred to Planning**

- How processing detects echo and sets span edges for R12 and R16, and how short a span can be.
- How the two capture clocks stay aligned over a four-hour note, and what happens when one source falls behind (anarlog pairs chunks and pads with silence).
- Where the Recording pane sits in the Settings sidebar, and how the microphone name and the Computer audio switch fit the recording page's attention budget (ux-redesign KD2).
- Whether a microphone change on the recording page starts a new segment.
- Whether R16 can recognize again only the spans where the live gate removed microphone speech, so that a call on speakers keeps more of the fast finish.

---

### Sources / Research

- `reference-source/Handy/src-tauri/src/managers/audio.rs` (`desired_microphone`, `persist_default_microphone_after_fallback`) and `src/components/settings/MicrophoneSelector.tsx`: Handy's microphone setting, its cached name lookup, its fallback and its dropdown. R4 differs: Longhand keeps the choice.
- `reference-source/anarlog/crates/audio-actual/src/capture/stream.rs` and `joiner.rs`: pairing of microphone and speaker chunks, the reference aligner and `cancel_linear_echo`.
- `reference-source/anarlog/crates/aec/`: the two-stage ONNX echo canceller (about 7.3 MB at the default size, CPU).
- `reference-source/anarlog/crates/listener-core/src/actors/source/stream.rs`: echo removal off for headphone output. `recorder/disk.rs`: the stereo file, microphone left and speaker right.
- `reference-source/anarlog/crates/owhisper-client/src/local_soniqo_live.rs` (`suppress_echo_dominant_mic`): the live echo gate in R15.
- `reference-source/anarlog/plugins/permissions/src/guidance.rs`: the Screen Recording permission for system audio.
- `crates/audio/src/lib.rs`: `CaptureFrame` and `AudioProvider` already name a microphone and a speaker stream.
- `src-tauri/src/managers/capture.rs` (`watch_for_device`, `device_returned`): today's device-loss path, which R5 keeps for the case of no input.
- `docs/plans/2026-09-25-1121-feat-recognition-models-and-live-transcript-plan.md`: the live worker, parts, gap fill and Transcribe Again that R15 to R17 build on.
