---
title: Remove Result Copies From Note Folders - Plan
type: refactor
date: 2026-09-30
topic: remove-result-copies
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Remove Result Copies From Note Folders - Plan

## Goal Capsule

- **Objective:** A note folder holds only the current transcript, summary, notes, session file and (while it exists) audio: no hidden copies of earlier results, and no copy of the transcript text after the audio is removed. Receiving computers stay correct with a few seconds of possible mismatch during a sync.
- **Means:** Remove result generations (`.longhand-results/`, `.longhand-result-transition.json`). Publish results by direct writes with `session.md` last, and use the fingerprints in `session.md` to recognize a sync in progress. Remove `recognition.jsonl` with the audio.
- **Product authority:** The user, in the brainstorm of 2026-09-30. This work starts from `main` after PR #28 (audio removal) merges.
- **Open blockers:** None.

---

## Product Contract

### Summary

Note folders stop keeping copies of results. Longhand writes `transcript.md` and `summary.md` directly and `session.md` last, and a crash finishes the write from this computer's local staging. The fingerprints in `session.md` let a receiving computer recognize a sync in progress. Audio removal also deletes `recognition.jsonl`, and a one-time cleanup deletes the existing copies.

### Problem Frame

Each time Longhand saves or replaces a result (Save, Transcribe Again, Regenerate Summary, Resume), it writes a complete copy of the transcript, summary and `recognition.jsonl` to `.longhand-results/<generation>/` in the note folder, and it never deletes the earlier copies. The folders grow with every replacement, sync to every computer, and keep old versions of the transcript. The user found them in the storage folder and asked what they are.

The copies exist so that a computer that receives a half-synced update keeps showing the last complete result. That protects against a mismatch that lasts a few seconds. The user's rule is that the app does not over-engineer sync edge cases: the last change wins, and conflicts belong to the sync service (rule 8 in `docs/solutions/integration-issues/shared-settings-in-a-synced-folder-rules.md`).

