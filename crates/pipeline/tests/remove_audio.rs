//! The removal step (U3 of the audio removal plan): it removes the audio of
//! one saved note, and a run after each partial effect finishes the work.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, TimeDelta};
use pipeline::SharedStore;
use pipeline::remove_audio::{
    Eligibility, EligibilityInputs, FolderCheck, Guard, RemoveAudioError, SegmentOutcome,
    StoreCheck, eligibility, finish_removal, no_guard, remove_audio, remove_audio_with,
    safe_audio_name,
};
use session_core::Segment;
use session_core::folder::{REMOVAL_MARKER_TEXT, SessionDoc, fingerprint, removal_marker};
use session_core::jobs::{Adoption, Claims};

const ID: &str = "20260901-0900-abcd";
const TRANSCRIPT: &str = "# Transcript\n\n- [00:00:01] Speaker 1: Hello.\n";
const SUMMARY: &str = "---\nstatus: stub\nfailed_stage: summarize\n---\n\nNo summary.\n";
const NOTES: &str = "My notes\n";

fn time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

/// A segment of one minute that ends at `end`.
fn segment(index: u32, end: &str, audio: &str) -> Segment {
    let end = time(end);
    Segment {
        index,
        offset_ms: u64::from(index) * 60_000,
        duration_ms: 60_000,
        wall_start: end - TimeDelta::minutes(1),
        wall_end: end,
        audio_file: audio.into(),
        channels: 1,
    }
}

fn audio(index: u32) -> String {
    format!("audio/segment-{:02}.opus", index + 1)
}

/// Three segments that end on 1 September.
fn three_segments() -> Vec<Segment> {
    (0..3)
        .map(|i| segment(i, &format!("2026-09-01T09:0{}:00-05:00", i + 1), &audio(i)))
        .collect()
}

fn doc(segments: &[Segment]) -> SessionDoc {
    let mut doc = SessionDoc::new(ID, None, segments).unwrap();
    doc.title = Some("Planning".into());
    doc.transcript_fingerprint = Some(fingerprint(TRANSCRIPT.as_bytes()));
    doc.summary_fingerprint = Some(fingerprint(SUMMARY.as_bytes()));
    doc
}

struct Note {
    /// Holds the storage folder and the files outside the home folder.
    _root: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    store: SharedStore,
    claims: Claims,
}

/// A saved note folder, as a save leaves it, with one audio file for each
/// segment whose audio name is safe, and its row in the store.
fn saved_note(segments: &[Segment]) -> Note {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let home = root_path.join("Planning");
    fs::create_dir_all(home.join("audio")).unwrap();
    let doc = doc(segments);
    fs::write(home.join("session.md"), doc.render()).unwrap();
    fs::write(home.join("transcript.md"), TRANSCRIPT).unwrap();
    fs::write(home.join("summary.md"), SUMMARY).unwrap();
    fs::write(home.join("notes.md"), NOTES).unwrap();
    for segment in segments {
        if safe_audio_name(&segment.audio_file) {
            fs::write(home.join(&segment.audio_file), b"opus").unwrap();
        }
    }
    let store = common::store();
    let adoption = Adoption::from_doc(
        &doc,
        NOTES.into(),
        home.to_string_lossy().into_owned(),
        Some(fingerprint(TRANSCRIPT.as_bytes())),
    );
    assert!(store.lock().unwrap().adopt(&adoption).unwrap());
    Note {
        _root: root,
        root: root_path,
        home,
        store,
        claims: Claims::default(),
    }
}

fn run(note: &Note) -> pipeline::remove_audio::Removal {
    remove_audio(&note.store, &note.claims, ID, no_guard()).unwrap()
}

fn outcomes(removal: &pipeline::remove_audio::Removal) -> Vec<SegmentOutcome> {
    removal.segments.iter().map(|s| s.outcome.clone()).collect()
}

fn marker(home: &Path, audio: &str) -> PathBuf {
    home.join(removal_marker(audio))
}

fn flags(note: &Note) -> Vec<String> {
    note.store.lock().unwrap().removed_audio(ID).unwrap()
}

/// Every file under `dir` with its content, sorted by path.
fn contents(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(contents(&path));
        } else {
            out.push((path.clone(), fs::read(&path).unwrap()));
        }
    }
    out.sort();
    out
}

