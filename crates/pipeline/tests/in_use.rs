//! The in-use marker (U4 of the audio removal plan): while this computer
//! resumes or processes a saved note, it keeps a hidden file in the home
//! folder, and the removal on another computer skips the note while any
//! such file is fresh (R10, AE4, KTD4).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, FixedOffset, TimeDelta};
use pipeline::SharedStore;
use pipeline::in_use::{
    CLOCK_MARGIN, FRESH_FOR, InUseKeeper, in_use, is_marker_name, marker_fresh, marker_name,
    remove_marker, write_marker,
};
use pipeline::remove_audio::{Eligibility, EligibilityInputs, eligibility};
use session_core::folder::{SessionDoc, fingerprint};
use session_core::jobs::Adoption;
use session_core::{Event, Segment, SessionState};

const ID: &str = "20260901-0900-abcd";
const RUN_A: &str = "4242-1790000000000000000";
const RUN_B: &str = "5151-1790000000000000001";

fn time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

fn now() -> DateTime<FixedOffset> {
    chrono::Local::now().fixed_offset()
}

fn home() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("Planning");
    fs::create_dir(&home).unwrap();
    (dir, home)
}

/// The names in `dir`, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// Set the modification time of `path` to `age` before the clock.
fn age_file(path: &Path, age: Duration) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - age).unwrap();
}

// ------------------------------------------------------------ the marker file

#[test]
fn the_marker_is_a_hidden_file_named_for_the_run() {
    let name = marker_name(RUN_A);
    assert_eq!(name, format!(".longhand-in-use-{RUN_A}"));
    assert!(name.starts_with('.'), "the watcher skips hidden names");
    assert!(is_marker_name(&name));
    assert!(!is_marker_name(".longhand-in-use-"));
    assert!(!is_marker_name("session.md"));
    assert!(!is_marker_name("audio/segment-01.opus.removed"));
}

#[test]
fn a_run_id_with_a_path_separator_stays_one_file_name() {
    let name = marker_name("../a/b");
    assert!(!name.contains('/'));
    assert!(!name.contains(".."));
    assert!(is_marker_name(&name));
}

#[test]
fn a_written_marker_is_fresh_and_holds_the_run_and_the_time() {
    let (_dir, home) = home();
    let at = now();
    assert!(write_marker(&home, RUN_A, at).unwrap());
    let path = home.join(marker_name(RUN_A));
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains(RUN_A), "{text}");
    // Compare the time, not its text: at UTC the file says "Z" and not
    // "+00:00".
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let written = chrono::DateTime::parse_from_rfc3339(json["refreshed_at"].as_str().unwrap());
    assert_eq!(written.unwrap(), at, "{text}");
    assert!(marker_fresh(&path, at));
    assert!(in_use(&home, at).unwrap());
    // Only the marker: no temporary file stays.
    assert_eq!(names(&home), vec![marker_name(RUN_A)]);
}

#[test]
fn a_refresh_replaces_the_time() {
    let (_dir, home) = home();
    let first = time("2026-09-30T09:00:00-05:00");
    write_marker(&home, RUN_A, first).unwrap();
    let later = first + TimeDelta::minutes(29);
    let path = home.join(marker_name(RUN_A));
    assert!(marker_fresh(&path, later));
    let last = first + TimeDelta::minutes(40);
    assert!(!marker_fresh(&path, last));
    write_marker(&home, RUN_A, later).unwrap();
    assert!(marker_fresh(&path, last));
}

#[test]
fn a_marker_older_than_30_minutes_does_not_block() {
    let (_dir, home) = home();
    let at = time("2026-09-30T09:00:00-05:00");
    write_marker(&home, RUN_B, at).unwrap();
    assert!(in_use(&home, at + FRESH_FOR - TimeDelta::seconds(1)).unwrap());
    assert!(!in_use(&home, at + FRESH_FOR).unwrap());
    assert!(!in_use(&home, at + TimeDelta::hours(5)).unwrap());
}

#[test]
fn a_marker_with_a_time_in_the_future_within_the_margin_is_fresh() {
    // The clock of the other computer is ahead.
    let (_dir, home) = home();
    let check = time("2026-09-30T09:00:00-05:00");
    write_marker(&home, RUN_B, check + CLOCK_MARGIN).unwrap();
    assert!(in_use(&home, check).unwrap());
}

