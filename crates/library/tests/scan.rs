use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::DateTime;
use library::scan::{
    Known, LibraryScan, MARKER, RootState, ScanOptions, Unavailable, Waiting, scan,
};
use session_core::Segment;
use session_core::folder::{SessionDoc, fingerprint};

const TRANSCRIPT: &str = "# Transcript\n\n- [00:00:01] Speaker 1: Hello.\n";
const SUMMARY: &str = "---\nstatus: stub\nfailed_stage: summarize\n---\n\nNo summary.\n";

fn options() -> ScanOptions {
    ScanOptions {
        read_timeout: Duration::from_millis(300),
    }
}

fn doc(id: &str) -> SessionDoc {
    let start = DateTime::parse_from_rfc3339("2026-09-25T09:00:00-05:00").unwrap();
    let segment = Segment {
        index: 0,
        offset_ms: 0,
        duration_ms: 60_000,
        wall_start: start,
        wall_end: start + chrono::TimeDelta::minutes(1),
        audio_file: "audio/segment-01.opus".into(),
        channels: 1,
    };
    let mut doc = SessionDoc::new(id, None, &[segment]).unwrap();
    doc.title = Some("Planning".into());
    doc.transcript_fingerprint = Some(fingerprint(TRANSCRIPT.as_bytes()));
    doc.summary_fingerprint = Some(fingerprint(SUMMARY.as_bytes()));
    doc
}

/// A complete note folder, as a save leaves it.
fn note(root: &Path, name: &str, id: &str) -> PathBuf {
    let dir = root.join(name);
    fs::create_dir_all(dir.join("audio")).unwrap();
    fs::write(dir.join("session.md"), doc(id).render()).unwrap();
    fs::write(dir.join("transcript.md"), TRANSCRIPT).unwrap();
    fs::write(dir.join("summary.md"), SUMMARY).unwrap();
    fs::write(dir.join("notes.md"), "My notes\n").unwrap();
    fs::write(dir.join("audio/segment-01.opus"), b"opus").unwrap();
    dir
}

fn marked_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(MARKER), "").unwrap();
    root
}

fn run(root: &Path, known: &Known) -> LibraryScan {
    scan(&[root.to_path_buf()], known, &options())
}

fn root_state(result: &LibraryScan) -> &RootState {
    &result.roots[0].state
}

#[test]
fn a_complete_folder_lists_with_nothing_waiting() {
    let root = marked_root();
    let dir = note(root.path(), "2026-09-25 0900 Planning", "A1");
    let result = run(root.path(), &Known::default());
    assert_eq!(root_state(&result), &RootState::Available);
    assert_eq!(result.notes.len(), 1);
    let found = &result.notes[0];
    assert_eq!(found.path, dir);
    assert_eq!(found.root, root.path());
    assert_eq!(found.doc.session_id, "A1");
    assert!(found.waiting.is_empty(), "{:?}", found.waiting);
    assert!(found.conflict_copies.is_empty());
    assert!(found.duplicate_folders.is_empty());
    assert_eq!(
        found.transcript_fingerprint.as_deref(),
        Some(fingerprint(TRANSCRIPT.as_bytes()).as_str())
    );
}

// Covers AE7.
#[test]
fn a_folder_without_notes_md_lists_with_notes_waiting() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    fs::remove_file(dir.join("notes.md")).unwrap();
    let result = run(root.path(), &Known::default());
    assert_eq!(result.notes.len(), 1);
    assert_eq!(result.notes[0].waiting, vec![Waiting::Notes]);
}

#[test]
fn missing_documents_and_audio_are_waiting() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    fs::remove_file(dir.join("transcript.md")).unwrap();
    fs::remove_file(dir.join("summary.md")).unwrap();
    fs::remove_file(dir.join("audio/segment-01.opus")).unwrap();
    let result = run(root.path(), &Known::default());
    assert_eq!(
        result.notes[0].waiting,
        vec![
            Waiting::Transcript,
            Waiting::Summary,
            Waiting::Audio("audio/segment-01.opus".into()),
        ]
    );
}

#[test]
fn a_transcript_with_another_fingerprint_is_waiting() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    fs::write(dir.join("transcript.md"), "# Transcript\n\nOther text.\n").unwrap();
    fs::write(dir.join("summary.md"), "another summary").unwrap();
    let result = run(root.path(), &Known::default());
    assert_eq!(
        result.notes[0].waiting,
        vec![Waiting::Transcript, Waiting::Summary]
    );
}

#[test]
fn a_session_md_without_fingerprints_accepts_any_transcript() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    let mut older = doc("A1");
    older.transcript_fingerprint = None;
    older.summary_fingerprint = None;
    fs::write(dir.join("session.md"), older.render()).unwrap();
    fs::write(dir.join("transcript.md"), "# Transcript\n").unwrap();
    let result = run(root.path(), &Known::default());
    assert!(result.notes[0].waiting.is_empty());
}