fn documents(home: &Path) -> Vec<Vec<u8>> {
    ["session.md", "transcript.md", "summary.md", "notes.md"]
        .iter()
        .map(|name| fs::read(home.join(name)).unwrap())
        .collect()
}

// ---------------------------------------------------------------- the step

#[test]
fn ae1_a_saved_note_loses_all_audio_and_keeps_its_documents() {
    // Covers AE1, R5, R6.
    let segments = three_segments();
    let note = saved_note(&segments);
    let before = documents(&note.home);

    let removal = run(&note);

    assert_eq!(outcomes(&removal), vec![SegmentOutcome::Removed; 3]);
    assert!(removal.row_changed);
    for segment in &segments {
        assert!(!note.home.join(&segment.audio_file).exists());
        assert_eq!(
            fs::read_to_string(marker(&note.home, &segment.audio_file)).unwrap(),
            REMOVAL_MARKER_TEXT
        );
    }
    assert_eq!(documents(&note.home), before);
    assert_eq!(flags(&note), vec![audio(0), audio(1), audio(2)]);
    assert_eq!(
        pipeline::save::audio_state(&note.store.lock().unwrap(), ID).unwrap(),
        pipeline::save::AudioState::Removed
    );
    // The removal writes no temporary file that stays.
    let names: Vec<String> = fs::read_dir(note.home.join("audio"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 3, "{names:?}");
}

#[test]
fn a_second_run_on_a_removed_note_changes_nothing() {
    let note = saved_note(&three_segments());
    run(&note);
    let folder = contents(&note.home);
    let flags_before = flags(&note);

    let removal = run(&note);

    assert_eq!(outcomes(&removal), vec![SegmentOutcome::AlreadyRemoved; 3]);
    assert!(!removal.row_changed);
    assert!(removal.complete());
    assert_eq!(contents(&note.home), folder);
    assert_eq!(flags(&note), flags_before);
}

#[test]
fn a_run_after_a_crash_between_the_marker_and_the_delete_deletes_the_file() {
    // KTD2: the marker is there and the audio is still there.
    let segments = three_segments();
    let note = saved_note(&segments);
    for segment in &segments {
        fs::write(marker(&note.home, &segment.audio_file), REMOVAL_MARKER_TEXT).unwrap();
    }

    let removal = run(&note);

    assert_eq!(outcomes(&removal), vec![SegmentOutcome::Removed; 3]);
    for segment in &segments {
        assert!(!note.home.join(&segment.audio_file).exists());
        assert!(marker(&note.home, &segment.audio_file).exists());
    }
    assert_eq!(flags(&note), vec![audio(0), audio(1), audio(2)]);
}

#[test]
fn a_run_after_the_marker_and_the_delete_only_sets_the_flags() {
    // The files are done, but the store did not get the flags. The refresh
    // gives the flags, and the step changes no file.
    let segments = three_segments();
    let note = saved_note(&segments);
    for segment in &segments {
        fs::write(marker(&note.home, &segment.audio_file), REMOVAL_MARKER_TEXT).unwrap();
        fs::remove_file(note.home.join(&segment.audio_file)).unwrap();
    }
    assert!(flags(&note).is_empty());
    let folder = contents(&note.home);

    let removal = run(&note);

    assert_eq!(outcomes(&removal), vec![SegmentOutcome::AlreadyRemoved; 3]);
    assert!(removal.row_changed);
    assert_eq!(contents(&note.home), folder);
    assert_eq!(flags(&note), vec![audio(0), audio(1), audio(2)]);
}

#[test]
fn a_run_after_one_of_two_segments_is_done_finishes_the_other() {
    let segments = &three_segments()[..2];
    let note = saved_note(segments);
    fs::write(marker(&note.home, &audio(0)), REMOVAL_MARKER_TEXT).unwrap();
    fs::remove_file(note.home.join(audio(0))).unwrap();

    let removal = run(&note);

    assert_eq!(
        outcomes(&removal),
        vec![SegmentOutcome::AlreadyRemoved, SegmentOutcome::Removed]
    );
    assert!(!note.home.join(audio(1)).exists());
    assert!(marker(&note.home, &audio(1)).exists());
    assert_eq!(flags(&note), vec![audio(0), audio(1)]);
}

#[test]
fn a_segment_with_no_audio_file_gets_its_marker() {
    // A Waiting segment: the sync did not deliver the audio yet. The marker
    // makes the audio removed, and a later pass deletes a late file (KTD2).
    let note = saved_note(&three_segments()[..1]);
    fs::remove_file(note.home.join(audio(0))).unwrap();

    let removal = run(&note);

    assert_eq!(outcomes(&removal), vec![SegmentOutcome::Removed]);
    assert!(marker(&note.home, &audio(0)).exists());
    assert_eq!(flags(&note), vec![audio(0)]);

    // The sync delivers the audio late. The next run deletes it.
    fs::write(note.home.join(audio(0)), b"opus").unwrap();
    let removal = run(&note);
    assert_eq!(outcomes(&removal), vec![SegmentOutcome::Removed]);
    assert!(!note.home.join(audio(0)).exists());
}

#[test]
fn a_failed_delete_leaves_the_marker_and_the_next_run_finishes() {
    let note = saved_note(&three_segments()[..2]);
    let failing = note.home.join(audio(1));

    let removal = remove_audio_with(&note.store, &note.claims, ID, no_guard(), |path| {
        if path == failing {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        } else {
            fs::remove_file(path)
        }
    })
    .unwrap();

    assert_eq!(removal.segments[0].outcome, SegmentOutcome::Removed);
    assert!(matches!(
        removal.segments[1].outcome,
        SegmentOutcome::Failed(_)
    ));
    assert!(!removal.complete());
    assert!(failing.exists());
    assert!(marker(&note.home, &audio(1)).exists());
    // The marker makes the audio removed, so the flag is set.
    assert_eq!(flags(&note), vec![audio(0), audio(1)]);

    let removal = run(&note);
    assert_eq!(
        outcomes(&removal),
        vec![SegmentOutcome::AlreadyRemoved, SegmentOutcome::Removed]
    );
    assert!(!failing.exists());
}

#[cfg(unix)]
#[test]
fn a_delete_that_the_file_system_refuses_is_a_failure() {
    use std::os::unix::fs::PermissionsExt;
    // The marker is there from a run before. The audio folder is read-only,
    // so the delete fails.
    let note = saved_note(&three_segments()[..1]);
    fs::write(marker(&note.home, &audio(0)), REMOVAL_MARKER_TEXT).unwrap();
    let dir = note.home.join("audio");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    // A process that ignores permissions (root) cannot test this.
    let probe = dir.join("probe");
    let ignores_permissions = fs::write(&probe, b"").is_ok();
    let removal = run(&note);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    if ignores_permissions {
        return;
    }

    assert!(matches!(
        removal.segments[0].outcome,
        SegmentOutcome::Failed(_)
    ));
    assert!(note.home.join(audio(0)).exists());
    assert!(marker(&note.home, &audio(0)).exists());

    let removal = run(&note);
    assert_eq!(outcomes(&removal), vec![SegmentOutcome::Removed]);
    assert!(!note.home.join(audio(0)).exists());
}

// ---------------------------------------------------------------- names

#[test]
fn only_an_audio_name_that_longhand_writes_is_safe() {
    // KTD10: `audio/segment-<digits>.opus` only.
    assert!(safe_audio_name("audio/segment-01.opus"));
    assert!(safe_audio_name("audio/segment-12.opus"));
    assert!(safe_audio_name("audio/segment-100.opus"));
    for name in [
        "",
        "segment-01.opus",
        "/tmp/segment-01.opus",
        "../segment-01.opus",
        "audio/../../segment-01.opus",
        "./audio/segment-01.opus",
        "audio/./segment-01.opus",
        "audio//segment-01.opus",
        "audio/",
        "audio/segment-.opus",
        "audio/segment-01.txt",
        "audio/segment-01.opus/",
        "audio/segment-1a.opus",
        "audio/Segment-01.opus",
        "Audio/segment-01.opus",
        "audio/sub/segment-01.opus",
        "audio/session.md",
        // The documents of the note, in any case, and conflict copies.
        "session.md",
        "transcript.md",
        "summary.md",
        "notes.md",
        "Session.md",
        "Transcript.md",
        "Notes.md",
        "SUMMARY.MD",
        "notes (1).md",
    ] {
        assert!(!safe_audio_name(name), "{name}");
    }
}

/// Run the step on a note with one safe segment and one segment named
/// `name`. The unsafe segment must get no marker, and each document must
/// stay.
fn assert_name_keeps_the_documents(name: &str) {
    let segments = vec![
        segment(0, "2026-09-01T09:01:00-05:00", &audio(0)),
        segment(1, "2026-09-01T09:02:00-05:00", name),
    ];
    let note = saved_note(&segments);
    let before = documents(&note.home);

    let removal = run(&note);

    assert_eq!(
        outcomes(&removal),
        vec![SegmentOutcome::Removed, SegmentOutcome::UnsafeName],
        "{name}"
    );
    assert_eq!(documents(&note.home), before, "{name}");
    assert!(!marker(&note.home, name).exists(), "{name}");
    assert_eq!(flags(&note), vec![audio(0)], "{name}");
}

#[test]
fn a_document_name_in_another_case_or_a_conflict_copy_is_not_deleted() {
    // On a file system that ignores case, `Session.md` is `session.md`.
    for name in ["Session.md", "SUMMARY.MD", "notes (1).md"] {
        assert_name_keeps_the_documents(name);
    }
}

#[test]
fn a_conflict_copy_of_a_document_stays() {
    let segments = vec![segment(0, "2026-09-01T09:01:00-05:00", "notes (1).md")];
    let note = saved_note(&segments);
    fs::write(note.home.join("notes (1).md"), b"other notes").unwrap();

    let removal = run(&note);

    assert_eq!(outcomes(&removal), vec![SegmentOutcome::UnsafeName]);
    assert_eq!(
        fs::read(note.home.join("notes (1).md")).unwrap(),
        b"other notes"
    );
}

#[test]
fn a_file_in_the_audio_folder_with_another_extension_stays() {
    let segments = vec![
        segment(0, "2026-09-01T09:01:00-05:00", &audio(0)),
        segment(1, "2026-09-01T09:02:00-05:00", "audio/segment-01.txt"),
    ];
    let note = saved_note(&segments);
    fs::write(note.home.join("audio/segment-01.txt"), b"keep").unwrap();

    let removal = run(&note);

    assert_eq!(
        outcomes(&removal),
        vec![SegmentOutcome::Removed, SegmentOutcome::UnsafeName]
    );
    assert_eq!(
        fs::read(note.home.join("audio/segment-01.txt")).unwrap(),
        b"keep"
    );
    assert!(!marker(&note.home, "audio/segment-01.txt").exists());
}

#[cfg(unix)]
#[test]
fn an_audio_folder_that_links_to_the_home_folder_deletes_no_document() {
    // KTD10: `audio -> .` makes `audio/session.md` the document of the
    // note, and `audio/segment-01.opus` a file in the home folder.
    let segments = vec![
        segment(0, "2026-09-01T09:01:00-05:00", &audio(0)),
        segment(1, "2026-09-01T09:02:00-05:00", "audio/session.md"),
    ];
    let note = saved_note(&segments);
    fs::remove_dir_all(note.home.join("audio")).unwrap();
    std::os::unix::fs::symlink(".", note.home.join("audio")).unwrap();
    fs::write(note.home.join("segment-01.opus"), b"keep").unwrap();
    let before = documents(&note.home);

    let removal = run(&note);

    assert_eq!(
        outcomes(&removal),
        vec![SegmentOutcome::UnsafeName, SegmentOutcome::UnsafeName]
    );
    assert_eq!(documents(&note.home), before);
    assert_eq!(
        fs::read(note.home.join("segment-01.opus")).unwrap(),
        b"keep"
    );
    assert!(!note.home.join("segment-01.opus.removed").exists());
    assert!(!note.home.join("session.md.removed").exists());
    assert!(flags(&note).is_empty());
}

#[test]
fn a_folder_with_an_audio_name_is_not_deleted() {
    let note = saved_note(&three_segments()[..2]);
    fs::remove_file(note.home.join(audio(1))).unwrap();
    fs::create_dir(note.home.join(audio(1))).unwrap();
    fs::write(note.home.join(audio(1)).join("inside"), b"keep").unwrap();

    let removal = run(&note);

    assert_eq!(
        outcomes(&removal),
        vec![SegmentOutcome::Removed, SegmentOutcome::UnsafeName]
    );
    assert_eq!(
        fs::read(note.home.join(audio(1)).join("inside")).unwrap(),
        b"keep"
    );
    assert!(!marker(&note.home, &audio(1)).exists());
    assert_eq!(flags(&note), vec![audio(0)]);
}

#[test]
fn an_unsafe_name_gets_no_marker_and_nothing_outside_the_home_folder_goes() {
    // R7, KTD10. The note's `session.md` names audio outside its home
    // folder: an absolute path, a path with `..`, and a document.
    let outside = tempfile::tempdir().unwrap();
    let absolute = outside.path().join("absolute.opus");
    fs::write(&absolute, b"keep").unwrap();
    let absolute_name = absolute.to_string_lossy().into_owned();
    let segments = vec![
        segment(0, "2026-09-01T09:01:00-05:00", &audio(0)),
        segment(1, "2026-09-01T09:02:00-05:00", &absolute_name),
        segment(2, "2026-09-01T09:03:00-05:00", "../parent.opus"),
        segment(3, "2026-09-01T09:04:00-05:00", "transcript.md"),
    ];
    let note = saved_note(&segments);
    let parent = note.root.join("parent.opus");
    fs::write(&parent, b"keep").unwrap();
    let before = documents(&note.home);

    let removal = run(&note);

    assert_eq!(
        outcomes(&removal),
        vec![
            SegmentOutcome::Removed,
            SegmentOutcome::UnsafeName,
            SegmentOutcome::UnsafeName,
            SegmentOutcome::UnsafeName,
        ]
    );
    assert!(!removal.complete());
    assert_eq!(fs::read(&absolute).unwrap(), b"keep");
    assert!(!Path::new(&removal_marker(&absolute_name)).exists());
    assert_eq!(fs::read(&parent).unwrap(), b"keep");
    assert!(!note.root.join("parent.opus.removed").exists());
    assert!(!marker(&note.home, "transcript.md").exists());
    assert_eq!(documents(&note.home), before);
    assert_eq!(flags(&note), vec![audio(0)]);
}

#[cfg(unix)]
#[test]
fn a_name_that_resolves_outside_the_home_folder_is_unsafe() {
    // R7, KTD10. The audio folder is a link to a folder outside the home
    // folder: the name is plain, but it resolves outside.
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("segment-01.opus"), b"keep").unwrap();
    let note = saved_note(&three_segments()[..1]);
    fs::remove_dir_all(note.home.join("audio")).unwrap();
    std::os::unix::fs::symlink(outside.path(), note.home.join("audio")).unwrap();

    let removal = run(&note);

    assert_eq!(outcomes(&removal), vec![SegmentOutcome::UnsafeName]);
    assert_eq!(
        fs::read(outside.path().join("segment-01.opus")).unwrap(),
        b"keep"
    );
    assert!(!outside.path().join("segment-01.opus.removed").exists());
    assert!(flags(&note).is_empty());
}