#[test]
fn a_marker_far_in_the_future_counts_by_its_file_time() {
    // A time past the margin is not trusted. The file time decides, so a
    // wrong clock cannot block the removal for ever.
    let (_dir, home) = home();
    let check = now();
    write_marker(&home, RUN_B, check + CLOCK_MARGIN + TimeDelta::hours(1)).unwrap();
    let path = home.join(marker_name(RUN_B));
    assert!(marker_fresh(&path, check), "a new file is fresh");
    age_file(&path, Duration::from_secs(31 * 60));
    assert!(!marker_fresh(&path, check));
}

#[test]
fn a_marker_that_does_not_parse_is_fresh_until_its_file_time_is_old() {
    let (_dir, home) = home();
    let path = home.join(marker_name(RUN_B));
    fs::write(&path, b"not a marker").unwrap();
    assert!(marker_fresh(&path, now()));
    assert!(in_use(&home, now()).unwrap());

    age_file(&path, Duration::from_secs(29 * 60));
    assert!(marker_fresh(&path, now()));
    age_file(&path, Duration::from_secs(31 * 60));
    assert!(!marker_fresh(&path, now()));
    assert!(!in_use(&home, now()).unwrap());

    // An empty file, as a sync can leave before the content comes.
    fs::write(&path, b"").unwrap();
    assert!(in_use(&home, now()).unwrap());
}

#[test]
fn other_hidden_files_do_not_block() {
    let (_dir, home) = home();
    fs::write(home.join(".DS_Store"), b"x").unwrap();
    fs::write(home.join(".longhand-in-use-"), b"x").unwrap();
    fs::create_dir(home.join(marker_name("a-folder"))).unwrap();
    assert!(!in_use(&home, now()).unwrap());
}

#[test]
fn a_home_folder_that_cannot_be_read_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(in_use(&dir.path().join("gone"), now()).is_err());
}

#[test]
fn the_write_never_makes_a_missing_home_folder() {
    // A rename on another computer moved the folder (KTD4).
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("Old title");
    assert!(!write_marker(&gone, RUN_A, now()).unwrap());
    assert!(!gone.exists());
    // A file where the folder was is not a folder either.
    fs::write(&gone, b"x").unwrap();
    assert!(!write_marker(&gone, RUN_A, now()).unwrap());
}

#[test]
fn a_run_removes_only_its_own_marker() {
    let (_dir, home) = home();
    write_marker(&home, RUN_A, now()).unwrap();
    write_marker(&home, RUN_B, now()).unwrap();
    remove_marker(&home, RUN_A).unwrap();
    assert_eq!(names(&home), vec![marker_name(RUN_B)]);
    // A second remove, and a remove in a missing folder, change nothing.
    remove_marker(&home, RUN_A).unwrap();
    remove_marker(&home.join("gone"), RUN_A).unwrap();
    assert_eq!(names(&home), vec![marker_name(RUN_B)]);
}

#[test]
fn two_computers_on_one_note_each_keep_the_note_in_use() {
    // Computer A and computer B work on one note at once. When A finishes
    // and removes its marker, the fresh marker of B still blocks.
    let (_dir, home) = home();
    let at = now();
    write_marker(&home, RUN_A, at).unwrap();
    write_marker(&home, RUN_B, at).unwrap();
    assert!(in_use(&home, at).unwrap());
    remove_marker(&home, RUN_A).unwrap();
    assert!(in_use(&home, at).unwrap());
    remove_marker(&home, RUN_B).unwrap();
    assert!(!in_use(&home, at).unwrap());
}

// ---------------------------------------------------------------- eligibility

fn segment(index: u32, end: &str) -> Segment {
    let end = time(end);
    Segment {
        index,
        offset_ms: u64::from(index) * 60_000,
        duration_ms: 60_000,
        wall_start: end - TimeDelta::minutes(1),
        wall_end: end,
        audio_file: format!("audio/segment-{:02}.opus", index + 1),
        channels: 1,
        ends_in_pause: false,
    }
}

