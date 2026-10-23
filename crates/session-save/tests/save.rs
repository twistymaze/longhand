//! Saving step by step, with renames that can be made to cross devices.

use std::cell::Cell;
use std::io;
use std::path::{Path, PathBuf};

use session_save::{Fs, Job, LEFT_BEHIND, MARKER, RealFs, SaveError, SaveState, step};

const NAME: &str = "2026-09-22 1400 Longhand session";

/// A working directory as processing leaves it.
fn working(root: &Path) -> PathBuf {
    let dir = root.join("work").join("s1");
    std::fs::create_dir_all(dir.join("audio")).unwrap();
    std::fs::create_dir_all(dir.join("proc")).unwrap();
    std::fs::create_dir_all(dir.join("audio-recovery")).unwrap();
    std::fs::write(dir.join("audio/segment-01.opus"), vec![7u8; 300_000]).unwrap();
    std::fs::write(dir.join("proc/segment-01.wav"), vec![1u8; 900_000]).unwrap();
    std::fs::write(dir.join("audio-recovery/chunk.wav"), vec![2u8; 900_000]).unwrap();
    for f in ["session.md", "transcript.md", "notes.md", "summary.md"] {
        std::fs::write(dir.join(f), format!("# {f}\n")).unwrap();
    }
    dir
}

fn destination(root: &Path) -> PathBuf {
    let dir = root.join("vault");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Renames that report crossing devices until told otherwise.
struct CrossDevice {
    cross: Cell<bool>,
}

impl Fs for CrossDevice {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        if self.cross.get() {
            return Err(io::Error::from(io::ErrorKind::CrossesDevices));
        }
        RealFs.rename(from, to)
    }
    fn rename_new(&self, from: &Path, to: &Path) -> io::Result<()> {
        RealFs.rename_new(from, to)
    }
}

fn run(job: &Job, fs: &dyn Fs, mut state: SaveState) -> Vec<SaveState> {
    let mut seen = vec![state.clone()];
    while !matches!(state, SaveState::Done { .. }) {
        state = step(&state, job, fs).unwrap();
        seen.push(state.clone());
    }
    seen
}

fn saved_files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in walk(dir) {
        out.push(
            entry
                .strip_prefix(dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/"),
        );
    }
    out.sort();
    out
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        if e.file_type().unwrap().is_dir() {
            out.extend(walk(&e.path()));
        } else {
            out.push(e.path());
        }
    }
    out
}

const EXPECTED: [&str; 5] = [
    "audio/segment-01.opus",
    "notes.md",
    "session.md",
    "summary.md",
    "transcript.md",
];

#[test]
fn a_same_volume_save_moves_once_and_leaves_the_raw_audio_behind() {
    let root = tempfile::tempdir().unwrap();
    let (work, dest) = (working(root.path()), destination(root.path()));
    let job = Job {
        note_id: "s1",
        working: &work,
        destination: &dest,
        folder_name: NAME,
    };

    // A watcher polling the destination sees nothing but hidden names and
    // the final one.
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watcher = {
        let (dest, stop) = (dest.clone(), std::sync::Arc::clone(&stop));
        std::thread::spawn(move || {
            let mut seen = std::collections::BTreeSet::new();
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                for e in std::fs::read_dir(&dest).unwrap().flatten() {
                    seen.insert(e.file_name().to_string_lossy().to_string());
                }
            }
            seen
        })
    };
    let states = run(&job, &RealFs, SaveState::Working);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let seen = watcher.join().unwrap();
    assert!(
        seen.iter().all(|n| n.starts_with('.') || n == NAME),
        "{seen:?}"
    );

    assert!(matches!(states[1], SaveState::Staged { copied: false, .. }));
    let path = dest.join(NAME);
    assert_eq!(states.last(), Some(&SaveState::Done { path: path.clone() }));
    assert!(!work.exists());
    assert_eq!(saved_files(&path), EXPECTED);
    for gone in LEFT_BEHIND {
        assert!(!path.join(gone).exists());
    }
    assert!(!path.join(MARKER).exists());
    let names: Vec<_> = std::fs::read_dir(&dest).unwrap().flatten().collect();
    assert_eq!(names.len(), 1, "no staging left behind");
}

