//! The early write of a note's home folder (U3, KTD3, KTD4): every new note
//! gets one home folder in the storage folder at its start, and a crash at
//! any step leaves at most one folder (R4).

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::{at, store, write_segment};
use pipeline::early_home::{self, EarlyHomes, EarlyState};
use pipeline::in_use::marker_name;
use pipeline::{SharedStore, StageError};
use session_core::folder::{ResultFlag, SessionDoc, fingerprint};
use session_core::{Event, SessionState};
use session_save::RealFs;
use session_save::naming::staging_name;

const RUN: &str = "run-1";

/// A note that records, with its early write begun, as capture makes it.
fn recording_note(store: &SharedStore, id: &str, notes: &str) {
    let mut s = store.lock().unwrap();
    s.create_session(id, at(0), None).unwrap();
    s.set_notes(id, notes).unwrap();
    early_home::begin(&s, id).unwrap();
}

/// A note that opened with no recording, with its early write begun.
fn notes_only_note(store: &SharedStore, id: &str, notes: &str) {
    let mut s = store.lock().unwrap();
    s.create_notes_only(id, at(0), None).unwrap();
    s.set_notes(id, notes).unwrap();
    early_home::begin(&s, id).unwrap();
}

fn early(store: &SharedStore, id: &str) -> Option<EarlyState> {
    early_home::state(&store.lock().unwrap(), id).unwrap()
}

fn home(store: &SharedStore, id: &str) -> Option<PathBuf> {
    store
        .lock()
        .unwrap()
        .session(id)
        .unwrap()
        .unwrap()
        .home_path
        .map(PathBuf::from)
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

/// The note folders in `dest`: every name that is not hidden.
fn folders(dest: &Path) -> Vec<String> {
    names(dest)
        .into_iter()
        .filter(|n| !n.starts_with('.'))
        .collect()
}

fn write(
    store: &SharedStore,
    id: &str,
    dest: Option<&Path>,
) -> Result<Option<PathBuf>, StageError> {
    early_home::write(store, id, dest, RUN, &RealFs)
}

/// Take one step and record it, as the job does.
fn step(store: &SharedStore, id: &str, dest: &Path) -> EarlyState {
    let state = early(store, id).unwrap();
    let next = early_home::step(store, id, &state, Some(dest), RUN, &RealFs).unwrap();
    early_home::record(store, id, &next).unwrap();
    next
}

#[test]
fn a_recording_note_gets_a_home_folder_with_its_notes_and_the_pending_flag() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "Agenda\n");

    let path = write(&store, "s", Some(dest.path())).unwrap().unwrap();

    assert_eq!(home(&store, "s"), Some(path.clone()));
    assert_eq!(
        early(&store, "s"),
        Some(EarlyState::Done { path: path.clone() })
    );
    assert_eq!(
        folders(dest.path()),
        vec!["2027-01-15 0800 Longhand session"]
    );
    assert!(!dest.path().join(staging_name("s")).exists());
    assert_eq!(
        std::fs::read_to_string(path.join("notes.md")).unwrap(),
        "Agenda\n"
    );
    let doc = session_core::note_folder::NoteFiles::read(&path)
        .map(|files| files.doc)
        .unwrap();
    assert_eq!(doc.note_id, "s");
    assert_eq!(doc.result, Some(ResultFlag::NotSaved));
    assert!(doc.segments.is_empty());
    assert_eq!(doc.started, at(0));
    // This run's in-use marker: the note records on this computer.
    assert!(path.join(marker_name(RUN)).is_file());
    // The store knows the files as this computer's (KTD8).
    let written = store.lock().unwrap().written_fingerprints("s").unwrap();
    let of = |name: &str| Some(fingerprint(&std::fs::read(path.join(name)).unwrap()));
    assert_eq!(written.session, Some(files_written(&path)));
    assert_eq!(written.notes, of("notes.md"));
    // The new layout (KTD1, KTD3): `longhand.json` holds the identity and
    // the records, and `session.md` has only a frontmatter.
    let files = session_core::note_folder::NoteFiles::read(&path).unwrap();
    assert_eq!(files.layout, session_core::note_folder::Layout::New);
    assert_eq!(files.record.note_id, "s");
    assert_eq!(
        std::fs::read_to_string(path.join("session.md")).unwrap(),
        "---\n---\n"
    );
}

/// Refuses each rename to a note name (not a hidden name) as in use
/// (`PermissionDenied`, as Windows does while another app holds a file)
/// `refusals` times, and counts those renames.
struct InUse {
    refusals: std::cell::Cell<usize>,
    tries: std::cell::Cell<usize>,
}

impl InUse {
    fn new(refusals: usize) -> Self {
        Self {
            refusals: std::cell::Cell::new(refusals),
            tries: std::cell::Cell::new(0),
        }
    }
}

impl session_save::Fs for InUse {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        session_save::Fs::rename(&RealFs, from, to)
    }
    fn rename_new(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        let hidden = to
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'));
        if !hidden {
            self.tries.set(self.tries.get() + 1);
            if self.refusals.get() > 0 {
                self.refusals.set(self.refusals.get() - 1);
                return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
            }
        }
        session_save::Fs::rename_new(&RealFs, from, to)
    }
}

#[test]
fn a_folder_in_use_for_a_moment_gets_its_own_name() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "Agenda\n");
    let fs = InUse::new(2);

    let path = early_home::write(&store, "s", Some(dest.path()), RUN, &fs)
        .unwrap()
        .unwrap();

    assert_eq!(
        folders(dest.path()),
        vec!["2027-01-15 0800 Longhand session"],
        "in use is not a taken name"
    );
    assert_eq!(home(&store, "s"), Some(path));
    assert_eq!(fs.tries.get(), 3);
}

#[test]
fn a_folder_that_stays_in_use_waits_saying_so_and_tries_no_other_name() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "Agenda\n");
    let fs = InUse::new(usize::MAX);

    let error = early_home::write(&store, "s", Some(dest.path()), RUN, &fs).unwrap_err();

    match error {
        StageError::NoDestination(message) => {
            assert!(message.contains("in use"), "{message}");
            assert!(!message.contains("no free name"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(fs.tries.get(), 1 + session_save::rename::IN_USE_RETRIES);
    assert!(folders(dest.path()).is_empty());
}

/// The value that the store records for the `session.md` and the
/// `longhand.json` in `path`.
fn files_written(path: &Path) -> String {
    let read = |name: &str| std::fs::read(path.join(name)).unwrap();
    session_core::note_folder::written_fingerprint(&read("session.md"), &read("longhand.json"))
}

#[test]
fn the_early_write_puts_the_tags_and_the_pending_type_of_the_row_in_session_md() {
    // KTD2 of the note types and tags plan.
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "");
    let pending = session_core::folder::PendingType {
        to: Some("One on One".into()),
        from: None,
        by: "computer-a".into(),
        at: at(0),
    };
    {
        let s = store.lock().unwrap();
        s.set_tags("s", &["Topic::Security".to_string()]).unwrap();
        s.set_pending_type("s", Some(&pending)).unwrap();
    }

    let path = write(&store, "s", Some(dest.path())).unwrap().unwrap();

    let doc = session_core::note_folder::NoteFiles::read(&path)
        .map(|files| files.doc)
        .unwrap();
    assert_eq!(doc.tags, vec!["Topic::Security".to_string()]);
    assert_eq!(doc.pending_type, Some(pending));
}

#[test]
fn a_notes_only_note_gets_a_home_folder_with_the_no_recording_flag_and_no_marker() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    notes_only_note(&store, "n", "");

    let path = write(&store, "n", Some(dest.path())).unwrap().unwrap();

    let doc = session_core::note_folder::NoteFiles::read(&path)
        .map(|files| files.doc)
        .unwrap();
    assert_eq!(doc.result, Some(ResultFlag::NoRecording));
    assert!(!path.join(marker_name(RUN)).exists());
    assert_eq!(
        names(&path),
        vec!["longhand.json", "notes.md", "session.md"]
    );
    // The row is no longer busy: it has its home folder.
    let record = store.lock().unwrap().session("n").unwrap().unwrap();
    assert!(!session_core::jobs::busy(&record, false));
}

#[test]
fn the_staging_folder_holds_the_marker_before_session_md_is_at_the_final_name() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "");

    let filled = step(&store, "s", dest.path());

    assert!(matches!(filled, EarlyState::Filled { .. }), "{filled:?}");
    let staging = dest.path().join(staging_name("s"));
    assert!(staging.join(marker_name(RUN)).is_file());
    assert!(staging.join("session.md").is_file());
    assert!(
        folders(dest.path()).is_empty(),
        "nothing at a final name yet"
    );
    assert_eq!(home(&store, "s"), None);
}

