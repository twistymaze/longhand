---
title: Longhand Walking Skeleton - Plan
type: feat
date: 2026-09-21
topic: longhand-walking-skeleton
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Longhand Walking Skeleton - Plan

## Goal Capsule

- **Objective:** A four-hour in-person session recorded on one microphone survives to become a transcript with legible turn-taking and a summary shaped by what the user typed while it happened — without the user having lost the session to sleep, a crash, or an accidental stop.
- **Product authority:** `STRATEGY.md` at the repo root. Its Boundaries section governs; this plan narrows within it and contradicts none of it.
- **Active scope:** The end-to-end capture-to-summary path only. Library and browsing, Obsidian sync, and the CLI are separate work and are not active scope here, though they consume what this writes.
- **Open blockers:** None.

---

## Product Contract

### Summary

Longhand records a session from a single keystroke, tolerates interruption without losing what came before, and afterward produces a local transcript with turn boundaries and a summary steered by the user's scratchpad notes. Sessions land as plain markdown and audio in a folder of the user's choosing.

### Problem Frame

The sessions most worth capturing are the ones that cannot be handed to a cloud transcription service, and the person capturing them is participating rather than taking minutes. Existing tools fail this case in three specific ways, each of which has already cost the user a session.

They stop. Recording caps, screensaver-triggered sleep, and unnoticed device changes end a capture partway through, and the loss is discovered afterward when the notes are needed. A three-hour session that silently became ninety minutes is worse than no recording, because the gap is invisible until it matters.

They flatten. Transcription of several people on one microphone yields undifferentiated text, so a downstream summarizer cannot follow an exchange and produces a summary that reads like a paraphrase of the loudest thread.

They ignore what the user bothered to write. Notes typed during a session are the clearest available signal about what mattered, and tools either merge them as undifferentiated text or discard them. The moment a note was typed — the most useful thing about it — is never captured at all, and cannot be recovered later.

### Key Decisions

- KD1. **Processing runs in the background after stop and survives quitting the app.** A multi-hour session should not hold the application hostage while it computes. *(session-settled: user-directed — chosen over a blocking progress screen, live-during-recording transcription, and a deferred queue: the user wants to close the laptop and get notes later.)* Governs R14, R15, R20.
- KD2. **A session is an ordered set of recording segments, not a single file.** Crash recovery, sleep recovery, accidental stop, and a deliberate dinner break become one mechanism instead of four. *(session-settled: user-directed — chosen over one-file-per-session: the user declined to be tied to a single audio artifact per session.)* Governs R3, R5, R6.
- KD3. **Interruption is both prevented and recovered from.** Holding a sleep assertion removes the most common cause; the recovery path is required regardless for device loss and crashes. *(session-settled: user-directed — chosen over prevention alone and recovery alone: prevention is cheap and recovery is needed anyway.)* Governs R4, R5.
- KD4. **A recording stages to a working location and lands in its destination folder once, on completion.** Nothing partial ever appears in a folder that external tools may be syncing, and the destination stays editable for the life of the session. *(session-settled: user-approved — chosen over recording directly into the destination folder: avoids moving files the recorder still holds open.)* Governs R21, R22, R23.
- KD5. **Folders hold the content; the index holds nothing that cannot be rebuilt from them.** This is what keeps "no third party holds the store" true when the index is the application's own. *(session-settled: user-directed — chosen over an application-owned store with markdown mirrored out: a mirrored copy can disagree with its source.)* Governs R24, R25.
- KD6. **Scratchpad lines carry the recording position at which they were typed.** Position can only be captured while recording; every other property of a note can be added later. *(session-settled: user-approved — chosen over notes as plain summary input and notes as un-anchored emphasis: the anchor is the only irreversible part of the decision.)* Governs R12, R13, R18.
- KD7. **Notes weight the summary; they do not filter it.** What the user typed marks what they cared about, but silence is not evidence of disinterest. *(session-settled: user-directed — the user stated this as the correction to treating notes as an outline.)* Governs R18.
- KD8. **The live-transcript view is a capability of the selected model, not a promise of the product.** When the model streams, the view is offered; when it does not, the surface says plainly that the transcript arrives at the end. *(session-settled: user-directed — chosen over building a second streaming path purely for display and over cutting the live view from `STRATEGY.md`.)* Governs R10.
- KD9. **A failed resume is announced audibly as well as visibly.** The user is in the room and not watching the screen, so a silent notification can go unseen for the rest of the session — which is the loss this design exists to prevent. *(session-settled: user-directed — chosen over a notification alone, an escalating cue, and a full-screen state: it has to carry across a table on the first try.)* Governs R5.
- KD10. **Borrowed code comes only from MIT-licensed paths.** `reference-source/anarlog/LICENSING.md` states that a file does not keep its previous license merely because it was moved, so the boundary is checked at the source path, not at the destination. Governs R26, R27.
- KD11. **The user decides which providers see their data; none is ever forced.** The system of record stays on the machine, so a provider the user points a stage at is a choice rather than a dependency — which is why a future remote recognition option would be consistent with this boundary rather than a breach of it. Governs R29, R30.
- KD12. **Recognition is local in this version; summarization reaches one OpenAI-compatible endpoint.** Audio is the data most worth keeping on the machine and local recognition is good enough, while summaries of a four-hour transcript are where model quality actually decides whether the product works. Targeting the OpenAI-compatible shape reaches a hosted provider, a gateway, and a local server through a single integration. *(session-settled: user-directed — chosen over both stages local and over multiple API shapes in this version.)* Governs R31, R32.

