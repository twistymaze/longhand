//! Removal of empty notes (U6, R18, R19, KTD8): an empty note that never
//! recorded leaves nothing behind, and work from another computer is never
//! deleted.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::TimeDelta;
use common::{at, store};
use pipeline::SharedStore;
use pipeline::early_home::{self, EarlyHomes, EarlyState};
use pipeline::empty_note::{Kept, Leave, REMOVING, Removal};
use pipeline::in_use::{FRESH_FOR, marker_name, write_marker};
use session_core::jobs::{Claims, RECORDED_INTO, TitleSource};
use session_core::{Event, SessionState};
use session_save::RealFs;
use session_save::naming::staging_name;

const RUN: &str = "run-1";

/// A note that opened with no recording, with its early write begun.
fn notes_only_note(store: &SharedStore, id: &str) {
    let mut s = store.lock().unwrap();
    s.create_notes_only(id, at(0), None).unwrap();
    early_home::begin(&s, id).unwrap();
}

/// [`notes_only_note`] with its home folder written in `dest`.
fn placed_note(store: &SharedStore, id: &str, dest: &Path) -> PathBuf {
    notes_only_note(store, id);
    early_home::write(store, id, Some(dest), RUN, &RealFs)
        .unwrap()
        .unwrap()
}

fn row_exists(store: &SharedStore, id: &str) -> bool {
    store.lock().unwrap().session(id).unwrap().is_some()
}

/// The names in `dir`, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The note folders and staging folders in `dest`: every name except the
/// library folder.
fn entries(dest: &Path) -> Vec<String> {
    names(dest)
        .into_iter()
        .filter(|n| n != library::scan::LONGHAND_FOLDER)
        .collect()
}

fn now() -> chrono::DateTime<chrono::FixedOffset> {
    chrono::Local::now().fixed_offset()
}

fn leave_with(
    store: &SharedStore,
    claims: &Claims,
    homes: Option<&Arc<EarlyHomes>>,
    dest: Option<&Path>,
    id: &str,
) -> Leave {
    Removal {
        store,
        claims,
        homes,
        destination: dest,
        run_id: RUN,
    }
    .leave(id, now())
    .unwrap()
}

fn leave(store: &SharedStore, dest: &Path, id: &str) -> Leave {
    leave_with(store, &Claims::default(), None, Some(dest), id)
}

// ---------------------------------------------------------------- the remove path

#[test]
fn an_empty_notes_only_note_is_removed_with_its_home_folder_when_the_user_leaves_it() {
    // AE4.
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Removed);

    assert!(!home.exists());
    assert!(!row_exists(&store, "n"));
    assert!(
        entries(dest.path()).is_empty(),
        "{:?}",
        entries(dest.path())
    );
}

#[test]
fn a_recording_discarded_with_no_notes_text_and_no_earlier_note_is_removed() {
    // R19: the note started with this recording. This run's in-use marker
    // is still in the folder.
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    {
        let mut s = store.lock().unwrap();
        s.create_session("s", at(0), None).unwrap();
        early_home::begin(&s, "s").unwrap();
    }
    let home = early_home::write(&store, "s", Some(dest.path()), RUN, &RealFs)
        .unwrap()
        .unwrap();
    assert!(home.join(marker_name(RUN)).is_file());
    store
        .lock()
        .unwrap()
        .apply("s", &Event::RecordingDropped)
        .unwrap();

    assert_eq!(leave(&store, dest.path(), "s"), Leave::Removed);

    assert!(!home.exists());
    assert!(!row_exists(&store, "s"));
}

#[test]
fn this_runs_own_marker_does_not_keep_the_note() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    assert!(write_marker(&home, RUN, now()).unwrap());

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Removed);
    assert!(!home.exists());
}

#[test]
fn a_stale_marker_of_another_run_does_not_keep_the_note() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    let old = now() - FRESH_FOR - TimeDelta::minutes(1);
    assert!(write_marker(&home, "other-run", old).unwrap());

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Removed);
    assert!(!home.exists());
}