// ---------------------------------------------------------------- refusals

#[test]
fn the_step_follows_the_folder_first() {
    // Another computer resumed the note and saved a second segment. The
    // row does not have it yet; the step reads it from `session.md`.
    let segments = three_segments();
    let note = saved_note(&segments[..1]);
    fs::write(note.home.join(audio(1)), b"opus").unwrap();
    fs::write(note.home.join("session.md"), doc(&segments[..2]).render()).unwrap();

    let removal = run(&note);

    assert_eq!(outcomes(&removal), vec![SegmentOutcome::Removed; 2]);
    assert!(!note.home.join(audio(1)).exists());
    assert_eq!(flags(&note), vec![audio(0), audio(1)]);
}

#[test]
fn a_note_that_is_not_saved_keeps_its_audio() {
    // R8.
    let dir = tempfile::tempdir().unwrap();
    let store = common::store();
    common::record_session(
        &store,
        dir.path(),
        ID,
        60,
        None,
        &[(16_000, vec![0.0; 16_000])],
    );
    let before = contents(dir.path());

    let result = remove_audio(&store, &Claims::default(), ID, no_guard());

    assert!(matches!(result, Err(RemoveAudioError::Busy("capturing"))));
    assert_eq!(contents(dir.path()), before);
}

#[test]
fn a_note_with_a_claim_keeps_its_audio() {
    let note = saved_note(&three_segments());
    let _claim = note.claims.regenerate(ID).unwrap();
    let before = contents(&note.home);

    let result = remove_audio(&note.store, &note.claims, ID, no_guard());

    assert!(matches!(
        result,
        Err(RemoveAudioError::Busy("regenerating"))
    ));
    assert_eq!(contents(&note.home), before);
}