#[test]
fn across_volumes_the_copy_is_verified_and_the_working_copy_goes_only_after_the_save() {
    let root = tempfile::tempdir().unwrap();
    let (work, dest) = (working(root.path()), destination(root.path()));
    let job = Job {
        note_id: "s1",
        working: &work,
        destination: &dest,
        folder_name: NAME,
    };
    let fs = CrossDevice {
        cross: Cell::new(true),
    };
    let states = run(&job, &fs, SaveState::Working);
    assert!(matches!(states[1], SaveState::Copying { .. }));
    assert!(matches!(states[2], SaveState::Staged { copied: true, .. }));
    let SaveState::Saved {
        working_retained: true,
        ..
    } = &states[3]
    else {
        panic!("{states:?}");
    };
    assert!(!work.exists());
    let path = dest.join(NAME);
    assert_eq!(saved_files(&path), EXPECTED);
    assert_eq!(
        std::fs::read(path.join("audio/segment-01.opus")).unwrap(),
        vec![7u8; 300_000]
    );
}

#[test]
fn a_kill_between_the_verified_copy_and_the_final_rename_keeps_both_copies_and_resumes() {
    let root = tempfile::tempdir().unwrap();
    let (work, dest) = (working(root.path()), destination(root.path()));
    let job = Job {
        note_id: "s1",
        working: &work,
        destination: &dest,
        folder_name: NAME,
    };
    let fs = CrossDevice {
        cross: Cell::new(true),
    };
    let copying = step(&SaveState::Working, &job, &fs).unwrap();
    let staged = step(&copying, &job, &fs).unwrap();
    // The process dies here; the recorded state is `staged`.
    assert!(work.join("audio/segment-01.opus").exists());
    let SaveState::Staged { staging, .. } = &staged else {
        panic!("{staged:?}");
    };
    assert!(staging.join("audio/segment-01.opus").exists());

    // Relaunch: resume from the record, and nothing is lost on the way.
    let states = run(&job, &RealFs, staged);
    assert!(matches!(states.last(), Some(SaveState::Done { .. })));
    assert_eq!(saved_files(&dest.join(NAME)), EXPECTED);
    assert!(!work.exists());
}

#[test]
fn a_final_rename_that_happened_before_its_record_is_found_by_its_marker() {
    for cross in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (work, dest) = (working(root.path()), destination(root.path()));
        let job = Job {
            note_id: "s1",
            working: &work,
            destination: &dest,
            folder_name: NAME,
        };
        let fs = CrossDevice {
            cross: Cell::new(cross),
        };
        let mut staged = step(&SaveState::Working, &job, &fs).unwrap();
        if matches!(staged, SaveState::Copying { .. }) {
            staged = step(&staged, &job, &fs).unwrap();
        }
        assert!(matches!(staged, SaveState::Staged { .. }), "{staged:?}");
        let saved = step(&staged, &job, &fs).unwrap();
        // The process dies here; the recorded state is still `staged`.
        let again = step(&staged, &job, &RealFs).unwrap();
        assert_eq!(again, saved, "cross-volume: {cross}");

        let states = run(&job, &RealFs, again);
        assert!(matches!(states.last(), Some(SaveState::Done { .. })));
        assert_eq!(saved_files(&dest.join(NAME)), EXPECTED);
        assert!(!dest.join(NAME).join(MARKER).exists());
        assert!(!dest.join(format!("{NAME}-2")).exists());
        assert!(!work.exists());
    }
}

#[test]
fn a_partial_copy_left_by_a_crash_is_this_sessions_and_is_redone() {
    let root = tempfile::tempdir().unwrap();
    let (work, dest) = (working(root.path()), destination(root.path()));
    let job = Job {
        note_id: "s1",
        working: &work,
        destination: &dest,
        folder_name: NAME,
    };
    let fs = CrossDevice {
        cross: Cell::new(true),
    };
    let copying = step(&SaveState::Working, &job, &fs).unwrap();
    let SaveState::Copying { staging } = &copying else {
        panic!("{copying:?}");
    };
    std::fs::create_dir_all(staging.join("audio")).unwrap();
    std::fs::write(staging.join("audio/segment-01.opus"), b"half").unwrap();
    let states = run(&job, &fs, copying);
    assert!(matches!(states.last(), Some(SaveState::Done { .. })));
    assert_eq!(
        std::fs::read(dest.join(NAME).join("audio/segment-01.opus"))
            .unwrap()
            .len(),
        300_000
    );
}