struct Note {
    _root: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    store: SharedStore,
}

/// A saved note folder and its row, as another computer's library scan
/// makes it.
fn saved_note() -> Note {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let home = root_path.join("Planning");
    fs::create_dir_all(home.join("audio")).unwrap();
    let segments = vec![segment(0, "2026-09-01T09:01:00-05:00")];
    let doc = SessionDoc::new(ID, None, &segments).unwrap();
    session_save::note_files::write_doc(&home, &doc).unwrap();
    fs::write(home.join("audio/segment-01.opus"), b"opus").unwrap();
    let store = common::store();
    let adoption = Adoption::from_doc(
        &doc,
        String::new(),
        home.to_string_lossy().into_owned(),
        Some(fingerprint(b"")),
    );
    assert!(store.lock().unwrap().adopt(&adoption).unwrap());
    Note {
        _root: root,
        root: root_path,
        home,
        store,
    }
}

fn check(note: &Note, now: DateTime<FixedOffset>, in_use: bool) -> Eligibility {
    let inputs = EligibilityInputs {
        now,
        delay_days: 1,
        claimed: false,
        in_use,
    };
    eligibility(&note.store.lock().unwrap(), ID, &inputs).unwrap()
}

#[test]
fn ae4_a_fresh_marker_of_another_run_makes_the_check_skip_the_note() {
    // Covers AE4, R10.
    let note = saved_note();
    let at = now();
    assert_eq!(
        check(&note, at, in_use(&note.home, at).unwrap()),
        Eligibility::Eligible
    );
    write_marker(&note.home, RUN_A, at).unwrap();
    assert_eq!(
        check(&note, at, in_use(&note.home, at).unwrap()),
        Eligibility::InUse
    );
    // After 30 minutes with no refresh, the note is eligible again.
    let later = at + FRESH_FOR;
    assert_eq!(
        check(&note, later, in_use(&note.home, later).unwrap()),
        Eligibility::Eligible
    );
}

#[test]
fn an_unsaved_note_is_not_saved_before_it_is_in_use() {
    // The check does not need the marker to protect a note that this
    // computer has open (R8).
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
    let inputs = EligibilityInputs {
        now: now(),
        delay_days: 1,
        claimed: false,
        in_use: true,
    };
    assert_eq!(
        eligibility(&store.lock().unwrap(), ID, &inputs).unwrap(),
        Eligibility::NotSaved
    );
}

// ------------------------------------------------------------------ the keeper

fn open(note: &Note) {
    // A resume of the saved note: the row leaves Saved.
    let mut store = note.store.lock().unwrap();
    store
        .apply(
            ID,
            &Event::UserResume {
                space_ok: true,
                audio_intact: true,
            },
        )
        .unwrap();
}

fn markers(home: &Path) -> Vec<String> {
    names(home)
        .into_iter()
        .filter(|n| is_marker_name(n))
        .collect()
}

#[test]
fn the_keeper_writes_no_marker_for_a_saved_note() {
    let note = saved_note();
    let keeper = InUseKeeper::new(note.store.clone(), RUN_A);
    keeper.refresh(now());
    assert!(markers(&note.home).is_empty());
}

#[test]
fn the_keeper_marks_a_resumed_note_and_removes_the_marker_at_saved() {
    let note = saved_note();
    let keeper = InUseKeeper::new(note.store.clone(), RUN_A);
    open(&note);
    let at = now();
    keeper.refresh(at);
    assert_eq!(markers(&note.home), vec![marker_name(RUN_A)]);
    assert!(in_use(&note.home, at).unwrap());

    // The work continues through the queue and processing: each refresh
    // keeps the marker fresh.
    {
        let mut store = note.store.lock().unwrap();
        store
            .apply(
                ID,
                &Event::UserStop {
                    at,
                    retained_audio: true,
                },
            )
            .unwrap();
        store.apply(ID, &Event::Dequeued).unwrap();
    }
    let later = at + TimeDelta::minutes(25);
    keeper.refresh(later);
    assert!(in_use(&note.home, later + TimeDelta::minutes(25)).unwrap());

    note.store.lock().unwrap().apply(ID, &Event::Saved).unwrap();
    keeper.refresh(later);
    assert!(markers(&note.home).is_empty());
    assert!(!in_use(&note.home, later).unwrap());
}