/// A guard whose folder check refuses as "in use".
fn refuse_folder() -> Guard<FolderCheck, StoreCheck> {
    Guard {
        folder: |_| Err("in use"),
        store: |_, _| Ok(()),
    }
}

/// A guard whose store check refuses as "in use".
fn refuse_store() -> Guard<FolderCheck, StoreCheck> {
    Guard {
        folder: |_| Ok(()),
        store: |_, _| Err("in use"),
    }
}

#[test]
fn a_guard_that_refuses_keeps_the_audio_and_releases_the_claim() {
    let note = saved_note(&three_segments());
    let before = contents(&note.home);

    let results = [
        remove_audio(&note.store, &note.claims, ID, refuse_folder()),
        remove_audio(&note.store, &note.claims, ID, refuse_store()),
    ];

    for result in results {
        assert!(matches!(result, Err(RemoveAudioError::Busy("in use"))));
    }
    assert_eq!(contents(&note.home), before);
    assert!(!note.claims.busy(ID));
}

#[test]
fn the_store_reason_comes_before_the_folder_reason() {
    let note = saved_note(&three_segments());
    let guard = Guard {
        folder: |_: &session_core::SessionRecord| Err("in use"),
        store: |_: &session_core::Store, _: &session_core::SessionRecord| Err("move open"),
    };

    let result = remove_audio(&note.store, &note.claims, ID, guard);

    assert!(matches!(result, Err(RemoveAudioError::Busy("move open"))));
}