#[test]
fn this_sessions_leftover_staging_does_not_stop_a_later_save() {
    // A copy that failed, and whose cleanup failed too (a file held open on
    // Windows, a share that went away), left this session's staging. The
    // save then starts over from the working copy, which is whole.
    let root = tempfile::tempdir().unwrap();
    let (work, dest) = (working(root.path()), destination(root.path()));
    let staging = dest.join(".longhand-staging-s1");
    std::fs::create_dir_all(staging.join("audio")).unwrap();
    std::fs::write(staging.join(MARKER), "s1").unwrap();
    std::fs::write(staging.join("audio/segment-01.opus"), b"half").unwrap();
    let job = Job {
        note_id: "s1",
        working: &work,
        destination: &dest,
        folder_name: NAME,
    };
    let fs = CrossDevice {
        cross: Cell::new(true),
    };
    let states = run(&job, &fs, SaveState::Working);
    assert_eq!(
        states.last(),
        Some(&SaveState::Done {
            path: dest.join(NAME)
        })
    );
    assert_eq!(
        std::fs::read(dest.join(NAME).join("audio/segment-01.opus"))
            .unwrap()
            .len(),
        300_000
    );
    assert!(!staging.exists());
}

#[cfg(unix)]
#[test]
fn a_copy_that_fails_part_way_leaves_staging_that_names_its_session() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let work = working(root.path());
    let dest = destination(root.path());
    // A working copy with no marker yet, and a file that cannot be read.
    let locked = work.join("transcript.md");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let staging = dest.join(".longhand-staging-s1");
    let copied = session_save::stage::copy_staging(&work, &staging, "s1", LEFT_BEHIND);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(copied.is_err());
    assert_eq!(
        std::fs::read_to_string(staging.join(MARKER)).unwrap(),
        "s1",
        "a partial copy names its session"
    );
}

#[test]
fn a_rename_that_happened_before_its_record_is_recognised_by_its_marker() {
    let root = tempfile::tempdir().unwrap();
    let (work, dest) = (working(root.path()), destination(root.path()));
    let job = Job {
        note_id: "s1",
        working: &work,
        destination: &dest,
        folder_name: NAME,
    };
    let staged = step(&SaveState::Working, &job, &RealFs).unwrap();
    // The record of `staged` is lost: the store still says `working`.
    let again = step(&SaveState::Working, &job, &RealFs).unwrap();
    assert_eq!(again, staged);
}

#[test]
fn a_taken_name_gets_a_suffix_and_an_empty_folder_is_never_replaced() {
    let root = tempfile::tempdir().unwrap();
    let (work, dest) = (working(root.path()), destination(root.path()));
    std::fs::create_dir_all(dest.join(NAME)).unwrap();
    std::fs::create_dir_all(dest.join(format!("{NAME}-2")).join("x")).unwrap();
    let job = Job {
        note_id: "s1",
        working: &work,
        destination: &dest,
        folder_name: NAME,
    };
    let states = run(&job, &RealFs, SaveState::Working);
    assert_eq!(
        states.last(),
        Some(&SaveState::Done {
            path: dest.join(format!("{NAME}-3"))
        })
    );
    assert!(std::fs::read_dir(dest.join(NAME)).unwrap().next().is_none());
}

/// Refuses the final rename as in use (`PermissionDenied`, as Windows does
/// while another app holds a file in the folder) `refusals` times.
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
fn a_final_rename_refused_for_a_moment_as_in_use_keeps_the_name() {
    let root = tempfile::tempdir().unwrap();
    let (work, dest) = (working(root.path()), destination(root.path()));
    let job = Job {
        note_id: "s1",
        working: &work,
        destination: &dest,
        folder_name: NAME,
    };
    let fs = InUse::new(2);
    let states = run(&job, &fs, SaveState::Working);
    assert_eq!(
        states.last(),
        Some(&SaveState::Done {
            path: dest.join(NAME)
        }),
        "a folder in use is not a taken name"
    );
    assert_eq!(fs.calls.get(), 3);
}

#[test]
fn a_final_rename_that_stays_in_use_says_so_and_tries_no_other_name() {
    let root = tempfile::tempdir().unwrap();
    let (work, dest) = (working(root.path()), destination(root.path()));
    let job = Job {
        note_id: "s1",
        working: &work,
        destination: &dest,
        folder_name: NAME,
    };
    let staged = step(&SaveState::Working, &job, &RealFs).unwrap();
    let fs = InUse::new(usize::MAX);
    let err = step(&staged, &job, &fs).unwrap_err();
    assert!(err.to_string().contains("in use"), "{err}");
    assert!(!err.to_string().contains("no free name"), "{err}");
    assert_eq!(
        fs.calls.get(),
        1 + session_save::rename_home::IN_USE_RETRIES
    );
}

