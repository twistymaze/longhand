---
title: Audio removal trusted the file names in a synced session.md before a delete
date: 2026-09-30
category: security-issues
module: Audio removal (crates/pipeline/src/remove_audio.rs)
problem_type: security_issue
component: service_layer
severity: high
symptoms:
  - Code review found that the delete step removed files that a synced session.md names, checked against a case-sensitive deny-list
  - On case-insensitive APFS, a session.md that names Session.md could delete a document instead of an audio file
  - A symlinked audio folder, for example audio -> ., could point the delete at a document of the note
root_cause: missing_validation
resolution_type: code_fix
related_components:
  - library
tags:
  - audio-removal
  - allowlist
  - path-traversal
  - symlink
  - case-insensitive-apfs
  - canonicalize
  - synced-document
---

# Audio removal trusted the file names in a synced session.md before a delete

## Problem

Audio removal deletes the audio files that the segments of a note name. The step reads each name from the note's `session.md` in the home folder. The storage folder syncs across computers, so `session.md` can come from another computer or from a person who edited it. Thus the names are not trusted data.

Before PR #28, `safe_audio_name` used a deny-list. It refused an empty name, an absolute name, a name with a `.` or `..` part, and the four document names in `library::conflict::DOCUMENTS` (`crates/library/src/conflict.rs:12`). The comparison was case-sensitive. The folder check then accepted any canonical parent folder that started with the home folder (`dir.starts_with(home)`). The code review of PR #28 (finding #1) found that these two checks let the step delete a file that is not audio.

## Symptoms

No user saw a lost file. The review found these paths:

- A `session.md` lists `Session.md` or `SUMMARY.MD` as a segment's audio. The deny-list compares case-sensitively, so the name passes. On case-insensitive APFS, `Session.md` opens `session.md`. The step writes a removal marker and deletes the session document of the note.
- A conflict copy such as `notes (1).md` is not in the deny-list, so the step deletes it.
- The `audio` folder is a symlink, for example `audio -> .` or `audio -> /some/other/folder`. The name `audio/session.md` is a plain relative name, so it passes. With `audio -> .`, the canonical parent is the home folder, so `starts_with(home)` is true and the step deletes `session.md`. The same check accepts any subfolder of the home folder.
- A segment name that points to a folder passes. The step does not check the kind of entry before the delete.

## What Didn't Work

- A deny-list of document names. Each new document, conflict-copy pattern or case variant needs one more entry. The file system decides whether two names are the same file (case folding, Unicode normalization), not the string comparison, so the list is never complete.
- A check that the canonical parent is inside the home folder (`starts_with`). This check stops `..` and absolute paths. It does not stop a symlinked `audio` folder that points back into the home folder, and it accepts every subfolder. "Inside the note" is too wide a target: the documents are also inside the note.

## Solution

PR #28 replaces the deny-list with an allowlist of the exact shape that Longhand writes, adds a directory refusal, and compares the canonical parent with the canonical `home/audio` folder.

Before (`safe_audio_name`, deny-list):

```rust
pub fn safe_audio_name(audio_file: &str) -> bool {
    if audio_file.is_empty() || audio_file.ends_with('/') || audio_file.ends_with('\\') {
        return false;
    }
    // A document of the note folder is not audio: the step does not
    // delete it.
    if library::conflict::DOCUMENTS.contains(&audio_file) {
        return false;
    }
    let path = Path::new(audio_file);
    !path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}
```

After (`crates/pipeline/src/remove_audio.rs:97-118`, allowlist):

```rust
const AUDIO_FOLDER: &str = "audio";

pub fn safe_audio_name(audio_file: &str) -> bool {
    let Some((folder, file)) = audio_file.split_once('/') else {
        return false;
    };
    let Some(number) = file
        .strip_prefix("segment-")
        .and_then(|rest| rest.strip_suffix(".opus"))
    else {
        return false;
    };
    folder == AUDIO_FOLDER && !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
}
```

Before (`remove_segment`, folder check):

```rust
let dir = match dir.canonicalize() {
    Ok(dir) if dir.starts_with(home) => dir,
    Ok(_) => return SegmentOutcome::UnsafeName,
    // ...
};
let path = dir.join(name);
// ...
let present = path.symlink_metadata().is_ok();
if present { /* delete */ }
```

After (`crates/pipeline/src/remove_audio.rs:326-357`):