#[test]
fn the_folder_check_runs_with_no_store_lock() {
    // A slow or online-only folder must not stop the other users of the
    // store. The store check runs under the lock.
    let note = saved_note(&three_segments());
    let store = note.store.clone();
    let folder_ran = std::cell::Cell::new(false);
    let guard = Guard {
        folder: |record: &session_core::SessionRecord| {
            assert!(store.try_lock().is_ok(), "the store is locked");
            assert_eq!(record.id, ID);
            folder_ran.set(true);
            Ok(())
        },
        store: |_: &session_core::Store, _: &session_core::SessionRecord| {
            assert!(store.try_lock().is_err(), "the store is not locked");
            Ok(())
        },
    };

    let removal = remove_audio(&note.store, &note.claims, ID, guard).unwrap();

    assert!(folder_ran.get());
    assert!(removal.complete());
}

#[test]
fn the_step_writes_the_marker_before_the_delete_and_holds_a_claim() {
    // KTD2: the marker comes first.
    let note = saved_note(&three_segments()[..1]);
    let claims = note.claims.clone();
    let home = note.home.clone();
    let removal = remove_audio_with(&note.store, &note.claims, ID, no_guard(), |path| {
        assert_eq!(
            fs::read_to_string(marker(&home, &audio(0))).unwrap(),
            REMOVAL_MARKER_TEXT
        );
        // A summary regeneration and a Move cannot start.
        assert!(claims.busy(ID));
        assert!(claims.regenerate(ID).is_none());
        assert!(claims.moving(ID).is_none());
        fs::remove_file(path)
    })
    .unwrap();
    assert!(removal.complete());
    assert!(!note.claims.busy(ID));
}

