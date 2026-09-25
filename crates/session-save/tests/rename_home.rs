//! The rename of a home folder (KTD8): each move is recorded before it is
//! taken, and a record whose end was lost settles from disk alone.

use std::cell::Cell;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use session_save::rename_home::{
    self, IN_USE_RETRIES, Move, RenameError, Resolved, exists_exactly, has_name, temp_name,
};
use session_save::{Fs, RealFs};

const OLD: &str = "2026-09-22 1400 Longhand session";
const NEW: &str = "2026-09-22 1400 Budget review";

/// A home folder as a save leaves it.
fn home(dest: &Path, name: &str) -> PathBuf {
    let dir = dest.join(name);
    std::fs::create_dir_all(dir.join("audio")).unwrap();
    std::fs::write(dir.join("audio/segment-01.opus"), [7u8; 64]).unwrap();
    std::fs::write(dir.join("session.md"), "# session\n").unwrap();
    dir
}

fn entries(dest: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dest)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

/// Rename and keep each recorded move.
fn rename(from: &Path, name: &str, fs: &dyn Fs) -> (Result<PathBuf, RenameError>, Vec<Move>) {
    let mut moves = Vec::new();
    let result = rename_home::rename(from, name, "s1", fs, &mut |m| {
        moves.push(m.clone());
        Ok(())
    });
    (result, moves)
}

/// Takes `moves` real renames, then stops the process: the next rename
/// panics before it changes anything.
struct StopsAfter {
    moves: Cell<usize>,
}

impl Fs for StopsAfter {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        RealFs.rename(from, to)
    }
    fn rename_new(&self, from: &Path, to: &Path) -> io::Result<()> {
        if self.moves.get() == 0 {
            panic!("the process stops here");
        }
        self.moves.set(self.moves.get() - 1);
        RealFs.rename_new(from, to)
    }
}

/// Run a rename that stops after `moves` renames, or that ends and whose
/// end the caller does not record, and return the last recorded move: the
/// record that a relaunch finds.
fn stopped_after(from: &Path, name: &str, moves: usize) -> Move {
    let fs = StopsAfter {
        moves: Cell::new(moves),
    };
    let mut recorded = Vec::new();
    let _ = catch_unwind(AssertUnwindSafe(|| {
        rename_home::rename(from, name, "s1", &fs, &mut |m| {
            recorded.push(m.clone());
            Ok(())
        })
    }));
    recorded.pop().expect("a move was recorded before the stop")
}

// ------------------------------------------------------------- lost records

#[test]
fn a_move_recorded_and_not_taken_resolves_to_the_old_folder() {
    let dest = tempfile::tempdir().unwrap();
    let from = home(dest.path(), OLD);
    let intent = stopped_after(&from, NEW, 0);
    assert_eq!(intent.to, dest.path().join(NEW));

    assert_eq!(
        rename_home::resolve(&intent, &RealFs).unwrap(),
        Resolved::NotMoved
    );
    assert_eq!(entries(dest.path()), [OLD]);
}

#[test]
fn a_move_taken_and_not_recorded_resolves_to_the_new_folder() {
    let dest = tempfile::tempdir().unwrap();
    let from = home(dest.path(), OLD);
    let intent = stopped_after(&from, NEW, 1);
    // The record of the end is lost.
    assert_eq!(
        rename_home::resolve(&intent, &RealFs).unwrap(),
        Resolved::Moved(dest.path().join(NEW))
    );
    assert_eq!(entries(dest.path()), [NEW]);
    assert!(dest.path().join(NEW).join("audio/segment-01.opus").exists());
}

#[test]
fn a_change_of_case_stopped_at_the_temporary_name_finishes_at_the_new_name() {
    let dest = tempfile::tempdir().unwrap();
    let from = home(dest.path(), "2026-09-22 1400 budget review");
    let intent = stopped_after(&from, NEW, 1);
    assert_eq!(intent.temp, Some(dest.path().join(temp_name("s1"))));
    assert_eq!(entries(dest.path()), [temp_name("s1")]);

    assert_eq!(
        rename_home::resolve(&intent, &RealFs).unwrap(),
        Resolved::Moved(dest.path().join(NEW))
    );
    assert_eq!(entries(dest.path()), [NEW]);
}