#[test]
fn a_note_whose_folder_is_not_written_yet_loses_only_its_row() {
    // The storage folder is there, and the early write did not start: no
    // file of the note is in the storage folder.
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    notes_only_note(&store, "n");

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Removed);
    assert!(!row_exists(&store, "n"));
    assert!(entries(dest.path()).is_empty());
}

#[test]
fn a_complete_staging_folder_of_the_note_is_removed() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    notes_only_note(&store, "n");
    let state = early_home::step(
        &store,
        "n",
        &EarlyState::Begun,
        Some(dest.path()),
        RUN,
        &RealFs,
    )
    .unwrap();
    early_home::record(&store, "n", &state).unwrap();
    assert!(matches!(state, EarlyState::Filled { .. }));
    assert!(dest.path().join(staging_name("n")).is_dir());

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Removed);
    assert!(!row_exists(&store, "n"));
    assert!(
        entries(dest.path()).is_empty(),
        "{:?}",
        entries(dest.path())
    );
}

#[test]
fn a_placed_folder_whose_record_a_crash_lost_is_found_and_removed() {
    // The rename to the final name happened, and its record did not.
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    notes_only_note(&store, "n");
    let filled = early_home::step(
        &store,
        "n",
        &EarlyState::Begun,
        Some(dest.path()),
        RUN,
        &RealFs,
    )
    .unwrap();
    early_home::record(&store, "n", &filled).unwrap();
    let done = early_home::step(&store, "n", &filled, Some(dest.path()), RUN, &RealFs).unwrap();
    let EarlyState::Done { path } = done else {
        panic!("expected Done, got {done:?}");
    };
    assert!(path.is_dir());

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Removed);
    assert!(!path.exists());
    assert!(!row_exists(&store, "n"));
}

#[test]
fn a_running_early_write_stops_before_the_removal_and_leaves_no_folder() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    notes_only_note(&store, "n");
    let target = dest.path().to_path_buf();
    let homes = EarlyHomes::new(
        Arc::clone(&store),
        RUN,
        Box::new(move || Some(target.clone())),
        Box::new(|_| {}),
    )
    .with_retry(Duration::from_millis(20));
    homes.start("n");

    let left = leave_with(
        &store,
        &Claims::default(),
        Some(&homes),
        Some(dest.path()),
        "n",
    );

    assert_eq!(left, Leave::Removed);
    homes.wait("n");
    assert!(!row_exists(&store, "n"));
    assert!(
        entries(dest.path()).is_empty(),
        "{:?}",
        entries(dest.path())
    );
}

#[test]
fn new_note_while_an_empty_note_is_shown_removes_it_and_opens_the_new_note() {
    // AE15: New Note runs the removal for the shown note first.
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let shown = placed_note(&store, "shown", dest.path());

    assert_eq!(leave(&store, dest.path(), "shown"), Leave::Removed);
    let new = placed_note(&store, "new", dest.path());

    assert!(!shown.exists() || shown == new);
    assert!(!row_exists(&store, "shown"));
    assert!(new.join("session.md").is_file());
    assert_eq!(entries(dest.path()).len(), 1);
}

// ---------------------------------------------------------------- keep: the row

#[test]
fn a_note_with_only_a_user_title_stays() {
    // AE5.
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    store
        .lock()
        .unwrap()
        .set_title("n", "Budget", TitleSource::User)
        .unwrap();

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Kept(Kept::NotEmpty));
    assert!(home.join("session.md").is_file());
    assert!(row_exists(&store, "n"));
}

#[test]
fn a_note_with_notes_text_stays() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    store.lock().unwrap().set_notes("n", "Agenda\n").unwrap();

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Kept(Kept::NotEmpty));
    assert!(home.is_dir());
}