#[test]
fn a_staging_folder_this_session_did_not_make_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (work, dest) = (working(root.path()), destination(root.path()));
    std::fs::create_dir_all(dest.join(".longhand-staging-s1")).unwrap();
    let job = Job {
        note_id: "s1",
        working: &work,
        destination: &dest,
        folder_name: NAME,
    };
    let err = step(&SaveState::Working, &job, &RealFs).unwrap_err();
    assert!(matches!(err, SaveError::StagingTaken(_)), "{err:?}");
    assert!(work.join("audio/segment-01.opus").exists());
}

#[test]
fn a_missing_destination_waits_with_the_session_whole_in_the_working_location() {
    let root = tempfile::tempdir().unwrap();
    let work = working(root.path());
    let dest = root.path().join("unplugged");
    let job = Job {
        note_id: "s1",
        working: &work,
        destination: &dest,
        folder_name: NAME,
    };
    let err = step(&SaveState::Working, &job, &RealFs).unwrap_err();
    let SaveError::Destination { problem, .. } = &err else {
        panic!("{err:?}");
    };
    assert_eq!(problem, "is missing");
    // The note page shows this text: the UI calls the folder "storage"
    // (KTD13).
    assert_eq!(
        err.to_string(),
        format!("the storage folder {} is missing", dest.display())
    );
    assert!(work.join("proc/segment-01.wav").exists());
}

#[test]
fn folder_names_follow_the_session_start() {
    let started = chrono::DateTime::parse_from_rfc3339("2026-09-22T14:05:09-05:00").unwrap();
    assert_eq!(
        session_save::naming::folder_name(started, None),
        "2026-09-22 1405 Longhand session"
    );
}

#[test]
fn a_staged_copy_is_saved_beside_itself_when_the_destination_has_changed() {
    let root = tempfile::tempdir().unwrap();
    let work = working(root.path());
    let first = destination(root.path());
    let job = Job {
        note_id: "s1",
        working: &work,
        destination: &first,
        folder_name: NAME,
    };
    let staged = step(&SaveState::Working, &job, &RealFs).unwrap();
    assert!(matches!(staged, SaveState::Staged { .. }));

    // The user chose another folder while the session waited; the staged
    // copy is the complete one, so it completes where it is (R22b).
    let other = root.path().join("elsewhere");
    std::fs::create_dir(&other).unwrap();
    let moved = Job {
        destination: &other,
        ..job
    };
    let SaveState::Saved { path, .. } = step(&staged, &moved, &RealFs).unwrap() else {
        panic!("expected the staged copy to be saved");
    };
    assert_eq!(path.parent(), Some(first.as_path()));
    assert!(std::fs::read_dir(&other).unwrap().next().is_none());
}

#[test]
fn the_launch_sweep_removes_only_staging_that_completed_sessions_left() {
    let root = tempfile::tempdir().unwrap();
    for name in [
        ".longhand-staging-done",
        ".longhand-staging-waiting",
        "2026-09-22 1400 Longhand session",
    ] {
        std::fs::create_dir(root.path().join(name)).unwrap();
        std::fs::write(root.path().join(name).join("session.md"), "x").unwrap();
    }

    let removed = session_save::sweep_staging(root.path(), |id| id == "done").unwrap();

    assert_eq!(removed, vec![root.path().join(".longhand-staging-done")]);
    assert!(root.path().join(".longhand-staging-waiting").exists());
    assert!(
        root.path()
            .join("2026-09-22 1400 Longhand session")
            .exists()
    );
    assert!(
        session_save::sweep_staging(&root.path().join("unmounted"), |_| true)
            .unwrap()
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn a_read_only_destination_waits_naming_its_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let work = working(root.path());
    let dest = destination(root.path());
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o555)).unwrap();
    let job = Job {
        note_id: "s1",
        working: &work,
        destination: &dest,
        folder_name: NAME,
    };
    let err = step(&SaveState::Working, &job, &RealFs).unwrap_err();
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).unwrap();
    let SaveError::Destination { problem, .. } = &err else {
        panic!("{err:?}");
    };
    assert!(problem.starts_with("is not writable"), "{problem}");
    assert!(work.join("proc/segment-01.wav").exists());
}
