---
title: One Library Across Computers - Plan
type: feat
date: 2026-09-25
topic: one-library-across-computers
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# One Library Across Computers - Plan

## Goal Capsule

- **Objective:** A person who uses Longhand on several computers that sync one folder sees the same notes in Home on each computer, and can do anything with a note on any of them, whichever computer recorded it.
- **Means:** `session.md` becomes the complete record of a saved note (KD2), and the note folder is the authority on every computer (KD4). Home lists saved notes from the storage folders, not only from this computer's database (R1). Longhand watches the storage folders while it is open (R7).
- **Product authority:** This plan covers the shared library, the complete `session.md`, folder watching, and the change of the UI term "Destination" to "Storage". Search and the other parts of `docs/ideas/library-and-browsing.md` are not active scope. The Product Contract wins on behavior.
- **Open blockers:** None.

---

## Product Contract

### Summary

Each computer lists every saved note in its storage folders. That includes notes that another computer recorded and a sync service delivered. `session.md` holds everything that Home and the note page need, so a computer can rebuild its list from the folders alone. While Longhand is open, it watches the storage folders, so notes appear, change and disappear within seconds after a sync. Every action on a saved note works on every computer, including Resume. When a sync service makes a conflict copy, the note shows a small notice. The UI calls the storage folder "Storage" everywhere.

### Problem Frame

The user records on more than one computer, for example a work laptop and a home desktop, and all of them save to one folder that OneDrive, Google Drive or Dropbox syncs. Saving already works: the folders from each computer sit side by side, and each computer writes only to folders that it created. But Home is built from each computer's own database (`list_sessions` in `src-tauri/src/commands/session.rs`). So each computer lists only the notes that it recorded, and a note from the other computer is visible only in Finder.

Walking-skeleton `KD5` and `R25` already say that the folders hold the content and that the index holds nothing that cannot be rebuilt from them. The code does not fully meet that rule yet. The title source, the recognition model and the number of speakers found are only in the database. Several writers also treat the database as the authority for a saved note: the launch reconcile renames a folder back to the stored title, a notes save writes the database copy, and Summarize Again builds its input from this computer's processing data (`transcript_turns` in `crates/pipeline/src/summarize.rs`).

### Key Decisions

- KD1. **One person, several computers, not several people.** The library is one user's notes across the user's own computers. Shared use by colleagues is out of scope. *(session-settled: user-directed — chosen over a folder that several people share: the user named this case explicitly.)*
- KD2. **The missing fields go into `session.md` frontmatter, not a second metadata file.** The frontmatter is already the authoritative manifest (`SessionDoc::parse` in `crates/session-core/src/folder.rs`), and one manifest cannot disagree with itself when a sync delivers only part of a folder. *(session-settled: user-directed — chosen over a separate machine-only metadata file and over deriving defaults with no new fields: two files can disagree after a partial sync, and defaults leave the R25 gap open.)* Governs R3, R4.
- KD3. **Every action works on every computer, including Resume.** Resume reprocesses every part from the audio in the note folder (walking-skeleton R6d), and that audio syncs, so Resume does not need the recording computer. *(session-settled: user-directed — chosen over Resume only on the recording computer, after review showed that the working-state reason for that limit was false.)* Governs R13, R14.
- KD4. **The note folder is the authority for a saved note on every computer, including the one that recorded it.** This computer's database is a copy that follows the folder. Governs R9, R14.
- KD5. **Live updates, not only at launch.** *(session-settled: user-directed — chosen over a rescan at launch and on window focus, and over a rescan at launch plus a Refresh action.)* Governs R7, R8, R11.
- KD6. **Longhand reads the real file and shows a notice when a conflict copy exists.** The user resolves the conflict in Finder, and the notice says that there is something to resolve. *(session-settled: user-directed — chosen over ignoring conflict copies silently, after review showed that edits on every computer make conflicts more likely, and over listing the copy as a second note.)* Governs R15.
- KD7. **A missing storage folder is not a delete.** When a whole storage folder is unavailable (an unmounted drive, a sync client that is signed out), its notes stay listed. Only a note folder that is gone from an available storage folder counts as deleted. Governs R10, R16.
- KD8. **The rename to "Storage" is UI text only.** "Destination" stays the term in code, docs and `CONCEPTS.md`, the same way that the UI says "note" and the code says "session". *(session-settled: user-directed — the user limited the rename to the UI.)* Governs R17.
- KD9. **Notes from the user's other computers do not conflict with the "Nothing writes into Longhand" boundary in `STRATEGY.md`.** They are the user's own recordings, made by Longhand. The boundary is about outside context.