#[test]
fn a_note_that_another_computer_started_is_never_removed() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    // The same folder, adopted as a note from another computer.
    let doc = session_core::folder::SessionDoc::read(&home).unwrap();
    let mut other = session_core::Adoption::from_doc(
        &doc,
        String::new(),
        home.to_string_lossy().into_owned(),
        None,
    );
    {
        let mut s = store.lock().unwrap();
        other.id = "b".into();
        s.adopt(&other).unwrap();
    }

    assert_eq!(
        leave(&store, dest.path(), "b"),
        Leave::Kept(Kept::StartedElsewhere)
    );
    assert!(home.is_dir());
    assert!(row_exists(&store, "b"));
}

#[test]
fn a_note_that_existed_before_a_discarded_recording_stays() {
    // R19, AE14: Record started the recording in a note with no audio.
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    store
        .lock()
        .unwrap()
        .set_result_state("n", RECORDED_INTO, &serde_json::Value::Bool(true))
        .unwrap();

    assert_eq!(
        leave(&store, dest.path(), "n"),
        Leave::Kept(Kept::ExistedBefore)
    );
    assert!(home.is_dir());
}

#[test]
fn a_note_that_records_stays() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    store
        .lock()
        .unwrap()
        .apply(
            "n",
            &Event::UserRecord {
                has_segments: false,
            },
        )
        .unwrap();

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Kept(Kept::Busy));
    assert!(home.is_dir());
}

#[test]
fn a_note_that_a_delete_or_a_move_claims_stays() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    let claims = Claims::default();
    let _moving = claims.moving("n").unwrap();

    assert_eq!(
        leave_with(&store, &claims, None, Some(dest.path()), "n"),
        Leave::Kept(Kept::Busy)
    );
    assert!(home.is_dir());
}

#[test]
fn a_note_with_no_row_is_kept_as_no_note() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    assert_eq!(leave(&store, dest.path(), "x"), Leave::Kept(Kept::NoNote));
}

// ---------------------------------------------------------------- keep: the folder

#[test]
fn notes_that_another_computer_changed_keep_the_note() {
    // AE11: computer B typed into the note, and the edit synced here.
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    std::fs::write(home.join("notes.md"), "typed on B\n").unwrap();

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Kept(Kept::Changed));
    assert_eq!(
        std::fs::read_to_string(home.join("notes.md")).unwrap(),
        "typed on B\n"
    );
    assert!(row_exists(&store, "n"));
}

#[test]
fn a_session_md_that_another_computer_changed_keeps_the_note() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    let mut doc = session_core::folder::SessionDoc::read(&home).unwrap();
    doc.title = Some("Typed on B".into());
    doc.title_source = Some(TitleSource::User);
    std::fs::write(home.join("session.md"), doc.render()).unwrap();

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Kept(Kept::Changed));
    assert!(home.join("session.md").is_file());
}

#[test]
fn a_missing_notes_md_keeps_the_note() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    std::fs::remove_file(home.join("notes.md")).unwrap();

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Kept(Kept::Changed));
    assert!(home.join("session.md").is_file());
}

/// The state after a removal deleted `notes.md` and could not delete
/// `session.md`: the row records the folder, and `notes.md` is gone.
fn stopped_removal(store: &SharedStore, home: &Path) {
    store
        .lock()
        .unwrap()
        .set_result_state(
            "n",
            REMOVING,
            &serde_json::Value::from(home.to_string_lossy()),
        )
        .unwrap();
    std::fs::remove_file(home.join("notes.md")).unwrap();
}

#[test]
fn a_removal_that_stopped_after_notes_md_continues_at_the_next_leave() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    stopped_removal(&store, &home);

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Removed);
    assert!(!home.exists());
    assert!(!row_exists(&store, "n"));
}

#[test]
fn a_stopped_removal_keeps_the_note_when_another_computer_changed_session_md() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    stopped_removal(&store, &home);
    let mut doc = session_core::folder::SessionDoc::read(&home).unwrap();
    doc.title = Some("Typed on B".into());
    doc.title_source = Some(TitleSource::User);
    std::fs::write(home.join("session.md"), doc.render()).unwrap();

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Kept(Kept::Changed));
    assert!(home.join("session.md").is_file());
    assert!(row_exists(&store, "n"));
}

