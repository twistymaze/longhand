//! The copy of the old audio of a resumed note (U5, KTD8 of the audio
//! removal plan).

mod common;

use std::path::{Path, PathBuf};

use pipeline::old_audio::{self, CopyState, OldSegment};
use session_core::Segment;
use session_core::folder::{REMOVAL_MARKER_TEXT, removal_marker};

const FIRST: &str = "audio/segment-01.opus";
const SECOND: &str = "audio/segment-02.opus";

fn segment(index: u32) -> Segment {
    Segment {
        index,
        offset_ms: u64::from(index) * 10_000,
        duration_ms: 5_000,
        wall_start: common::at(i64::from(index) * 10),
        wall_end: common::at(i64::from(index) * 10 + 5),
        audio_file: pipeline::recover::segment_audio_file(index),
        channels: 1,
        ends_in_pause: false,
    }
}

struct Case {
    _root: tempfile::TempDir,
    home: PathBuf,
    working: PathBuf,
}

/// A saved note with two segments whose audio is in `home`, resumed: its
/// old segments are listed in `working`.
fn resumed() -> Case {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let working = root.path().join("working");
    std::fs::create_dir_all(home.join("audio")).unwrap();
    std::fs::write(home.join(FIRST), b"first old audio").unwrap();
    std::fs::write(home.join(SECOND), b"second old audio").unwrap();
    old_audio::begin(&working, &[segment(0), segment(1)]).unwrap();
    Case {
        _root: root,
        home,
        working,
    }
}

impl Case {
    fn copy(&self) {
        let home = self.home.clone();
        old_audio::copy(&self.working, &move || Some(home.clone())).unwrap();
    }
    fn states(&self) -> Vec<CopyState> {
        old_audio::old_segments(&self.working)
            .unwrap()
            .unwrap()
            .into_iter()
            .map(|s| s.state)
            .collect()
    }
}

fn mark(home: &Path, audio: &str) {
    std::fs::write(home.join(removal_marker(audio)), REMOVAL_MARKER_TEXT).unwrap();
}

#[test]
fn a_resume_lists_its_old_segments_with_no_copy_and_the_copy_makes_each() {
    let case = resumed();
    assert_eq!(
        old_audio::old_segments(&case.working).unwrap().unwrap(),
        vec![
            OldSegment {
                audio_file: FIRST.into(),
                state: CopyState::Pending
            },
            OldSegment {
                audio_file: SECOND.into(),
                state: CopyState::Pending
            },
        ]
    );
    assert!(!case.working.join(FIRST).exists());
    case.copy();
    assert_eq!(case.states(), vec![CopyState::Copied, CopyState::Copied]);
    assert_eq!(
        std::fs::read(case.working.join(FIRST)).unwrap(),
        b"first old audio"
    );
    assert!(old_audio::is_old(&case.working, FIRST).unwrap());
    assert!(!old_audio::is_old(&case.working, "audio/segment-03.opus").unwrap());
}

#[test]
fn a_marker_before_the_copy_gives_the_removed_state_and_no_copy() {
    let case = resumed();
    mark(&case.home, FIRST);
    case.copy();
    assert_eq!(case.states(), vec![CopyState::Removed, CopyState::Copied]);
    assert!(!case.working.join(FIRST).exists());
    assert!(case.working.join(SECOND).is_file());
}