### Requirements

**The library**

- R1. Home lists every saved note in every storage folder that this computer retains (walking-skeleton R25a), whichever computer recorded it.
- R2. Home also lists this computer's notes that are not saved yet (recording, processing, waiting for a folder), as it does today. A note that is recording or processing on another computer does not appear until that computer saves it.
- R3. `session.md` frontmatter holds every field that Home and the note page need to show a saved note: at least the title and whether the user set it, the recognition model and the speakers found. The body of `session.md` stays readable for people.
- R4. A computer that has no database row for a note builds its view of the note from the note folder alone, with no loss. Deleting the database on any computer loses no user content (walking-skeleton R25).
- R5. Notes saved before this change, whose `session.md` lacks the new fields, still appear and open. Missing fields get safe defaults: a title counts as set by the user, so it is never replaced automatically.
- R6. A note appears in Home when its `session.md` is valid. Until its transcript, summary and notes files arrive, the note page shows those parts as waiting for the sync, and notes edits, Summarize Again and Resume wait for them. A hidden staging folder is never listed.

**Watching the storage folders**

- R7. While Longhand is open, a note folder that a sync adds to a storage folder appears in Home within seconds after its `session.md` is valid.
- R8. A change that a sync delivers to a listed note (a new summary, edited notes, a new title, a renamed folder) shows in Home and on an open note page within seconds. Longhand follows a renamed folder by the note's id, not by its path.
- R9. Before Longhand writes to a saved note (rename, notes, Summarize Again, Resume, the launch reconcile), it updates its own record of the note from the folder. No automatic step rewrites a title, notes or a folder name that a sync delivered.
- R10. A note counts as deleted only when its folder is gone from an available storage folder and no folder with the same note id appears there within a short wait. Then the note leaves Home, and an open note page for it stays open and says that the note was deleted.
- R11. At launch, Longhand brings Home up to date with changes that happened while it was closed, with no user action.
- R12. When an audio file is online-only on the sync service, the note still lists and opens. Playback waits for the file to download.

**Working with a note from another computer**

- R13. On any computer, the user can open a saved note, read its transcript and summary, edit its notes, rename it, summarize it again, resume it and move it to the Trash.
- R14. Summarize Again and Resume read the transcript from `transcript.md` and the notes from `notes.md` in the note folder, on every computer.
- R15. When a sync service makes a conflict copy of a note's file, or a second folder with the same note id, Longhand reads the real file or folder and ignores the copy for content. The note page shows a short notice that another version of the note exists and names the file, so the user can resolve it in Finder. A folder with a different note id is a different note and is listed, whatever its name.
- R16. A note whose storage folder is unavailable stays in Home and shows that its storage folder is unavailable. Opening it and all actions wait until the folder is available again.

**The term "Storage"**

- R17. Every place in the UI that names the folder where notes are saved says "Storage". Today the only remaining place is the recording-bar warning (`src/session/RecordingBar.tsx`, `Destination: …`).

### Acceptance Examples

