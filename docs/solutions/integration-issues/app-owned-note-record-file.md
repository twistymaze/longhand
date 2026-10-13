---
title: Longhand's note records are in an app-owned file, and every .md file is the user's
date: 2026-10-13
category: integration-issues
module: "Note folders (crates/session-core/src/note_record.rs, note_folder.rs; crates/session-save/src/publication.rs, note_files.rs; crates/library/src/scan.rs, reconcile.rs, status.rs; crates/pipeline/src/convert.rs)"
problem_type: integration_issue
component: service_layer
severity: high
applies_when:
  - "A new record of a note is added, or a writer of a note folder changes"
  - "A person, an agent or a sync service changes a file of a note folder while Longhand runs"
  - "A run (Transcribe Again, Regenerate Summary, Resume) checks whether the note changed during the run"
  - "A reader of a note folder must decide whether the folder is a note, a problem or a partial sync"
symptoms:
  - "A title fix or a different YAML style in session.md makes a run refuse to save"
  - "A session.md that does not parse removes the note from the note list with no message"
  - "A title that a sync delivers before longhand.json reads as a title that the user set"
resolution_type: design_rule
tags: [note-folder, longhand-json, agents, sync, storage-folder, conversion, old-layout, fingerprint]
related_components: [library, pipeline, session_save]
---

# Longhand's note records are in an app-owned file, and every .md file is the user's

## Problem

Before this change, `session.md` held the title, the tags and all of Longhand's records of a note. The storage folder is plain files so that a person or an AI agent can work on the notes outside the app. Some edits that looked safe caused faults:

- Transcribe Again, Regenerate Summary and Resume compared a fingerprint of the whole `session.md`, body included. A title fix, or a YAML tool that wrote the same keys in a different style, made the run refuse to save.
- A `session.md` that did not parse removed the note from the note list, and Longhand showed no message.

## Rules

1. **Each `.md` file of a note folder is the user's. `longhand.json` is Longhand's.** A person or an agent can edit `session.md`, `notes.md`, `transcript.md` and `summary.md`. `session.md` holds only `title` and `longhand_tags`. Longhand changes it with a frontmatter patcher that replaces only the lines of the keys that it changes, and keeps other keys, comments, the key order and the body. Longhand never writes a body into `session.md`. The note record file `longhand.json` holds every record: the session id, the start time, the segments, the result flag, the recognition data, the fingerprints of `transcript.md` and `summary.md`, the summary data, the pending type and the title that Longhand last wrote (`NoteRecord` in `note_record.rs`). Never add a record to an `.md` file. The files that are not text (`recognition.jsonl`, the audio, the in-use markers) are Longhand's too. The skill `plugins/longhand/skills/longhand-notes/` gives agents the same list.

2. **`longhand.json` is the last file of each write.** A publication writes `recognition.jsonl`, `transcript.md`, `summary.md`, `session.md` when the folder has none, and then `longhand.json` (`publication.rs`). The early home write ends with `longhand.json` too. A sync service can deliver the files in any order, and a crash can stop a write at any file. The fingerprints in `longhand.json` tell a reader whether `transcript.md` and `summary.md` are the files of the same write. A difference is a partial sync for a short time, and a hand edit when it lasts about one minute (`HAND_EDIT_WAIT` in `reconcile.rs`). A folder with a new-layout `session.md` and no `longhand.json` is a partial sync, not a note: no adopt and no write (`waiting_for_record` in `scan.rs`). The folder can be a home folder that a rename moved, so while it waits, no note of that storage folder is forgotten.

3. **`longhand.json` is deterministic.** It has sorted keys, LF line ends, two spaces of indent and no write time (`NoteRecord::render`). Two computers that write the same records write the same bytes, so a sync service makes no conflict copy for an equal write. A `layout` number larger than this build knows means a newer Longhand wrote the file. This build then does not use or write the records of that note.