#[test]
fn a_missing_home_folder_is_not_created() {
    let note = saved_note(&three_segments());
    fs::remove_dir_all(&note.home).unwrap();

    let result = remove_audio(&note.store, &note.claims, ID, no_guard());

    assert!(matches!(result, Err(RemoveAudioError::NoFolder)));
    assert!(!note.home.exists());
    assert!(flags(&note).is_empty());
}

#[test]
fn an_unknown_note_is_an_error() {
    let store = common::store();
    let result = remove_audio(&store, &Claims::default(), "no-such-note", no_guard());
    assert!(matches!(result, Err(RemoveAudioError::Failed(_))));
}

// ------------------------------------------------------- finish the removal

#[test]
fn finish_deletes_only_the_audio_of_marked_segments_and_writes_no_marker() {
    // KTD2: the setting is off, and one segment has its marker and its
    // file. The other segments keep their audio and get no marker.
    let segments = three_segments();
    let note = saved_note(&segments);
    fs::write(marker(&note.home, &audio(1)), REMOVAL_MARKER_TEXT).unwrap();
    let before = documents(&note.home);

    let removal = finish_removal(&note.store, &note.claims, ID, no_guard()).unwrap();

    assert_eq!(removal.segments.len(), 1);
    assert_eq!(removal.segments[0].audio_file, audio(1));
    assert_eq!(removal.segments[0].outcome, SegmentOutcome::Removed);
    assert!(removal.row_changed);
    assert!(!note.home.join(audio(1)).exists());
    for i in [0, 2] {
        assert!(note.home.join(audio(i)).is_file());
        assert!(!marker(&note.home, &audio(i)).exists());
    }
    assert_eq!(documents(&note.home), before);
    assert_eq!(flags(&note), vec![audio(1)]);
}