- AE1. **Covers R1, R7.** Given the laptop and the desktop both save to one OneDrive folder, and the desktop app is open on Home. When the laptop saves a note and OneDrive syncs it to the desktop, then the note appears in the desktop's Home within seconds, with its title and date.
- AE2. **Covers R4, R11.** Given the desktop has a new install with an empty database. When the user chooses the shared folder in Settings > Storage, then Home lists every note that is already in that folder.
- AE3. **Covers R13, R14.** Given a note that the laptop recorded. When the user resumes it on the desktop, records two minutes and stops, then the note folder ends with the old audio, one new audio file, and a transcript and summary that cover both recordings. The laptop then shows the updated note.
- AE4. **Covers R8, R10.** Given the user renamed a note on the desktop, and the sync delivers the rename to the laptop as a removed folder followed by a new one. Then the laptop's Home shows the new title, and the laptop's open note page stays open on the same note.
- AE5. **Covers R9.** Given the laptop recorded a note, and the user renamed it and edited its notes on the desktop. When the user later runs Summarize Again on the laptop, then the summary uses the desktop's notes, and the folder keeps the desktop's title.
- AE6. **Covers R10, R16, KD7.** Given notes in a storage folder on an external drive. When the drive is unmounted, then the notes stay in Home and say that their storage folder is unavailable. When the user moves one note to the Trash on the other computer and the sync removes its folder, then that note leaves Home.
- AE7. **Covers R6.** Given a sync that delivered a note's `session.md` but not yet its `notes.md`. Then the note is listed, its notes show as waiting for the sync, and the notes editor does not accept edits. When `notes.md` arrives, the notes show and the editor accepts edits.
- AE8. **Covers R15.** Given the user edited notes on both computers before the sync caught up, and the sync service made `notes (1).md` beside `notes.md`. Then Longhand shows `notes.md`, and the note page shows a notice that names `notes (1).md`.

### Scope Boundaries

- Several people sharing one library (KD1).
- Search, and any other part of `docs/ideas/library-and-browsing.md`.
- Any in-app way to merge or resolve a sync conflict (KD6).
- Showing notes that another computer is recording or processing (R2).
- Renaming "Destination" in code, docs or `CONCEPTS.md` (KD8).
- Longhand syncs nothing itself. It depends on the sync service that the user already runs.

### Dependencies / Assumptions

- The user's sync service delivers each file whole, even if it delivers the files of one folder at different times. OneDrive, Google Drive and Dropbox all write to a temporary name and then rename.
- The user rarely works on one note on two computers at the same time. When that happens, R15 is the safety net.

### Outstanding Questions

**Deferred to Planning**

- How Home and the database divide the work: whether saved notes are always read from the folders, or the database keeps a rebuildable index that the watcher updates (R1, R4, R8, R9).
- How long the wait in R10 is, and how the watcher behaves on each sync service's folder on macOS and Windows, including events that come in bursts, files that are online-only, and File Provider clients that deliver a rename as a remove and an add (R7, R10, R12).
- Two computers can start notes in the same minute and choose the same folder name before either sees the other's folder. Some sync services then merge the two folders instead of making a conflict copy. How does a new note get a folder name that cannot clash across computers, while the title still names the folder (R15)?
- On a computer with no database row, how does Longhand tell the real folder from a conflict copy when two folders hold the same note id (R15)?
- When a sync client is signed out, it can remove online-only files but leave the storage folder in place, so notes look deleted under R10. Removing a note from Home must never delete this computer's own working state for it. How is that kept safe (R10, KD7)?
- A computer that runs an older Longhand can rewrite `session.md` without the new fields. How does a newer computer keep a proposed title from turning into a title set by the user (R3, R5)?
- What an open note page does when a sync changes a file that the user is editing on this computer at the same time (R8, R13).
- In what order the launch catch-up (R11) and the existing folder reconcile run, so that the reconcile never acts on a database record that a sync made old (R9, R11).

### Sources / Research

- `docs/ideas/cross-machine-library.md`: the idea this plan picks up.
- `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`: KD5, R6d (resume reprocesses from the folder's audio), R25, R25a, KTD10 (the database as working state plus a rebuildable row per saved session).
- `crates/session-core/src/folder.rs`: `SessionDoc`, the `session.md` frontmatter (id, started, title, speaker count, segments).
- `crates/session-core/migrations/0001_init.sql`, `0002_titles_and_notes.sql`: `title_source`, `asr_model`, `found_speakers` and `home_path`, which only the database holds.
- `crates/pipeline/src/save.rs`: the files a save writes (`session.md`, `transcript.md`, `summary.md`, `notes.md`), `set_notes`, which rewrites `notes.md` from the database, and `home_intact`, which checks the folder's audio before Resume.
- `crates/pipeline/src/summarize.rs`: `transcript_turns` and the notes read, which take Summarize Again's input from this computer's database.
- `crates/pipeline/src/folder_rename.rs`, `crates/session-save/src/naming.rs`: rename, the launch reconcile and the `-2` suffix rule.
- No crate in the workspace watches files today.
