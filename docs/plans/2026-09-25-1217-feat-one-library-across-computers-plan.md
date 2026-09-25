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

- **Objective:** A person who uses Longhand on several computers that sync one folder sees the same notes in Home on each computer, and can work with a note on any of them, whichever computer recorded it.
- **Means:** `session.md` becomes the complete record of a saved note (KD2). Home lists saved notes from the storage folders, not only from this computer's database (R1). Longhand watches the storage folders while it is open (R6).
- **Product authority:** This plan covers the shared library, the complete `session.md`, folder watching, and the change of the UI term "Destination" to "Storage". Search and the other parts of `docs/ideas/library-and-browsing.md` are not active scope. The Product Contract wins on behavior.
- **Open blockers:** None.

---

## Product Contract

### Summary

Each computer lists every saved note in its storage folders. That includes notes that another computer recorded and a sync service delivered. `session.md` holds everything that Home and the note page need, so a computer can rebuild its list from the folders alone. While Longhand is open, it watches the storage folders, so notes appear, change and disappear within seconds after a sync. A note can be opened, edited, renamed, summarized again and deleted on any computer. Only the computer that recorded a note can resume recording into it. The UI calls the storage folder "Storage" everywhere.

### Problem Frame

The user records on more than one computer, for example a work laptop and a home desktop, and all of them save to one folder that OneDrive, Google Drive or Dropbox syncs. Saving already works: the folders from each computer sit side by side, and each computer writes only to folders that it created. But Home is built from each computer's own database (`list_sessions` in `src-tauri/src/commands/session.rs`). So each computer lists only the notes that it recorded, and a note from the other computer is visible only in Finder.

Walking-skeleton `KD5` and `R25` already say that the folders hold the content and that the index holds nothing that cannot be rebuilt from them. The code does not fully meet that rule yet. The title source, the recognition model and the number of speakers found are only in the database, and `session.md` does not hold them.

### Key Decisions

- KD1. **One person, several computers, not several people.** The library is one user's notes across the user's own computers. Shared use by colleagues is out of scope. *(session-settled: user-directed — chosen over a folder that several people share: the user named this case explicitly.)*
- KD2. **The missing fields go into `session.md` frontmatter, not a second metadata file.** The frontmatter is already the authoritative manifest (`SessionDoc::parse` in `crates/session-core/src/folder.rs`), and one manifest cannot disagree with itself when a sync delivers only part of a folder. *(session-settled: user-directed — chosen over a separate machine-only metadata file and over deriving defaults with no new fields: two files can disagree after a partial sync, and defaults leave the R25 gap open.)* Governs R3, R4.
- KD3. **A note from another computer can do everything except resume recording.** Resume needs the working state (processing audio, recovery chunks), which stays in the app storage of the computer that recorded the note. *(session-settled: user-directed — chosen over read-only, over read-rename-delete, and over full parity with resume.)* Governs R11, R12.
- KD4. **Live updates, not only at launch.** *(session-settled: user-directed — chosen over a rescan at launch and on window focus, and over a rescan at launch plus a Refresh action.)* Governs R6, R7, R9.
- KD5. **Longhand ignores sync conflict copies.** For one person, conflicts are rare, and the user resolves them in Finder. *(session-settled: user-directed — chosen over a warning on the note and over listing the copy as a second note.)* Governs R13.
- KD6. **A missing storage folder is not a delete.** When a whole storage folder is unavailable (an unmounted drive, a sync client that is signed out), its notes stay listed. Only a note folder that is gone from an available storage folder counts as deleted. Governs R8.
- KD7. **The rename to "Storage" is UI text only.** "Destination" stays the term in code, docs and `CONCEPTS.md`, the same way that the UI says "note" and the code says "session". *(session-settled: user-directed — the user limited the rename to the UI.)* Governs R14.
- KD8. **Notes from the user's other computers do not conflict with the "Nothing writes into Longhand" boundary in `STRATEGY.md`.** They are the user's own recordings, made by Longhand. The boundary is about outside context.

### Requirements

**The library**

- R1. Home lists every saved note in every storage folder that this computer retains (walking-skeleton R25a), whichever computer recorded it.
- R2. Home also lists this computer's notes that are not saved yet (recording, processing, waiting for a folder), as it does today. A note that is recording or processing on another computer does not appear until that computer saves it.
- R3. `session.md` frontmatter holds every field that Home and the note page need to show a saved note: at least the title and whether the user set it, the recognition model, the speakers found, and the computer that recorded the note (R12). The body of `session.md` stays readable for people.
- R4. A computer that has no database row for a note builds its view of the note from the note folder alone, with no loss. Deleting the database on any computer loses no user content (walking-skeleton R25).
- R5. Notes saved before this change, whose `session.md` lacks the new fields, still appear and open. Missing fields get safe defaults: a title counts as set by the user, so it is never replaced automatically.

**Watching the storage folders**