#[test]
fn a_crash_while_the_staging_folder_is_partly_written_completes_it_in_one_folder() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "typed\n");
    // The step wrote part of the staging folder; its record was lost.
    let state = early(&store, "s").unwrap();
    early_home::step(&store, "s", &state, Some(dest.path()), RUN, &RealFs).unwrap();
    let staging = dest.path().join(staging_name("s"));
    std::fs::remove_file(staging.join("session.md")).unwrap();
    assert_eq!(early(&store, "s"), Some(EarlyState::Begun));

    let path = write(&store, "s", Some(dest.path())).unwrap().unwrap();

    assert_eq!(folders(dest.path()).len(), 1);
    assert!(!staging.exists());
    assert_eq!(
        session_core::note_folder::NoteFiles::read(&path)
            .map(|files| files.doc)
            .unwrap()
            .note_id,
        "s"
    );
    assert_eq!(
        std::fs::read_to_string(path.join("notes.md")).unwrap(),
        "typed\n"
    );
}

#[test]
fn a_crash_after_the_rename_and_before_its_record_finds_the_folder_by_note_id() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "");
    let filled = step(&store, "s", dest.path());
    // The rename happened; its record (the home path) was lost.
    let placed = early_home::step(&store, "s", &filled, Some(dest.path()), RUN, &RealFs).unwrap();
    let EarlyState::Done { path } = &placed else {
        panic!("{placed:?}");
    };
    assert_eq!(home(&store, "s"), None);

    // Run again from the recorded state: the same folder.
    let again = early_home::step(&store, "s", &filled, Some(dest.path()), RUN, &RealFs).unwrap();
    assert_eq!(again, placed);
    let written = write(&store, "s", Some(dest.path())).unwrap().unwrap();

    assert_eq!(&written, path);
    assert_eq!(home(&store, "s"), Some(path.clone()));
    assert_eq!(folders(dest.path()).len(), 1);
    assert!(
        !dest
            .path()
            .join("2027-01-15 0800 Longhand session-2")
            .exists()
    );
}

// Covers R23, R24: the folder was at its final name, and another computer
// or the user removed it before the Done record.
#[test]
fn a_folder_removed_after_the_rename_and_before_its_record_is_not_made_again() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "");
    let filled = step(&store, "s", dest.path());
    // The rename happened; its record was lost.
    let placed = early_home::step(&store, "s", &filled, Some(dest.path()), RUN, &RealFs).unwrap();
    let EarlyState::Done { path } = placed else {
        panic!("{placed:?}");
    };
    std::fs::remove_dir_all(&path).unwrap();

    let written = write(&store, "s", Some(dest.path())).unwrap().unwrap();

    assert!(folders(dest.path()).is_empty());
    assert!(!dest.path().join(staging_name("s")).exists());
    assert!(!written.exists());
    assert!(!early_home::unfinished(&store.lock().unwrap(), "s").unwrap());
    // A second run makes no folder either.
    write(&store, "s", Some(dest.path())).unwrap();
    assert!(folders(dest.path()).is_empty());
    assert!(!dest.path().join(staging_name("s")).exists());
}

#[test]
fn a_folder_at_the_name_that_cannot_be_read_keeps_the_filled_state() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "");
    let filled = step(&store, "s", dest.path());
    let EarlyState::Done { path } =
        early_home::step(&store, "s", &filled, Some(dest.path()), RUN, &RealFs).unwrap()
    else {
        panic!("not placed");
    };
    // The `session.md` of the folder cannot be read, as a file that does
    // not download yet.
    let session = path.join("session.md");
    let text = std::fs::read(&session).unwrap();
    std::fs::remove_file(&session).unwrap();
    std::fs::create_dir(&session).unwrap();

    let error = write(&store, "s", Some(dest.path())).unwrap_err();

    assert!(matches!(error, StageError::NoDestination(_)), "{error:?}");
    assert_eq!(early(&store, "s"), Some(filled));
    assert_eq!(folders(dest.path()).len(), 1);
    assert!(!dest.path().join(staging_name("s")).exists());
    // The file is back: the write finds the folder.
    std::fs::remove_dir(&session).unwrap();
    std::fs::write(&session, text).unwrap();
    assert_eq!(write(&store, "s", Some(dest.path())).unwrap(), Some(path));
    assert_eq!(folders(dest.path()).len(), 1);
}

#[test]
fn a_folder_of_another_note_at_the_name_is_not_taken_for_this_one() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "");
    let other = dest.path().join("2027-01-15 0800 Longhand session");
    std::fs::create_dir(&other).unwrap();
    std::fs::write(
        other.join("session.md"),
        SessionDoc::without_audio("other", at(0), None).render(),
    )
    .unwrap();

    let path = write(&store, "s", Some(dest.path())).unwrap().unwrap();

    assert_eq!(path, dest.path().join("2027-01-15 0800 Longhand session-2"));
    assert_eq!(
        session_core::note_folder::NoteFiles::read(&other)
            .map(|files| files.doc)
            .unwrap()
            .note_id,
        "other"
    );
}

#[test]
fn a_crash_after_the_home_path_and_before_the_done_record_uses_that_folder() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "");
    let filled = step(&store, "s", dest.path());
    let EarlyState::Done { path } =
        early_home::step(&store, "s", &filled, Some(dest.path()), RUN, &RealFs).unwrap()
    else {
        panic!("not placed");
    };
    // The home path is recorded; the state still says Filled.
    store
        .lock()
        .unwrap()
        .set_home_path("s", &path.to_string_lossy())
        .unwrap();
    assert!(early_home::unfinished(&store.lock().unwrap(), "s").unwrap());

    let written = write(&store, "s", Some(dest.path())).unwrap().unwrap();

    assert_eq!(written, path);
    assert_eq!(folders(dest.path()).len(), 1);
    assert!(!early_home::unfinished(&store.lock().unwrap(), "s").unwrap());
}

#[test]
fn a_crash_after_the_done_record_keeps_the_folder() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "");
    let path = write(&store, "s", Some(dest.path())).unwrap().unwrap();

    let again = write(&store, "s", Some(dest.path())).unwrap().unwrap();

    assert_eq!(again, path);
    assert_eq!(folders(dest.path()).len(), 1);
}

#[test]
fn a_crash_before_any_write_makes_one_folder_at_launch() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "");
    // A notes-only row whose begin record was lost with the crash.
    store
        .lock()
        .unwrap()
        .create_notes_only("n", at(10), None)
        .unwrap();

    let waiting = early_home::waiting(&store.lock().unwrap()).unwrap();
    assert_eq!(waiting, vec!["n".to_string(), "s".to_string()]);
    for id in &waiting {
        write(&store, id, Some(dest.path())).unwrap().unwrap();
    }

    assert_eq!(folders(dest.path()).len(), 2);
    assert!(
        early_home::waiting(&store.lock().unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn an_unavailable_storage_folder_waits_and_the_write_succeeds_when_it_is_back() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "");
    let gone = dest.path().join("storage");

    let error = write(&store, "s", Some(&gone)).unwrap_err();

    assert!(matches!(error, StageError::NoDestination(_)), "{error:?}");
    assert_eq!(early(&store, "s"), Some(EarlyState::Begun));
    assert!(!gone.exists(), "the write never makes the storage folder");
    std::fs::create_dir(&gone).unwrap();
    let path = write(&store, "s", Some(&gone)).unwrap().unwrap();
    assert_eq!(path.parent(), Some(gone.as_path()));
}