`recognition.jsonl` holds the timed words and speaker labels of a note. Longhand reuses it only for Transcribe Again and speaker-only reprocessing, and for the saved prefix of a Resume. All of these also need the audio. After audio removal (PR #28) the file has no use, but it stays in the folder with a second copy of the transcript text.

### Key Decisions

- **Remove the result copies instead of adding cleanup for them.** A few seconds of mismatch on a receiving computer is acceptable. Governs R1, R2, R3, R4. (session-settled: user-approved — chosen over keeping generations and deleting all but the newest: the app does not over-engineer sync edge cases.)
- **During a sync, show the files as they are and hold actions.** Governs R6, R7. (session-settled: user-directed — chosen over keeping the last complete result on screen until the files match, and over no detection at all.)
- **A mismatch that lasts over about a minute is a hand edit.** The note shows the edited file, and actions run and replace it. Governs R8, R9. (session-settled: user-directed — chosen over saving a conflict copy before a replacement, and over blocking actions until the user undoes the edit.)
- **Remove `recognition.jsonl` with the audio.** Governs R11, R12. (session-settled: user-directed — chosen over keeping it: it has no use without the audio and keeps a copy of the transcript text.)
- **Any computer does the one-time cleanup.** The new code no longer reads the copies, and nobody else has installs to migrate. Governs R13.

### Requirements

**Result publication**

- R1. A note folder has no `.longhand-results/` folder and no `.longhand-result-transition.json` file after this work, on every computer that runs the new build.
- R2. A save, Transcribe Again, Regenerate Summary or Resume writes `transcript.md` and `summary.md` in the note folder and then writes `session.md` last, with the fingerprints of the new files.
- R3. The new result is complete in this computer's local staging before the first write to the note folder. A failed run changes no file in the note folder, so the previous result stays (R6 and R25 of `docs/plans/2026-09-27-1526-feat-flexible-speaker-recognition-plan.md` still hold).
- R4. A crash during the writes of R2 is repaired at the next launch by finishing the writes from local staging. Only the computer that started the publication writes in its recovery.
- R5. `notes.md` keeps its present rules. The publication does not overwrite a changed `notes.md`.

**Receiving computers and syncs in progress**

- R6. When `transcript.md` or `summary.md` does not match its fingerprint in `session.md`, the note shows the files as they are on disk. The note shows no error.
- R7. While such a mismatch lasts, actions that read or replace the result (Transcribe Again, Regenerate Summary, Resume, and speaker-only reprocessing) do not start, and the note says it is waiting for the note to sync. The library scan and adoption do not refuse the note for a mismatch.
- R8. A mismatch that lasts over about one minute counts as a hand edit. The actions of R7 then run. Regenerate Summary reads the edited `transcript.md`, and Transcribe Again replaces it.
- R9. A hand edit makes the saved timed text of the note unusable. Transcribe Again then runs in full, and a save or Regenerate Summary does not fail because the saved timed text no longer matches the transcript.
- R10. The folder watcher refreshes an open note when its files arrive, as it does today for other note files.

**recognition.jsonl and audio removal**

- R11. When audio removal removes the last audio of a note, it also deletes `recognition.jsonl` from the note folder. Its reference in `session.md` stops blocking any later save.
- R12. Summary regeneration, title and rename writes, and notes saves keep working for a note without `recognition.jsonl`.

**One-time cleanup**

- R13. Longhand deletes existing `.longhand-results/` folders and `.longhand-result-transition.json` files in the note folders of the storage folder, and deletes `recognition.jsonl` from notes whose audio is already removed. It does this without a prompt and it does not touch a note that another computer uses (the in-use marker rules of the audio removal plan).
- R14. The cleanup never deletes `transcript.md`, `summary.md`, `notes.md`, `session.md` or audio. If a note has a result only in its copies (no current `transcript.md` or `summary.md`), the cleanup leaves that note's copies in place and logs it.

### Acceptance Examples

- AE1. **Covers R6, R7.** Given computer A saved a new transcript and summary, when computer B has the new `session.md` and the old `summary.md`, then B shows the new transcript with the old summary, and Regenerate Summary on B says it is waiting for the note to sync. When the new `summary.md` arrives, B shows it and the action runs.
- AE2. **Covers R8, R9.** Given the user edits `transcript.md` in a text editor, when more than a minute passes, then the note shows the edited transcript, Regenerate Summary makes a summary from it, and Transcribe Again runs in full and replaces it.
- AE3. **Covers R3, R4.** Given this computer crashes after it wrote `transcript.md` and before it wrote `summary.md`, when Longhand launches again, then it writes `summary.md` and `session.md` from local staging, and the note shows the new result.
- AE4. **Covers R11, R12.** Given a note whose last audio is removed, then its folder has no `recognition.jsonl`, and a later Regenerate Summary or rename works.
- AE5. **Covers R13, R14.** Given a storage folder with `.longhand-results/` in most note folders, when the new build launches, then those folders and transition files are gone, and every note shows the same transcript and summary as before.

### Scope Boundaries

- A note does not keep its last complete result on screen during a sync (the rejected option of the second Key Decision).
- No conflict copy of a hand-edited `transcript.md` or `summary.md`.
- No change to the rules for `notes.md` conflicts, audio, removal markers or in-use markers.
- The Resume gap recorded in the audio removal plan (a removal before the old audio copy finishes) stays a separate follow-up.

### Dependencies / Assumptions

- Syncs of one note usually finish in a few seconds. About one minute separates a sync in progress from a hand edit.
- Every computer of the user runs the new build soon after it ships. Until the preview build on another computer updates, it can show a note as waiting to sync after the cleanup deleted the copies that it still expects.

### Outstanding Questions

**Deferred to Planning**

- Whether the fingerprints in `session.md` need more than the present `transcript_fingerprint` and `summary_fingerprint` (for example the size, or the fingerprint of `recognition.jsonl`) to recognize a sync in progress.
- How the one-minute rule is measured: from when this computer first saw the mismatch, or from file times.
- How R4's recovery finds a publication that crashed, now that no transition file is in the note folder.
- Where speaker-only reprocessing and the Resume saved prefix read `recognition.jsonl` from once no copy exists, and how they treat a hand-edited transcript.

### Sources / Research

- Result generations: `crates/session-core/src/generation.rs`, `crates/session-save/src/result.rs`; call sites in `crates/pipeline/src/save.rs`, `summarize.rs`, `reprocess.rs`, `recognition/saved_prefix.rs`, `recognition/durable.rs`, `crates/library/src/scan.rs`, `refresh.rs`, `reconcile.rs`, `src-tauri/src/commands/session.rs`, `src-tauri/src/managers/library.rs`.
- `crates/pipeline/src/save.rs` `session_doc()` reads the header of `recognition.jsonl` on every save and fails when its transcript digest does not match. R9 and R12 depend on this.
- Old generation folders are never deleted: the only removals in `result.rs` are rollback of canonical files, the transition file and the local journal.
- Superseded decisions: KTD8, KTD13 and KTD14 of `docs/plans/2026-09-27-1526-feat-flexible-speaker-recognition-plan.md` (generations, rollback copies, readers that reject incomplete generations, external-edit conflicts).
- Audio removal: `docs/plans/2026-09-30-0357-feat-remove-old-audio-plan.md` (in-use markers, the exception for writes by any computer).
