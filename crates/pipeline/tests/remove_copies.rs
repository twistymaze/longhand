//! The cleanup of old result copies (U7 of the result copies plan): it
//! deletes only the fixed names that older builds wrote (KTD9), and keeps
//! the copies of a note that has no current result (R14).

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, TimeDelta};
use pipeline::SharedStore;
use pipeline::remove_audio::{RemoveAudioError, no_guard};
use pipeline::remove_copies::{
    RESULTS_FOLDER, TRANSITION_FILE, cleanup_due, remove_copies, remove_copies_with,
};
use session_core::Segment;
use session_core::folder::{REMOVAL_MARKER_TEXT, SessionDoc, fingerprint, removal_marker};
use session_core::jobs::{Adoption, Claims};
use session_core::recognition::ARTIFACT_FILE;

const ID: &str = "20260901-0900-abcd";
const AUDIO: &str = "audio/segment-01.opus";
const TRANSCRIPT: &str = "# Transcript\n\n- [00:00:01] Speaker 1: Hello.\n";
const SUMMARY: &str = "---\nstatus: stub\nfailed_stage: summarize\n---\n\nNo summary.\n";
const NOTES: &str = "My notes\n";
const LEFTOVER: &str = ".summary.md.0123456789abcdef.tmp";

struct Note {
    _root: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    store: SharedStore,
    claims: Claims,
}

fn segments() -> Vec<Segment> {
    let end = DateTime::<FixedOffset>::parse_from_rfc3339("2026-09-01T09:01:00-05:00").unwrap();
    vec![Segment {
        index: 0,
        offset_ms: 0,
        duration_ms: 60_000,
        wall_start: end - TimeDelta::minutes(1),
        wall_end: end,
        audio_file: AUDIO.into(),
        channels: 1,
    }]
}

/// A saved note folder in a storage folder with its library marker, with
/// its audio, its row, and the copies that an older build left: two
/// generations, a transition file, and a `.DS_Store` in the results folder.
fn note_with_copies() -> Note {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    assert!(library::scan::write_marker(&root_path));
    let home = root_path.join("Planning");
    fs::create_dir_all(home.join("audio")).unwrap();
    let segments = segments();
    let mut doc = SessionDoc::new(ID, None, &segments).unwrap();
    doc.transcript_fingerprint = Some(fingerprint(TRANSCRIPT.as_bytes()));
    doc.summary_fingerprint = Some(fingerprint(SUMMARY.as_bytes()));
    fs::write(home.join("session.md"), doc.render()).unwrap();
    fs::write(home.join("transcript.md"), TRANSCRIPT).unwrap();
    fs::write(home.join("summary.md"), SUMMARY).unwrap();
    fs::write(home.join("notes.md"), NOTES).unwrap();
    fs::write(home.join(ARTIFACT_FILE), b"{\"version\":1}\n").unwrap();
    fs::write(home.join(AUDIO), b"opus").unwrap();
    for generation in ["committed-1", "0f3a-2b"] {
        let dir = home.join(RESULTS_FOLDER).join(generation);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("transcript.md"), "old transcript").unwrap();
        fs::write(dir.join("summary.md"), "old summary").unwrap();
        fs::write(dir.join(ARTIFACT_FILE), "old recognition").unwrap();
    }
    fs::write(home.join(RESULTS_FOLDER).join(".DS_Store"), b"finder").unwrap();
    fs::write(home.join(TRANSITION_FILE), b"{}").unwrap();
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

fn run(note: &Note) -> pipeline::remove_copies::Cleanup {
    remove_copies(&note.store, &note.claims, ID, no_guard()).unwrap()
}

fn results(home: &Path) -> PathBuf {
    home.join(RESULTS_FOLDER)
}