#[test]
fn a_storage_folder_that_goes_away_after_the_staging_keeps_the_state_and_makes_no_second_folder() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "");
    let storage = dest.path().join("storage");
    std::fs::create_dir(&storage).unwrap();
    let filled = step(&store, "s", &storage);
    let away = dest.path().join("away");
    std::fs::rename(&storage, &away).unwrap();

    let error = write(&store, "s", Some(&storage)).unwrap_err();

    assert!(matches!(error, StageError::NoDestination(_)), "{error:?}");
    assert_eq!(early(&store, "s"), Some(filled));
    std::fs::rename(&away, &storage).unwrap();
    write(&store, "s", Some(&storage)).unwrap().unwrap();
    assert_eq!(folders(&storage).len(), 1);
}

#[test]
fn notes_saved_while_the_note_records_go_into_the_home_folder() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "");
    // Before the folder exists, the text stays in the store.
    {
        let mut s = store.lock().unwrap();
        let written = pipeline::save::set_notes(&mut s, "s", "early\n").unwrap();
        assert!(!written);
    }
    let path = write(&store, "s", Some(dest.path())).unwrap().unwrap();
    assert_eq!(
        std::fs::read_to_string(path.join("notes.md")).unwrap(),
        "early\n"
    );

    let written = {
        let mut s = store.lock().unwrap();
        pipeline::save::set_notes(&mut s, "s", "during\n").unwrap()
    };

    assert!(written);
    assert_eq!(
        std::fs::read_to_string(path.join("notes.md")).unwrap(),
        "during\n"
    );
    let stored = store
        .lock()
        .unwrap()
        .written_fingerprints("s")
        .unwrap()
        .notes;
    assert_eq!(stored, Some(fingerprint(b"during\n")));
}

#[test]
fn notes_typed_after_the_staging_and_before_the_rename_reach_the_home_folder() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    notes_only_note(&store, "n", "first\n");
    step(&store, "n", dest.path());
    {
        let mut s = store.lock().unwrap();
        pipeline::save::set_notes(&mut s, "n", "second\n").unwrap();
    }

    let path = write(&store, "n", Some(dest.path())).unwrap().unwrap();

    assert_eq!(
        std::fs::read_to_string(path.join("notes.md")).unwrap(),
        "second\n"
    );
    let stored = store
        .lock()
        .unwrap()
        .written_fingerprints("n")
        .unwrap()
        .notes;
    assert_eq!(stored, Some(fingerprint(b"second\n")));
}

#[test]
fn record_into_a_notes_only_note_writes_the_marker_and_then_the_pending_flag() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    notes_only_note(&store, "n", "");
    let path = write(&store, "n", Some(dest.path())).unwrap().unwrap();
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

    early_home::sync(&store, "n", RUN).unwrap();

    assert!(path.join(marker_name(RUN)).is_file());
    let doc = session_core::note_folder::NoteFiles::read(&path)
        .map(|files| files.doc)
        .unwrap();
    assert_eq!(doc.result, Some(ResultFlag::NotSaved));
    let written = store
        .lock()
        .unwrap()
        .written_fingerprints("n")
        .unwrap()
        .session;
    assert_eq!(written, Some(files_written(&path)));
}

#[test]
fn a_flag_update_keeps_a_title_from_another_computer() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    notes_only_note(&store, "n", "");
    let path = write(&store, "n", Some(dest.path())).unwrap().unwrap();
    let mut doc = session_core::note_folder::NoteFiles::read(&path)
        .map(|files| files.doc)
        .unwrap();
    doc.title = Some("Planning".into());
    doc.title_source = Some(session_core::TitleSource::User);
    session_save::note_files::write_doc(&path, &doc).unwrap();
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

    early_home::sync(&store, "n", RUN).unwrap();

    let doc = session_core::note_folder::NoteFiles::read(&path)
        .map(|files| files.doc)
        .unwrap();
    assert_eq!(doc.title.as_deref(), Some("Planning"));
    assert_eq!(doc.result, Some(ResultFlag::NotSaved));
}

#[test]
fn a_flag_update_after_the_save_began_writes_nothing() {
    let dest = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let store = store();
    notes_only_note(&store, "n", "");
    let path = write(&store, "n", Some(dest.path())).unwrap().unwrap();
    {
        let mut s = store.lock().unwrap();
        s.apply(
            "n",
            &Event::UserRecord {
                has_segments: false,
            },
        )
        .unwrap();
        let segment = write_segment(
            &root.path().join("n"),
            0,
            0,
            16_000,
            &vec![0.1; 32_000],
            false,
        );
        s.put_segment("n", &segment).unwrap();
        // The save claimed the note (its state is recorded).
        s.put_checkpoint(
            "n",
            session_core::Stage::Save,
            pipeline::save::STATE,
            &serde_json::to_string(&session_save::SaveState::Working).unwrap(),
        )
        .unwrap();
    }
    let before = std::fs::read(path.join("session.md")).unwrap();

    early_home::sync(&store, "n", RUN).unwrap();

    assert_eq!(std::fs::read(path.join("session.md")).unwrap(), before);
    assert!(!path.join(marker_name(RUN)).exists());
}

#[test]
fn a_job_cancelled_while_it_retries_makes_no_folder_later() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    notes_only_note(&store, "cancel-n", "");
    let storage = dest.path().join("storage");
    let target = storage.clone();
    let homes = EarlyHomes::new(
        Arc::clone(&store),
        RUN,
        Box::new(move || Some(target.clone())),
        Box::new(|_| {}),
    )
    .with_retry(Duration::from_millis(20));
    homes.start("cancel-n");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while homes.problem("cancel-n").is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        homes.problem("cancel-n").is_some(),
        "the job reports why it waits"
    );

    homes.cancel("cancel-n");
    std::fs::create_dir(&storage).unwrap();
    homes.wake();
    std::thread::sleep(Duration::from_millis(100));

    assert!(names(&storage).is_empty());
    assert_eq!(home(&store, "cancel-n"), None);
}

#[test]
fn a_job_retries_until_the_storage_folder_is_back_and_reports_the_placed_folder() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "retry-s", "");
    let storage = dest.path().join("storage");
    let target = storage.clone();
    let (placed_tx, placed_rx) = std::sync::mpsc::channel();
    let homes = EarlyHomes::new(
        Arc::clone(&store),
        RUN,
        Box::new(move || Some(target.clone())),
        Box::new(move |id| placed_tx.send(id.to_string()).unwrap()),
    )
    .with_retry(Duration::from_millis(20));
    homes.start("retry-s");
    std::thread::sleep(Duration::from_millis(60));

    std::fs::create_dir(&storage).unwrap();
    homes.wake();

    let placed = placed_rx.recv().unwrap();
    assert_eq!(placed, "retry-s");
    homes.wait("retry-s");
    assert_eq!(folders(&storage).len(), 1);
    assert!(homes.problem("retry-s").is_none());
    assert!(!early_home::unfinished(&store.lock().unwrap(), "retry-s").unwrap());
}