4. **The run-change check compares the parsed records, not file text.** `NoteRecord::run_fingerprint` (`note_folder.rs`) is the fingerprint of the parsed records without the title that Longhand last wrote, the pending type and `layout`. A run takes it at the start and compares it before it saves. The text style of a file does not count, and no edit to an `.md` file changes it. A title proposal or a type move during a run then does not make the run refuse. A new result of another computer changes the records, so the run refuses for it, as before.

5. **A `session.md` title is the user's title only after about one minute.** A title change writes `session.md` and `longhand.json`, and a sync can deliver them in either order. When the title in `session.md` differs from `written_title` in `longhand.json`, the reconcile starts a wait (`TitleWait` in `reconcile.rs`, with `HAND_EDIT_WAIT`). When the difference stays the same for about one minute, a person or an agent changed the title, and Longhand never replaces it automatically. Until then, Longhand keeps the title source that it has and writes nothing for that note. When a scan finds other values, the wait starts again. Without the wait, a computer that got the new `session.md` first would mark a title that processing proposed as the user's title.

6. **A problem note stays visible, and Longhand writes nothing into it.** A note folder whose `session.md` or `longhand.json` does not parse, or that waits for its `longhand.json`, is a problem folder (`ProblemKind` in `scan.rs`). Longhand does not change its files. After about one minute (`PROBLEM_WAIT` in `status.rs`), Home shows one notice that counts the problem notes and names each one with what the user can do: correct the line of `session.md` that has the error, wait for `longhand.json` to sync or update Longhand on the other computer, or update Longhand on this computer for a newer layout. A file of a newer layout shows at once, because a sync does not change it. A shorter problem is part of a sync and shows nothing. A note that this computer knows stays in the note list. Rename, Record and the actions that write to the note wait and show the same reason. Delete stays available.

7. **The conversion of old notes writes the new file first and removes the old records last, and it can run again.** In the old layout, `session.md` holds `session_id` and all the records, and `summary.md` holds the summary data in its frontmatter. A `session.md` that holds `session_id` is the old layout, also when the folder has a `longhand.json`. The conversion (`convert.rs`) writes one note in this order:
   1. `longhand.json`, from the unchanged `session.md` and `summary.md`, with the fingerprints of the files as they are after the conversion.
   2. `summary.md` without the summary data. Other keys, comments and the body stay.
   3. `session.md` without the records. The title, the tags, other keys and the body stay.

   Until step 3, `session.md` holds `session_id`, so every reader still reads the old records, and no state between two steps loses a record. A run after a stop at any step reads the same records and writes the same files. After a stop at step 2, `summary.md` no longer has the summary data, so the second run takes it from the `longhand.json` of the first run. The output has no write time, so two computers that convert one note write the same bytes. The conversion skips a note that a computer records or processes, and a note whose `session.md` changed during the conversion. The next run converts them. Only the user starts the conversion, with Convert in the notice on Home: a read, a scan or a launch never converts a note.

## Prevention

- A new record of a note goes into `NoteRecord`, never into an `.md` file. Decide whether `run_fingerprint` must compare it. A record that a title change or a type move writes must stay outside the compare.
- A new writer of a note folder writes `longhand.json` last and adds a test that stops the write before `longhand.json`.
- A change to a writer makes the tests on `testdata/storage-folder/` and `crates/library/tests/skill_drift.rs` fail until the test folder is written again and the skill scripts know the change ([Skill script tests and the test storage folder](../../development-workflow.md#skill-script-tests-and-the-test-storage-folder)).
- A new problem state of a note folder follows [the folder notice rule](../ui-bugs/folder-notice-for-a-move-the-app-does-without-help.md): the normal path shows nothing, and a partial sync shows nothing for about one minute.

## Related Issues

- [shared-settings-in-a-synced-folder-rules](shared-settings-in-a-synced-folder-rules.md): the same rules for the files in `.longhand/`.
- [receiving-computer-renamed-synced-folder-back](../logic-errors/receiving-computer-renamed-synced-folder-back.md): a computer that only receives a sync writes nothing.
- Plan: `docs/plans/2026-10-12-1827-feat-safe-agent-edits-plan.md` (KTD1 to KTD7, R1 to R13).