#[test]
fn a_file_deleted_before_the_copy_gives_the_removed_state_and_deletes_an_older_copy() {
    let case = resumed();
    // A copy left from an earlier attempt.
    std::fs::create_dir_all(case.working.join("audio")).unwrap();
    std::fs::write(case.working.join(FIRST), b"first old").unwrap();
    std::fs::remove_file(case.home.join(FIRST)).unwrap();
    case.copy();
    assert_eq!(case.states(), vec![CopyState::Removed, CopyState::Copied]);
    assert!(!case.working.join(FIRST).exists());
    let leftovers: Vec<_> = std::fs::read_dir(case.working.join("audio"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(leftovers, vec![std::ffi::OsString::from("segment-02.opus")]);
}

#[test]
fn a_home_folder_that_is_not_there_keeps_the_copy_pending() {
    let case = resumed();
    let away = case.home.with_file_name("away");
    std::fs::rename(&case.home, &away).unwrap();
    case.copy();
    assert_eq!(case.states(), vec![CopyState::Pending, CopyState::Pending]);
    // The folder is back: the next copy makes both.
    std::fs::rename(&away, &case.home).unwrap();
    case.copy();
    assert_eq!(case.states(), vec![CopyState::Copied, CopyState::Copied]);
}

#[test]
fn a_home_folder_that_moves_during_the_copy_is_read_at_its_new_path() {
    let case = resumed();
    let moved = case.home.with_file_name("renamed");
    std::fs::rename(&case.home, &moved).unwrap();
    // The first answer is the old path; after that, the store has the new
    // path.
    let asked = std::sync::atomic::AtomicUsize::new(0);
    let (old, new) = (case.home.clone(), moved.clone());
    std::fs::create_dir_all(old.join("audio")).unwrap();
    old_audio::copy(&case.working, &|| {
        if asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            Some(old.clone())
        } else {
            Some(new.clone())
        }
    })
    .unwrap();
    assert_eq!(case.states(), vec![CopyState::Copied, CopyState::Copied]);
    assert_eq!(
        std::fs::read(case.working.join(FIRST)).unwrap(),
        b"first old audio"
    );
}

#[cfg(unix)]
#[test]
fn an_online_only_file_blocks_only_the_copy() {
    // A named pipe blocks its read until a writer opens it, as an
    // online-only file blocks its read until it downloads.
    let case = resumed();
    std::fs::remove_file(case.home.join(FIRST)).unwrap();
    let status = std::process::Command::new("mkfifo")
        .arg(case.home.join(FIRST))
        .status()
        .unwrap();
    assert!(status.success());
    // The list of a new Resume does not read the audio: it returns while
    // the file cannot be read.
    old_audio::begin(&case.working, &[segment(0), segment(1)]).unwrap();

    let (working, home) = (case.working.clone(), case.home.clone());
    let copy = std::thread::spawn(move || {
        old_audio::copy(&working, &move || Some(home.clone())).unwrap();
    });
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(!copy.is_finished(), "the read waits for the file");
    assert_eq!(case.states(), vec![CopyState::Pending, CopyState::Pending]);

    // The file downloads.
    std::fs::write(case.home.join(FIRST), b"first old audio").unwrap();
    copy.join().unwrap();
    assert_eq!(case.states(), vec![CopyState::Copied, CopyState::Copied]);
    assert_eq!(
        std::fs::read(case.working.join(FIRST)).unwrap(),
        b"first old audio"
    );
}

#[test]
fn a_failed_note_deletes_only_the_copies_whose_home_file_has_a_marker() {
    let case = resumed();
    case.copy();
    mark(&case.home, FIRST);
    std::fs::remove_file(case.home.join(FIRST)).unwrap();
    let dropped = old_audio::drop_removed(&case.working, &case.home).unwrap();
    assert_eq!(dropped, vec![FIRST.to_string()]);
    assert_eq!(case.states(), vec![CopyState::Removed, CopyState::Copied]);
    assert!(!case.working.join(FIRST).exists());
    assert!(case.working.join(SECOND).is_file());
    // A repeated call changes nothing.
    assert!(
        old_audio::drop_removed(&case.working, &case.home)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_discard_deletes_every_old_copy_and_the_list_and_keeps_the_new_audio() {
    let case = resumed();
    case.copy();
    std::fs::write(case.working.join("audio/segment-03.opus"), b"new audio").unwrap();
    old_audio::discard(&case.working).unwrap();
    assert!(!case.working.join(FIRST).exists());
    assert!(!case.working.join(SECOND).exists());
    assert!(!case.working.join(old_audio::LIST_FILE).exists());
    assert!(case.working.join("audio/segment-03.opus").is_file());
    assert!(old_audio::old_segments(&case.working).unwrap().is_none());
}