#[test]
fn a_change_of_case_with_both_moves_taken_and_not_recorded_resolves_to_the_new_name() {
    let dest = tempfile::tempdir().unwrap();
    let from = home(dest.path(), "2026-09-22 1400 budget review");
    let intent = stopped_after(&from, NEW, 2);
    assert_eq!(entries(dest.path()), [NEW]);
    assert_eq!(
        rename_home::resolve(&intent, &RealFs).unwrap(),
        Resolved::Moved(dest.path().join(NEW))
    );
    assert_eq!(entries(dest.path()), [NEW]);
}

#[test]
fn a_change_of_case_recorded_and_not_taken_resolves_to_the_old_folder() {
    let dest = tempfile::tempdir().unwrap();
    let from = home(dest.path(), "2026-09-22 1400 budget review");
    let intent = stopped_after(&from, NEW, 0);
    assert_eq!(
        rename_home::resolve(&intent, &RealFs).unwrap(),
        Resolved::NotMoved
    );
    assert_eq!(entries(dest.path()), ["2026-09-22 1400 budget review"]);
}

#[test]
fn a_move_whose_folder_is_at_no_recorded_name_is_missing() {
    let dest = tempfile::tempdir().unwrap();
    let intent = Move {
        from: dest.path().join(OLD),
        to: dest.path().join(NEW),
        temp: None,
    };
    assert_eq!(
        rename_home::resolve(&intent, &RealFs).unwrap(),
        Resolved::Missing
    );
}

// ------------------------------------------------------------- renames

#[test]
fn a_rename_moves_the_folder_to_the_new_name_after_it_records_the_move() {
    let dest = tempfile::tempdir().unwrap();
    let from = home(dest.path(), OLD);
    let (result, moves) = rename(&from, NEW, &RealFs);
    assert_eq!(result.unwrap(), dest.path().join(NEW));
    assert_eq!(
        moves,
        [Move {
            from: from.clone(),
            to: dest.path().join(NEW),
            temp: None
        }]
    );
    assert_eq!(entries(dest.path()), [NEW]);
    assert!(dest.path().join(NEW).join("session.md").exists());
}

#[test]
fn a_folder_that_has_the_name_is_not_moved_or_recorded() {
    let dest = tempfile::tempdir().unwrap();
    let from = home(dest.path(), NEW);
    let (result, moves) = rename(&from, NEW, &RealFs);
    assert_eq!(result.unwrap(), from);
    assert!(moves.is_empty());
}

#[test]
fn a_name_that_another_folder_uses_takes_the_next_suffix() {
    let dest = tempfile::tempdir().unwrap();
    let from = home(dest.path(), OLD);
    home(dest.path(), NEW);
    let (result, _) = rename(&from, NEW, &RealFs);
    let suffixed = format!("{NEW}-2");
    assert_eq!(result.unwrap(), dest.path().join(&suffixed));
    assert_eq!(entries(dest.path()), [NEW.to_string(), suffixed]);
}

#[test]
fn a_change_of_case_alone_ends_with_the_new_case() {
    let dest = tempfile::tempdir().unwrap();
    let from = home(dest.path(), "2026-09-22 1400 budget review");
    let (result, moves) = rename(&from, NEW, &RealFs);
    assert_eq!(result.unwrap(), dest.path().join(NEW));
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].temp, Some(dest.path().join(temp_name("s1"))));
    assert_eq!(entries(dest.path()), [NEW]);
    assert!(exists_exactly(&dest.path().join(NEW)));
    assert!(!exists_exactly(
        &dest.path().join("2026-09-22 1400 budget review")
    ));
}