#[test]
fn a_stopped_removal_of_another_folder_does_not_accept_a_missing_notes_md() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    let other = dest.path().join("another folder");
    store
        .lock()
        .unwrap()
        .set_result_state(
            "n",
            REMOVING,
            &serde_json::Value::from(other.to_string_lossy()),
        )
        .unwrap();
    std::fs::remove_file(home.join("notes.md")).unwrap();

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Kept(Kept::Changed));
    assert!(home.join("session.md").is_file());
}

// ---------------------------------------------------------------- keep: shown again

fn leave_shown(store: &SharedStore, dest: &Path, shown: &dyn Fn(&str) -> bool) -> Leave {
    Removal {
        store,
        claims: &Claims::default(),
        homes: None,
        destination: Some(dest),
        run_id: RUN,
    }
    .leave_unless_shown("n", now(), shown)
    .unwrap()
}

#[test]
fn a_note_that_the_main_window_shows_stays() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());

    assert_eq!(
        leave_shown(&store, dest.path(), &|id| id == "n"),
        Leave::Kept(Kept::Shown)
    );
    assert_eq!(names(&home), vec!["notes.md", "session.md"]);
    assert!(row_exists(&store, "n"));
}

#[test]
fn a_note_that_the_main_window_shows_again_during_the_removal_stays() {
    // The first check passes. The user opens the note again before the
    // files go.
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    let checks = std::cell::Cell::new(0);
    let shown = |_: &str| {
        checks.set(checks.get() + 1);
        checks.get() > 1
    };

    assert_eq!(
        leave_shown(&store, dest.path(), &shown),
        Leave::Kept(Kept::Shown)
    );
    assert_eq!(checks.get(), 2);
    assert_eq!(names(&home), vec!["notes.md", "session.md"]);
    assert!(row_exists(&store, "n"));
    assert_eq!(
        store.lock().unwrap().result_state("n", REMOVING).unwrap(),
        None
    );
}

#[test]
fn a_fresh_marker_of_another_run_keeps_the_note() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    assert!(write_marker(&home, "other-run", now()).unwrap());
    // This run's marker goes first; the other one stays.
    assert!(write_marker(&home, RUN, now()).unwrap());

    assert_eq!(leave(&store, dest.path(), "n"), Leave::Kept(Kept::InUse));
    assert!(home.join(marker_name("other-run")).is_file());
    assert!(home.join("session.md").is_file());
    assert!(home.join("notes.md").is_file());
}

#[test]
fn an_extra_file_in_the_folder_keeps_the_note() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    std::fs::write(home.join("notes (1).md"), "a conflict copy\n").unwrap();

    assert_eq!(
        leave(&store, dest.path(), "n"),
        Leave::Kept(Kept::OtherFiles)
    );
    assert_eq!(names(&home), vec!["notes (1).md", "notes.md", "session.md"]);
}

#[test]
fn a_document_name_in_another_case_keeps_the_note() {
    // On case-insensitive APFS, `Notes.md` replaces the file name of
    // `notes.md`. The exact name is not the one that Longhand writes.
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    let text = std::fs::read(home.join("notes.md")).unwrap();
    std::fs::remove_file(home.join("notes.md")).unwrap();
    std::fs::write(home.join("Notes.md"), text).unwrap();

    assert!(matches!(
        leave(&store, dest.path(), "n"),
        Leave::Kept(Kept::OtherFiles | Kept::Changed)
    ));
    assert!(home.join("Notes.md").is_file());
    assert!(home.join("session.md").is_file());
}

#[test]
fn a_subfolder_in_the_folder_keeps_the_note() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    std::fs::create_dir(home.join("audio")).unwrap();

    assert_eq!(
        leave(&store, dest.path(), "n"),
        Leave::Kept(Kept::OtherFiles)
    );
    assert!(home.join("audio").is_dir());
}