<!-- ce-section: work-relationships -->
### How This Work Fits Together

This plan covers the capture-to-summary path only. The breakdown below is how the surrounding work is currently understood, not a committed roadmap; a later plan may revise, split, merge, or discard any of it.

- **Library and browsing** — finding a session weeks later.
  - Depends on this plan for the on-disk shape (R24), the retained destination roots (R25a), and the rule any index must satisfy (R25). The index itself is built with this track.
  - Can proceed independently of Obsidian sync and the CLI.
- **Obsidian vault synchronization.**
  - Depends on this plan for sessions landing atomically as plain markdown (R22, R24).
  - Shares the folder model with Library.
- **Local CLI over the session store.**
  - Depends on this plan for the folder contract (R24) and the index constraint (R25).
  - Can proceed independently of Library.
- **System audio capture** — the second channel, deferred from this plan.
  - Depends on this plan for the segment model (R3), which already admits more than one source.
  - Still to decide: whether a dual-channel session changes the transcript's shape or only its inputs.

### Key Flows

- F1. Start and record
  - **Trigger:** The user presses the global shortcut.
  - **Steps:** Recording begins with no prompt or picker; a sleep assertion is taken; audio streams to a working location as it arrives; the scratchpad and confidence rail become available.
  - **Outcome:** A session is capturing, and the user's attention is free.
  - **Covered by:** R1, R2, R4, R7, R9

- F2. Interruption and recovery
  - **Trigger:** Capture stops without the user stopping it — device removed, permission revoked, process crash, or sleep the assertion did not prevent.
  - **Steps:** The stoppage is detected; capture resumes into a new segment when resumption is possible; when it is not, the user is alerted in a way they will notice during a session they are not watching. A crash is the exception — nothing in the dead process can alert, so that case is covered by recovering the written audio at next launch.
  - **Outcome:** Audio recorded before the interruption is intact, and the gap is recorded rather than concealed.
  - **Covered by:** R3, R5, R5a, R6, R8

- F3. Stop and process
  - **Trigger:** The user stops the session.
  - **Steps:** The session closes; transcription and diarization run in the background across all segments; the summary is generated from the transcript and the anchored notes; the completed session lands in its destination folder.
  - **Outcome:** The user is notified that a complete session is ready. Quitting the application partway does not restart the work.
  - **Covered by:** R11a, R14, R15, R16, R16a, R17, R17a, R18, R20, R22, R22a, R31, R32

