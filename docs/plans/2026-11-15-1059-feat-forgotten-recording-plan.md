---
title: Forgotten Recording - Plan
type: feat
date: 2026-11-15
topic: forgotten-recording
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Forgotten Recording - Plan

## Goal Capsule

- **Objective:** A recording that nobody attends ends by itself soon after the last speech, and a note with long silence keeps, prepares and syncs only the audio around the speech.
- **Product authority:** The project owner. The decisions below were made in a brainstorm on 2026-11-15.
- **Open blockers:** None.

---

## Product Contract

### Summary

Longhand watches each recording for 30 minutes with no speech and for 6 hours of total recording time. Either one shows "Still recording?" with a 2-minute countdown, and if nobody answers, Longhand stops the note. At every Stop, Longhand removes long stretches with no speech and a low sound level before it prepares the audio, so the saved note holds the meeting and not the empty room.

### Problem Frame

On Friday 2026-11-13 at 15:01, the user started a note on the work Mac and did not stop it after the meeting. The note recorded for 43.5 hours over the weekend. The user saw it only because another computer showed the note as in use on another computer, and then stopped it remotely. The user has left notes running after a meeting several times before (`docs/ideas/done/recording-state-widget.md`). The recording widget did not help, because nobody was at the computer.

The cost has four parts, and the user rates all four as critical:

- **Privacy.** The microphone recorded an office for two days without the user's knowledge.
- **Processing time.** After Stop, processing took 17 minutes. Preparing the audio took 13 min 17 s of that time, because it converts every second of the recording, silence too. The live transcript and live speaker recognition had already done most of the other work during the 43 hours.
- **Disk and sync.** The full 43.5 hours of audio is saved in the home folder and syncs to the user's other computers.
- **The note itself.** The useful meeting is a short part at the start of a very long note.

A warning alone does not solve this, because the user is not there to see it.

### Key Decisions

- **Two checks, each with a prompt before the stop.** A silence check reacts fast after a meeting ends. The time limit is the backstop for an empty room where a TV or a cleaner talks. The user can set the time of each check and turn each off. Governs R2, R3, R5, R10. (session-settled: user-directed — chosen over a silence check only, a time limit only, and both checks with no prompt: both triggers are needed, and the prompt protects a quiet meeting where the user is present)
- **No answer stops the note and cuts the silent end.** The user finds a finished note of normal length, not an open note. Governs R7, R12. (session-settled: user-directed — chosen over a stop that keeps all audio, and over a Pause that leaves the note open and not processed)
- **Defaults are 30 minutes, 6 hours and 2 minutes.** A quiet stretch of 30 minutes in a real meeting is rare, and 6 hours covers a workshop day. Governs R2, R3, R5, R10. (session-settled: user-directed — chosen over 15 min / 4 h / 2 min and 60 min / 8 h / 5 min)
- **A countdown always ends in a stop, also when people talk.** A time limit is the user's own setting, so the user wants the note to stop at that time. Governs R7. (session-settled: user-directed — chosen over speech that holds the stop until 2 minutes with no speech: the user set the limit and wants it kept)
- **Cut long silence only when both channels are quiet and have no speech.** Each doubt keeps the audio, so an error keeps audio and never removes speech. A noisy room is not cut. Governs R12, R13. (session-settled: user-directed — chosen over a cut on no speech alone with a 30-day backup of the removed audio, and over keeping all audio and making Preparing faster: the user wants the cut to be reliable nearly every time)
- **The cut applies at every Stop, not only after an automatic stop.** The silent end of an automatic stop is removed by this cut, not by a separate rule. A note that the user stopped late, or that ran with both checks off, gets the same benefit. Governs R12, R14. (session-settled: user-approved — proposed in the scoping synthesis with the effect on noisy notes shown, and confirmed)
- **The prompt plays no sound.** The recording surface must not draw a meeting's attention to the screen (U7's attention budget). Governs R5. (session-settled: user-approved — proposed in the scoping synthesis and confirmed)
- **A notice on an automatic stop, no notice for a cut.** An automatic stop is not normal and is important to the user. A cut is normal behaviour, and the silence line in the transcript shows it. Governs R8, R16. (session-settled: user-approved — proposed in the scoping synthesis and confirmed)
- **The settings are shared settings.** One user with several computers wants one behaviour, the same as the recording widget setting. Governs R11. (session-settled: user-approved — proposed in the scoping synthesis and confirmed)
- **A settings change applies to the next note.** This follows the existing rule for recording settings. Keep Recording is the way to extend the current note. Governs R11.