fn present(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

/// The bytes of the documents and the audio of the note.
fn documents(home: &Path) -> Vec<Vec<u8>> {
    [
        "session.md",
        "transcript.md",
        "summary.md",
        "notes.md",
        AUDIO,
    ]
    .iter()
    .map(|name| fs::read(home.join(name)).unwrap())
    .collect()
}

/// Mark the audio removed, as an older build left it: the marker is there
/// and the audio file is not.
fn remove_audio_as_older_build(home: &Path) {
    fs::write(home.join(removal_marker(AUDIO)), REMOVAL_MARKER_TEXT).unwrap();
    fs::remove_file(home.join(AUDIO)).unwrap();
}

// Covers AE5: the copies and the transition file go, and the documents and
// the audio stay byte-equal.
#[test]
fn ae5_the_copies_and_the_transition_file_go_and_the_documents_stay() {
    let note = note_with_copies();
    let before = documents(&note.home);
    assert!(cleanup_due(&note.home, &segments()));

    let cleanup = run(&note);

    assert!(!present(&results(&note.home)));
    assert!(!present(&note.home.join(TRANSITION_FILE)));
    assert_eq!(documents(&note.home), before);
    // The audio is still there, so `recognition.jsonl` stays.
    assert!(present(&note.home.join(ARTIFACT_FILE)));
    assert!(cleanup.kept.is_empty(), "{:?}", cleanup.kept);
    assert!(cleanup.deleted.contains(&results(&note.home)));
    assert!(!cleanup_due(&note.home, &segments()));
}

// KTD4: a note in a type folder is reachable through its storage folder.
#[test]
fn the_copies_of_a_note_in_a_type_folder_go() {
    let mut note = note_with_copies();
    let typed = note.root.join("Pathfinder/Planning");
    fs::create_dir(note.root.join("Pathfinder")).unwrap();
    fs::rename(&note.home, &typed).unwrap();
    note.store
        .lock()
        .unwrap()
        .set_home_path(ID, &typed.to_string_lossy())
        .unwrap();
    note.home = typed;
    let before = documents(&note.home);

    let cleanup = run(&note);

    assert!(!present(&results(&note.home)));
    assert!(!present(&note.home.join(TRANSITION_FILE)));
    assert_eq!(documents(&note.home), before);
    assert!(cleanup.kept.is_empty(), "{:?}", cleanup.kept);
}

#[test]
fn a_child_folder_with_finder_data_and_a_write_leftover_and_an_empty_child_folder_go() {
    let note = note_with_copies();
    let child = results(&note.home).join("committed-1");
    fs::write(child.join(".DS_Store"), b"finder").unwrap();
    fs::write(child.join(LEFTOVER), b"part").unwrap();
    fs::create_dir(results(&note.home).join("empty")).unwrap();

    run(&note);

    assert!(!present(&results(&note.home)));
}

// R14: a note whose result is only in its copies keeps them.
#[test]
fn a_note_with_no_transcript_or_no_summary_keeps_its_copies() {
    for missing in ["transcript.md", "summary.md"] {
        let note = note_with_copies();
        fs::remove_file(note.home.join(missing)).unwrap();

        let cleanup = run(&note);

        assert!(present(&results(&note.home)), "{missing}");
        assert!(present(&note.home.join(TRANSITION_FILE)), "{missing}");
        assert!(present(
            &results(&note.home).join("committed-1/transcript.md")
        ));
        assert!(cleanup.deleted.is_empty(), "{missing}");
        assert!(!cleanup.kept.is_empty(), "{missing}");
    }
}

#[cfg(unix)]
#[test]
fn a_results_folder_that_is_a_link_is_not_followed_and_not_deleted() {
    let note = note_with_copies();
    fs::remove_dir_all(results(&note.home)).unwrap();
    let other = note.root.join("Other");
    fs::create_dir_all(other.join("committed-1")).unwrap();
    fs::write(other.join("committed-1/transcript.md"), "other").unwrap();
    std::os::unix::fs::symlink(&other, results(&note.home)).unwrap();

    let cleanup = run(&note);

    assert!(results(&note.home).symlink_metadata().unwrap().is_symlink());
    assert!(other.join("committed-1/transcript.md").exists());
    assert!(cleanup.kept.contains(&results(&note.home)));
    // The transition file is a separate fixed name, and it goes.
    assert!(!present(&note.home.join(TRANSITION_FILE)));
}

#[test]
fn a_child_folder_with_an_unexpected_entry_stays_and_the_others_go() {
    let unexpected = [
        "notes.md",
        "transcript.txt",
        "session.md",
        ".notes.md.0123456789abcdef.tmp",
        ".summary.md.0123456789ABCDEF.tmp",
        ".summary.md.0123456789abcde.tmp",
        ".summary.md.0123456789abcdef.part",
    ];
    for name in unexpected {
        let note = note_with_copies();
        let kept = results(&note.home).join("committed-1");
        fs::write(kept.join(name), b"keep").unwrap();

        let cleanup = run(&note);

        assert!(kept.join(name).exists(), "{name}");
        assert!(kept.join("transcript.md").exists(), "{name}");
        assert!(!present(&results(&note.home).join("0f3a-2b")), "{name}");
        assert!(cleanup.kept.contains(&kept), "{name}");
        // The results folder is not empty, so it stays.
        assert!(present(&results(&note.home)), "{name}");
        assert!(!present(&note.home.join(TRANSITION_FILE)), "{name}");
    }
}

#[test]
fn a_child_folder_inside_a_child_folder_stays() {
    let note = note_with_copies();
    let kept = results(&note.home).join("committed-1");
    fs::create_dir(kept.join("audio")).unwrap();

    run(&note);

    assert!(kept.join("audio").is_dir());
    assert!(kept.join("transcript.md").exists());
}

#[test]
fn a_child_with_a_name_that_older_builds_do_not_write_stays() {
    for name in ["generation 1", "gen_1", ".hidden"] {
        let note = note_with_copies();
        let kept = results(&note.home).join(name);
        fs::create_dir(&kept).unwrap();
        fs::write(kept.join("transcript.md"), "old").unwrap();

        let cleanup = run(&note);

        assert!(kept.join("transcript.md").exists(), "{name}");
        assert!(cleanup.kept.contains(&kept), "{name}");
        assert!(!present(&results(&note.home).join("committed-1")), "{name}");
    }
}

#[test]
fn an_unexpected_file_in_the_results_folder_stays() {
    let note = note_with_copies();
    let kept = results(&note.home).join("readme.txt");
    fs::write(&kept, "keep").unwrap();

    let cleanup = run(&note);

    assert!(kept.exists());
    assert!(cleanup.kept.contains(&kept));
    assert!(!present(&results(&note.home).join("committed-1")));
}

#[cfg(unix)]
#[test]
fn a_linked_file_in_a_child_folder_stays() {
    let note = note_with_copies();
    let kept = results(&note.home).join("committed-1");
    fs::remove_file(kept.join("summary.md")).unwrap();
    std::os::unix::fs::symlink(note.home.join("summary.md"), kept.join("summary.md")).unwrap();

    run(&note);

    assert!(
        kept.join("summary.md")
            .symlink_metadata()
            .unwrap()
            .is_symlink()
    );
    assert_eq!(
        fs::read(note.home.join("summary.md")).unwrap(),
        SUMMARY.as_bytes()
    );
}

#[test]
fn a_transition_name_that_is_a_folder_stays() {
    let note = note_with_copies();
    let transition = note.home.join(TRANSITION_FILE);
    fs::remove_file(&transition).unwrap();
    fs::create_dir(&transition).unwrap();

    let cleanup = run(&note);

    assert!(transition.is_dir());
    assert!(cleanup.kept.contains(&transition));
}

// KTD8: the audio of the note was removed by an older build, or a crash
// stopped the removal after the last audio delete.
#[test]
fn a_note_with_no_audio_left_loses_recognition_jsonl() {
    let note = note_with_copies();
    remove_audio_as_older_build(&note.home);
    fs::remove_dir_all(results(&note.home)).unwrap();
    fs::remove_file(note.home.join(TRANSITION_FILE)).unwrap();
    assert!(cleanup_due(&note.home, &segments()));

    run(&note);

    assert!(!present(&note.home.join(ARTIFACT_FILE)));
    assert!(!cleanup_due(&note.home, &segments()));
}

#[test]
fn a_note_with_a_marker_and_its_audio_still_there_keeps_recognition_jsonl() {
    let note = note_with_copies();
    fs::write(note.home.join(removal_marker(AUDIO)), REMOVAL_MARKER_TEXT).unwrap();

    run(&note);

    assert!(present(&note.home.join(ARTIFACT_FILE)));
    assert!(present(&note.home.join(AUDIO)));
}

#[test]
fn a_recognition_name_that_is_a_folder_stays() {
    let note = note_with_copies();
    remove_audio_as_older_build(&note.home);
    let artifact = note.home.join(ARTIFACT_FILE);
    fs::remove_file(&artifact).unwrap();
    fs::create_dir(&artifact).unwrap();

    let cleanup = run(&note);

    assert!(artifact.is_dir());
    assert!(cleanup.kept.contains(&artifact));
}

#[test]
fn a_second_run_with_nothing_to_clean_changes_nothing() {
    let note = note_with_copies();
    run(&note);
    let before = documents(&note.home);

    let cleanup = run(&note);

    assert_eq!(cleanup, Default::default());
    assert_eq!(documents(&note.home), before);
}

#[test]
fn a_note_with_a_claim_keeps_its_copies() {
    let note = note_with_copies();
    let _claim = note.claims.regenerate(ID).unwrap();

    let result = remove_copies(&note.store, &note.claims, ID, no_guard());

    assert!(matches!(result, Err(RemoveAudioError::Busy(_))));
    assert!(present(&results(&note.home)));
}

#[test]
fn a_guard_that_refuses_keeps_the_copies() {
    let note = note_with_copies();
    let guard = pipeline::remove_audio::Guard {
        folder: |_: &session_core::SessionRecord| Err("in use"),
        store: |_: &session_core::Store, _: &session_core::SessionRecord| Ok(()),
    };

    let result = remove_copies(&note.store, &note.claims, ID, guard);

    assert!(matches!(result, Err(RemoveAudioError::Busy("in use"))));
    assert!(present(&results(&note.home)));
    assert!(present(&note.home.join(TRANSITION_FILE)));
}

// The storage folder goes away after the first delete: the step deletes
// nothing more.
#[test]
fn a_storage_folder_that_goes_away_during_the_cleanup_stops_the_next_delete() {
    let note = note_with_copies();
    let marker = library::scan::marker(&note.root);
    let mut deletes = 0;

    let result = remove_copies_with(&note.store, &note.claims, ID, no_guard(), |path| {
        deletes += 1;
        fs::remove_file(path)?;
        if marker.exists() {
            fs::remove_file(&marker)?;
        }
        Ok(())
    });

    assert!(matches!(result, Err(RemoveAudioError::NoFolder)));
    assert_eq!(deletes, 1);
    assert!(present(&results(&note.home)));
    assert!(present(&note.home.join(TRANSITION_FILE)));
}