- F4. Read and revise
  - **Trigger:** The user opens a completed session.
  - **Steps:** Notes appear beside a pane showing either the summary or the transcript; a note can be followed back to the passage it refers to; the summary can be regenerated without re-transcribing.
  - **Outcome:** The user recovers the specifics they did not write down.
  - **Covered by:** R11, R12, R16a, R19

### Requirements

**Capture and survivability**

- R1. Recording starts from a single global shortcut with no dialog, picker, or decision at the moment of starting.
- R1a. Microphone authorization and an initial destination are established before the shortcut is armed, and the surface shows whether it is armed, so that the first press of a session never prompts.
- R1b. Audio is captured from the moment the shortcut is pressed; device acquisition and staging setup happen before the press rather than in response to it.
- R2. This version captures the microphone only.
- R3. A session comprises one or more recording segments, each carrying its own wall-clock start and end, and the elapsed gap between consecutive segments is preserved as a gap rather than closed by concatenation.
- R4. While recording, the application prevents the system from entering idle sleep.
- R5. When capture stops for any reason other than the user stopping it, the application resumes into a new segment if resumption is possible, and otherwise raises an audible cue, posts a system notification, and shows a failed-capture state in the application that remains until the user acknowledges it.
- R5a. At the start of recording the application verifies that its alert path can reach the user — audio output audible and notification delivery permitted and unsuppressed — and says so in the recording rail when it cannot.
- R6. A user-initiated resume continues the existing session rather than starting a new one, using the same mechanism as automatic recovery.
- R6a. Resume remains available until the session lands in its destination folder; resuming cancels any in-flight transcription or summarization for that session and returns it to capture. Once a session has landed, recording starts a new session.
- R7. Captured audio is written to durable storage continuously during the session; no code path accumulates a whole session in memory.
- R7a. Capture reserves a free-space floor; on reaching it the application closes and preserves all completed audio and treats the stop as an interruption under R5.
- R8. Audio written before an application crash remains recoverable after restart.

**Session surface**

- R9. While recording, the surface shows the scratchpad alongside a rail carrying recent audio-level history, input status, and remaining disk headroom, so that a glance distinguishes "still capturing" from "has been silent."
- R10. The surface states whether a live transcript is available for the selected model, and when the model cannot stream it says plainly that the transcript arrives when processing finishes. The live view itself is deferred, so no live view is offered in either case.
- R11. After processing, the surface shows the notes pane beside a single adjacent pane that the user switches between the summary and the transcript.
- R11a. While a stopped session is still processing, the surface shows that session with its current stage and progress, keeps the notes editable, and names the failed stage when processing fails. Reopening the application during processing returns to this state.
- R12. Each scratchpad line records the recording position of its first keystroke, offset by a lookback interval so that a note lands at the subject it refers to rather than at the keystroke, and the anchor does not move when the line is later edited.
- R12a. Note anchors are expressed on the session timeline defined in R16a. A lookback that would land inside the gap between two segments resolves to the end of the preceding segment.
- R13. Notes added when no recording is in progress are retained and usable without a position anchor.

**Processing**

- R14. Processing runs after the session stops, leaves the application usable, and resumes from where it left off after the application is quit and reopened.
- R15. The user is notified when a session has finished processing.
- R16. The transcript marks changes of speaker so that turn-taking is legible to a reader and to a downstream model, without identifying who any speaker is.
- R16a. The stored transcript records the time position of each speaker turn on a single session timeline that includes the gaps between segments, so that a note anchor, a transcript passage, and a point in the recorded audio name the same moment.
- R17. Summarization handles a transcript longer than any single model context by summarizing in parts and combining, rather than submitting the whole transcript in one request.
- R17a. Transcription submits bounded audio windows rather than a whole session in one request, and the application verifies that the resulting transcript spans the full recorded duration of every segment. A transcript that does not is treated as a failed stage under R20 rather than delivered.
- R18. The summary weights material the user annotated while still surfacing significant unannotated material, and does not treat the absence of a note as evidence that a passage is unimportant.
- R19. A summary can be regenerated from an existing transcript without re-running transcription.
- R20. A processing failure leaves the audio and any completed transcript intact and reports which stage failed.