#[test]
fn a_folder_with_only_audio_is_not_listed() {
    let root = marked_root();
    let dir = root.path().join("Audio only");
    fs::create_dir_all(dir.join("audio")).unwrap();
    fs::write(dir.join("audio/segment-01.opus"), b"opus").unwrap();
    let result = run(root.path(), &Known::default());
    assert!(result.notes.is_empty());
    assert!(result.unreadable.is_empty());
    assert!(result.invalid.is_empty());
}

#[test]
fn a_folder_with_an_invalid_session_md_is_not_a_note() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    fs::write(dir.join("session.md"), "---\nsession_id: [\n---\n").unwrap();
    let result = run(root.path(), &Known::default());
    assert!(result.notes.is_empty());
    assert_eq!(result.invalid.len(), 1);
    assert_eq!(result.invalid[0].path, dir);
}

#[test]
fn hidden_folders_and_files_are_skipped() {
    let root = marked_root();
    note(root.path(), &session_save::naming::staging_name("S1"), "S1");
    note(
        root.path(),
        &session_save::rename_home::temp_name("R1"),
        "R1",
    );
    note(root.path(), ".Planning.tmp", "T1");
    fs::write(root.path().join(".DS_Store"), "").unwrap();
    fs::write(root.path().join("loose.md"), "not a folder").unwrap();
    let result = run(root.path(), &Known::default());
    assert!(result.notes.is_empty(), "{:?}", result.notes);
}

// Covers AE8.
#[test]
fn each_services_conflict_copy_gives_a_notice() {
    let copies = [
        "notes (1).md",
        "notes (Erv's conflicted copy 2026-09-25).md",
        "notes-DESKTOP-ABC.md",
        "notes 2.md",
        "session (1).md",
        "transcript-LAPTOP.md",
        "summary 3.md",
        "notes (Longhand conflict 2026-09-25 1015).md",
    ];
    for copy in copies {
        let root = marked_root();
        let dir = note(root.path(), "Planning", "A1");
        fs::write(dir.join(copy), "other version").unwrap();
        let result = run(root.path(), &Known::default());
        assert_eq!(
            result.notes[0].conflict_copies,
            vec![dir.join(copy)],
            "{copy}"
        );
    }
}

#[test]
fn other_files_are_not_conflict_copies() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    for name in [
        "notes.txt",
        "agenda.md",
        "notesX.md",
        "notes (1).txt",
        ".notes.md.0123456789abcdef.tmp",
    ] {
        fs::write(dir.join(name), "x").unwrap();
    }
    let result = run(root.path(), &Known::default());
    assert!(
        result.notes[0].conflict_copies.is_empty(),
        "{:?}",
        result.notes[0].conflict_copies
    );
}

#[test]
fn a_duplicate_id_prefers_the_home_path() {
    let root = marked_root();
    let first = note(root.path(), "Planning", "A1");
    let second = note(root.path(), "Planning (1)", "A1");
    let mut known = Known::default();
    known.add("A1", &second);
    let result = run(root.path(), &known);
    assert_eq!(result.notes.len(), 1);
    assert_eq!(result.notes[0].path, second);
    assert_eq!(result.notes[0].duplicate_folders, vec![first]);
}

#[test]
fn a_duplicate_id_prefers_the_name_without_a_conflict_pattern() {
    let root = marked_root();
    let copy = note(
        root.path(),
        "Planning (Erv's conflicted copy 2026-09-25)",
        "A1",
    );
    std::thread::sleep(Duration::from_millis(50));
    let real = note(root.path(), "Planning", "A1");
    let result = run(root.path(), &Known::default());
    assert_eq!(result.notes.len(), 1);
    assert_eq!(result.notes[0].path, real);
    assert_eq!(result.notes[0].duplicate_folders, vec![copy]);
}

#[test]
fn a_duplicate_id_otherwise_prefers_the_older_folder() {
    let root = marked_root();
    let older = note(root.path(), "Planning", "A1");
    std::thread::sleep(Duration::from_millis(50));
    let newer = note(root.path(), "Retro", "A1");
    let result = run(root.path(), &Known::default());
    assert_eq!(result.notes.len(), 1);
    assert_eq!(result.notes[0].path, older);
    assert_eq!(result.notes[0].duplicate_folders, vec![newer]);
}

#[test]
fn different_ids_are_different_notes_whatever_the_name() {
    let root = marked_root();
    note(root.path(), "Planning", "A1");
    note(root.path(), "Planning (1)", "B2");
    let result = run(root.path(), &Known::default());
    assert_eq!(result.notes.len(), 2);
    assert!(result.notes.iter().all(|n| n.duplicate_folders.is_empty()));
}

#[test]
fn a_missing_root_is_unavailable() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("gone");
    let result = run(&root, &Known::default());
    assert_eq!(
        root_state(&result),
        &RootState::Unavailable(Unavailable::Missing)
    );
    assert!(!result.is_available(&root));
}