### Requirements

**Speech and silence during recording**

- R1. During every recording, Longhand detects speech on the microphone channel and on the app audio channel, also when the live transcript is off.
- R2. The silence check triggers after 30 minutes of recording in which no speech was detected on either channel. Sound that is not speech, such as a fan, air conditioning or traffic, counts as silence, and speech on either channel starts the 30 minutes again.
- R3. The time limit triggers after 6 hours of recording time.
- R4. Paused time does not count for either check. After Continue, the silence check starts again from zero, and the time limit continues from the recording time before the pause.

**The prompt and the automatic stop**

- R5. When a check triggers, the computer that records shows "Still recording?" with a 2-minute countdown and two choices, Keep Recording and Stop. The prompt shows on the recording widget, in the main window and as a system notification, and it plays no sound.
- R6. Keep Recording closes the prompt and starts the check that triggered again. The silence check counts again from zero, and the time limit asks again after a further 6 hours of recording time.
- R7. Stop, or the end of the countdown with no answer, stops the note the same way as the user's Stop, and processing starts. Speech during the countdown does not hold the stop.
- R8. A note that Longhand stopped shows a short notice on the note that says which check stopped it.
- R9. When both checks trigger at the same time, Longhand shows one prompt.

**Settings**

- R10. Settings > Recording has an on/off switch and a time for the silence check, the time limit and the long-silence cut, and a time for the countdown. The defaults are on for all three, with 30 minutes, 6 hours, 10 minutes and a 2-minute countdown.
- R11. These settings are shared settings, so they apply on every computer that uses the storage folder. A change applies to the next note.

**Long-silence cut**

- R12. At every Stop, by the user or by Longhand, Longhand removes each stretch of at least 10 minutes in which every recorded channel has no speech and a low sound level.
- R13. 1 minute of audio stays between a removed stretch and the speech on each side of it.
- R14. Longhand finds and removes the stretches before Preparing the audio, so Preparing and all later stages process only the audio that stays.
- R15. The transcript shows each removed stretch as one silence line with the duration that was removed.
- R16. Removed audio is deleted permanently. The saved audio holds only the audio that stays, and the note shows no notice for a cut.

### Acceptance Examples

- AE1. Forgotten after a meeting
  - **Covers:** R2, R5, R7, R8, R12, R13
  - **Given:** A meeting ends at 15:45, and the note keeps recording in a quiet, empty office.
  - **When:** No speech is detected until 16:15, and nobody answers the prompt.
  - **Then:** The note stops at 16:17. The audio after 15:46 is removed, the transcript ends with one silence line, and the note shows that Longhand stopped it after 30 minutes with no speech.
- AE2. Noisy empty room
  - **Covers:** R2, R7, R12
  - **Given:** The meeting ends, and a loud air conditioner runs in the empty room.
  - **When:** No speech is detected for 30 minutes, and nobody answers the prompt.
  - **Then:** The note stops 32 minutes after the meeting. The noisy end is not removed, because its sound level is not low.
- AE3. Quiet stretch in a real meeting
  - **Covers:** R2, R6, R12, R13, R15
  - **Given:** A workshop has a 35-minute quiet writing exercise.
  - **When:** The prompt shows at 30 minutes, and the user selects Keep Recording.
  - **Then:** The note records on. At Stop, 33 minutes of the quiet stretch are removed, and the transcript shows one silence line of 33 minutes.