#[test]
fn a_pending_update_queued_on_a_job_writes_the_flag_in_order() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    notes_only_note(&store, "queue-n", "");
    let target = dest.path().to_path_buf();
    let homes = EarlyHomes::new(
        Arc::clone(&store),
        RUN,
        Box::new(move || Some(target.clone())),
        Box::new(|_| {}),
    );
    homes.start("queue-n");
    store
        .lock()
        .unwrap()
        .apply(
            "queue-n",
            &Event::UserRecord {
                has_segments: false,
            },
        )
        .unwrap();
    homes.mark_pending("queue-n");
    homes.wait("queue-n");

    let path = home(&store, "queue-n").unwrap();
    assert_eq!(
        session_core::note_folder::NoteFiles::read(&path)
            .map(|files| files.doc)
            .unwrap()
            .result,
        Some(ResultFlag::NotSaved)
    );
    assert!(path.join(marker_name(RUN)).is_file());
    assert_eq!(
        store
            .lock()
            .unwrap()
            .session("queue-n")
            .unwrap()
            .unwrap()
            .state,
        SessionState::Capturing
    );
}

// Covers AE6 (R19): a recording into a note with no audio that keeps no
// segment makes it a note with no audio again, and its flag says so.
#[test]
fn a_dropped_recording_into_a_notes_only_note_writes_the_no_recording_flag_again() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    notes_only_note(&store, "n", "Agenda\n");
    let path = write(&store, "n", Some(dest.path())).unwrap().unwrap();
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
    early_home::sync(&store, "n", RUN).unwrap();
    assert_eq!(
        session_core::note_folder::NoteFiles::read(&path)
            .map(|files| files.doc)
            .unwrap()
            .result,
        Some(ResultFlag::NotSaved)
    );
    assert!(path.join(marker_name(RUN)).is_file());
    // The marker of another run stays.
    let other = path.join(marker_name("run-2"));
    std::fs::write(&other, "{}").unwrap();

    store
        .lock()
        .unwrap()
        .apply("n", &Event::RecordingDropped)
        .unwrap();
    early_home::sync(&store, "n", RUN).unwrap();

    let doc = session_core::note_folder::NoteFiles::read(&path)
        .map(|files| files.doc)
        .unwrap();
    assert_eq!(doc.result, Some(ResultFlag::NoRecording));
    // The note does not record any more: this run's marker goes, so other
    // computers do not show the note as in use.
    assert!(!path.join(marker_name(RUN)).exists());
    assert!(other.is_file());
    // A second update with no marker is not an error.
    early_home::sync(&store, "n", RUN).unwrap();
    assert_eq!(
        std::fs::read_to_string(path.join("notes.md")).unwrap(),
        "Agenda\n"
    );
}

/// A notes-only note "n" with its home folder in `dest`, typed and saved.
fn placed_notes_only_note(store: &SharedStore, dest: &Path) -> PathBuf {
    notes_only_note(store, "n", "First\n");
    write(store, "n", Some(dest)).unwrap().unwrap()
}

// Covers R24: another computer removed the folder of a note with no audio.
// The notes save keeps the text in a new note with no audio.
#[test]
fn a_notes_save_after_the_home_folder_was_removed_makes_a_new_note_with_the_text() {
    use pipeline::save::{NotesRescue, NotesSave, rescue_notes, save_notes};
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let path = placed_notes_only_note(&store, dest.path());
    store
        .lock()
        .unwrap()
        .set_title("n", "Planning", session_core::TitleSource::User)
        .unwrap();
    std::fs::remove_dir_all(&path).unwrap();

    let saved = save_notes(&mut store.lock().unwrap(), "n", "Second\n", None).unwrap();
    assert_eq!(saved, NotesSave::FolderGone);
    assert!(!path.exists(), "the save makes no folder");

    let rescued = rescue_notes(&store, "n", "m", at(100)).unwrap();
    assert_eq!(rescued, NotesRescue::NewNote);
    {
        let s = store.lock().unwrap();
        assert!(
            s.session("n").unwrap().is_none(),
            "the old row is forgotten"
        );
        let record = s.session("m").unwrap().unwrap();
        assert_eq!(record.state, SessionState::Saved);
        assert_eq!(record.started_at, Some(at(100)));
        assert_eq!(record.title.map(|t| t.text).as_deref(), Some("Planning"));
        assert!(s.segments("m").unwrap().is_empty());
        assert_eq!(s.notes("m").unwrap(), "Second\n");
        assert!(s.started_here("m").unwrap());
    }
    assert_eq!(early(&store, "m"), Some(EarlyState::Begun));

    // The early write of the new note places its folder with the text.
    let placed = write(&store, "m", Some(dest.path())).unwrap().unwrap();
    assert_eq!(
        std::fs::read_to_string(placed.join("notes.md")).unwrap(),
        "Second\n"
    );
    assert_eq!(
        session_core::note_folder::NoteFiles::read(&placed)
            .map(|files| files.doc)
            .unwrap()
            .result,
        Some(ResultFlag::NoRecording)
    );
}

#[test]
fn a_notes_save_follows_a_folder_that_has_another_name() {
    use pipeline::save::{NotesRescue, NotesSave, rescue_notes, save_notes};
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let path = placed_notes_only_note(&store, dest.path());
    let moved = dest.path().join("Renamed");
    std::fs::rename(&path, &moved).unwrap();

    let saved = save_notes(&mut store.lock().unwrap(), "n", "Second\n", None).unwrap();
    assert_eq!(saved, NotesSave::FolderGone);
    assert_eq!(
        rescue_notes(&store, "n", "m", at(100)).unwrap(),
        NotesRescue::Moved
    );
    assert_eq!(home(&store, "n"), Some(moved.clone()));
    assert!(store.lock().unwrap().session("m").unwrap().is_none());

    let saved = save_notes(&mut store.lock().unwrap(), "n", "Second\n", None).unwrap();
    assert_eq!(saved, NotesSave::Saved { written: true });
    assert_eq!(
        std::fs::read_to_string(moved.join("notes.md")).unwrap(),
        "Second\n"
    );
}

/// The note "n" of [`placed_notes_only_note`] with one recorded segment:
/// a note with audio.
fn with_audio(store: &SharedStore) {
    let audio = tempfile::tempdir().unwrap();
    let segment = write_segment(audio.path(), 0, 0, 16_000, &[0.0; 16_000], false);
    store.lock().unwrap().put_segment("n", &segment).unwrap();
}

/// An agent renamed the folder of a note with audio, and the user typed
/// before the library scan found it. The save goes into the renamed
/// folder, so the scan does not replace the edit with the older notes.
#[test]
fn a_notes_save_of_a_note_with_audio_follows_a_folder_that_has_another_name() {
    use pipeline::save::{NotesRescue, NotesSave, rescue_notes, save_notes};
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let path = placed_notes_only_note(&store, dest.path());
    with_audio(&store);
    let moved = dest.path().join("Renamed");
    std::fs::rename(&path, &moved).unwrap();

    let saved = save_notes(&mut store.lock().unwrap(), "n", "Second\n", None).unwrap();
    assert_eq!(saved, NotesSave::FolderGone);
    assert_eq!(store.lock().unwrap().notes("n").unwrap(), "Second\n");
    assert_eq!(
        rescue_notes(&store, "n", "m", at(100)).unwrap(),
        NotesRescue::Moved
    );
    assert_eq!(home(&store, "n"), Some(moved.clone()));

    let saved = save_notes(&mut store.lock().unwrap(), "n", "Second\n", None).unwrap();
    assert_eq!(saved, NotesSave::Saved { written: true });
    assert_eq!(
        std::fs::read_to_string(moved.join("notes.md")).unwrap(),
        "Second\n"
    );
}