```rust
if !safe_audio_name(audio) {
    return SegmentOutcome::UnsafeName;
}
// ...
// The audio folder must be a real folder in the home folder. A link,
// for example `audio -> .`, resolves to a different path.
let dir = match dir.canonicalize() {
    Ok(dir) if dir == home.join(AUDIO_FOLDER) => dir,
    Ok(_) => return SegmentOutcome::UnsafeName,
    // ...
};
let path = dir.join(name);
// Metadata only: an online-only file is not downloaded (R12).
let entry = path.symlink_metadata().ok();
// The step deletes files only, never a folder.
if entry.as_ref().is_some_and(|meta| meta.is_dir()) {
    return SegmentOutcome::UnsafeName;
}
```

`home` is the canonical home folder (`crates/pipeline/src/remove_audio.rs:283-288`), so the comparison is canonical path against canonical path. An unsafe name gets the `SegmentOutcome::UnsafeName` outcome. The step writes no removal marker for it and does not set its removed flag (`crates/pipeline/src/remove_audio.rs:309-315`). `finish_removal` also filters by `safe_audio_name` before it reads a marker (`crates/pipeline/src/remove_audio.rs:293-298`).

## Why This Works

- The allowlist accepts one shape only: `audio/segment-<digits>.opus`. The shape is case-sensitive and has exactly one `/`. No document name, conflict copy, case variant, `.`, `..`, empty part or absolute path can match it. The check does not depend on a list of the files that must stay, so a new document type needs no change.
- The name check uses text only. The folder check uses the file system: `canonicalize` resolves every symlink, and the result must be equal to `<canonical home>/audio`. A symlinked `audio` folder resolves to a different path, so the step refuses it. Equality is narrower than `starts_with`, so a subfolder of the home folder also fails.
- The directory refusal uses `symlink_metadata`, which reads metadata only. It does not follow a final symlink and does not download an online-only file (R12). A folder with an audio name stays.
- The three checks are independent. Each one blocks a different path: the name check blocks text tricks, the canonical check blocks links, and the kind check blocks folders.

General lesson: when synced (untrusted, cross-computer) data names a file to delete, accept only the exact shape that the app itself writes, and confirm the resolved location against the one folder that holds that shape. Do not deny-list names. Case folding, `..` and symlinks defeat a deny-list.

## Prevention

- For each delete whose path comes from a synced file, write the name check as an allowlist of the shape that the app writes. Keep the shape in one constant or function, and use the same function in each step that reads the name (here `remove_audio_with` and `finish_removal`).
- Compare the canonical parent with the canonical expected folder by equality. Do not use `starts_with` on a home or storage folder.
- Refuse a directory before the delete. Read the entry with `symlink_metadata`, not `metadata`.
- Test the name check with a table of hostile names, and test the file-system cases with real symlinks and folders. The tests in `crates/pipeline/tests/remove_audio.rs` do this:
  - `only_an_audio_name_that_longhand_writes_is_safe` (line 338): a table of names, which includes `Session.md`, `SUMMARY.MD`, `notes (1).md`, `Audio/segment-01.opus`, `audio/Segment-01.opus`, `audio//segment-01.opus` and `audio/../../segment-01.opus`.
  - `a_document_name_in_another_case_or_a_conflict_copy_is_not_deleted` (line 400).
  - `an_audio_folder_that_links_to_the_home_folder_deletes_no_document` (line 446).
  - `a_folder_with_an_audio_name_is_not_deleted` (line 476).
  - `a_name_that_resolves_outside_the_home_folder_is_unsafe` (line 538).
  - `finish_deletes_no_document_that_a_marker_names` (line 787).

Test sketch for a new delete step that reads names from synced data:

```rust
#[cfg(unix)]
#[test]
fn a_linked_audio_folder_and_a_case_variant_delete_no_document() {
    let segments = vec![
        segment(0, "2026-09-01T09:01:00-05:00", "Session.md"),
        segment(1, "2026-09-01T09:02:00-05:00", "audio/session.md"),
    ];
    let note = saved_note(&segments);
    // `audio -> .` makes `audio/session.md` the session document.
    fs::remove_dir_all(note.home.join("audio")).unwrap();
    std::os::unix::fs::symlink(".", note.home.join("audio")).unwrap();
    let before = documents(&note.home);

    let removal = run(&note);

    assert_eq!(
        outcomes(&removal),
        vec![SegmentOutcome::UnsafeName, SegmentOutcome::UnsafeName]
    );
    assert_eq!(documents(&note.home), before);
    assert!(!note.home.join("session.md.removed").exists());
    assert!(flags(&note).is_empty());
}
```

Run the case-variant test on case-insensitive APFS (the macOS default). On a case-sensitive volume, `Session.md` and `session.md` are different files, and a deny-list bug does not show.

## Related Issues

- [Shared settings in a synced folder: rules](../integration-issues/shared-settings-in-a-synced-folder-rules.md): the same storage folder syncs; that doc covers how the app reads shared files, this doc covers deletes that synced files name.
- PR #28 (audio removal), code review finding #1.