#[test]
fn a_missing_folder_is_reported_and_nothing_is_recorded() {
    let dest = tempfile::tempdir().unwrap();
    let (result, moves) = rename(&dest.path().join(OLD), NEW, &RealFs);
    assert!(matches!(result, Err(RenameError::Missing(_))), "{result:?}");
    assert!(moves.is_empty());
}

/// Refuses the first `refusals` renames as `PermissionDenied`, as Windows
/// does while another process holds a file in the folder.
struct InUse {
    refusals: Cell<usize>,
    calls: Cell<usize>,
}

impl InUse {
    fn new(refusals: usize) -> Self {
        Self {
            refusals: Cell::new(refusals),
            calls: Cell::new(0),
        }
    }
}

impl Fs for InUse {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        RealFs.rename(from, to)
    }
    fn rename_new(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.calls.set(self.calls.get() + 1);
        if self.refusals.get() > 0 {
            self.refusals.set(self.refusals.get() - 1);
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        RealFs.rename_new(from, to)
    }
}

#[test]
fn a_folder_in_use_is_tried_three_more_times_then_reported_in_use_with_its_old_name() {
    let dest = tempfile::tempdir().unwrap();
    let from = home(dest.path(), OLD);
    let fs = InUse::new(usize::MAX);
    let (result, moves) = rename(&from, NEW, &fs);
    assert!(
        matches!(result, Err(RenameError::InUse { .. })),
        "in use, not a taken name: {result:?}"
    );
    assert!(result.unwrap_err().to_string().contains("in use"));
    assert_eq!(fs.calls.get(), 1 + IN_USE_RETRIES);
    assert_eq!(moves.len(), 1, "no other name was tried");
    assert_eq!(entries(dest.path()), [OLD]);
    assert_eq!(
        rename_home::resolve(&moves[0], &RealFs).unwrap(),
        Resolved::NotMoved
    );
}

#[test]
fn a_folder_in_use_for_a_moment_is_moved_on_a_later_try() {
    let dest = tempfile::tempdir().unwrap();
    let from = home(dest.path(), OLD);
    let fs = InUse::new(2);
    let (result, _) = rename(&from, NEW, &fs);
    assert_eq!(result.unwrap(), dest.path().join(NEW));
    assert_eq!(fs.calls.get(), 3);
}

/// Refuses every rename to `to` as in use, and takes the others.
struct TargetInUse {
    to: PathBuf,
}

impl Fs for TargetInUse {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        RealFs.rename(from, to)
    }
    fn rename_new(&self, from: &Path, to: &Path) -> io::Result<()> {
        if to == self.to {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        RealFs.rename_new(from, to)
    }
}

#[test]
fn a_change_of_case_refused_at_the_second_move_puts_the_folder_back() {
    let dest = tempfile::tempdir().unwrap();
    let old = "2026-09-22 1400 budget review";
    let from = home(dest.path(), old);
    let fs = TargetInUse {
        to: dest.path().join(NEW),
    };
    let (result, moves) = rename(&from, NEW, &fs);
    assert!(
        matches!(result, Err(RenameError::InUse { .. })),
        "{result:?}"
    );
    assert_eq!(entries(dest.path()), [old]);
    assert_eq!(
        rename_home::resolve(&moves[0], &RealFs).unwrap(),
        Resolved::NotMoved
    );
}

#[test]
fn a_suffixed_folder_already_has_its_name() {
    assert!(has_name(NEW, NEW));
    assert!(has_name(&format!("{NEW}-2"), NEW));
    assert!(has_name(&format!("{NEW}-17"), NEW));
    assert!(!has_name(&format!("{NEW}-1"), NEW));
    assert!(!has_name(&format!("{NEW}-x"), NEW));
    assert!(!has_name(&format!("{NEW} 2"), NEW));
    assert!(!has_name(OLD, NEW));
    assert!(!has_name("2026-09-22 1400 budget review", NEW));
}