/// A folder that holds the note is found also when another note folder
/// in the storage folder cannot be read, for example a cloud file that
/// is not downloaded.
#[cfg(unix)]
#[test]
fn a_renamed_folder_is_found_when_another_note_folder_cannot_be_read() {
    use pipeline::save::{NotesRescue, NotesSave, rescue_notes, save_notes};
    use std::os::unix::fs::PermissionsExt;
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let path = placed_notes_only_note(&store, dest.path());
    with_audio(&store);
    let moved = dest.path().join("Renamed");
    std::fs::rename(&path, &moved).unwrap();
    let other = dest.path().join("Other");
    std::fs::create_dir(&other).unwrap();
    let blocked = other.join(session_core::note_folder::SESSION_FILE);
    std::fs::write(&blocked, "---\ntitle: Other\n---\n").unwrap();
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).unwrap();

    let saved = save_notes(&mut store.lock().unwrap(), "n", "Second\n", None).unwrap();
    assert_eq!(saved, NotesSave::FolderGone);
    let rescued = rescue_notes(&store, "n", "m", at(100)).unwrap();
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(rescued, NotesRescue::Moved);
    assert_eq!(home(&store, "n"), Some(moved));
}

/// The renamed folder cannot be read, and a readable copy of it has the
/// same note id. The rescue cannot tell which folder is the note, so it
/// changes nothing and does not follow the copy.
#[cfg(unix)]
#[test]
fn a_rescue_does_not_follow_a_copy_while_a_folder_of_the_note_cannot_be_read() {
    use pipeline::save::{NotesRescue, rescue_notes, save_notes};
    use std::os::unix::fs::PermissionsExt;
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let path = placed_notes_only_note(&store, dest.path());
    with_audio(&store);
    let moved = dest.path().join("Renamed");
    std::fs::rename(&path, &moved).unwrap();
    let copy = dest.path().join("Renamed copy");
    std::fs::create_dir(&copy).unwrap();
    for entry in std::fs::read_dir(&moved).unwrap().flatten() {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), copy.join(entry.file_name())).unwrap();
        }
    }
    let blocked = moved.join(session_core::note_folder::SESSION_FILE);
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).unwrap();

    save_notes(&mut store.lock().unwrap(), "n", "Second\n", None).unwrap();
    let rescued = rescue_notes(&store, "n", "m", at(100)).unwrap();
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(rescued, NotesRescue::Unavailable);
    assert_eq!(home(&store, "n"), Some(path));
}

#[test]
fn a_notes_save_while_the_storage_folder_is_not_available_makes_no_new_note() {
    use pipeline::save::{NotesRescue, rescue_notes, save_notes};
    let dest = tempfile::tempdir().unwrap();
    let storage = dest.path().join("storage");
    std::fs::create_dir(&storage).unwrap();
    let store = store();
    let path = placed_notes_only_note(&store, &storage);
    std::fs::remove_dir_all(&storage).unwrap();

    save_notes(&mut store.lock().unwrap(), "n", "Second\n", None).unwrap();
    assert_eq!(
        rescue_notes(&store, "n", "m", at(100)).unwrap(),
        NotesRescue::Unavailable
    );
    let s = store.lock().unwrap();
    assert!(s.session("m").unwrap().is_none());
    let record = s.session("n").unwrap().unwrap();
    assert_eq!(record.home_path.map(PathBuf::from), Some(path));
    // The store keeps the edit.
    assert_eq!(s.notes("n").unwrap(), "Second\n");
}

/// Move the home folder of "n" into the type folder `name` of `dest`, as
/// a person does in Finder, and record the new home path, as the library
/// scan does. Returns the new home folder.
fn into_type(store: &SharedStore, path: &Path, dest: &Path, name: &str) -> PathBuf {
    let folder = dest.join(name);
    std::fs::create_dir_all(&folder).unwrap();
    let moved = folder.join(path.file_name().unwrap());
    std::fs::rename(path, &moved).unwrap();
    store
        .lock()
        .unwrap()
        .set_home_path("n", &moved.to_string_lossy())
        .unwrap();
    moved
}

// KTD4: `locate` finds the storage folder of a note in a type folder.
#[test]
fn a_notes_save_follows_a_typed_folder_that_has_another_name() {
    use pipeline::save::{NotesRescue, NotesSave, rescue_notes, save_notes};
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let path = placed_notes_only_note(&store, dest.path());
    let typed = into_type(&store, &path, dest.path(), "Pathfinder");
    let moved = dest.path().join("Pathfinder/Renamed");
    std::fs::rename(&typed, &moved).unwrap();

    let saved = save_notes(&mut store.lock().unwrap(), "n", "Second\n", None).unwrap();
    assert_eq!(saved, NotesSave::FolderGone);
    assert_eq!(
        rescue_notes(&store, "n", "m", at(100)).unwrap(),
        NotesRescue::Moved
    );
    assert_eq!(home(&store, "n"), Some(moved));
    assert!(!dest.path().join("Pathfinder/.longhand").exists());
}

#[test]
fn a_typed_home_folder_that_another_computer_removed_is_deleted() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    let path = placed_notes_only_note(&store, dest.path());
    let typed = into_type(&store, &path, dest.path(), "Pathfinder");
    assert!(!early_home::home_deleted(&store, "n").unwrap());
    std::fs::remove_dir_all(&typed).unwrap();
    assert!(early_home::home_deleted(&store, "n").unwrap());
    assert!(!dest.path().join("Pathfinder/.longhand").exists());
}

// ---------------------------------------------------------------- note types (U7)

/// [`recording_note`] with the type `name` in its row, as a start with a
/// type makes it.
fn typed_note(store: &SharedStore, id: &str, name: &str) {
    recording_note(store, id, "");
    store.lock().unwrap().set_note_type(id, Some(name)).unwrap();
}

fn note_type(store: &SharedStore, id: &str) -> Option<String> {
    store
        .lock()
        .unwrap()
        .session(id)
        .unwrap()
        .unwrap()
        .note_type
}

// Covers F1, R4: a note of a type gets its home folder in the type folder,
// and the write makes the type folder when it is missing.
#[test]
fn a_note_of_a_type_gets_its_home_folder_in_the_type_folder() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    typed_note(&store, "s", "Pathfinder");

    let path = write(&store, "s", Some(dest.path())).unwrap().unwrap();

    assert_eq!(
        path,
        dest.path()
            .join("Pathfinder")
            .join("2027-01-15 0800 Longhand session")
    );
    assert_eq!(
        session_core::note_folder::NoteFiles::read(&path)
            .map(|files| files.doc)
            .unwrap()
            .note_id,
        "s"
    );
    assert_eq!(home(&store, "s"), Some(path.clone()));
    assert_eq!(note_type(&store, "s"), Some("Pathfinder".to_string()));
    assert_eq!(folders(dest.path()), vec!["Pathfinder"]);
    assert!(!dest.path().join(staging_name("s")).exists());
    // The library marker is in the storage folder, never in the type
    // folder.
    assert!(library::scan::marker(dest.path()).exists());
    assert!(!dest.path().join("Pathfinder/.longhand").exists());
    assert_eq!(early(&store, "s"), Some(EarlyState::Done { path }));
}

#[test]
fn a_note_of_a_type_uses_the_type_folder_that_is_there() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    std::fs::create_dir_all(
        dest.path()
            .join("Pathfinder/2027-01-15 0800 Longhand session"),
    )
    .unwrap();
    typed_note(&store, "s", "Pathfinder");

    let path = write(&store, "s", Some(dest.path())).unwrap().unwrap();

    assert_eq!(
        path,
        dest.path()
            .join("Pathfinder")
            .join("2027-01-15 0800 Longhand session-2")
    );
}

// The crash recovery searches the type folder (KTD4).
#[test]
fn a_crash_after_the_rename_into_a_type_folder_finds_the_folder_there() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    typed_note(&store, "s", "Pathfinder");
    let filled = step(&store, "s", dest.path());
    assert!(matches!(filled, EarlyState::Filled { .. }), "{filled:?}");
    // The rename happened; its record was lost.
    let placed = early_home::step(&store, "s", &filled, Some(dest.path()), RUN, &RealFs).unwrap();
    let EarlyState::Done { path } = &placed else {
        panic!("{placed:?}");
    };
    assert_eq!(
        path.parent(),
        Some(dest.path().join("Pathfinder").as_path())
    );
    assert_eq!(home(&store, "s"), None);

    let again = early_home::step(&store, "s", &filled, Some(dest.path()), RUN, &RealFs).unwrap();
    assert_eq!(again, placed);
    let written = write(&store, "s", Some(dest.path())).unwrap().unwrap();

    assert_eq!(&written, path);
    assert_eq!(folders(&dest.path().join("Pathfinder")).len(), 1);
}

