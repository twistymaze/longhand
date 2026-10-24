//! The Move (U8, KTD9, KTD10): every note of the old storage folder goes to
//! the new one, one step at a time, and a lost record at any step gives one
//! complete copy and one listing. Notes are saved through the real save
//! stage. A rename that crosses volumes is simulated.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, FixedOffset};
use library::reconcile::{Reconciler, Target};
use library::scan::{Known, ScanOptions};
use pipeline::move_notes::{self, MARKER, MoveError, Mover, PassEnd, STAGING_PREFIX, Taken};
use pipeline::{SharedStore, StageError};
use session_core::jobs::Claims;
use session_core::{Event, MoveJournal, MovePause, MoveStep, Segment, SessionState, Stage, Store};
use session_save::{Fs, RealFs};

fn started() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-09-25T09:00:00-05:00").unwrap()
}

/// An old and a new storage folder, and the store of one computer.
struct Folders {
    _dir: tempfile::TempDir,
    old: PathBuf,
    new: PathBuf,
    working: PathBuf,
    store: SharedStore,
    claims: Claims,
}

fn folders() -> Folders {
    let dir = tempfile::tempdir().unwrap();
    let (old, new, working) = (
        dir.path().join("old"),
        dir.path().join("new"),
        dir.path().join("working"),
    );
    for path in [&old, &new, &working] {
        std::fs::create_dir_all(path).unwrap();
    }
    assert!(library::scan::write_marker(&old));
    Folders {
        _dir: dir,
        old,
        new,
        working,
        store: Arc::new(Mutex::new(Store::open_in_memory().unwrap())),
        claims: Claims::default(),
    }
}

/// Renames that cross devices between the old and the new folder, when
/// `cross` is set.
struct Volumes {
    old: PathBuf,
    new: PathBuf,
    cross: bool,
}

impl Volumes {
    fn of(f: &Folders, cross: bool) -> Self {
        Self {
            old: f.old.clone(),
            new: f.new.clone(),
            cross,
        }
    }

    fn crosses(&self, from: &Path, to: &Path) -> bool {
        self.cross
            && (from.starts_with(&self.old) && to.starts_with(&self.new)
                || from.starts_with(&self.new) && to.starts_with(&self.old))
    }
}

impl Fs for Volumes {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        if self.crosses(from, to) {
            return Err(io::Error::from(io::ErrorKind::CrossesDevices));
        }
        RealFs.rename(from, to)
    }
    fn rename_new(&self, from: &Path, to: &Path) -> io::Result<()> {
        if self.crosses(from, to) {
            return Err(io::Error::from(io::ErrorKind::CrossesDevices));
        }
        RealFs.rename_new(from, to)
    }
}

/// A save whose final rename fails: the save stops with its staged copy.
struct StopsBeforeFinalRename;

impl Fs for StopsBeforeFinalRename {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        RealFs.rename(from, to)
    }
    fn rename_new(&self, _: &Path, _: &Path) -> io::Result<()> {
        Err(io::Error::other("stopped before the final rename"))
    }
}

fn segment() -> Segment {
    Segment {
        index: 0,
        offset_ms: 0,
        duration_ms: 60_000,
        wall_start: started(),
        wall_end: started() + chrono::TimeDelta::minutes(1),
        audio_file: "audio/segment-01.opus".into(),
        channels: 1,
        ends_in_pause: false,
    }
}

impl Folders {
    /// Record and process the note `id` with the title `title`, then save
    /// it through the real save stage into `root`.
    fn save_with(
        &self,
        id: &str,
        title: &str,
        root: &Path,
        fs: &dyn Fs,
    ) -> Result<PathBuf, StageError> {
        let segment = segment();
        let working = self.working.join(id);
        if self.store.lock().unwrap().session(id).unwrap().is_none() {
            let mut s = self.store.lock().unwrap();
            s.create_session(id, started(), None).unwrap();
            s.put_segment(id, &segment).unwrap();
            s.set_asr_model(id, "m").unwrap();
            s.set_notes(id, "first\n").unwrap();
            s.set_title(id, title, session_core::TitleSource::User)
                .unwrap();
            s.apply(
                id,
                &Event::UserStop {
                    at: segment.wall_end,
                    retained_audio: true,
                },
            )
            .unwrap();
            s.dequeue_session(id, "test", segment.wall_end).unwrap();
            drop(s);
            std::fs::create_dir_all(working.join("audio")).unwrap();
            std::fs::write(working.join(&segment.audio_file), vec![7u8; 50_000]).unwrap();
            std::fs::write(
                working.join("transcript.md"),
                "# Transcript\n\n- [00:00:01] Speaker 1: We start.\n",
            )
            .unwrap();
            pipeline::checkpoint::mark_complete(&self.store, id, Stage::Verify, 1).unwrap();
        }
        pipeline::save::run(
            &self.store,
            id,
            &working,
            &[segment],
            Some(root),
            fs,
            &pipeline::Unattended,
        )
        .map(|saved| saved.path)
    }

    fn save_note(&self, id: &str, title: &str) -> PathBuf {
        let path = self.save_with(id, title, &self.old, &RealFs).unwrap();
        assert_eq!(self.state(id), SessionState::Saved);
        path
    }

    fn state(&self, id: &str) -> SessionState {
        self.store
            .lock()
            .unwrap()
            .session(id)
            .unwrap()
            .unwrap()
            .state
    }

    fn home(&self, id: &str) -> PathBuf {
        PathBuf::from(
            self.store
                .lock()
                .unwrap()
                .session(id)
                .unwrap()
                .unwrap()
                .home_path
                .unwrap(),
        )
    }

    fn step(&self, id: &str) -> Option<MoveStep> {
        self.store.lock().unwrap().move_step(id).unwrap()
    }

    fn open(&self) {
        assert!(move_notes::open(&self.store, &self.old, &self.new).unwrap());
    }

    fn roots(&self) -> Vec<PathBuf> {
        move_notes::library_roots(&self.store.lock().unwrap(), Some(self.new.clone()))
    }

    /// How many times the library lists the note `id`: a scan of the
    /// library roots gives one entry for each note id.
    fn listed(&self, id: &str) -> usize {
        let known = Known::from_records(&self.store.lock().unwrap().sessions().unwrap());
        let scan = library::scan::scan(&self.roots(), &known, &ScanOptions::default());
        scan.notes
            .iter()
            .filter(|note| note.doc.note_id == id)
            .count()
    }