#[cfg(unix)]
#[test]
fn a_home_folder_that_is_a_link_keeps_the_note_and_the_target() {
    let dest = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let store = store();
    let home = placed_note(&store, "n", dest.path());
    // The same files in another folder, and the home path links there.
    for name in ["notes.md", "session.md"] {
        std::fs::copy(home.join(name), other.path().join(name)).unwrap();
    }
    std::fs::remove_dir_all(&home).unwrap();
    std::os::unix::fs::symlink(other.path(), &home).unwrap();

    assert!(matches!(leave(&store, dest.path(), "n"), Leave::Kept(_)));
    assert_eq!(names(other.path()), vec!["notes.md", "session.md"]);
}

#[test]
fn an_unavailable_storage_folder_removes_nothing_and_the_next_pass_removes_the_note() {
    let base = tempfile::tempdir().unwrap();
    let dest = base.path().join("storage");
    std::fs::create_dir(&dest).unwrap();
    let store = store();
    let home = placed_note(&store, "n", &dest);
    let away = base.path().join("away");
    std::fs::rename(&dest, &away).unwrap();

    assert_eq!(leave(&store, &dest, "n"), Leave::Kept(Kept::Unavailable));
    assert!(row_exists(&store, "n"));
    assert!(away.join(home.file_name().unwrap()).is_dir());

    std::fs::rename(&away, &dest).unwrap();
    let removal = Removal {
        store: &store,
        claims: &Claims::default(),
        homes: None,
        destination: Some(&dest),
        run_id: RUN,
    };
    let passed = removal.launch_pass(at(10), now()).unwrap();
    assert_eq!(passed, vec![("n".to_string(), Leave::Removed)]);
    assert!(!home.exists());
    assert!(!row_exists(&store, "n"));
}

#[test]
fn a_note_not_written_yet_with_no_storage_folder_stays() {
    let base = tempfile::tempdir().unwrap();
    let store = store();
    notes_only_note(&store, "n");

    assert_eq!(
        leave(&store, &base.path().join("missing"), "n"),
        Leave::Kept(Kept::Unavailable)
    );
    assert_eq!(
        leave_with(&store, &Claims::default(), None, None, "n"),
        Leave::Kept(Kept::Unavailable)
    );
    assert!(row_exists(&store, "n"));
}

// ---------------------------------------------------------------- the launch pass

#[test]
fn the_launch_pass_removes_only_empty_notes_that_this_computer_started_before_the_launch() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let empty = placed_note(&store, "empty", dest.path());
    let typed = placed_note(&store, "typed", dest.path());
    store
        .lock()
        .unwrap()
        .set_notes("typed", "Agenda\n")
        .unwrap();
    // A note that opened in this run: the user can see it now.
    {
        let mut s = store.lock().unwrap();
        s.create_notes_only("new", at(100), None).unwrap();
        early_home::begin(&s, "new").unwrap();
    }
    // A note from another computer.
    {
        let doc = session_core::folder::SessionDoc::read(&empty).unwrap();
        let mut other = session_core::Adoption::from_doc(
            &doc,
            String::new(),
            empty.to_string_lossy().into_owned(),
            None,
        );
        other.id = "b".into();
        store.lock().unwrap().adopt(&other).unwrap();
    }
    // A note with audio.
    store
        .lock()
        .unwrap()
        .create_session("rec", at(0), None)
        .unwrap();

    let removal = Removal {
        store: &store,
        claims: &Claims::default(),
        homes: None,
        destination: Some(dest.path()),
        run_id: RUN,
    };
    let passed = removal.launch_pass(at(50), now()).unwrap();

    assert_eq!(passed, vec![("empty".to_string(), Leave::Removed)]);
    assert!(!row_exists(&store, "empty"));
    assert!(typed.is_dir());
    assert!(row_exists(&store, "typed"));
    assert!(row_exists(&store, "new"));
    assert!(row_exists(&store, "b"));
    assert!(row_exists(&store, "rec"));
    let state = store.lock().unwrap().session("rec").unwrap().unwrap().state;
    assert_eq!(state, SessionState::Capturing);
}