#[test]
fn finish_on_a_note_with_no_marker_changes_nothing() {
    let note = saved_note(&three_segments());
    let folder = contents(&note.home);

    let removal = finish_removal(&note.store, &note.claims, ID, no_guard()).unwrap();

    assert!(removal.segments.is_empty());
    assert!(!removal.row_changed);
    assert_eq!(contents(&note.home), folder);
    assert!(flags(&note).is_empty());
}

#[test]
fn finish_keeps_the_audio_of_a_busy_note_or_a_refused_guard() {
    let note = saved_note(&three_segments()[..1]);
    fs::write(marker(&note.home, &audio(0)), REMOVAL_MARKER_TEXT).unwrap();
    {
        let _claim = note.claims.regenerate(ID).unwrap();
        let result = finish_removal(&note.store, &note.claims, ID, no_guard());
        assert!(matches!(
            result,
            Err(RemoveAudioError::Busy("regenerating"))
        ));
    }
    let results = [
        finish_removal(&note.store, &note.claims, ID, refuse_folder()),
        finish_removal(&note.store, &note.claims, ID, refuse_store()),
    ];
    for result in results {
        assert!(matches!(result, Err(RemoveAudioError::Busy("in use"))));
    }
    assert!(note.home.join(audio(0)).is_file());
}

#[test]
fn finish_deletes_no_document_that_a_marker_names() {
    // A synced `session.md` names a document, and the folder has a marker
    // for it. The setting can be off: the step still deletes nothing.
    let segments = vec![
        segment(0, "2026-09-01T09:01:00-05:00", "Session.md"),
        segment(1, "2026-09-01T09:02:00-05:00", "notes (1).md"),
    ];
    let note = saved_note(&segments);
    for name in ["Session.md", "notes (1).md", "session.md"] {
        fs::write(marker(&note.home, name), REMOVAL_MARKER_TEXT).unwrap();
    }
    fs::write(note.home.join("notes (1).md"), b"other notes").unwrap();
    let before = documents(&note.home);

    let removal = finish_removal(&note.store, &note.claims, ID, no_guard()).unwrap();

    assert!(removal.segments.is_empty());
    assert_eq!(documents(&note.home), before);
    assert_eq!(
        fs::read(note.home.join("notes (1).md")).unwrap(),
        b"other notes"
    );
}

// ---------------------------------------------------------------- eligibility

fn inputs(now: &str, delay_days: u32) -> EligibilityInputs {
    EligibilityInputs {
        now: time(now),
        delay_days,
        claimed: false,
        in_use: false,
    }
}

fn check(note: &Note, inputs: &EligibilityInputs) -> Eligibility {
    eligibility(&note.store.lock().unwrap(), ID, inputs).unwrap()
}