// Covers R5 before the folder exists: a type change while the storage
// folder is not available changes only the row, and the write puts the
// home folder in the folder of the new type.
#[test]
fn a_type_change_while_the_write_waits_puts_the_folder_in_the_new_type_folder() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    typed_note(&store, "s", "Pathfinder");
    let gone = dest.path().join("storage");
    assert!(write(&store, "s", Some(&gone)).is_err());

    store
        .lock()
        .unwrap()
        .set_note_type("s", Some("One on One"))
        .unwrap();
    std::fs::create_dir(&gone).unwrap();
    let path = write(&store, "s", Some(&gone)).unwrap().unwrap();

    assert_eq!(path.parent(), Some(gone.join("One on One").as_path()));
    assert_eq!(folders(&gone), vec!["One on One"]);
    assert_eq!(note_type(&store, "s"), Some("One on One".to_string()));
}

// A type change after the staging step and before the rename: the folder
// goes to the type of the staging step, and the change becomes a type move
// that this computer owes (KTD5), so the change is not lost.
#[test]
fn a_type_change_during_the_rename_becomes_an_owed_type_move() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    typed_note(&store, "s", "Pathfinder");
    step(&store, "s", dest.path());
    store
        .lock()
        .unwrap()
        .set_note_type("s", Some("One on One"))
        .unwrap();

    let path = write(&store, "s", Some(dest.path())).unwrap().unwrap();

    assert_eq!(
        path.parent(),
        Some(dest.path().join("Pathfinder").as_path())
    );
    let s = store.lock().unwrap();
    let owed = s.type_move("s").unwrap().expect("a type move");
    assert_eq!(owed.to.as_deref(), Some("One on One"));
    assert_eq!(owed.from.as_deref(), Some("Pathfinder"));
    let record = s.session("s").unwrap().unwrap();
    assert_eq!(record.note_type.as_deref(), Some("Pathfinder"));
    // The app shows the new type while the move waits (KTD3).
    assert_eq!(
        pipeline::type_move::shown_type(&record),
        (Some("One on One".to_string()), true)
    );
}

// KTD8: a type name that is not valid never becomes a path. The note gets
// no type.
#[test]
fn a_row_type_that_is_not_a_valid_name_puts_the_folder_in_the_storage_folder() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    typed_note(&store, "s", "../outside");

    let path = write(&store, "s", Some(dest.path())).unwrap().unwrap();

    assert_eq!(path.parent(), Some(dest.path()));
    assert_eq!(note_type(&store, "s"), None);
    assert_eq!(
        folders(dest.path()),
        vec!["2027-01-15 0800 Longhand session"]
    );
}

// A file with the name of the type folder: the folder goes in the storage
// folder, and the note has no type. A retry would not fix it.
#[test]
fn a_file_with_the_type_name_puts_the_folder_in_the_storage_folder() {
    let dest = tempfile::tempdir().unwrap();
    let store = store();
    std::fs::write(dest.path().join("Pathfinder"), b"").unwrap();
    typed_note(&store, "s", "Pathfinder");

    let path = write(&store, "s", Some(dest.path())).unwrap().unwrap();

    assert_eq!(path.parent(), Some(dest.path()));
    assert_eq!(note_type(&store, "s"), None);
    assert!(store.lock().unwrap().type_move("s").unwrap().is_none());
}

// A Filled state from a build before note types has no type: the folder
// goes in the storage folder.
#[test]
fn a_filled_state_with_no_type_reads_as_the_storage_folder() {
    let state: EarlyState = serde_json::from_value(
        serde_json::json!({"step": "filled", "root": "/r", "folder_name": "n"}),
    )
    .unwrap();
    assert_eq!(
        state,
        EarlyState::Filled {
            root: PathBuf::from("/r"),
            folder_name: "n".into(),
            note_type: None,
        }
    );
}

// ------------------------------------------- a change of storage folder

/// A note that records, with its early home folder in the storage folder `old`.
fn placed_recording_note(store: &SharedStore, id: &str, old: &Path) -> PathBuf {
    recording_note(store, id, "Agenda\n");
    write(store, id, Some(old)).unwrap().unwrap()
}

fn old_home_key(store: &SharedStore, id: &str) -> Option<serde_json::Value> {
    store
        .lock()
        .unwrap()
        .result_state(id, early_home::OLD_HOMES)
        .unwrap()
}

#[test]
fn leave_them_puts_the_folder_of_a_note_that_is_not_saved_in_the_new_storage_folder() {
    let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let store = store();
    let first = placed_recording_note(&store, "s", old.path());

    let rehomed = early_home::rehome_outside(&store, new.path()).unwrap();
    assert_eq!(rehomed, vec!["s".to_string()]);
    // Until the new folder is written, the save waits for it, and the store
    // keeps the row.
    assert_eq!(early(&store, "s"), Some(EarlyState::Begun));
    assert!(early_home::unfinished(&store.lock().unwrap(), "s").unwrap());

    let path = write(&store, "s", Some(new.path())).unwrap().unwrap();

    assert!(path.starts_with(new.path()), "{}", path.display());
    assert_eq!(home(&store, "s"), Some(path.clone()));
    assert_eq!(
        std::fs::read_to_string(path.join("notes.md")).unwrap(),
        "Agenda\n"
    );
    assert!(path.join("session.md").is_file());
    let doc = session_core::note_folder::NoteFiles::read(&path)
        .map(|files| files.doc)
        .unwrap();
    assert_eq!(doc.note_id, "s");
    assert_eq!(doc.result, Some(ResultFlag::NotSaved));
    // The early home folder in the old storage folder is gone, and the row no
    // longer records it.
    assert!(!first.exists(), "{}", first.display());
    assert!(folders(old.path()).is_empty());
    assert_eq!(old_home_key(&store, "s"), None);
}

#[test]
fn the_old_folder_goes_also_after_a_notes_edit_while_the_new_write_waits() {
    let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let store = store();
    let first = placed_recording_note(&store, "s", old.path());
    early_home::rehome_outside(&store, new.path()).unwrap();

    {
        let mut s = store.lock().unwrap();
        pipeline::save::set_notes(&mut s, "s", "edited\n").unwrap();
    }
    let path = write(&store, "s", Some(new.path())).unwrap().unwrap();

    assert_eq!(
        std::fs::read_to_string(path.join("notes.md")).unwrap(),
        "edited\n"
    );
    assert!(!first.exists(), "{}", first.display());
    assert_eq!(old_home_key(&store, "s"), None);
}

#[test]
fn an_old_folder_that_another_computer_changed_stays() {
    let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let store = store();
    let first = placed_recording_note(&store, "s", old.path());
    early_home::rehome_outside(&store, new.path()).unwrap();
    std::fs::write(first.join("notes.md"), "from another computer\n").unwrap();

    let path = write(&store, "s", Some(new.path())).unwrap().unwrap();

    assert!(path.starts_with(new.path()));
    assert_eq!(
        std::fs::read_to_string(first.join("notes.md")).unwrap(),
        "from another computer\n"
    );
    assert_eq!(old_home_key(&store, "s"), None);
}