**Storage and handoff**

- R21. A session in progress resides in an application-managed working location, not in any user-facing folder.
- R22. On completion, the session is placed into its destination folder in a single operation, with no intermediate partial state visible there.
- R22a. When the working location and the destination are on different filesystems, the session is first assembled in a hidden staging directory on the destination's own filesystem and then renamed into place, so the final step is always a same-volume rename.
- R23. The destination folder can be set or changed at any point during or after the session, up to the moment the session lands.
- R24. A landed session folder contains markdown and audio readable by any tool that understands those formats, with no application-proprietary content file.
- R25. The application's index contains nothing that cannot be rebuilt from the session folders; deleting it costs speed and loses no user content.
- R25a. Every destination root the user has selected is retained as durable configuration held separately from the index, and rebuilding the index means scanning those retained roots for session folders.

**Licensing and provenance**

- R26. Code is borrowed only from MIT-licensed source paths; nothing is taken from `reference-source/anarlog/enterprise/`.
- R27. Each borrowed file carries a provenance record beside the consuming code naming the upstream repository, the exact revision, the source paths, the copyright holder, the license, and whether the material was copied, modified, or used only as reference.
- R28. Model weights are obtained directly from their upstream source under terms accepted by this project, rather than inherited from a reference repository's committed copy.

**Locality and the data boundary**

- R29. The system of record — session audio, transcripts, notes, and summaries — is held on the user's machine. No provider holds it, and no feature depends on one holding it.
- R30. No processing stage sends data anywhere the user has not chosen, and the surface names the destination a stage is using.
- R31. Speech recognition and diarization run on the user's machine in this version. No remote recognition option is offered, so session audio does not leave the machine.
- R32. Summarization is directed at an OpenAI-compatible endpoint the user configures with a base URL, a model name, and a credential. That one shape covers a hosted provider, a gateway, and a local server alike, so no second integration is needed to reach any of them.
- R33. Configuration and credentials live in application-managed storage, never in a session folder: settings in the application's own configuration store, credentials in the operating system's secret store. A session folder contains only the content of that session, so nothing a vault sync or the CLI reads carries settings or secrets.

### Acceptance Examples

- AE1. **Covers R3, R5.** **Given** a session recording for 90 minutes, **when** the microphone is unplugged and reconnected four minutes later, **then** the session contains two segments, the four-minute interval is represented as a gap, and the first 90 minutes are intact.
- AE2. **Covers R4.** **Given** a session recording and no user input for an hour, **when** the system's idle-sleep timer would ordinarily fire, **then** the machine stays awake and capture continues.
- AE3. **Covers R5, R5a.** **Given** a session recording while the user is not watching the screen, **when** capture stops and cannot be resumed, **then** an audible cue sounds, a notification is posted, and the application holds a failed-capture state until acknowledged — so the user learns of it during the session rather than afterward.
- AE4. **Covers R14.** **Given** a four-hour session that has been processing for ten minutes, **when** the user quits and reopens the application, **then** processing continues from its existing progress rather than restarting.
- AE5. **Covers R10.** **Given** a selected model that cannot stream, **when** a session is recording, **then** the surface states that the transcript will be available at the end and offers no live transcript view.
- AE6. **Covers R18.** **Given** a transcript in which a decision was reached that the user never annotated, **when** the summary is generated, **then** that decision still appears.
- AE7. **Covers R22, R23.** **Given** a session whose destination folder is changed twice while it records, **when** it completes, **then** it appears exactly once, in the folder selected last, with no partial content having appeared in either folder.
- AE8. **Covers R16.** **Given** a recording of six people around one microphone, **when** the transcript is produced, **then** consecutive contributions by different speakers are separated rather than run together.
- AE9. **Covers R10.** **Given** a selected model that can stream, **when** a session is recording, **then** the surface states that a live transcript is available for this model and still offers no live view, because the view is deferred.
- AE10. **Covers R1a, R1b.** **Given** the application running in the background with the shortcut armed, **when** the user presses it mid-sentence, **then** recording begins with no prompt and that sentence is present in the recorded audio.
- AE11. **Covers R17a.** **Given** a four-hour session whose transcription returns text covering only its first portion, **when** processing runs, **then** the shortfall is detected against the recorded duration and reported as a failed stage rather than delivered as a complete transcript.
- AE12. **Covers R6a.** **Given** a session the user stopped by accident that has begun transcribing, **when** they resume within the same session, **then** the in-flight work is cancelled, recording continues into a new segment, and the eventual summary covers both segments.
- AE13. **Covers R32, R33.** **Given** a configured endpoint and credential, **when** a session finishes processing, **then** the summary is produced through that endpoint, the surface names it, and neither the credential nor any setting appears anywhere in the landed session folder.