#[test]
fn the_keeper_removes_the_marker_when_the_transcribe_again_run_fails_back_to_saved() {
    let note = saved_note();
    let keeper = InUseKeeper::new(note.store.clone(), RUN_A);
    {
        let mut store = note.store.lock().unwrap();
        store
            .apply(
                ID,
                &Event::UserTranscribeAgain {
                    at: now(),
                    audio_intact: true,
                    capture_active: false,
                    regenerating: false,
                },
            )
            .unwrap();
    }
    keeper.refresh(now());
    assert_eq!(markers(&note.home), vec![marker_name(RUN_A)]);
    {
        let mut store = note.store.lock().unwrap();
        store.apply(ID, &Event::Dequeued).unwrap();
        store.apply(ID, &Event::TranscribeAgainFailed).unwrap();
    }
    keeper.refresh(now());
    assert!(markers(&note.home).is_empty());
}

#[test]
fn the_keeper_removes_the_marker_after_a_discard() {
    // A Discard ends the work: the row is discarded or gone.
    let note = saved_note();
    let keeper = InUseKeeper::new(note.store.clone(), RUN_A);
    open(&note);
    keeper.refresh(now());
    assert_eq!(markers(&note.home), vec![marker_name(RUN_A)]);
    note.store
        .lock()
        .unwrap()
        .apply(
            ID,
            &Event::UserStop {
                at: now(),
                retained_audio: false,
            },
        )
        .unwrap();
    assert_eq!(
        note.store
            .lock()
            .unwrap()
            .session(ID)
            .unwrap()
            .unwrap()
            .state,
        SessionState::Discarded
    );
    keeper.refresh(now());
    assert!(markers(&note.home).is_empty());
}

#[test]
fn the_keeper_removes_the_marker_of_a_note_whose_row_is_gone() {
    let note = saved_note();
    let keeper = InUseKeeper::new(note.store.clone(), RUN_A);
    open(&note);
    {
        let mut store = note.store.lock().unwrap();
        store
            .apply(
                ID,
                &Event::UserStop {
                    at: now(),
                    retained_audio: true,
                },
            )
            .unwrap();
        store.apply(ID, &Event::Dequeued).unwrap();
        store
            .apply(ID, &Event::StageFailed(session_core::Stage::Transcribe))
            .unwrap();
    }
    keeper.refresh(now());
    assert_eq!(markers(&note.home), vec![marker_name(RUN_A)]);
    // Delete Note of the failed note. The remove keeps the folder, so the
    // test can see the marker.
    let working = tempfile::tempdir().unwrap();
    session_core::jobs::delete_session(
        &note.store,
        ID,
        working.path(),
        &session_core::jobs::Claims::default(),
        |_| Ok(()),
    )
    .unwrap();
    assert!(note.store.lock().unwrap().session(ID).unwrap().is_none());
    keeper.refresh(now());
    assert!(markers(&note.home).is_empty());
}

#[test]
fn the_keeper_leaves_the_marker_of_another_run() {
    let note = saved_note();
    write_marker(&note.home, RUN_B, now()).unwrap();
    let keeper = InUseKeeper::new(note.store.clone(), RUN_A);
    open(&note);
    keeper.refresh(now());
    note.store
        .lock()
        .unwrap()
        .apply(
            ID,
            &Event::UserStop {
                at: now(),
                retained_audio: false,
            },
        )
        .unwrap();
    keeper.refresh(now());
    assert_eq!(markers(&note.home), vec![marker_name(RUN_B)]);
}