#[test]
fn a_note_that_was_saved_before_is_not_rehomed() {
    let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let store = store();
    let first = placed_recording_note(&store, "s", old.path());
    // A Resume or a Transcribe Again of a saved note: its folder has a
    // result, so it has no "not saved" flag.
    let mut doc = session_core::note_folder::NoteFiles::read(&first)
        .map(|files| files.doc)
        .unwrap();
    doc.result = None;
    session_save::note_files::write_doc(&first, &doc).unwrap();

    let rehomed = early_home::rehome_outside(&store, new.path()).unwrap();

    assert!(rehomed.is_empty());
    assert_eq!(home(&store, "s"), Some(first.clone()));
    assert_eq!(
        early_home::state(&store.lock().unwrap(), "s").unwrap(),
        Some(EarlyState::Done { path: first })
    );
    assert_eq!(old_home_key(&store, "s"), None);
}

#[test]
fn a_note_whose_folder_is_written_or_whose_save_runs_makes_the_change_wait() {
    let old = tempfile::tempdir().unwrap();
    let store = store();
    let saving = |store: &SharedStore| {
        early_home::saving_into(store, old.path(), &|id| id == "filled").unwrap()
    };
    // The early write is between its staging folder and its rename.
    recording_note(&store, "filled", "");
    step(&store, "filled", old.path());
    assert!(saving(&store));
    write(&store, "filled", Some(old.path())).unwrap();
    assert!(
        !saving(&store),
        "a done early write does not stop the change"
    );

    // The first save began to publish into the early home folder, and it
    // runs now.
    store
        .lock()
        .unwrap()
        .put_checkpoint(
            "filled",
            session_core::Stage::Save,
            pipeline::save::STATE,
            &serde_json::to_string(&session_save::SaveState::Working).unwrap(),
        )
        .unwrap();
    assert!(saving(&store));
    // A begun first Save that does not run now does not stop the change.
    assert!(!early_home::saving_into(&store, old.path(), &|_| false).unwrap());
    // A note in another storage folder does not stop the change.
    let other = tempfile::tempdir().unwrap();
    assert!(!early_home::saving_into(&store, other.path(), &|_| true).unwrap());
}

#[test]
fn a_filled_early_write_whose_folder_is_not_available_does_not_make_the_change_wait() {
    let parent = tempfile::tempdir().unwrap();
    let old = parent.path().join("Old");
    std::fs::create_dir(&old).unwrap();
    let store = store();
    recording_note(&store, "s", "");
    step(&store, "s", &old);
    assert!(early_home::saving_into(&store, &old, &|_| false).unwrap());
    // The volume of the storage folder goes away.
    std::fs::rename(&old, parent.path().join("Away")).unwrap();
    assert!(!early_home::saving_into(&store, &old, &|_| false).unwrap());
}

#[test]
fn the_launch_finishes_the_removal_of_the_old_folder_after_a_stop() {
    let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let store = store();
    let first = placed_recording_note(&store, "s", old.path());
    early_home::rehome_outside(&store, new.path()).unwrap();
    // The app stops after the new folder is recorded and before the old one
    // goes.
    step(&store, "s", new.path());
    let done = step(&store, "s", new.path());
    assert!(matches!(&done, EarlyState::Done { path } if path.starts_with(new.path())));
    assert!(first.exists());
    assert!(old_home_key(&store, "s").is_some());
    assert_eq!(
        early_home::waiting(&store.lock().unwrap()).unwrap(),
        vec!["s".to_string()]
    );

    let target = new.path().to_path_buf();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let homes = EarlyHomes::new(
        Arc::clone(&store),
        RUN,
        Box::new(move || Some(target.clone())),
        Box::new(move |id| done_tx.send(id.to_string()).unwrap()),
    );
    homes.resume();
    assert_eq!(done_rx.recv().unwrap(), "s");
    homes.wait("s");

    assert!(!first.exists(), "{}", first.display());
    assert_eq!(old_home_key(&store, "s"), None);
    assert_eq!(folders(new.path()).len(), 1);
}

/// Record `state` as the early write state of `id`, as a stop between two
/// records leaves it.
fn set_early(store: &SharedStore, id: &str, state: &EarlyState) {
    store
        .lock()
        .unwrap()
        .set_result_state(id, early_home::STATE, &serde_json::to_value(state).unwrap())
        .unwrap();
}

/// The jobs of a launch whose storage folder is `dest`. Returns the jobs
/// and the receiver of each run that succeeded.
// The receiver gets the id after each job run that succeeded. Wait for
// it with `recv`, not with a time limit: the jobs of the other tests take
// the same process-wide note guard of "s", so under load a run can start
// many seconds late.
fn launch_jobs(
    store: &SharedStore,
    dest: &Path,
) -> (Arc<EarlyHomes>, std::sync::mpsc::Receiver<String>) {
    let target = dest.to_path_buf();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let homes = EarlyHomes::new(
        Arc::clone(store),
        RUN,
        Box::new(move || Some(target.clone())),
        Box::new(move |id| done_tx.send(id.to_string()).unwrap()),
    );
    (homes, done_rx)
}

#[test]
fn a_stop_between_the_two_records_of_the_rehome_puts_the_note_in_the_new_folder_at_launch() {
    let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let store = store();
    let first = placed_recording_note(&store, "s", old.path());
    early_home::rehome_outside(&store, new.path()).unwrap();
    // The app stopped after the record of the old home folder and before
    // the Begun record.
    set_early(
        &store,
        "s",
        &EarlyState::Done {
            path: first.clone(),
        },
    );

    // The launch applies the rule again, before the jobs continue.
    assert_eq!(
        early_home::rehome_outside(&store, new.path()).unwrap(),
        vec!["s".to_string()]
    );
    // The folder has one entry.
    assert_eq!(
        old_home_key(&store, "s").unwrap().as_array().map(Vec::len),
        Some(1)
    );
    let (homes, done) = launch_jobs(&store, new.path());
    homes.resume();
    assert_eq!(done.recv().unwrap(), "s");
    homes.wait("s");

    let path = home(&store, "s").unwrap();
    assert!(path.starts_with(new.path()), "{}", path.display());
    assert!(!first.exists(), "{}", first.display());
    assert_eq!(old_home_key(&store, "s"), None);
}