    /// Reconcile the store with a scan of the library roots, as the
    /// watcher does.
    fn reconcile(&self) -> library::reconcile::Changes {
        Reconciler::default()
            .run(&self.store, &Target::Full(self.roots()), &|_| false)
            .unwrap()
    }
}

fn mover<'a>(f: &'a Folders, fs: &'a dyn Fs) -> Mover<'a> {
    Mover {
        store: &f.store,
        claims: &f.claims,
        fs,
        copy: &move_notes::copy_note,
    }
}

fn pass(m: &Mover) -> PassEnd {
    m.pass(&mut |_, _| {}).unwrap()
}

/// Run passes until the Move is finished.
fn finish(m: &Mover) {
    for _ in 0..10 {
        match pass(m) {
            PassEnd::Finished => return,
            PassEnd::Waiting => continue,
            PassEnd::Paused(pause) => panic!("the move paused: {pause:?}"),
        }
    }
    panic!("the move did not finish");
}

fn journal(m: &Mover) -> MoveJournal {
    m.prepare().unwrap().unwrap()
}

/// Take and record steps of `id` until its step is one that `stop`
/// accepts. Returns that step.
fn drive(
    f: &Folders,
    m: &Mover,
    j: &MoveJournal,
    id: &str,
    stop: &dyn Fn(&MoveStep) -> bool,
) -> MoveStep {
    for _ in 0..10 {
        let step = f.step(id).unwrap();
        if stop(&step) {
            return step;
        }
        let Taken::Next(next) = m.step(j, id, &step).unwrap() else {
            panic!("{id} did not take its step from {step:?}");
        };
        f.store.lock().unwrap().set_move_step(id, &next).unwrap();
    }
    panic!("{id} did not reach the step");
}

/// The folders in `root` that hold the note `id`.
fn folders_of(root: &Path, id: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|path| {
            session_core::note_folder::NoteFiles::read(path)
                .is_ok_and(|files| files.doc.note_id == id)
        })
        .collect();
    found.sort();
    found
}

fn staging_left(root: &Path) -> Vec<String> {
    std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(STAGING_PREFIX))
        .collect()
}

/// The note `id` has one complete copy, in the new folder, and the library
/// lists it one time.
fn assert_moved(f: &Folders, id: &str) -> PathBuf {
    let home = f.home(id);
    assert_eq!(home.parent(), Some(f.new.as_path()), "{id}");
    assert_eq!(folders_of(&f.new, id), vec![home.clone()], "{id}");
    assert!(folders_of(&f.old, id).is_empty(), "{id}");
    for file in [
        "session.md",
        "transcript.md",
        "notes.md",
        "audio/segment-01.opus",
    ] {
        assert!(home.join(file).is_file(), "{id}: {file}");
    }
    assert_eq!(
        std::fs::read(home.join("audio/segment-01.opus")).unwrap(),
        vec![7u8; 50_000]
    );
    assert!(!home.join(MARKER).exists(), "{id}");
    assert!(staging_left(&f.new).is_empty());
    assert_eq!(f.listed(id), 1, "{id}");
    let changes = f.reconcile();
    assert!(changes.removed.is_empty(), "{changes:?}");
    assert_eq!(f.home(id), home);
    home
}

fn is_pending(step: &MoveStep) -> bool {
    matches!(step, MoveStep::Pending)
}
fn is_copied(step: &MoveStep) -> bool {
    matches!(step, MoveStep::Copied { .. })
}
fn is_placed(step: &MoveStep) -> bool {
    matches!(step, MoveStep::Placed { .. })
}
fn is_homed(step: &MoveStep) -> bool {
    matches!(step, MoveStep::Homed { .. })
}

/// A crash after the step from the first step that `before` accepts, and
/// before its record: the step runs again from the same record, gives the
/// same result, and the Move then finishes with one copy.
fn lost_record(cross: bool, before: fn(&MoveStep) -> bool, label: &str) {
    let f = folders();
    let fs = Volumes::of(&f, cross);
    f.save_note("a", "Budget");
    f.open();
    let m = mover(&f, &fs);
    let j = journal(&m);
    assert_eq!(j.cross_volume, Some(cross));
    let recorded = drive(&f, &m, &j, "a", &|s| before(s));
    let after = m.step(&j, "a", &recorded).unwrap();
    // The note is listed one time while its record is lost.
    assert_eq!(f.listed("a"), 1, "{label}");
    let again = m.step(&j, "a", &recorded).unwrap();
    assert_eq!(again, after, "{label}, cross-volume: {cross}");
    let Taken::Next(next) = again else {
        panic!("{label}: {again:?}");
    };
    f.store.lock().unwrap().set_move_step("a", &next).unwrap();
    finish(&m);
    assert_moved(&f, "a");
}

#[test]
fn a_lost_record_after_each_step_on_one_volume_gives_one_copy() {
    lost_record(false, is_pending, "the rename");
    lost_record(false, is_placed, "the home change");
    lost_record(false, is_homed, "the source removal");
}

#[test]
fn a_lost_record_after_each_step_across_volumes_gives_one_copy() {
    lost_record(true, is_pending, "the copy");
    lost_record(true, is_copied, "the rename into place");
    lost_record(true, is_placed, "the home change");
    lost_record(true, is_homed, "the source removal");
}

#[test]
fn a_move_on_one_volume_renames_each_note_and_across_volumes_copies_it() {
    for cross in [false, true] {
        let f = folders();
        let fs = Volumes::of(&f, cross);
        let a = f.save_note("a", "Budget");
        f.save_note("b", "Planning");
        f.open();
        let m = mover(&f, &fs);
        let mut seen = Vec::new();
        assert_eq!(m.pass(&mut |_, p| seen.push(p)).unwrap(), PassEnd::Finished);
        assert_eq!(seen.last().map(|p| (p.moved, p.total)), Some((2, 2)));
        assert_eq!(assert_moved(&f, "a").file_name(), a.file_name());
        assert_moved(&f, "b");
        assert!(f.store.lock().unwrap().move_journal().unwrap().is_none());
    }
}