#[test]
fn a_root_that_is_a_file_is_unavailable() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("file");
    fs::write(&root, "").unwrap();
    let result = run(&root, &Known::default());
    assert_eq!(
        root_state(&result),
        &RootState::Unavailable(Unavailable::NotAFolder)
    );
}

#[test]
fn a_root_without_its_marker_and_with_saved_rows_is_unavailable() {
    let root = tempfile::tempdir().unwrap();
    let mut known = Known::default();
    known.add("A1", &root.path().join("Planning"));
    let result = run(root.path(), &known);
    assert_eq!(
        root_state(&result),
        &RootState::Unavailable(Unavailable::NoMarker)
    );
    assert!(!root.path().join(MARKER).exists());
}

#[test]
fn a_root_without_its_marker_and_with_note_folders_gets_the_marker() {
    let root = tempfile::tempdir().unwrap();
    note(root.path(), "Planning", "A1");
    let mut known = Known::default();
    known.add("A1", &root.path().join("Planning"));
    let result = run(root.path(), &known);
    assert_eq!(root_state(&result), &RootState::Available);
    assert!(result.roots[0].marker_written);
    assert!(root.path().join(MARKER).exists());
    assert_eq!(result.notes.len(), 1);
}

#[test]
fn a_root_without_its_marker_and_without_saved_rows_gets_the_marker() {
    let root = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let mut known = Known::default();
    // A row in another Destination does not count.
    known.add("A1", &other.path().join("Planning"));
    let result = run(root.path(), &known);
    assert!(result.roots[0].marker_written);
    assert_eq!(root_state(&result), &RootState::Available);
    assert!(root.path().join(MARKER).exists());
}

#[test]
fn a_root_with_its_marker_and_no_note_folders_is_available_and_empty() {
    let root = marked_root();
    let mut known = Known::default();
    known.add("A1", &root.path().join("Planning"));
    let result = run(root.path(), &known);
    assert_eq!(root_state(&result), &RootState::Available);
    assert!(!result.roots[0].marker_written);
    assert!(result.is_available(root.path()));
    assert!(result.notes.is_empty());
}

#[test]
fn a_duplicate_id_across_two_roots_gives_one_note() {
    let a = marked_root();
    let b = marked_root();
    let home = note(a.path(), "Planning", "A1");
    let other = note(b.path(), "Planning", "A1");
    let mut known = Known::default();
    known.add("A1", &home);
    let result = scan(
        &[a.path().to_path_buf(), b.path().to_path_buf()],
        &known,
        &options(),
    );
    assert_eq!(result.notes.len(), 1);
    assert_eq!(result.notes[0].path, home);
    assert_eq!(result.notes[0].duplicate_folders, vec![other]);
}

#[cfg(unix)]
#[test]
fn a_session_md_that_cannot_be_read_is_unreadable_not_missing() {
    use std::os::unix::fs::PermissionsExt;
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    let session = dir.join("session.md");
    fs::set_permissions(&session, fs::Permissions::from_mode(0o000)).unwrap();
    let mut known = Known::default();
    known.add("A1", &dir);
    let result = run(root.path(), &known);
    fs::set_permissions(&session, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(result.notes.is_empty());
    assert_eq!(result.unreadable.len(), 1);
    assert_eq!(result.unreadable[0].path, dir);
    assert_eq!(result.unreadable[0].known_id.as_deref(), Some("A1"));
}

#[cfg(unix)]
fn fifo(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

/// Run `f` on another thread and fail when it does not finish in time, so
/// a read that blocks fails the test instead of stalling it.
#[cfg(unix)]
fn within<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(5))
        .expect("the scan blocked on a read")
}

#[cfg(unix)]
#[test]
fn a_session_md_read_that_blocks_times_out_as_unreadable() {
    let root = marked_root();
    let dir = root.path().join("Planning");
    fs::create_dir_all(&dir).unwrap();
    // A read of a FIFO with no writer blocks, as an online-only file that
    // does not download does.
    fifo(&dir.join("session.md"));
    let path = root.path().to_path_buf();
    let result = within(move || scan(&[path], &Known::default(), &options()));
    assert!(result.notes.is_empty());
    assert_eq!(result.unreadable.len(), 1);
    assert_eq!(result.unreadable[0].path, dir);
    assert_eq!(result.unreadable[0].known_id, None);
}

#[cfg(unix)]
#[test]
fn no_audio_file_is_opened() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    let audio = dir.join("audio/segment-01.opus");
    fs::remove_file(&audio).unwrap();
    // An open of a FIFO with no writer blocks, so a scan that opens the
    // audio does not finish.
    fifo(&audio);
    let path = root.path().to_path_buf();
    let result = within(move || scan(&[path], &Known::default(), &options()));
    assert_eq!(result.notes.len(), 1);
    assert!(result.notes[0].waiting.is_empty());
}