#[test]
fn a_stop_after_the_settings_change_and_before_any_rehome_is_completed_at_launch() {
    let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let store = store();
    let first = placed_recording_note(&store, "s", old.path());
    // The settings hold the new storage folder, and the app stopped before
    // the rehome: the store has no record of the change.
    assert_eq!(old_home_key(&store, "s"), None);
    assert!(
        early_home::waiting(&store.lock().unwrap())
            .unwrap()
            .is_empty()
    );

    let rehomed = early_home::rehome_outside(&store, new.path()).unwrap();
    assert_eq!(rehomed, vec!["s".to_string()]);
    let (homes, done) = launch_jobs(&store, new.path());
    homes.resume();
    assert_eq!(done.recv().unwrap(), "s");
    homes.wait("s");

    let path = home(&store, "s").unwrap();
    assert!(path.starts_with(new.path()), "{}", path.display());
    assert!(!first.exists(), "{}", first.display());
    assert_eq!(old_home_key(&store, "s"), None);
    // The rule does not apply again to a note in the storage folder.
    assert!(
        early_home::rehome_outside(&store, new.path())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_folder_that_cannot_be_read_at_the_change_is_rehomed_at_a_later_try() {
    let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let store = store();
    let first = placed_recording_note(&store, "s", old.path());
    // The `longhand.json` of the folder cannot be read, as a file that does
    // not download yet.
    let record = first.join("longhand.json");
    let text = std::fs::read(&record).unwrap();
    std::fs::remove_file(&record).unwrap();
    std::fs::create_dir(&record).unwrap();

    assert!(!early_home::rehome(&store, "s", new.path()).unwrap());
    assert!(
        early_home::rehome_outside(&store, new.path())
            .unwrap()
            .is_empty()
    );
    assert_eq!(home(&store, "s"), Some(first.clone()));

    // The file is back: the next try (a retry of the notes that wait for
    // a folder) gives the note its new early write.
    std::fs::remove_dir(&record).unwrap();
    std::fs::write(&record, text).unwrap();
    assert_eq!(
        early_home::rehome_outside(&store, new.path()).unwrap(),
        vec!["s".to_string()]
    );
    let path = write(&store, "s", Some(new.path())).unwrap().unwrap();
    assert!(path.starts_with(new.path()), "{}", path.display());
    assert!(!first.exists(), "{}", first.display());
}

#[test]
fn the_rule_does_not_apply_while_a_move_is_open() {
    let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let store = store();
    let first = placed_recording_note(&store, "s", old.path());
    pipeline::move_notes::open(&store, old.path(), new.path()).unwrap();

    assert!(
        early_home::rehome_outside(&store, new.path())
            .unwrap()
            .is_empty()
    );
    assert_eq!(write(&store, "s", Some(new.path())).unwrap(), Some(first));
    assert_eq!(old_home_key(&store, "s"), None);
}

#[test]
fn an_early_write_that_crosses_the_change_ends_in_the_new_storage_folder() {
    let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let store = store();
    recording_note(&store, "s", "Agenda\n");
    // The staging folder is complete in the old storage folder, and then
    // the storage folder changes.
    step(&store, "s", old.path());

    let path = write(&store, "s", Some(new.path())).unwrap().unwrap();

    assert!(path.starts_with(new.path()), "{}", path.display());
    assert!(folders(old.path()).is_empty(), "{:?}", folders(old.path()));
    assert_eq!(
        std::fs::read_to_string(path.join("notes.md")).unwrap(),
        "Agenda\n"
    );
}

#[test]
fn a_second_change_keeps_the_old_home_folders_of_both_changes() {
    let (a, b, c) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let store = store();
    let first = placed_recording_note(&store, "s", a.path());
    early_home::rehome_outside(&store, b.path()).unwrap();
    // A fresh in-use marker of another computer keeps the folder in A for
    // now.
    pipeline::in_use::write_marker(
        &first,
        "other-computer",
        chrono::Local::now().fixed_offset(),
    )
    .unwrap();
    let second = write(&store, "s", Some(b.path())).unwrap().unwrap();
    assert!(first.exists());
    early_home::rehome_outside(&store, c.path()).unwrap();
    // The other computer is done with the folder.
    std::fs::remove_file(first.join(marker_name("other-computer"))).unwrap();

    let third = write(&store, "s", Some(c.path())).unwrap().unwrap();

    assert!(third.starts_with(c.path()), "{}", third.display());
    assert!(!second.exists(), "{}", second.display());
    assert!(!first.exists(), "{}", first.display());
    assert_eq!(old_home_key(&store, "s"), None);
}

#[test]
fn a_fresh_marker_of_another_run_keeps_the_old_home_folder_for_a_later_run() {
    let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let store = store();
    let first = placed_recording_note(&store, "s", old.path());
    early_home::rehome_outside(&store, new.path()).unwrap();
    pipeline::in_use::write_marker(
        &first,
        "other-computer",
        chrono::Local::now().fixed_offset(),
    )
    .unwrap();

    write(&store, "s", Some(new.path())).unwrap().unwrap();

    assert!(first.exists());
    assert!(old_home_key(&store, "s").is_some());
    // The marker goes: a later run removes the folder.
    std::fs::remove_file(first.join(marker_name("other-computer"))).unwrap();
    let (homes, done) = launch_jobs(&store, new.path());
    homes.resume();
    assert_eq!(done.recv().unwrap(), "s");
    homes.wait("s");
    assert!(!first.exists(), "{}", first.display());
    assert_eq!(old_home_key(&store, "s"), None);
}

#[test]
fn an_old_storage_folder_that_is_not_available_keeps_the_entry_and_a_later_run_removes_it() {
    let parent = tempfile::tempdir().unwrap();
    let old = parent.path().join("Old");
    let away = parent.path().join("Away");
    std::fs::create_dir(&old).unwrap();
    let new = tempfile::tempdir().unwrap();
    let store = store();
    let first = placed_recording_note(&store, "s", &old);
    early_home::rehome_outside(&store, new.path()).unwrap();
    // The volume of the old storage folder goes away.
    std::fs::rename(&old, &away).unwrap();

    write(&store, "s", Some(new.path())).unwrap().unwrap();

    assert!(old_home_key(&store, "s").is_some());
    std::fs::rename(&away, &old).unwrap();
    assert!(first.exists());
    let (homes, done) = launch_jobs(&store, new.path());
    homes.resume();
    assert_eq!(done.recv().unwrap(), "s");
    homes.wait("s");
    assert!(!first.exists(), "{}", first.display());
    assert_eq!(old_home_key(&store, "s"), None);
}

#[test]
fn a_title_edit_while_the_note_is_rehomed_writes_nothing_into_the_old_home_folder() {
    let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let store = store();
    let first = placed_recording_note(&store, "s", old.path());
    early_home::rehome_outside(&store, new.path()).unwrap();
    let before = files_written(&first);

    pipeline::folder_rename::rename_session(&store, "s", "Budget", &|_| false, &RealFs).unwrap();
    pipeline::tags::set_note_tags(
        &store,
        &session_core::jobs::Claims::default(),
        "s",
        &["Person::Ann".to_string()],
        &[],
    )
    .unwrap();

    assert_eq!(files_written(&first), before);
    let path = write(&store, "s", Some(new.path())).unwrap().unwrap();
    assert!(!first.exists(), "{}", first.display());
    let doc = session_core::note_folder::NoteFiles::read(&path)
        .map(|files| files.doc)
        .unwrap();
    assert_eq!(doc.title.as_deref(), Some("Budget"));
    assert_eq!(doc.tags, vec!["Person::Ann".to_string()]);
}

#[test]
fn a_title_edit_after_the_new_write_began_is_in_the_new_home_folder() {
    let (old, new) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let store = store();
    let first = placed_recording_note(&store, "s", old.path());
    early_home::rehome_outside(&store, new.path()).unwrap();
    // The staging folder in the new storage folder has the files, and then
    // the user changes the title.
    step(&store, "s", new.path());
    pipeline::folder_rename::rename_session(&store, "s", "Budget", &|_| false, &RealFs).unwrap();

    let path = write(&store, "s", Some(new.path())).unwrap().unwrap();

    assert!(!first.exists(), "{}", first.display());
    let doc = session_core::note_folder::NoteFiles::read(&path)
        .map(|files| files.doc)
        .unwrap();
    assert_eq!(doc.title.as_deref(), Some("Budget"));
}

#[test]
fn a_filled_write_in_an_old_storage_folder_that_is_not_available_is_written_again() {
    let parent = tempfile::tempdir().unwrap();
    let old = parent.path().join("Old");
    let away = parent.path().join("Away");
    std::fs::create_dir(&old).unwrap();
    let new = tempfile::tempdir().unwrap();
    let store = store();
    recording_note(&store, "s", "Agenda\n");
    // The staging folder is complete in the old storage folder, then the
    // storage folder changes and the volume of the old one goes away.
    step(&store, "s", &old);
    std::fs::rename(&old, &away).unwrap();

    let path = write(&store, "s", Some(new.path())).unwrap().unwrap();

    assert!(path.starts_with(new.path()), "{}", path.display());
    assert_eq!(
        std::fs::read_to_string(path.join("notes.md")).unwrap(),
        "Agenda\n"
    );
    // The volume is back: a later run removes the staging folder there.
    std::fs::rename(&away, &old).unwrap();
    assert!(old.join(staging_name("s")).exists());
    let (homes, done) = launch_jobs(&store, new.path());
    homes.resume();
    assert_eq!(done.recv().unwrap(), "s");
    homes.wait("s");
    assert!(!old.join(staging_name("s")).exists());
    assert_eq!(old_home_key(&store, "s"), None);
}