### Success Criteria

- A complete Friday Pathfinder session — three to four hours, six to eight people, one microphone — is captured end to end and produces a usable recap, as the acceptance test for the whole skeleton.
- An interruption during that session costs the interval itself and nothing more.
- The interval between deciding to record and recording is short enough that the decision is never deferred.
- One summary shape holds up across session types rather than needing to change from session to session. Pathfinder is the first case it is measured against, not the case it is tuned for. The broader bar — a summary shareable without apology — is a product-level metric in `STRATEGY.md` and is not graded on this slice.

### Scope Boundaries

**Deferred for later**

- System audio capture and the dual-channel session that follows from it. The flagship case is a room, and the second capture path carries its own permission surface.
- Summary kinds derived from the destination folder. This version produces a single summary shape, so a Pathfinder recap and a one-on-one are treated alike until it returns.
- Custom vocabulary correction for proper nouns and domain jargon. Cheap to add, and deliberately left out so its absence can be felt before it is specified.
- The live streaming transcript view itself, for models that support one. The surface must be honest about its availability now (R10); the view can follow.
- The session index. R25 fixes the rule any index must satisfy and R25a retains the roots it would be rebuilt from, but the index is built with Library and browsing, whose queries should shape it.
- Remote speech recognition. Recognition is local in this version (R31); a remote option is permitted by the boundary but is not built here.
- API shapes other than OpenAI-compatible, such as Anthropic's. One shape reaches every endpoint worth reaching on day one, and a second integration can follow when a specific provider justifies it.
- Library and browsing, Obsidian vault synchronization, and the local CLI. Each is a separate piece of work that reads what this one writes.

**Outside this product's identity**

- Speaker identity of any kind — enrollment, voiceprints, or names attached to speakers. Turn boundaries are the requirement; who is speaking is not.
- Calendar integration in any form, including auto-start and prefill.
- Any organization scheme beyond folders.
- Any inbound integration that writes into Longhand.

### Dependencies / Assumptions

- `reference-source/` holds read-only upstream clones that are never edited and never tracked by this repository.
- The transcription runtime exposes the model's own capabilities at runtime, so R10 can be satisfied by querying the loaded model rather than maintaining a capability table.
- Diarization operates on complete audio after a session ends, which is consistent with KD1 and means turn boundaries are not available during recording.
- Assumed and unverified: that diarization quality on one microphone with six to eight speakers at varying distances is good enough to make turn boundaries useful. The approach is sound and the acoustic condition is the hardest one. This is measured first, against a recording of a real session at full length and full table size, before the summary and session-reading surfaces are built on turn boundaries. Speaker-change boundaries are compared against a hand-marked reference over a sampled interval. A result that does not clear that bar settles the diarization question below toward a different approach, and R16 degrades to delivering the transcript without speaker marks and saying so, rather than presenting unreliable turns as fact.
- The diarization segmentation stage represents at most three concurrent speakers within each analysis window. Sequential turns among six to eight people are unaffected; several people talking at once are not all separable. The measurement above should be designed to expose that rather than average over it.