- AE4. All-day workshop
  - **Covers:** R3, R7
  - **Given:** A workshop is still running after 6 hours of recording.
  - **When:** The time limit prompt shows, and nobody answers within 2 minutes, while people talk.
  - **Then:** The note stops and processing starts.
- AE5. Call through an app
  - **Covers:** R1, R2, R12
  - **Given:** A note records the microphone and a Teams call. The room is silent, and the far side of the call talks.
  - **When:** 40 minutes pass.
  - **Then:** The silence check does not trigger, and no part of those 40 minutes is removed.
- AE6. Checks turned off
  - **Covers:** R10, R12, R14
  - **Given:** The silence check and the time limit are off, and a note records for 43 hours, most of it in a quiet room.
  - **When:** The user stops it.
  - **Then:** The quiet stretches are removed before Preparing the audio, so Preparing takes about as long as for a note of the kept length.
- AE7. Lunch pause
  - **Covers:** R4
  - **Given:** A note records for 20 minutes with no speech, and then the user pauses it for one hour.
  - **When:** The user selects Continue.
  - **Then:** No prompt shows at Continue, and the silence check counts from zero.

### Success Criteria

- A note left recording in a quiet room after a meeting ends as a finished note that is at most a few minutes longer than the meeting.
- After the forgotten note in the Problem Frame, Preparing the audio took 13 min 17 s for 43.5 hours. With the cut, Preparing takes about as long as for a note of the kept length.
- In test recordings with quiet or far speech, the cut removes no stretch that has speech in it.

### Scope Boundaries

**Deferred for later**

- Stopping a note from another computer. The user's laptop showed the note as in use on another computer, but could not stop it.
- A faster Preparing the audio for audio that stays, for example a long noisy note that is not cut.
- Less live transcript and live speaker recognition work during long silence.

**Not in this work**

- Notes that exist before this change, such as the 43-hour note.
- A forgotten Pause. The user rejected a speech-listening reminder for a paused note in the pause-recording brainstorm (`docs/ideas/done/recording-state-widget.md`).
- A backup of removed audio.
- Imported audio. An import is not a recording and has no Stop.

### Outstanding Questions

**Deferred to Planning**

- How Longhand detects speech during recording when the live transcript is off, and what this costs in CPU. Today speech is known during recording only through the live transcript.
- The sound level below which a stretch counts as quiet, and a test set of quiet and far speech that proves the threshold.
- How the stretches are found before Preparing: from data that the recording collects, or from a fast pass over the chunks.
- How transcript times and the timeline of manual notes relate to the original recording time after a cut.
- How the prompt behaves when the recording widget is hidden, when system notifications are off, or when the computer sleeps during the countdown.

### Sources

- `crates/pipeline/src/assemble.rs`: Preparing the audio reads, resamples and encodes every second of each segment. Nothing skips silence.
- `crates/asr/src/windows.rs` and `crates/vad/src/lib.rs`: recognition after Stop uses Earshot VAD and transcribes only voiced windows.
- `src-tauri/src/managers/live.rs` and `crates/asr/src/stream.rs`: speech is detected during recording only when the live transcript is on (`live_transcript` setting).
- `src-tauri/src/managers/capture.rs`: live peak levels for the microphone and the app channel; the app channel starts at the first app sound.
- `src/recording/inputStatus.ts`: the existing "quiet" status, after 20 s below -60 dBFS.
- `src-tauri/src/managers/alerts.rs`: the existing system notification and alert cue for a failed capture.
- `src/widget/RecordingWidget.tsx`: the recording widget with Pause, Continue and Stop.
- `src-tauri/src/managers/settings.rs`: local and shared settings; `show_recording_widget` is a shared setting that applies to the next note.
- `crates/pipeline/src/transcribe.rs` and `crates/pipeline/src/join.rs`: the silence, pause and gap lines in the transcript.
- `docs/ideas/done/recording-state-widget.md` and `docs/plans/2026-10-09-1504-feat-pause-recording-plan.md`: the earlier decisions about a recording left running and a forgotten Pause.