/// A note with no audio is a Saved row with zero segments (KTD2 of the
/// notes without a recording plan): the Move moves it like any saved note.
#[test]
fn a_move_moves_a_note_with_no_audio_like_any_saved_note() {
    for cross in [false, true] {
        let f = folders();
        let fs = Volumes::of(&f, cross);
        f.store
            .lock()
            .unwrap()
            .create_notes_only("n", started(), None)
            .unwrap();
        f.store.lock().unwrap().set_notes("n", "Agenda\n").unwrap();
        let old = pipeline::early_home::write(&f.store, "n", Some(&f.old), "run", &RealFs)
            .unwrap()
            .unwrap();
        f.open();
        finish(&mover(&f, &fs));

        let home = f.home("n");
        assert_eq!(home.parent(), Some(f.new.as_path()));
        assert_eq!(home.file_name(), old.file_name());
        assert_eq!(folders_of(&f.new, "n"), vec![home.clone()]);
        assert!(folders_of(&f.old, "n").is_empty());
        assert_eq!(
            std::fs::read_to_string(home.join("notes.md")).unwrap(),
            "Agenda\n"
        );
        assert!(staging_left(&f.new).is_empty());
        assert_eq!(f.listed("n"), 1);
        assert_eq!(f.state("n"), SessionState::Saved);
        let changes = f.reconcile();
        assert!(changes.removed.is_empty(), "{changes:?}");
        assert_eq!(f.home("n"), home);
    }
}

/// While a note is in both folders, the library does not show its other
/// folder as another version of the note (R6). After the Move, a second
/// folder with the note's id shows again.
#[test]
fn a_note_that_the_move_placed_shows_no_other_version() {
    // Across volumes, the copy is placed before the source goes.
    {
        let f = folders();
        let fs = Volumes::of(&f, true);
        f.save_note("a", "Budget");
        f.open();
        let m = mover(&f, &fs);
        let j = journal(&m);
        let MoveStep::Placed { path, .. } = drive(&f, &m, &j, "a", &|s| is_placed(s)) else {
            unreachable!();
        };
        assert!(path.join("session.md").is_file());
        assert!(f.home("a").join("session.md").is_file(), "in both folders");
        let status = Arc::new(library::status::LibraryStatus::default());
        let mut reconciler = Reconciler::default().with_status(Arc::clone(&status));
        reconciler
            .run(&f.store, &Target::Full(f.roots()), &|_| false)
            .unwrap();
        assert_eq!(status.note("a").other_versions, Vec::<PathBuf>::new());

        finish(&m);
        let moved = assert_moved(&f, "a");
        let copy = f.new.join("Budget copy");
        copy_folder(&moved, &copy);
        // The other version shows after the scans found it for the wait.
        let start = std::time::Instant::now();
        let target = Target::Full(f.roots());
        reconciler
            .run_at(&f.store, &target, &|_| false, start)
            .unwrap();
        assert_eq!(status.note("a").other_versions, Vec::<PathBuf>::new());
        let later = start + library::status::OTHER_VERSION_WAIT;
        reconciler
            .run_at(&f.store, &target, &|_| false, later)
            .unwrap();
        assert_eq!(status.note("a").other_versions, [copy]);
    }
}

