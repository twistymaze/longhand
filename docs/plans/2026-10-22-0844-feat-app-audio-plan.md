---
title: App Audio - Plan
type: feat
date: 2026-10-22
topic: app-audio
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# App Audio - Plan

## Goal Capsule

- **Objective:** A user records a meeting on a call (Teams, Zoom, Discord, FaceTime or a browser) with Longhand only, and gets a transcript and summary as good as the user's notes recorded through a Rogue Amoeba Loopback device. No other audio app is necessary.
- **Means:** Longhand records the microphone plus the audio of the apps on a list that the user makes, as Loopback does. The note saves two channels and processes a plain mix of them.
- **Product authority:** This Product Contract. It replaces the Computer audio part of `docs/plans/2026-09-26-0827-feat-system-audio-and-microphone-choice-plan.md` (R6–R17 of that plan). The microphone choice of that plan stays as it is.
- **Open blockers:** None.

---

## Product Contract

### Summary

Settings gets a list of apps whose audio Longhand records. The list is empty on a new install. The user adds running apps one at a time, or adds the common call apps that are running in one step. Every note records the microphone plus the apps on the list. The note menu can add or remove an app for one note. The note saves the microphone and the apps as two channels and processes a plain mix of the two.

### Problem Frame

Longhand records one input. For calls, the user made a Loopback virtual device, "Mic & Meeting Audio", that mixes the MacBook Pro microphone with Discord, FaceTime, Google Chrome, Teams, Zoom and other apps into one stereo stream. Longhand records that device. For several weeks this gave good transcripts and summaries. The user always heard the calls on the speakers, so the microphone also heard the call. Another user would have to buy Loopback and build the same device.

Longhand already has parked system audio capture (`docs/ideas/parked/system-audio-capture.md`). It records all audio that the computer plays. It is hidden because two Teams test calls failed Verify:

- A mixer app (Elgato Wave Link) played the call again about four times, and it played a late copy of the microphone onto the system channel.
- Speaker echo gave false hits in the microphone-copy check.

The parked fix was a new processing design for each channel. The Loopback weeks show a simpler path. A list of chosen apps keeps mixer apps out, and a plain mix of the microphone and those apps transcribes well, also with speaker echo.

### Key Decisions

