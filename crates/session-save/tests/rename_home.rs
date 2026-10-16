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

/// A folder that another computer makes during a change of case: on a
/// case-insensitive volume it takes the new name and the old name, so
/// both moves from the temporary name are refused as taken. `other` gives
/// another error for the move back instead.
struct BothNamesTaken {
    temp: PathBuf,
    names: [PathBuf; 2],
    other: bool,
}

impl Fs for BothNamesTaken {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        RealFs.rename(from, to)
    }
    fn rename_new(&self, from: &Path, to: &Path) -> io::Result<()> {
        if from == self.temp && to == self.names[0] {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        if from == self.temp && to == self.names[1] {
            if self.other {
                return Err(io::Error::other("the disk went away"));
            }
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        RealFs.rename_new(from, to)
    }
}

#[test]
fn a_change_of_case_whose_names_are_both_taken_goes_to_a_free_name() {
    let dest = tempfile::tempdir().unwrap();
    let old = "2026-09-22 1400 budget review";
    let from = home(dest.path(), old);
    let temp = dest.path().join(temp_name("s1"));
    let fs = BothNamesTaken {
        temp: temp.clone(),
        names: [dest.path().join(NEW), from.clone()],
        other: false,
    };
    let (result, moves) = rename(&from, NEW, &fs);
    let suffixed = dest.path().join(format!("{NEW}-2"));
    assert_eq!(result.unwrap(), suffixed, "the folder gets a visible name");
    assert_eq!(entries(dest.path()), [format!("{NEW}-2")]);
    // The record of each move names the temporary name too.
    let last = moves.last().unwrap();
    assert_eq!(
        (&last.from, &last.to, last.temp.as_ref()),
        (&from, &suffixed, Some(&temp))
    );
}

#[test]
fn a_crash_after_a_change_of_case_reached_the_temporary_name_still_finds_the_folder() {
    let dest = tempfile::tempdir().unwrap();
    let old = "2026-09-22 1400 budget review";
    let from = home(dest.path(), old);
    let temp = dest.path().join(temp_name("s1"));
    let fs = BothNamesTaken {
        temp: temp.clone(),
        names: [dest.path().join(NEW), from.clone()],
        other: false,
    };
    let suffixed = dest.path().join(format!("{NEW}-2"));
    let mut moves = Vec::new();
    // The process stops after the move to the next name is recorded and
    // before it is taken.
    let stopped = catch_unwind(AssertUnwindSafe(|| {
        rename_home::rename(&from, NEW, "s1", &fs, &mut |m| {
            moves.push(m.clone());
            if m.to == suffixed {
                panic!("the process stops here");
            }
            Ok(())
        })
    }));
    assert!(stopped.is_err());
    assert_eq!(entries(dest.path()), [temp_name("s1")]);
    // At the next launch, the last record finds the folder at the
    // temporary name and finishes the move.
    assert_eq!(
        rename_home::resolve(moves.last().unwrap(), &RealFs).unwrap(),
        Resolved::Moved(suffixed.clone())
    );
    assert_eq!(entries(dest.path()), [format!("{NEW}-2")]);
}

#[test]
fn a_change_of_case_that_cannot_go_back_keeps_the_record_of_the_temporary_name() {
    let dest = tempfile::tempdir().unwrap();
    let old = "2026-09-22 1400 budget review";
    let from = home(dest.path(), old);
    let temp = dest.path().join(temp_name("s1"));
    let fs = BothNamesTaken {
        temp: temp.clone(),
        names: [dest.path().join(NEW), from.clone()],
        other: true,
    };
    let (result, moves) = rename(&from, NEW, &fs);
    let error = result.unwrap_err().to_string();
    assert!(error.contains(&temp_name("s1")), "{error}");
    assert_eq!(entries(dest.path()), [temp_name("s1")]);
    let last = moves.last().unwrap();
    assert_eq!(
        last.temp.as_ref(),
        Some(&temp),
        "the record names the folder"
    );
    // While the names stay taken, the record stays.
    assert!(rename_home::resolve(last, &fs).is_err());
    // Once the block is gone, the folder goes to its new name.
    assert_eq!(
        rename_home::resolve(last, &RealFs).unwrap(),
        Resolved::Moved(dest.path().join(NEW))
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

/// Move into `parent` and keep each recorded move.
fn move_into(
    from: &Path,
    parent: &Path,
    name: &str,
    fs: &dyn Fs,
) -> (Result<PathBuf, RenameError>, Vec<Move>) {
    let mut moves = Vec::new();
    let result = rename_home::rename_into(from, parent, name, "s1", fs, &mut |m| {
        moves.push(m.clone());
        Ok(())
    });
    (result, moves)
}

#[test]
fn a_move_into_another_folder_keeps_the_name_and_records_the_move() {
    let dest = tempfile::tempdir().unwrap();
    let from = home(dest.path(), OLD);
    let typed = dest.path().join("One on One");
    std::fs::create_dir(&typed).unwrap();

    let (result, moves) = move_into(&from, &typed, OLD, &RealFs);
    assert_eq!(result.unwrap(), typed.join(OLD));
    assert_eq!(entries(&typed), [OLD]);
    assert_eq!(entries(dest.path()), ["One on One"]);
    assert_eq!(
        moves,
        [Move {
            from: from.clone(),
            to: typed.join(OLD),
            temp: None,
        }]
    );
    assert!(typed.join(OLD).join("audio/segment-01.opus").exists());
}

#[test]
fn a_move_into_a_folder_with_the_same_name_takes_the_free_name() {
    // Covers R27.
    let dest = tempfile::tempdir().unwrap();
    let from = home(dest.path(), OLD);
    let typed = dest.path().join("Pathfinder");
    home(&typed, OLD);

    let (result, _) = move_into(&from, &typed, OLD, &RealFs);
    let suffixed = format!("{OLD}-2");
    assert_eq!(result.unwrap(), typed.join(&suffixed));
    assert_eq!(entries(&typed), [OLD.to_string(), suffixed]);
    // The other folder keeps its files.
    assert!(typed.join(OLD).join("session.md").exists());
}

#[test]
fn a_move_out_of_a_type_folder_goes_to_the_parent() {
    let dest = tempfile::tempdir().unwrap();
    let typed = dest.path().join("Pathfinder");
    let from = home(&typed, OLD);

    let (result, _) = move_into(&from, dest.path(), OLD, &RealFs);
    assert_eq!(result.unwrap(), dest.path().join(OLD));
    assert!(entries(&typed).is_empty());
}

#[test]
fn a_move_into_the_same_folder_is_a_rename() {
    let dest = tempfile::tempdir().unwrap();
    let from = home(dest.path(), OLD);
    let (result, _) = move_into(&from, dest.path(), OLD, &RealFs);
    assert_eq!(result.unwrap(), from);
    assert_eq!(entries(dest.path()), [OLD]);
}

#[test]
fn a_move_into_another_folder_resolves_from_disk_after_a_crash() {
    let dest = tempfile::tempdir().unwrap();
    let from = home(dest.path(), OLD);
    let typed = dest.path().join("One on One");
    std::fs::create_dir(&typed).unwrap();
    let fs = StopsAfter {
        moves: Cell::new(0),
    };
    let mut moves = Vec::new();
    let stopped = catch_unwind(AssertUnwindSafe(|| {
        rename_home::rename_into(&from, &typed, OLD, "s1", &fs, &mut |m| {
            moves.push(m.clone());
            Ok(())
        })
    }));
    assert!(stopped.is_err());
    assert_eq!(
        rename_home::resolve(&moves[0], &RealFs).unwrap(),
        Resolved::NotMoved
    );
    std::fs::rename(&from, typed.join(OLD)).unwrap();
    assert_eq!(
        rename_home::resolve(&moves[0], &RealFs).unwrap(),
        Resolved::Moved(typed.join(OLD))
    );
}