#[test]
fn two_keepers_on_one_note_the_second_still_blocks_after_the_first_ends() {
    // Two computers, two stores, one home folder.
    let note = saved_note();
    let other = common::store();
    {
        let doc = session_core::note_folder::NoteFiles::read(&note.home)
            .unwrap()
            .doc;
        let adoption = Adoption::from_doc(
            &doc,
            String::new(),
            note.home.to_string_lossy().into_owned(),
            Some(fingerprint(b"")),
        );
        assert!(other.lock().unwrap().adopt(&adoption).unwrap());
    }
    let first = InUseKeeper::new(note.store.clone(), RUN_A);
    let second = InUseKeeper::new(other.clone(), RUN_B);
    open(&note);
    other
        .lock()
        .unwrap()
        .apply(
            ID,
            &Event::UserTranscribeAgain {
                at: now(),
                audio_intact: true,
                capture_active: false,
                regenerating: false,
            },
        )
        .unwrap();
    let at = now();
    first.refresh(at);
    second.refresh(at);
    assert_eq!(
        markers(&note.home),
        vec![marker_name(RUN_A), marker_name(RUN_B)]
    );

    note.store
        .lock()
        .unwrap()
        .apply(
            ID,
            &Event::UserStop {
                at,
                retained_audio: false,
            },
        )
        .unwrap();
    first.refresh(at);
    assert_eq!(markers(&note.home), vec![marker_name(RUN_B)]);
    assert!(in_use(&note.home, at).unwrap());
    // A third computer's check skips the note.
    let third = saved_note_row_for(&note);
    let inputs = EligibilityInputs {
        now: at,
        delay_days: 1,
        claimed: false,
        in_use: in_use(&note.home, at).unwrap(),
    };
    assert_eq!(
        eligibility(&third.lock().unwrap(), ID, &inputs).unwrap(),
        Eligibility::InUse
    );
}

/// A third store with the saved row of `note`.
fn saved_note_row_for(note: &Note) -> SharedStore {
    let store = common::store();
    let doc = session_core::note_folder::NoteFiles::read(&note.home)
        .unwrap()
        .doc;
    let adoption = Adoption::from_doc(
        &doc,
        String::new(),
        note.home.to_string_lossy().into_owned(),
        Some(fingerprint(b"")),
    );
    assert!(store.lock().unwrap().adopt(&adoption).unwrap());
    store
}

#[test]
fn a_refresh_does_not_make_a_home_folder_that_a_rename_moved() {
    let note = saved_note();
    let keeper = InUseKeeper::new(note.store.clone(), RUN_A);
    open(&note);
    keeper.refresh(now());
    // Another computer renamed the folder; the marker moved with it.
    let renamed = note.root.join("New title");
    fs::rename(&note.home, &renamed).unwrap();
    keeper.refresh(now() + TimeDelta::minutes(2));
    assert!(!note.home.exists(), "the refresh made the old folder again");
    assert_eq!(markers(&renamed), vec![marker_name(RUN_A)]);
}

#[test]
fn the_keeper_removes_the_marker_after_save_renames_the_folder() {
    // A Resume or Transcribe Again gives a new title: Save applies Saved
    // and renames the home folder. The marker moves with the folder.
    let note = saved_note();
    let keeper = InUseKeeper::new(note.store.clone(), RUN_A);
    open(&note);
    keeper.refresh(now());
    assert_eq!(markers(&note.home), vec![marker_name(RUN_A)]);
    let renamed = note.root.join("New title");
    fs::rename(&note.home, &renamed).unwrap();
    {
        let mut store = note.store.lock().unwrap();
        store
            .apply(
                ID,
                &Event::UserStop {
                    at: now(),
                    retained_audio: true,
                },
            )
            .unwrap();
        store.apply(ID, &Event::Dequeued).unwrap();
        store.apply(ID, &Event::Saved).unwrap();
        store.set_home_path(ID, &renamed.to_string_lossy()).unwrap();
    }
    keeper.refresh(now());
    assert!(markers(&renamed).is_empty());
    assert!(!in_use(&renamed, now()).unwrap());
}

#[test]
fn the_keeper_thread_writes_at_once_when_woken_and_stops() {
    let note = saved_note();
    let keeper = InUseKeeper::new(note.store.clone(), RUN_A);
    keeper.start().unwrap();
    open(&note);
    keeper.wake();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while markers(&note.home).is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "no marker after a wake"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    note.store
        .lock()
        .unwrap()
        .apply(
            ID,
            &Event::UserStop {
                at: now(),
                retained_audio: false,
            },
        )
        .unwrap();
    keeper.wake();
    while !markers(&note.home).is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the marker stayed after the discard"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    keeper.stop();
}