- KD1. **Record the apps on a list, not all computer audio.** (session-settled: user-directed — chosen over the parked whole-computer capture: the list keeps mixer apps out and copies the proven Loopback setup.) Governs R1, R6, R8.
- KD2. **Every note records the apps on the list. There is no Computer audio switch for each note.** (session-settled: user-directed — chosen over a switch for each note and over "always on, with an off switch": an app that is not in a call makes no sound, and the user has nothing to remember.) Governs R6.
- KD3. **The list is empty on a new install. App audio is recorded only after the user adds an app.** (session-settled: user-directed — chosen over a list of installed call apps that is filled in advance: Longhand must not record other apps until the user explicitly asks.) Governs R1, R3.
- KD4. **The user adds only apps that are running.** (session-settled: user-directed — chosen over any installed app: a running app shows the processes that actually play its sound, and this is how Loopback works.) Governs R2, R3.
- KD5. **The note saves two channels and processes a plain mix of them.** (session-settled: user-approved — chosen over one mixed stream after the tradeoff was shown: the mix is the same as a live mix, so the risk is the same as with Loopback, and per-channel processing stays possible later at about twice the file size.) Governs R12, R13, R14.
- KD6. **No echo removal, no level balancing and no microphone-copy check.** (session-settled: user-approved — the user's Loopback notes, always on speakers, had none of these and were good.) Governs R14.
- KD7. **A change in the note menu applies to that note only.** (session-settled: user-directed — chosen over a change that also updates the Settings list: one rule for the Microphone and app audio menus.) Governs R10.
- KD8. **A change to the Settings list during a recording applies from the next note.** (session-settled: user-approved — confirmed in the scope summary: the note menu changes the current note.) Governs R11.
- KD9. **App audio replaces the parked Computer audio feature.** (session-settled: user-approved — confirmed in the scope summary.) Governs R18.
- KD10. **Windows is in scope.** (session-settled: user-approved — confirmed in the scope summary, from "cross-platform correctness from day one" in `STRATEGY.md`.) Governs R17.
- KD11. **On Windows, app audio needs Windows 11.** (session-settled: user-directed — chosen over a whole-device capture on Windows 10: per-process capture needs build 20348 or later, Windows 10 support ended in October 2025, and a whole-device capture brings back the mixer-app problem.) Governs R17.

---

### Requirements

**The app list**

- R1. The Recording pane in Settings holds a list of apps whose audio Longhand records. The list is empty on a new install.
- R2. The user adds an app from a list of the running apps that are not on the list yet. Longhand reads the running apps each time this list opens. When there is no app to add, the control is disabled with a short reason.
- R3. One action adds the common call apps that are running at that moment and are not on the list yet. When there is no such app, the action is disabled with a short reason.
- R4. The user can remove an app from the list.
- R5. Each app on the list shows its name and icon, also when the app is not running.

**Recording**

- R6. Every note records the microphone plus the audio of each app on the list, from the start of the note.
- R7. An app on the list does not have to run. When it starts, quits or starts again during a note, Longhand records it while it plays, and the microphone recording continues with no gap. The recording page shows no message for this.
- R8. The audio of an app includes the sound of all its processes, also the helper processes that play its sound (for example, Chrome tabs).
- R9. Longhand's own sounds, for example the alert sound, are never in the app audio.

**Changes during a recording**

- R10. During a recording, the note menu has an app audio submenu next to Microphone. It lists the running apps and also the apps that the note records but that are not running, with a check mark next to each app that the note records. Choosing an app starts or stops its recording at once, for this note only. Only running apps can be added. The Settings list does not change.
- R11. A change to the Settings list during a recording applies from the next note.

**Saved audio and processing**

- R12. From the first moment that a recorded app plays, the note saves the microphone on the left channel and the app audio on the right channel. Before that moment, the note saves only the microphone. From that moment, the right channel holds silence while no recorded app plays. A note in which no recorded app plays saves its audio as today.
- R13. With a streaming model and the Live transcript switch on, the live transcript reads a plain mix of the microphone and the app audio.
- R14. Processing after Stop and Transcribe Again use the same plain mix as R13. They do not remove echo, balance levels or check for copies of the microphone.

**Permission and failures**

- R15. On macOS, Longhand requests the "Screen & System Audio Recording" permission when the user adds the first app, in Settings or in the note menu. The General pane shows this permission next to the microphone permission.
- R16. When the permission is denied or app audio fails during a note, the microphone recording continues. The recording page says that app audio is not recorded and, for a denied permission, how to allow it. This includes a denial when the user adds the first app in the note menu. There is no message when no app is on the list or checked, or when a recorded app is not running or makes no sound. The messages of the parked Computer audio feature for silence, another output device and a copy of the microphone are removed.

**Platforms and the parked feature**

- R17. App audio works on macOS and on Windows 11. On older Windows, the app list in Settings is disabled with a short reason, and notes record the microphone only.
- R18. The parked Computer audio feature is removed: the Computer audio default in Settings, the "Record Computer Audio" item in the note menu, and the whole-computer capture.

---

### Acceptance Examples

- AE1. **Covers R6, R13, R14.** Given Teams on the list and a Teams call on the laptop speakers, when the user stops the note, the transcript holds the remote people and the user, and its quality is equal to a note recorded through the user's Loopback device.
- AE2. **Covers R7.** Given Zoom on the list and not running when the note starts, when the user joins a Zoom call 10 minutes later, the note records Zoom from that time, and the recording page shows no message.
- AE3. **Covers R10, R11.** Given a note that records with Discord not on the list, when the meeting moves to Discord and the user checks Discord in the note menu, the note records Discord from that time. The next note does not record Discord.
- AE4. **Covers R1, R6.** Given Elgato Wave Link running and not on the list, when a call on a listed app is recorded, the app audio channel holds no sound that Wave Link plays again and no late copy of the microphone. Call sound that the microphone hears from the speakers is expected in the microphone channel and in the mix.
- AE5. **Covers R16.** Given the permission is denied and Teams on the list, when a note starts, the microphone records, and the recording page says that app audio is not recorded and how to allow it.
- AE6. **Covers R12, R16.** Given an empty list, when the user records a room meeting, the note is the same as today: one channel and no message.
- AE7. **Covers R8.** Given Chrome on the list and a video playing in a Chrome tab during a room meeting, the note records the video audio. This is expected.

---

### Success Criteria

- The user stops using Loopback for meeting notes, and the notes are as good as before.

### Scope Boundaries

- Per-app volume, mute and on/off switches, as in Loopback. To stop recording an app during a note, the user unchecks it in the note menu (R10). A removal from the Settings list applies from the next note (R11).
- Apps that are installed but not running in the add list.
- Recording of all computer audio, and the exclusion of mixer apps.
- Echo removal, level balancing and the microphone-copy check.
- Per-channel recognition and diarization, and Room/Call labels on speakers. The two-channel recording keeps these possible later ([per-channel processing](../ideas/parked/system-audio-capture.md#process-each-channel-on-its-own)).
- A message or hint when the list is empty.
- A choice of output device.

### Dependencies / Assumptions

- The existing Core Audio process tap (macOS 15.0 or later, KD15 of the earlier plan) can record a chosen set of processes.
- On Windows, app audio uses per-process loopback, which needs Windows 11 (build 20348 or later).
- The parked capture code can be used again: the permission check, the two-channel recording and the capture restarts. Planning decides what to keep.

### Outstanding Questions

**Deferred to Planning**

- Which apps does "add the common call apps" add? Assumption: dedicated call apps (for example Teams, Zoom, Discord, Slack, Webex, FaceTime), and no browsers, because a browser records every tab.
- How does Longhand find the helper processes of an app, also for an app that plays sound through a separate helper app outside its own bundle? Check Chrome, Teams, Zoom, FaceTime (call audio comes from the avconferenced system service) and Safari (web audio comes from a WebKit helper process).
- Earlier research found that new Teams gives silence to per-app capture on Windows. Does this still apply, and what is the fix?
- Which running apps does the add list show: all running apps, or only apps that have an audio process?
- How are the microphone and the app audio aligned in time, for the live mix and the saved channels alike?

### Sources / Research

- `docs/ideas/parked/system-audio-capture.md` — why the parked feature is hidden, the mixer-app findings, and per-channel processing.
- `docs/plans/2026-09-26-0827-feat-system-audio-and-microphone-choice-plan.md` — the parked Computer audio plan and the microphone choice.
- `docs/solutions/integration-issues/tcc-preflight-stale-after-tap-prompt-grant.md` — the permission check runs again in a child process.
- `crates/audio-system/src/macos.rs:120-165` — the tap lists the Core Audio process objects with their bundle IDs and excludes Longhand and the mixer apps. App audio needs an include list.
- `crates/audio-system/src/windows.rs` — WASAPI loopback of one output device; no per-process loopback yet.
- `src-tauri/src/managers/capture.rs:1376` (`SYSTEM_AUDIO_AVAILABLE`) and `src/lib/features.ts:8` (`systemAudio`) — the switches that hide the parked feature.
- `src/session/SessionPage.tsx:580-605` — the note menu's Microphone submenu, the pattern for R10.
