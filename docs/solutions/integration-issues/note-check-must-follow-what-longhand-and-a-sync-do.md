---
title: The note check told agents to remove Longhand's own folders, and accepted files that Longhand cannot read
date: 2026-10-17
category: integration-issues
module: longhand-notes agent skill (plugins/longhand/skills/longhand-notes/scripts/check.py, _longhand.py, list_notes.py)
problem_type: integration_issue
component: tooling
severity: medium
symptoms:
  - "check.py compare said 'Remove it.' for a .longhand-staging-*, .longhand-rename-* or .longhand-move-* folder that Longhand made while the agent worked"
  - "A note that Longhand moved into a rename work folder was reported as 'the note folder is gone'"
  - "A new folder that a sync had not finished was a problem with 'Remove it.'"
  - "A session.md that starts with a byte order mark passed, but Longhand cannot read it"
  - "Two folders with one session_id from before the snapshot were reported as a new copy and a move that the agent did not make"
root_cause: logic_error
resolution_type: code_fix
tags:
  - agent-skill
  - check-py
  - work-folder
  - sync
  - byte-order-mark
  - rust-python-agreement
  - snapshot
---

# The note check told agents to remove Longhand's own folders, and accepted files that Longhand cannot read

## Problem

`check.py snapshot` and `check.py compare` prove what an agent edit changed in a storage folder. The check is only correct when it knows what Longhand and a sync service also do in that folder at the same time. It did not know that Longhand makes short-lived work folders, that a sync delivers a note in parts, or how the Rust readers treat a byte order mark and an empty prompt. So it told agents to delete folders that Longhand owned, and it passed files that the app cannot read (PR #33).

## Root cause and solution

**Longhand's work folders.** Longhand makes these hidden names for a short time:

| Name | Made by | Where |
|---|---|---|
| `.longhand-staging-<id>` | `STAGING_PREFIX`, `crates/session-save/src/naming.rs` | storage folder root |
| `.longhand-rename-<id>` | `TEMP_PREFIX`, `crates/session-save/src/rename_home.rs` | the note's parent folder, so a type folder for a typed note |
| `.longhand-move-<id>` | `STAGING_PREFIX`, `crates/pipeline/src/move_notes.rs` | the new storage folder |
| `.longhand-move-probe` | `PROBE`, `move_notes.rs` | both storage folders |
| `.longhand-probe-<pid>` | `crates/session-save/src/lib.rs` | the destination folder |

`check.py` now gives a warning ("Do not touch it") for a new work folder and passes. It skips the probes. `.longhand-move-probe` starts with the move prefix, so `is_skipped` checks the exact probe name before `is_work_folder` sees the prefix. A note that is gone while a new work folder is present is "not checked" (exit 2), not a problem.

The review found a second defect in the first fix: the type-folder loop of `other_entries` took every child that holds `session.md` as a note folder, also a hidden one. A note in `<type>/.longhand-rename-<id>` was then neither a note (`lh.note_folders` skips hidden names) nor a new entry, so it was "gone". **Every loop that decides "this is a note folder" must skip hidden names, as `lh.note_folders` does.** The root loop already did.

**Folders that a sync delivers.** A new folder at the root or in a type folder, hidden or not, that is not a complete note can be a note that a sync has not delivered yet, or another app's folder. It is "not checked", and the text says to compare again later and to remove it only if the agent made it. The check never says "Remove it" for a folder. A new file is still a problem.

**Rust reader agreement, per file kind.** The Python readers must follow the Rust reader of each file, not one rule for all files:

- `SessionMd::parse` (crates/session-core) does not remove a byte order mark, so Longhand cannot read a `session.md` that starts with one. `read_frontmatter` no longer removes it. `read_session_md` raises `Unreadable`, `compare` reports it as a problem when the note was touched or moved, and `list_notes.py` does not show the note as ready.
- `parse_prompt_file` in `crates/shared-settings/src/prompts.rs` removes the byte order mark, so `parse_prompt` keeps its strip.
- `read_entry` in `prompts.rs` does not use a prompt when `!too_large && parsed.text.trim().is_empty()`. Rust's `trim` uses the Unicode White_Space property. Python's `str.isspace` also accepts U+001C to U+001F, so the check uses `lh.WHITE_SPACE`, not `strip()`.

**Copied notes from before the snapshot.** `compare_notes` keyed notes by `session_id` with a dict, so for two folders with one key it kept the last folder in one state and the first in the other. Now `notes_by_key` groups both states in path order. Each path is compared with itself first. The paths that are left pair in path order as moves. A duplicate is reported only when the key has more folders than before.

**Snapshot format.** `other_entries` now maps each path to whether it is a folder, so `SNAPSHOT_FORMAT` went from 2 to 3. An old snapshot stops with "not a snapshot of this script". Bump the format each time the snapshot shape changes.

## Prevention

- When Longhand gets a new hidden work name in the storage folder or a type folder, add it to `check.py` (`WORK_FOLDER_PREFIXES`, or the probe names). The names are copies of the Rust constants above, and no test links the two lists.
- When you add a rule to a Python reader, find the Rust reader of that file kind and copy its rule, with a test for the case where they differ (a byte order mark, white space, quotes).
- A check result for a folder that Longhand or a sync can own is a warning or "not checked", never a problem that tells the agent to delete.

## Related

- [app-owned-note-record-file.md](app-owned-note-record-file.md): a partial sync is a normal state.
- [../logic-errors/folder-in-use-taken-for-a-taken-name.md](../logic-errors/folder-in-use-taken-for-a-taken-name.md): the case-only rename through `.longhand-rename-<id>`.
- [../logic-errors/session-md-tag-and-title-text-changed-by-yaml-parse.md](../logic-errors/session-md-tag-and-title-text-changed-by-yaml-parse.md): earlier Rust and Python reader differences.
