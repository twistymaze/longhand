---
title: A synced note id became the path of a delete
date: 2026-10-15
category: security-issues
module: Note id read and delete (crates/session-core/src/note_record.rs, crates/session-core/src/note_folder.rs, crates/session-core/src/jobs.rs, crates/library/src/reconcile.rs)
problem_type: security_issue
component: service_layer
severity: critical
symptoms:
  - A codebase sweep found that any text in the session_id of a synced longhand.json or old-layout session.md became a row id
  - The delete of that note removed working_root.join(id), which is the id itself for an absolute path, a folder above the app data for "../..", and the whole working root for an empty id
root_cause: missing_validation
resolution_type: code_fix
framework_version: rust 1.94.0
related_components:
  - library
tags:
  - path-traversal
  - allowlist
  - synced-document
  - note-id
  - working-folder
  - delete
---

# A synced note id became the path of a delete

## Problem

The storage folder syncs between computers, and agents and people can edit its files. The library adopted each note folder with the `session_id` of its `longhand.json` (or of an old-layout `session.md`) as the row id. Nothing checked the form of the id. The app joins the row id to the working root in many places: the delete of a note, the failed Transcribe Again cleanup, launch recovery and capture. `Path::join` with an absolute id gives the id itself. So a note with `session_id: /Users/erv` made a delete of that note move the home folder to the Trash. PR #24 fixed it.

## Symptoms

- No user lost a folder. A sweep finder traced the path from the parse to `remove_dir_all`.
- An id of `../..` pointed above the app data folder. An empty id pointed at the working root, which holds the recovery chunks of every note.

## Solution

- `session_core::note_record::valid_session_id` is an allowlist of the form that Longhand writes: 1 to 64 characters, each a lowercase ASCII letter, a digit, `-` or `_`. Longhand writes lowercase UUID v4 text.
- Uppercase is refused. macOS and Windows file systems ignore case, so two ids that differ only in case would share one working folder, and a delete of one would remove the folder of the other.
- Windows device names (`con`, `prn`, `aux`, `nul`, `com1`-`com9`, `lpt1`-`lpt9`) are refused on each platform. The note folder syncs to each computer, and no folder on Windows can have such a name.
- `NoteRecord::parse` and the old-layout branch of `NoteFiles::parse` call the check, so every reader of a note folder gets it. The scan then lists the folder as invalid, and it is not adopted. `session_id_of` gives no id for a newer-layout file with a bad id.
- `delete_session` refuses a bad id before it takes its claim or removes anything (`DeleteError::InvalidId`).

## Why This Works

The check runs where untrusted text becomes an id, not where an id becomes a path. Every path join downstream then gets only ids of one safe shape. An allowlist of the written shape cannot miss a case that a deny-list of bad shapes would miss.

## Prevention

- Text from a synced file that the app later uses as a file name is untrusted input. Validate it at the parse, with an allowlist of the shape that the app writes.
- Ask two more questions about each allowlist: do two accepted values name the same file on a file system that ignores case, and can each accepted value be a file name on each supported platform?
- Add a second check at each destructive sink, for example the delete. It costs one line and stops an old or forgotten path.
- A folder that the rule refuses has no id, so it holds every row of its storage folder until it is corrected (see `Reconciler::apply`, the `unidentified` set). This keeps data during a rename, at the cost of a stale row.

## Related Issues

- [audio-removal-trusted-synced-file-names-before-delete.md](audio-removal-trusted-synced-file-names-before-delete.md): the same lesson for audio file names in `session.md`.