#[test]
fn ae1_a_note_that_stopped_on_1_september_is_eligible_on_1_october() {
    // Covers AE1, R4.
    let note = saved_note(&three_segments());
    assert_eq!(
        check(&note, &inputs("2026-10-01T12:00:00-05:00", 30)),
        Eligibility::Eligible
    );
}

#[test]
fn ae2_the_delay_starts_at_the_latest_segment_end() {
    // Covers AE2, R4, KTD7. The note stopped on 1 September, was resumed
    // and stopped again on 20 September.
    let mut segments = three_segments();
    segments.push(segment(3, "2026-09-20T15:00:00-05:00", &audio(3)));
    let note = saved_note(&segments);

    let due = time("2026-10-20T15:00:00-05:00");
    assert_eq!(
        check(&note, &inputs("2026-10-01T12:00:00-05:00", 30)),
        Eligibility::NotYet { due }
    );
    assert_eq!(
        check(&note, &inputs("2026-10-20T14:59:59-05:00", 30)),
        Eligibility::NotYet { due }
    );
    assert_eq!(
        check(&note, &inputs("2026-10-20T15:00:00-05:00", 30)),
        Eligibility::Eligible
    );
    // The time zone of `now` does not change the result.
    assert_eq!(
        check(&note, &inputs("2026-10-20T20:00:00+00:00", 30)),
        Eligibility::Eligible
    );
}

#[test]
fn a_note_with_no_segments_is_never_eligible() {
    // KTD7.
    let store = common::store();
    let home = tempfile::tempdir().unwrap();
    let mut adoption = Adoption::from_doc(
        &doc(&three_segments()),
        String::new(),
        home.path().to_string_lossy().into_owned(),
        None,
    );
    adoption.segments.clear();
    store.lock().unwrap().adopt(&adoption).unwrap();

    assert_eq!(
        eligibility(
            &store.lock().unwrap(),
            ID,
            &inputs("2036-01-01T00:00:00+00:00", 1)
        )
        .unwrap(),
        Eligibility::NoRecording
    );
}

#[test]
fn a_note_that_is_not_saved_or_claimed_is_not_eligible() {
    // R8, KTD5.
    let dir = tempfile::tempdir().unwrap();
    let store = common::store();
    common::queue_session(
        &store,
        dir.path(),
        ID,
        60,
        None,
        &[(16_000, vec![0.0; 16_000])],
    );
    assert_eq!(
        eligibility(
            &store.lock().unwrap(),
            ID,
            &inputs("2036-01-01T00:00:00+00:00", 1)
        )
        .unwrap(),
        Eligibility::NotSaved
    );

    let note = saved_note(&three_segments());
    let mut claimed = inputs("2026-10-01T12:00:00-05:00", 30);
    claimed.claimed = true;
    assert_eq!(check(&note, &claimed), Eligibility::Claimed);

    // R10, KTD4: a fresh in-use marker in the home folder.
    let mut in_use = inputs("2026-10-01T12:00:00-05:00", 30);
    in_use.in_use = true;
    assert_eq!(check(&note, &in_use), Eligibility::InUse);
}

#[test]
fn a_note_whose_audio_is_all_removed_is_not_eligible() {
    let note = saved_note(&three_segments());
    let now = inputs("2026-10-01T12:00:00-05:00", 30);
    run(&note);
    assert_eq!(check(&note, &now), Eligibility::NoAudio);

    // A note that lost only some of its audio still has audio to remove
    // (R16).
    let note = saved_note(&three_segments());
    fs::write(marker(&note.home, &audio(0)), REMOVAL_MARKER_TEXT).unwrap();
    note.store
        .lock()
        .unwrap()
        .set_removed_audio(ID, &[audio(0)])
        .unwrap();
    assert_eq!(check(&note, &now), Eligibility::Eligible);
}

#[test]
fn the_eligibility_check_opens_no_file() {
    // KTD11: the check uses the store only. A home folder that is gone
    // does not change it.
    let note = saved_note(&three_segments());
    fs::remove_dir_all(&note.home).unwrap();
    assert_eq!(
        check(&note, &inputs("2026-10-01T12:00:00-05:00", 30)),
        Eligibility::Eligible
    );
}