- R6. While Longhand is open, a note folder that a sync adds to a storage folder appears in Home within seconds after its `session.md` is complete.
- R7. A change that a sync delivers to a listed note (a new summary, edited notes, a new title, a renamed folder) shows in Home and on an open note page within seconds. Longhand follows a renamed folder by the note's id, not by its path.
- R8. A note folder that disappears from an available storage folder disappears from Home. An open note page for it closes or says that the note was deleted. Notes in an unavailable storage folder stay listed (KD6).
- R9. At launch, Longhand brings Home up to date with changes that happened while it was closed, with no user action.
- R10. A folder that is not complete yet (partly synced, with no valid `session.md`) is not listed. A hidden staging folder is never listed. When an audio file is online-only on the sync service, the note still lists and opens. Playback waits for the file to download.

**Working with a note from another computer**

- R11. On any computer, the user can open a saved note, read its transcript and summary, edit its notes, rename it, summarize it again and move it to the Trash.
- R12. Resume is available only on the computer that recorded the note. On any other computer, the Resume control is not offered, and a short line says that the note can be resumed on the computer that recorded it.
- R13. A conflict copy that a sync service makes (a duplicate file or a duplicate note folder with the same note id) is not listed and not read. The real note folder and its files win.

**The term "Storage"**

- R14. Every place in the UI that names the folder where notes are saved says "Storage". Today the only remaining place is the recording-bar warning (`src/session/RecordingBar.tsx`, `Destination: …`).

### Acceptance Examples

- AE1. **Covers R1, R6.** Given the laptop and the desktop both save to one OneDrive folder, and the desktop app is open on Home. When the laptop saves a note and OneDrive syncs it to the desktop, then the note appears in the desktop's Home within seconds, with its title and date.
- AE2. **Covers R4, R9.** Given the desktop has a new install with an empty database. When the user chooses the shared folder in Settings > Storage, then Home lists every note that is already in that folder.
- AE3. **Covers R11, R12.** Given a note that the laptop recorded. When the user opens it on the desktop, then the user can edit its notes and summarize it again, and the page says that the note can be resumed on the computer that recorded it, with no Resume control.
- AE4. **Covers R7.** Given the user renamed a note on the desktop. When the rename syncs to the laptop, then the laptop's Home shows the new title and the laptop's open note page stays open on the same note.
- AE5. **Covers R8, KD6.** Given notes in a storage folder on an external drive. When the drive is unmounted, then the notes stay in Home. When the user moves one note to the Trash on the other computer and the sync removes its folder, then that note disappears from Home.
- AE6. **Covers R10.** Given a sync that delivered a note's audio but not yet its `session.md`. Then the note is not listed. When `session.md` arrives, then it appears.
- AE7. **Covers R13.** Given the user edited notes on both computers before the sync caught up, and the sync service made `notes (1).md` beside `notes.md`. Then Longhand shows `notes.md` and ignores the copy.

### Scope Boundaries

- Several people sharing one library (KD1).
- Search, and any other part of `docs/ideas/library-and-browsing.md`.
- Resume from a computer that did not record the note (KD3).
- Any UI to resolve sync conflicts (KD5).
- Showing notes that another computer is recording or processing (R2).
- Renaming "Destination" in code, docs or `CONCEPTS.md` (KD7).
- Longhand syncs nothing itself. It depends on the sync service that the user already runs.

### Dependencies / Assumptions

- The user's sync service delivers each file whole, even if it delivers the files of one folder at different times. OneDrive, Google Drive and Dropbox all write to a temporary name and then rename.
- Two computers that start a note in the same minute get distinct folder names from the existing `-2` suffix rule, or the sync service makes a conflict copy of the folder (R13 covers that case).
- Each computer can identify itself in a way that survives restarts and updates. If a reinstall gives a computer a new identity, its older notes lose Resume. That is acceptable, because a reinstall also removes the working state that Resume needs.

### Outstanding Questions

**Deferred to Planning**

- How Longhand identifies the computer that recorded a note, and how an old note from before R3 counts (R5, R12).
- How Home and the database divide the work: whether saved notes are always read from the folders, or the database keeps a rebuildable index that the watcher updates (R1, R4, R7).
- How the watcher behaves on each sync service's folder on macOS and Windows, including events that come in bursts and files that are online-only (R6, R10).
- What an open note page does when a sync changes a file that the user is editing on this computer at the same time (R7, R11).

### Sources / Research

- `docs/ideas/cross-machine-library.md`: the idea this plan picks up.
- `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`: KD5, R25, R25a, KTD10 (the database as working state plus a rebuildable row per saved session).
- `crates/session-core/src/folder.rs`: `SessionDoc`, the `session.md` frontmatter (id, started, title, speaker count, segments).
- `crates/session-core/migrations/0001_init.sql`, `0002_titles_and_notes.sql`: `title_source`, `asr_model`, `found_speakers` and `home_path`, which only the database holds.
- `crates/pipeline/src/save.rs`: the files a save writes (`session.md`, `transcript.md`, `summary.md`, `notes.md`), and `set_notes`, which rewrites `notes.md` on a saved note.
- `crates/pipeline/src/folder_rename.rs`, `crates/session-save/src/naming.rs`: rename and the `-2` suffix rule.
- No crate in the workspace watches files today.