fn copy_folder(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_folder(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// KTD9: the Move staging prefix is not the save's, so the launch sweep
/// keeps a staged copy. The Move removes its own leftovers when it resumes.
#[test]
fn a_crash_at_copied_keeps_the_staged_copy_through_the_launch_sweep() {
    let f = folders();
    let fs = Volumes::of(&f, true);
    f.save_note("a", "Budget");
    f.open();
    let m = mover(&f, &fs);
    let j = journal(&m);
    let MoveStep::Copied { staging, .. } = drive(&f, &m, &j, "a", &|s| is_copied(s)) else {
        unreachable!();
    };
    // A relaunch: the staging sweep runs on every library root.
    for root in f.roots() {
        session_save::sweep_staging(&root, |_| true).unwrap();
    }
    assert!(staging.join("session.md").is_file());
    // A copy that a crash stopped before its record is a leftover.
    let leftover = f.new.join(move_notes::staging_name("gone"));
    std::fs::create_dir_all(&leftover).unwrap();
    finish(&m);
    assert!(!leftover.exists());
    assert_moved(&f, "a");
}

/// AE13: a full disk pauses the Move, the partial copy goes, the source
/// stays, and Retry finishes the Move with no note listed twice.
#[test]
fn a_full_disk_pauses_the_move_and_retry_finishes_it() {
    let f = folders();
    let fs = Volumes::of(&f, true);
    let source = f.save_note("a", "Budget");
    f.open();
    let full = AtomicBool::new(true);
    let copy = |from: &Path, to: &Path| -> io::Result<u64> {
        if full.load(Ordering::SeqCst) {
            std::fs::create_dir(to)?;
            std::fs::write(to.join("part"), b"x")?;
            return Err(io::Error::from(io::ErrorKind::StorageFull));
        }
        move_notes::copy_note(from, to)
    };
    let m = Mover {
        store: &f.store,
        claims: &f.claims,
        fs: &fs,
        copy: &copy,
    };
    let full_disk = MovePause::DiskFull {
        folder: f.new.to_string_lossy().into_owned(),
    };
    assert_eq!(pass(&m), PassEnd::Paused(full_disk.clone()));
    assert!(staging_left(&f.new).is_empty());
    assert!(source.join("audio/segment-01.opus").is_file());
    assert_eq!(f.home("a"), source);
    assert_eq!(f.listed("a"), 1);
    let status = move_notes::status(&f.store.lock().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(status.journal.pause, Some(full_disk.clone()));
    // The Move stays paused until Retry.
    full.store(false, Ordering::SeqCst);
    assert_eq!(pass(&m), PassEnd::Paused(full_disk));
    move_notes::retry(&f.store).unwrap();
    finish(&m);
    assert_moved(&f, "a");
}

#[test]
fn a_new_folder_that_is_not_reachable_pauses_the_move() {
    let f = folders();
    let fs = Volumes::of(&f, false);
    f.save_note("a", "Budget");
    f.open();
    std::fs::remove_dir_all(&f.new).unwrap();
    let m = mover(&f, &fs);
    assert_eq!(
        pass(&m),
        PassEnd::Paused(MovePause::NotReachable {
            folder: f.new.to_string_lossy().into_owned(),
        })
    );
    assert_eq!(f.step("a"), Some(MoveStep::Pending));
    std::fs::create_dir_all(&f.new).unwrap();
    move_notes::retry(&f.store).unwrap();
    finish(&m);
    assert_moved(&f, "a");
}

/// A signed-out sync client removes the files and the marker of the old
/// folder. The notes are not gone, so the Move pauses and drops none.
#[test]
fn an_old_folder_with_no_marker_pauses_the_move() {
    let f = folders();
    let fs = Volumes::of(&f, false);
    f.save_note("a", "Budget");
    f.open();
    std::fs::remove_file(library::scan::marker(&f.old)).unwrap();
    let m = mover(&f, &fs);
    assert!(matches!(
        pass(&m),
        PassEnd::Paused(MovePause::NotReachable { .. })
    ));
    assert_eq!(f.step("a"), Some(MoveStep::Pending));
}

fn placed_copy(f: &Folders, m: &Mover, j: &MoveJournal) -> (MoveStep, PathBuf) {
    let step = drive(f, m, j, "a", &|s| is_placed(s));
    let MoveStep::Placed { path, .. } = &step else {
        unreachable!();
    };
    let path = path.clone();
    (step, path)
}

/// Across volumes, the source goes after the copy is placed and before
/// the final check: another computer deleted the note. The Move drops it
/// and removes its own copy, so the deleted note does not come back.
#[test]
fn a_source_that_goes_before_the_final_check_drops_the_note_and_its_copy() {
    let f = folders();
    let fs = Volumes::of(&f, true);
    f.save_note("a", "Budget");
    f.open();
    let m = mover(&f, &fs);
    let j = journal(&m);
    let (step, path) = placed_copy(&f, &m, &j);
    std::fs::remove_dir_all(f.home("a")).unwrap();

    assert!(matches!(m.step(&j, "a", &step).unwrap(), Taken::Dropped));
    assert!(!path.exists(), "the copy of the Move is removed");
}

/// The source and the library marker of the old folder go before the
/// final check: the old folder is not reachable, so the Move pauses and
/// keeps its copy.
#[test]
fn a_source_that_goes_with_the_old_folder_pauses_the_final_check() {
    let f = folders();
    let fs = Volumes::of(&f, true);
    f.save_note("a", "Budget");
    f.open();
    let m = mover(&f, &fs);
    let j = journal(&m);
    let (step, path) = placed_copy(&f, &m, &j);
    std::fs::remove_dir_all(f.home("a")).unwrap();
    std::fs::remove_file(library::scan::marker(&f.old)).unwrap();

    let paused = m.step(&j, "a", &step);
    assert!(
        matches!(
            paused,
            Err(MoveError::Paused(MovePause::NotReachable { .. }))
        ),
        "{paused:?}"
    );
    assert!(path.exists(), "the copy stays");
}

/// The folder in the new folder has no move marker: it is a finished note
/// that another computer moved there, and its source is gone. The Move
/// takes it as the home folder and does not delete it.
#[test]
fn a_finished_note_in_the_new_folder_is_homed_when_the_source_is_gone() {
    let f = folders();
    let fs = Volumes::of(&f, true);
    f.save_note("a", "Budget");
    f.open();
    let m = mover(&f, &fs);
    let j = journal(&m);
    let (step, path) = placed_copy(&f, &m, &j);
    std::fs::remove_file(path.join(MARKER)).unwrap();
    std::fs::remove_dir_all(f.home("a")).unwrap();

    assert!(matches!(
        m.step(&j, "a", &step).unwrap(),
        Taken::Next(MoveStep::Homed { .. })
    ));
    assert!(path.join("session.md").is_file());
    assert_eq!(f.home("a"), path);
}

/// The old folder goes away during a pass (a removed disk or a lost
/// share). The notes that remain in it are not gone, so the Move pauses,
/// keeps their rows, and drops none.
#[test]
fn an_old_folder_that_goes_away_during_a_pass_pauses_the_move() {
    for cross in [false, true] {
        let f = folders();
        let fs = Volumes::of(&f, cross);
        f.save_note("a", "Budget");
        f.save_note("b", "Planning");
        f.open();
        let away = f.working.join("away");
        let m = mover(&f, &fs);
        let mut done = Vec::new();
        let end = m
            .pass(&mut |id, _| {
                if done.is_empty() {
                    std::fs::rename(&f.old, &away).unwrap();
                }
                done.push(id.to_owned());
            })
            .unwrap();
        assert_eq!(
            end,
            PassEnd::Paused(MovePause::NotReachable {
                folder: f.old.to_string_lossy().into_owned(),
            }),
            "cross-volume: {cross}"
        );
        assert_eq!(done.len(), 1, "cross-volume: {cross}");
        let left = if done[0] == "a" { "b" } else { "a" };
        assert!(f.store.lock().unwrap().move_journal().unwrap().is_some());
        assert_eq!(
            f.step(left),
            Some(MoveStep::Pending),
            "cross-volume: {cross}"
        );
        // The folder comes back, and Retry finishes the Move.
        std::fs::rename(&away, &f.old).unwrap();
        move_notes::retry(&f.store).unwrap();
        finish(&m);
        assert_moved(&f, "a");
        assert_moved(&f, "b");
    }
}

#[test]
fn a_note_that_processes_when_the_move_starts_moves_after_processing() {
    for cross in [false, true] {
        let f = folders();
        let fs = Volumes::of(&f, cross);
        let source = f.save_note("a", "Budget");
        {
            let mut s = f.store.lock().unwrap();
            s.apply(
                "a",
                &Event::UserTranscribeAgain {
                    at: started(),
                    audio_intact: true,
                    capture_active: false,
                    regenerating: false,
                },
            )
            .unwrap();
            s.dequeue("run", started()).unwrap();
        }
        f.open();
        let m = mover(&f, &fs);
        assert_eq!(pass(&m), PassEnd::Waiting);
        assert_eq!(f.step("a"), Some(MoveStep::Pending));
        assert_eq!(f.home("a"), source);
        f.store
            .lock()
            .unwrap()
            .apply("a", &Event::TranscribeAgainFailed)
            .unwrap();
        finish(&m);
        assert_moved(&f, "a");
    }
}

#[test]
fn a_note_whose_summary_is_made_again_moves_after_that() {
    let f = folders();
    let fs = Volumes::of(&f, true);
    f.save_note("a", "Budget");
    f.open();
    let m = mover(&f, &fs);
    let regeneration = f.claims.regenerate("a").unwrap();
    assert_eq!(pass(&m), PassEnd::Waiting);
    assert_eq!(f.step("a"), Some(MoveStep::Pending));
    drop(regeneration);
    finish(&m);
    assert_moved(&f, "a");
}

/// An edit after the copy and before the final check is in the new copy.
#[test]
fn an_edit_after_the_copy_is_in_the_moved_note() {
    let f = folders();
    let fs = Volumes::of(&f, true);
    let source = f.save_note("a", "Budget");
    f.open();
    let m = mover(&f, &fs);
    let j = journal(&m);
    drive(&f, &m, &j, "a", &|s| is_placed(s));
    pipeline::save::set_notes(&mut f.store.lock().unwrap(), "a", "edited\n").unwrap();
    assert!(
        std::fs::read_to_string(source.join("notes.md"))
            .unwrap()
            .contains("edited")
    );
    finish(&m);
    let home = assert_moved(&f, "a");
    assert!(
        std::fs::read_to_string(home.join("notes.md"))
            .unwrap()
            .contains("edited")
    );
    assert_eq!(f.store.lock().unwrap().notes("a").unwrap(), "edited\n");
}

/// A My Notes edit just after the home change goes to the new folder and
/// is kept when the source goes.
#[test]
fn an_edit_just_after_the_home_change_is_kept() {
    let f = folders();
    let fs = Volumes::of(&f, true);
    f.save_note("a", "Budget");
    f.open();
    let m = mover(&f, &fs);
    let j = journal(&m);
    drive(&f, &m, &j, "a", &|s| is_homed(s));
    assert_eq!(f.home("a").parent(), Some(f.new.as_path()));
    pipeline::save::set_notes(&mut f.store.lock().unwrap(), "a", "after\n").unwrap();
    finish(&m);
    let home = assert_moved(&f, "a");
    assert!(
        std::fs::read_to_string(home.join("notes.md"))
            .unwrap()
            .contains("after")
    );
}

/// From the final check to the home change, a note edit waits.
#[test]
fn a_note_edit_waits_while_the_move_holds_the_note() {
    let f = folders();
    f.save_note("a", "Budget");
    let hold = f.claims.hold("a").unwrap();
    let edited = Arc::new(AtomicBool::new(false));
    let editor = {
        let (store, claims, edited) = (Arc::clone(&f.store), f.claims.clone(), Arc::clone(&edited));
        std::thread::spawn(move || {
            let mut guard = move_notes::lock_unheld(&store, &claims, "a");
            pipeline::save::set_notes(&mut guard, "a", "later\n").unwrap();
            edited.store(true, Ordering::SeqCst);
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    assert!(!edited.load(Ordering::SeqCst));
    assert_eq!(f.store.lock().unwrap().notes("a").unwrap(), "first\n");
    drop(hold);
    editor.join().unwrap();
    assert_eq!(f.store.lock().unwrap().notes("a").unwrap(), "later\n");
}

/// A rename during the Move waits, and the folder takes its name when the
/// Move is done with it.
#[test]
fn a_rename_during_the_move_names_the_moved_folder() {
    let f = folders();
    let fs = Volumes::of(&f, true);
    f.save_note("a", "Budget");
    f.open();
    let renamed =
        pipeline::folder_rename::rename_session(&f.store, "a", "Review", &|_| false, &RealFs)
            .unwrap();
    assert_eq!(renamed, pipeline::folder_rename::FolderRename::Later);
    let m = mover(&f, &fs);
    finish(&m);
    let home = assert_moved(&f, "a");
    assert!(
        home.to_string_lossy().ends_with("Review"),
        "{}",
        home.display()
    );
}

/// A rename during the Move that keeps the folder name rewrites
/// `session.md` after the Move. The fingerprints stay equal to the files
/// (KTD6).
#[test]
fn a_moved_note_whose_files_match_keeps_fingerprints_equal_to_the_files() {
    let f = folders();
    let fs = Volumes::of(&f, true);
    f.save_note("a", "Budget");
    f.open();
    let renamed =
        pipeline::folder_rename::rename_session(&f.store, "a", "Budget?", &|_| false, &RealFs)
            .unwrap();
    assert_eq!(renamed, pipeline::folder_rename::FolderRename::Later);
    let m = mover(&f, &fs);
    finish(&m);
    let home = assert_moved(&f, "a");
    let doc = session_core::note_folder::NoteFiles::read(&home)
        .map(|files| files.doc)
        .unwrap();
    assert_eq!(doc.title.as_deref(), Some("Budget?"));
    let of = |name: &str| {
        std::fs::read(home.join(name))
            .ok()
            .map(|bytes| session_core::folder::fingerprint(&bytes))
    };
    assert!(doc.transcript_fingerprint.is_some());
    assert_eq!(doc.transcript_fingerprint, of("transcript.md"));
    assert_eq!(doc.summary_fingerprint, of("summary.md"));
}

#[test]
fn a_source_that_another_computer_moved_away_leaves_the_move() {
    for cross in [false, true] {
        let f = folders();
        let fs = Volumes::of(&f, cross);
        let source = f.save_note("a", "Budget");
        f.open();
        let elsewhere = f.working.join("elsewhere");
        std::fs::rename(&source, &elsewhere).unwrap();
        let m = mover(&f, &fs);
        assert_eq!(pass(&m), PassEnd::Finished);
        assert!(f.store.lock().unwrap().move_journal().unwrap().is_none());
        assert!(folders_of(&f.new, "a").is_empty());
        assert!(elsewhere.join("session.md").is_file());
    }
}

/// KTD10: a save that a crash stopped resumes into the old folder during
/// the Move. The Move waits for it and moves that note too.
#[test]
fn a_note_that_a_resumed_save_puts_in_the_old_folder_also_moves() {
    for cross in [false, true] {
        let f = folders();
        let fs = Volumes::of(&f, cross);
        f.save_note("a", "Budget");
        assert!(
            f.save_with("b", "Planning", &f.old, &StopsBeforeFinalRename)
                .is_err()
        );
        assert_ne!(f.state("b"), SessionState::Saved);
        f.open();
        let m = mover(&f, &fs);
        assert_eq!(pass(&m), PassEnd::Waiting);
        assert_moved(&f, "a");
        // The save resumes: its staged copy is in the old folder, so the
        // note is saved there, whatever the storage folder is now.
        let saved = f.save_with("b", "Planning", &f.new, &RealFs).unwrap();
        assert_eq!(saved.parent(), Some(f.old.as_path()));
        finish(&m);
        assert_moved(&f, "b");
    }
}

/// AE6: a note recorded during the Move saves to the new folder and is
/// listed one time.
#[test]
fn a_new_recording_during_the_move_saves_to_the_new_folder_once() {
    let f = folders();
    let fs = Volumes::of(&f, true);
    f.save_note("a", "Budget");
    f.open();
    let m = mover(&f, &fs);
    let regeneration = f.claims.regenerate("a").unwrap();
    assert_eq!(pass(&m), PassEnd::Waiting);
    let b = f.save_with("b", "Planning", &f.new, &RealFs).unwrap();
    assert_eq!(b.parent(), Some(f.new.as_path()));
    f.reconcile();
    assert_eq!(f.listed("b"), 1);
    assert_eq!(f.listed("a"), 1);
    drop(regeneration);
    finish(&m);
    assert_moved(&f, "a");
    assert_eq!(f.home("b"), b);
    assert_eq!(folders_of(&f.new, "b"), vec![b]);
    assert_eq!(f.listed("b"), 1);
}

#[test]
fn a_name_that_is_taken_in_the_new_folder_gives_a_free_name() {
    for cross in [false, true] {
        let f = folders();
        let fs = Volumes::of(&f, cross);
        let source = f.save_note("a", "Budget");
        let name = source.file_name().unwrap();
        let taken = f.new.join(name);
        std::fs::create_dir_all(&taken).unwrap();
        std::fs::write(taken.join("mine.txt"), "keep").unwrap();
        f.open();
        finish(&mover(&f, &fs));
        let home = assert_moved(&f, "a");
        assert_eq!(
            home.file_name().unwrap().to_string_lossy(),
            format!("{}-2", name.to_string_lossy())
        );
        assert_eq!(
            std::fs::read_to_string(taken.join("mine.txt")).unwrap(),
            "keep"
        );
    }
}

/// [`Volumes`], with each rename to a note name in the new folder refused
/// as in use (`PermissionDenied`, as Windows does while another app holds
/// a file) `refusals` times. Hidden names (the volume probe) are not
/// refused.
struct InUseInNew {
    volumes: Volumes,
    refusals: std::cell::Cell<usize>,
}

impl Fs for InUseInNew {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.volumes.rename(from, to)
    }
    fn rename_new(&self, from: &Path, to: &Path) -> io::Result<()> {
        let hidden = to
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'));
        if to.starts_with(&self.volumes.new) && !hidden && self.refusals.get() > 0 {
            self.refusals.set(self.refusals.get() - 1);
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        self.volumes.rename_new(from, to)
    }
}

#[test]
fn a_note_in_use_for_a_moment_moves_with_its_own_name() {
    for cross in [false, true] {
        let f = folders();
        let fs = InUseInNew {
            volumes: Volumes::of(&f, cross),
            refusals: std::cell::Cell::new(2),
        };
        let source = f.save_note("a", "Budget");
        let name = source.file_name().unwrap().to_owned();
        f.open();
        finish(&mover(&f, &fs));
        let home = assert_moved(&f, "a");
        assert_eq!(
            home.file_name().unwrap(),
            name,
            "in use is not a taken name"
        );
    }
}

#[test]
fn a_note_that_stays_in_use_pauses_the_move_saying_so() {
    for cross in [false, true] {
        let f = folders();
        let fs = InUseInNew {
            volumes: Volumes::of(&f, cross),
            refusals: std::cell::Cell::new(usize::MAX),
        };
        f.save_note("a", "Budget");
        f.open();
        match pass(&mover(&f, &fs)) {
            PassEnd::Paused(MovePause::Failed { message }) => {
                assert!(message.contains("in use"), "{message}");
                assert!(!message.contains("no free name"), "{message}");
            }
            end => panic!("{end:?}"),
        }
        // Only the first name was tried.
        assert_eq!(
            fs.refusals.get(),
            usize::MAX - 1 - session_save::rename_home::IN_USE_RETRIES
        );
    }
}

// ---------------------------------------------------------------- type folders (KTD14)

impl Folders {
    /// [`Folders::save_note`], then move the home folder into the type
    /// folder `note_type` of the old folder, and record the new home path,
    /// as the library scan does.
    fn save_typed_note(&self, id: &str, title: &str, note_type: &str) -> PathBuf {
        let saved = self.save_note(id, title);
        let folder = self.old.join(note_type);
        std::fs::create_dir_all(&folder).unwrap();
        let typed = folder.join(saved.file_name().unwrap());
        std::fs::rename(&saved, &typed).unwrap();
        self.store
            .lock()
            .unwrap()
            .set_home_path(id, &typed.to_string_lossy())
            .unwrap();
        typed
    }
}

/// The note `id` has one complete copy, in the type folder `note_type` of
/// the new folder, and the library lists it one time.
fn assert_moved_typed(f: &Folders, id: &str, note_type: &str) -> PathBuf {
    let home = f.home(id);
    let folder = f.new.join(note_type);
    assert_eq!(home.parent(), Some(folder.as_path()), "{id}");
    assert_eq!(folders_of(&folder, id), vec![home.clone()], "{id}");
    assert!(folders_of(&f.old.join(note_type), id).is_empty(), "{id}");
    assert!(folders_of(&f.new, id).is_empty(), "{id}");
    for file in ["session.md", "transcript.md", "notes.md"] {
        assert!(home.join(file).is_file(), "{id}: {file}");
    }
    assert_eq!(
        std::fs::read(home.join("audio/segment-01.opus")).unwrap(),
        vec![7u8; 50_000]
    );
    assert!(!home.join(MARKER).exists(), "{id}");
    assert!(staging_left(&f.new).is_empty());
    assert!(staging_left(&folder).is_empty());
    assert_eq!(f.listed(id), 1, "{id}");
    home
}

// Covers R8, R27 (KTD14).
#[test]
fn a_note_in_a_type_folder_moves_into_the_same_type_folder() {
    for cross in [false, true] {
        let f = folders();
        let fs = Volumes::of(&f, cross);
        let typed = f.save_typed_note("a", "Budget", "Pathfinder");
        f.save_note("b", "Planning");
        f.open();
        finish(&mover(&f, &fs));
        let home = assert_moved_typed(&f, "a", "Pathfinder");
        assert_eq!(home.file_name(), typed.file_name());
        assert_moved(&f, "b");
        assert!(f.store.lock().unwrap().move_journal().unwrap().is_none());
    }
}

#[test]
fn a_name_that_is_taken_in_the_new_type_folder_gives_a_free_name() {
    for cross in [false, true] {
        let f = folders();
        let fs = Volumes::of(&f, cross);
        let source = f.save_typed_note("a", "Budget", "Pathfinder");
        let name = source.file_name().unwrap();
        let taken = f.new.join("Pathfinder").join(name);
        std::fs::create_dir_all(&taken).unwrap();
        std::fs::write(taken.join("mine.txt"), "keep").unwrap();
        f.open();
        finish(&mover(&f, &fs));
        let home = assert_moved_typed(&f, "a", "Pathfinder");
        assert_eq!(
            home.file_name().unwrap().to_string_lossy(),
            format!("{}-2", name.to_string_lossy())
        );
        assert_eq!(
            std::fs::read_to_string(taken.join("mine.txt")).unwrap(),
            "keep"
        );
    }
}

/// A cross-volume flag, the step before the crash, and its label.
type LostStep = (bool, fn(&MoveStep) -> bool, &'static str);

#[test]
fn a_lost_record_after_each_step_of_a_typed_note_gives_one_copy() {
    let steps: [LostStep; 6] = [
        (false, is_pending, "the rename"),
        (false, is_placed, "the home change"),
        (true, is_pending, "the copy"),
        (true, is_copied, "the rename into place"),
        (true, is_placed, "the home change"),
        (true, is_homed, "the source removal"),
    ];
    for (cross, before, label) in steps {
        let f = folders();
        let fs = Volumes::of(&f, cross);
        f.save_typed_note("a", "Budget", "Pathfinder");
        f.open();
        let m = mover(&f, &fs);
        let j = journal(&m);
        let recorded = drive(&f, &m, &j, "a", &|s| before(s));
        let after = m.step(&j, "a", &recorded).unwrap();
        assert_eq!(f.listed("a"), 1, "{label}");
        let again = m.step(&j, "a", &recorded).unwrap();
        assert_eq!(again, after, "{label}, cross-volume: {cross}");
        let Taken::Next(next) = again else {
            panic!("{label}: {again:?}");
        };
        f.store.lock().unwrap().set_move_step("a", &next).unwrap();
        finish(&m);
        assert_moved_typed(&f, "a", "Pathfinder");
    }
}

/// A note that is in the new folder already gets no second copy.
#[test]
fn a_note_that_is_in_the_new_folder_already_is_not_placed_twice() {
    for cross in [false, true] {
        let f = folders();
        let fs = Volumes::of(&f, cross);
        let source = f.save_note("a", "Budget");
        let there = f.new.join(source.file_name().unwrap());
        move_notes::copy_note(&source, &there).unwrap();
        let expected = std::fs::read_to_string(source.join("notes.md")).unwrap();
        std::fs::write(there.join("notes.md"), "older\n").unwrap();
        f.open();
        finish(&mover(&f, &fs));
        let home = assert_moved(&f, "a");
        assert_eq!(home, there);
        // The final check found the difference and copied the source again.
        assert_eq!(
            std::fs::read_to_string(home.join("notes.md")).unwrap(),
            expected
        );
    }
}

/// After the Move, the old folder is not a library root, and its
/// `.longhand/` folder stays for another computer that uses it (R21).
#[test]
fn after_the_move_the_old_folder_is_not_watched_and_keeps_its_longhand_folder() {
    let f = folders();
    let fs = Volumes::of(&f, false);
    f.save_note("a", "Budget");
    let settings = f.old.join(".longhand").join("settings.json");
    std::fs::write(&settings, "{}").unwrap();
    f.open();
    assert_eq!(f.roots(), vec![f.new.clone(), f.old.clone()]);
    finish(&mover(&f, &fs));
    assert_eq!(f.roots(), vec![f.new.clone()]);
    assert!(library::scan::marker(&f.old).exists());
    assert!(settings.is_file());
    assert_moved(&f, "a");
}

/// KTD11: one journal at a time.
#[test]
fn a_second_move_is_refused_while_one_is_open() {
    let f = folders();
    f.save_note("a", "Budget");
    f.open();
    assert!(!move_notes::open(&f.store, &f.new, &f.old).unwrap());
}

#[test]
fn same_contents_compares_every_file_but_the_marker() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    for d in [&a, &b] {
        std::fs::create_dir_all(d.join("audio")).unwrap();
        std::fs::write(d.join("audio/x.opus"), [1, 2, 3]).unwrap();
        std::fs::write(d.join("notes.md"), "n").unwrap();
    }
    std::fs::write(b.join(MARKER), "a").unwrap();
    assert!(move_notes::same_contents(&a, &b).unwrap());
    std::fs::write(b.join("audio/x.opus"), [1, 2, 4]).unwrap();
    assert!(!move_notes::same_contents(&a, &b).unwrap());
    std::fs::write(b.join("audio/x.opus"), [1, 2, 3]).unwrap();
    std::fs::write(b.join("extra.md"), "e").unwrap();
    assert!(!move_notes::same_contents(&a, &b).unwrap());
}

#[test]
fn same_contents_does_not_count_the_in_use_markers() {
    // Another computer refreshes its in-use marker every 2 minutes (KTD4
    // of the audio removal plan). A refresh between the copy and the final
    // check does not make the Move copy the note again.
    use pipeline::in_use::{marker_name, write_marker};
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    for d in [&a, &b] {
        std::fs::create_dir_all(d.join("audio")).unwrap();
        std::fs::write(d.join("notes.md"), "n").unwrap();
    }
    let now = chrono::Local::now().fixed_offset();
    write_marker(&a, "run-a", now).unwrap();
    write_marker(&b, "run-a", now + chrono::TimeDelta::minutes(2)).unwrap();
    write_marker(&a, "run-b", now).unwrap();
    assert!(move_notes::same_contents(&a, &b).unwrap());
    // Only at the top of the note folder.
    std::fs::write(a.join("audio").join(marker_name("run-c")), "x").unwrap();
    assert!(!move_notes::same_contents(&a, &b).unwrap());
}

// ------------------------------------------------------ import record (R11)

const THIS_COMPUTER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_COMPUTER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn identity(n: u8) -> import_record::Identity {
    import_record::Identity::parse(&format!("{n:02x}").repeat(32)).unwrap()
}

/// Write an import record entry of `computer` for the identity `n` into
/// `root`.
fn record(root: &Path, n: u8, computer: &str, state: import_record::State) {
    let entry = import_record::Entry::new(
        state,
        &format!("Recording {n}.m4a"),
        1000 + u64::from(n),
        import_record::ComputerId::parse(computer).unwrap(),
        started(),
    );
    import_record::write(root, &identity(n), &entry).unwrap();
}

fn entry_bytes(root: &Path, n: u8, computer: &str) -> Vec<u8> {
    let computer = import_record::ComputerId::parse(computer).unwrap();
    std::fs::read(import_record::entry_path(root, &identity(n), &computer)).unwrap()
}

/// R11: the Move takes every import record entry with the notes. The
/// file of a deleted note (an `imported` entry with no note) stays
/// imported in the new storage folder, so the watcher does not import it
/// again (R12).
#[test]
fn a_move_copies_every_import_record_entry_into_the_new_folder() {
    let f = folders();
    let fs = Volumes::of(&f, true);
    f.save_note("a", "Budget");
    record(&f.old, 1, THIS_COMPUTER, import_record::State::Imported);
    record(&f.old, 1, OTHER_COMPUTER, import_record::State::Skipped);
    record(&f.old, 2, OTHER_COMPUTER, import_record::State::Skipped);
    // The file of a deleted note: no note has it.
    record(&f.old, 3, THIS_COMPUTER, import_record::State::Imported);
    f.open();
    finish(&mover(&f, &fs));
    assert!(f.store.lock().unwrap().move_journal().unwrap().is_none());
    for (n, computer) in [
        (1, THIS_COMPUTER),
        (1, OTHER_COMPUTER),
        (2, OTHER_COMPUTER),
        (3, THIS_COMPUTER),
    ] {
        assert_eq!(
            entry_bytes(&f.new, n, computer),
            entry_bytes(&f.old, n, computer)
        );
    }
    assert!(matches!(
        import_record::lookup(&f.new, &identity(3)),
        import_record::Lookup::Imported(_)
    ));
    assert_moved(&f, "a");
}

/// A crash during the merge leaves some entries in the new folder and a
/// staged file of an entry write. The Move journal stays open until the
/// merge is complete, so the launch continues the Move, and the merge
/// copies the rest with no lost entry.
#[test]
fn a_crash_during_the_import_record_merge_completes_at_the_next_launch() {
    let f = folders();
    let fs = Volumes::of(&f, false);
    f.save_note("a", "Budget");
    for n in 1..=4 {
        record(&f.old, n, THIS_COMPUTER, import_record::State::Imported);
    }
    f.open();
    // The state on disk at the crash: the note moved, entries 1 and 2 are
    // in the new folder, and entry 3 has only its staged file.
    let m = mover(&f, &fs);
    let j = journal(&m);
    drive(&f, &m, &j, "a", &|s| *s == MoveStep::Done);
    for n in [1, 2] {
        let to = import_record::identity_folder(&f.new, &identity(n));
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(
            to.join(format!("{THIS_COMPUTER}.json")),
            entry_bytes(&f.old, n, THIS_COMPUTER),
        )
        .unwrap();
    }
    let third = import_record::identity_folder(&f.new, &identity(3));
    std::fs::create_dir_all(&third).unwrap();
    std::fs::write(third.join(".0123.new"), b"{").unwrap();
    assert!(f.store.lock().unwrap().move_journal().unwrap().is_some());
    // The launch: a new pass of the open Move.
    finish(&mover(&f, &fs));
    assert!(f.store.lock().unwrap().move_journal().unwrap().is_none());
    for n in 1..=4 {
        assert_eq!(
            entry_bytes(&f.new, n, THIS_COMPUTER),
            entry_bytes(&f.old, n, THIS_COMPUTER),
            "{n}"
        );
    }
}

/// A merge that cannot write pauses the Move: the journal stays open, so
/// the old folder stays a library root, and Retry merges again.
#[test]
fn a_merge_that_cannot_write_pauses_the_move_and_retry_finishes_it() {
    let f = folders();
    let fs = Volumes::of(&f, false);
    f.save_note("a", "Budget");
    record(&f.old, 1, THIS_COMPUTER, import_record::State::Imported);
    f.open();
    // A file where the imports folder must be.
    let imports = import_record::imports_folder(&f.new);
    std::fs::create_dir_all(imports.parent().unwrap()).unwrap();
    std::fs::write(&imports, b"not a folder").unwrap();
    let m = mover(&f, &fs);
    assert!(matches!(
        pass(&m),
        PassEnd::Paused(MovePause::Failed { .. })
    ));
    assert!(f.store.lock().unwrap().move_journal().unwrap().is_some());
    assert_eq!(f.roots(), vec![f.new.clone(), f.old.clone()]);
    std::fs::remove_file(&imports).unwrap();
    move_notes::retry(&f.store).unwrap();
    finish(&m);
    assert_eq!(
        entry_bytes(&f.new, 1, THIS_COMPUTER),
        entry_bytes(&f.old, 1, THIS_COMPUTER)
    );
}

/// An old folder that goes away after the last note moved is not an
/// empty import record: the Move pauses before the merge and does not
/// close (`docs/solutions/logic-errors/move-dropped-notes-when-old-folder-went-away.md`).
#[test]
fn an_old_folder_that_goes_away_before_the_merge_pauses_the_move() {
    let f = folders();
    let fs = Volumes::of(&f, false);
    f.save_note("a", "Budget");
    record(&f.old, 1, THIS_COMPUTER, import_record::State::Imported);
    f.open();
    let m = mover(&f, &fs);
    let j = journal(&m);
    drive(&f, &m, &j, "a", &|s| *s == MoveStep::Done);
    let away = f.working.join("away");
    std::fs::rename(&f.old, &away).unwrap();
    assert_eq!(
        pass(&m),
        PassEnd::Paused(MovePause::NotReachable {
            folder: f.old.to_string_lossy().into_owned(),
        }),
    );
    assert!(f.store.lock().unwrap().move_journal().unwrap().is_some());
    std::fs::rename(&away, &f.old).unwrap();
    move_notes::retry(&f.store).unwrap();
    finish(&m);
    assert_eq!(
        entry_bytes(&f.new, 1, THIS_COMPUTER),
        entry_bytes(&f.old, 1, THIS_COMPUTER)
    );
}