### Outstanding Questions

**Deferred to planning**

- Which model is the default, and whether turn boundaries come from a separate diarization pass or from a diarizing transcription model — constrained by AE8, since the diarizing transcription models in this catalog cap at four speakers while the separate-pass route does not.
- The transcription window length, and how both it and the segment boundaries relate to the chunking that long-transcript summarization requires.
- Whether the user may state how many people a session has, used to bound diarization. It is a count rather than an identity, so the no-speaker-identity boundary does not exclude it, and the clustering stage accepts one. Resolve against the diarization measurement above: if that clears its bar without a stated count, this stays deferred; if it does not, this is the first mitigation to try before changing approach.
- The lookback interval that offsets a note's anchor from its first keystroke.
- How the configured endpoint's context limit is discovered or declared, given that an OpenAI-compatible server can silently truncate an oversized request rather than refusing it — which would produce a summary of part of a session with no error. R17 chunks regardless; the open question is what the chunk budget is measured against.
- The elapsed threshold at which a within-segment silence is rendered as a break rather than continuous speech.

### Sources / Research

- `STRATEGY.md` — product authority for purpose, users, boundaries, and metrics.
- `reference-source/anarlog/LICENSING.md` — path-based licensing; names local capture, transcription, storage, export, and sync clients as MIT, and states that a file does not retain its license when moved out of `enterprise/`.
- `reference-source/anarlog/crates/listener-core/src/actors/recorder/chunks.rs` — chunked recording to disk with per-chunk durability, a free-space floor, and recovery of partial chunks at startup. The reference implementation for R7 and R8.
- `reference-source/anarlog/apps/desktop/src/stt/detect-events.ts` — ends the session unconditionally when the system reports sleep. The behavior R4 and R5 exist to replace.
- `reference-source/anarlog/apps/desktop/src/session/components/outer-header/overflow/listening.tsx` — user-initiated resume of an existing session; `reference-source/anarlog/apps/desktop/src/stt/capture-lifecycle.ts` covers backfilling a gap from saved audio. Prior art for R6.
- `reference-source/anarlog/crates/listener-core/src/actors/session/supervisor/children.rs` — bounded restart supervision for a failing audio source, with backoff excluded from the failure budget. Prior art for R5.
- `reference-source/anarlog/crates/pyannote-local/` — speaker segmentation and clustering. Clustering imposes no ceiling on how many distinct speakers a session may contain, but segmentation represents at most three concurrent speakers per analysis window, so overlapping speech is bounded independently of clustering quality. Note also that its audio-source implementations are in-memory; the file-backed implementation that keeps long recordings non-resident lives in `reference-source/anarlog/crates/listener2-core/`, which is not being taken, so R7's constraint extends to diarization input.
- `reference-source/anarlog/crates/template-app/assets/_macros.jinja` — renders a transcript by flat iteration into one prompt. Confirms that no chunked or hierarchical summarization exists upstream and that R17 is new work.
- `reference-source/Handy/src-tauri/src/audio_toolkit/audio/recorder.rs` — accumulates a whole recording in memory and returns it at stop, with no duration guard. The behavior R7 exists to replace.
- `reference-source/Handy/src-tauri/Cargo.toml` — MIT, and the per-platform transcription backend configuration.
- `reference-source/Handy/src-tauri/src/settings.rs` with `reference-source/Handy/src-tauri/src/portable.rs` — settings held in a `tauri-plugin-store` JSON file resolved to an application data directory rather than to any user content folder. The pattern to mirror for ordinary configuration under R33.
- `reference-source/anarlog/plugins/store2/` — a wrapper over the `keyring` crate for secrets, with per-platform handling for an unavailable or locked secret service. The pattern to mirror for credentials under R33; the two stores are complementary rather than alternatives.
- `docs/reference/borrowing-notes.md` — verified findings about both reference projects, including traps that affect implementation choices.
