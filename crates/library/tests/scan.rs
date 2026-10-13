use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::DateTime;
use library::scan::{
    DocumentState, Known, LibraryScan, RootState, ScanOptions, Unavailable, Waiting, scan,
    scan_folders,
};
use session_core::Segment;
use session_core::folder::{SessionDoc, fingerprint};

const TRANSCRIPT: &str = "# Transcript\n\n- [00:00:01] Speaker 1: Hello.\n";
const SUMMARY: &str = "---\nstatus: stub\nfailed_stage: summarize\n---\n\nNo summary.\n";

/// The options of the app, with its read time limit. A shorter limit fails
/// a read of a new file on a busy Windows runner, where the antivirus scans
/// the file when it is first opened.
fn options() -> ScanOptions {
    ScanOptions::default()
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
        ends_in_pause: false,
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
    session_save::note_files::write_doc(&dir, &doc(id)).unwrap();
    fs::write(dir.join("transcript.md"), TRANSCRIPT).unwrap();
    fs::write(dir.join("summary.md"), SUMMARY).unwrap();
    fs::write(dir.join("notes.md"), "My notes\n").unwrap();
    fs::write(dir.join("audio/segment-01.opus"), b"opus").unwrap();
    dir
}

fn marked_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    library::scan::write_marker(root.path());
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

#[test]
fn a_note_with_old_result_copies_reads_only_its_note_files() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    // An older build named a copy of the result in `session.md`. The scan
    // reads the note files and ignores the copies.
    let session = doc("A1").render().replacen(
        "---\n",
        &format!("---\nresult_generation:\n  id: committed-1\n  files:\n    transcript.md: {}\n    summary.md: {}\n",
            fingerprint(b"other transcript"), fingerprint(b"other summary")),
        1,
    );
    fs::write(dir.join("session.md"), session).unwrap();
    let result = run(root.path(), &Known::default());
    assert!(
        result.notes[0].waiting.is_empty(),
        "{:?}",
        result.notes[0].waiting
    );

    fs::create_dir_all(dir.join(".longhand-results/committed-1")).unwrap();
    fs::write(
        dir.join(".longhand-results/committed-1/transcript.md"),
        "other transcript",
    )
    .unwrap();
    let result = run(root.path(), &Known::default());
    assert!(
        result.notes[0].waiting.is_empty(),
        "{:?}",
        result.notes[0].waiting
    );
    assert_eq!(
        result.notes[0].transcript_fingerprint.as_deref(),
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

// Covers R8: the reconciler tells a file that differs or is missing from
// one that cannot be read.
#[test]
fn the_scan_reports_the_state_of_each_result_document() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    let result = run(root.path(), &Known::default());
    assert_eq!(result.notes[0].transcript, DocumentState::Matches);
    assert_eq!(result.notes[0].summary, DocumentState::Matches);

    fs::write(dir.join("transcript.md"), "# Transcript\n\nOther text.\n").unwrap();
    fs::remove_file(dir.join("summary.md")).unwrap();
    let result = run(root.path(), &Known::default());
    assert_eq!(result.notes[0].transcript, DocumentState::Differs);
    assert_eq!(result.notes[0].summary, DocumentState::Missing);

    // A folder in place of the file: the read fails.
    fs::remove_file(dir.join("transcript.md")).unwrap();
    fs::create_dir(dir.join("transcript.md")).unwrap();
    let result = run(root.path(), &Known::default());
    assert_eq!(result.notes[0].transcript, DocumentState::Unreadable);
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
    session_save::note_files::write_doc(&dir, &older).unwrap();
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
fn a_duplicate_id_prefers_the_name_that_session_md_gives_over_the_home_path() {
    // The conflict copy synced first and became the home folder. The real
    // folder has the name that its start time and title give.
    let root = marked_root();
    let copy = note(root.path(), "2026-09-25 0900 Planning (1)", "A1");
    let real = note(root.path(), "2026-09-25 0900 Planning", "A1");
    let mut known = Known::default();
    known.add("A1", &copy);
    let result = run(root.path(), &known);
    assert_eq!(result.notes.len(), 1);
    assert_eq!(result.notes[0].path, real);
    assert_eq!(result.notes[0].duplicate_folders, vec![copy]);
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
    assert!(!library::scan::marker(root.path()).exists());
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
    assert!(library::scan::marker(root.path()).exists());
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
    assert!(library::scan::marker(root.path()).exists());
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

/// A root that lists its entries but cannot tell what one of them is gives
/// an incomplete list, so its missing notes are not deletes (KTD5).
#[cfg(unix)]
#[test]
fn a_root_whose_entries_cannot_be_read_is_unreadable() {
    use std::os::unix::fs::PermissionsExt;
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    let mut known = Known::default();
    known.add("A1", &dir);
    // Read but no search permission: the list works, the metadata does not.
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o444)).unwrap();
    let with_row = run(root.path(), &known);
    let without_row = run(root.path(), &Known::default());
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
    for result in [&with_row, &without_row] {
        assert!(
            matches!(
                root_state(result),
                RootState::Unavailable(Unavailable::Unreadable(_))
            ),
            "{:?}",
            root_state(result)
        );
        assert!(result.notes.is_empty());
    }
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
    // A short limit, so the read times out before `within` fails the test.
    let options = ScanOptions {
        read_timeout: Duration::from_millis(300),
        ..ScanOptions::default()
    };
    let result = within(move || scan(&[path], &Known::default(), &options));
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

fn folders(paths: &[&Path]) -> HashSet<PathBuf> {
    paths.iter().map(|p| p.to_path_buf()).collect()
}

#[test]
fn a_targeted_scan_reads_only_the_folders_it_names() {
    let root = marked_root();
    let planning = note(root.path(), "Planning", "A1");
    note(root.path(), "Budget", "B1");
    let staging = note(root.path(), ".staging-C1", "C1");
    let gone = root.path().join("Gone");
    let (result, read) = scan_folders(
        root.path(),
        &folders(&[&planning, &staging, &gone]),
        &Known::default(),
        &options(),
    )
    .unwrap();
    assert_eq!(root_state(&result), &RootState::Available);
    let ids: Vec<&str> = result
        .notes
        .iter()
        .map(|n| n.doc.session_id.as_str())
        .collect();
    assert_eq!(ids, ["A1"]);
    assert_eq!(read, folders(&[&planning, &staging, &gone]));
}

#[test]
fn a_targeted_scan_of_a_duplicate_also_reads_the_home_folder() {
    let root = marked_root();
    let home = note(root.path(), "Planning", "A1");
    let copy = note(root.path(), "Planning (1)", "A1");
    let mut known = Known::default();
    known.add("A1", &home);
    let (result, read) = scan_folders(root.path(), &folders(&[&copy]), &known, &options()).unwrap();
    assert_eq!(result.notes.len(), 1);
    assert_eq!(result.notes[0].path, home);
    assert_eq!(result.notes[0].duplicate_folders, [copy.as_path()]);
    assert_eq!(read, folders(&[&home, &copy]));
}

#[test]
fn a_targeted_scan_of_a_root_without_its_marker_gives_no_scan() {
    let root = tempfile::tempdir().unwrap();
    let planning = note(root.path(), "Planning", "A1");
    let targeted = scan_folders(
        root.path(),
        &folders(&[&planning]),
        &Known::default(),
        &options(),
    );
    assert!(targeted.is_none());
    assert!(!library::scan::marker(root.path()).exists());
    let missing = root.path().join("missing");
    assert!(scan_folders(&missing, &HashSet::new(), &Known::default(), &options()).is_none());
}

// ---------------------------------------------------------------- type folders (note types U3)

/// The ids and note types of the notes that `result` lists, sorted by id.
fn types(result: &LibraryScan) -> Vec<(String, Option<String>)> {
    let mut types: Vec<(String, Option<String>)> = result
        .notes
        .iter()
        .map(|n| (n.doc.session_id.clone(), n.note_type.clone()))
        .collect();
    types.sort();
    types
}

fn typed(id: &str, note_type: Option<&str>) -> (String, Option<String>) {
    (id.to_string(), note_type.map(str::to_string))
}

// Covers R8 (KTD4).
#[test]
fn a_note_in_a_type_folder_and_a_note_at_the_root_both_list_with_their_types() {
    let root = marked_root();
    let typed_home = note(
        &root.path().join("Pathfinder"),
        "2026-10-02 1900 Session 12",
        "A1",
    );
    let untyped = note(root.path(), "2026-10-07 0900 Standup", "B1");
    let result = run(root.path(), &Known::default());
    assert_eq!(root_state(&result), &RootState::Available);
    assert_eq!(
        types(&result),
        [typed("A1", Some("Pathfinder")), typed("B1", None)]
    );
    let a1 = result.notes.iter().find(|n| n.path == typed_home).unwrap();
    assert_eq!(a1.root, root.path());
    assert!(result.notes.iter().any(|n| n.path == untyped));
    // No automatic step writes because of a type folder.
    assert!(!root.path().join("Pathfinder/.longhand").exists());
}

#[test]
fn a_folder_with_only_system_files_lists_no_note_and_gets_no_marker() {
    let root = marked_root();
    let empty = root.path().join("Pathfinder");
    fs::create_dir(&empty).unwrap();
    fs::write(empty.join(".DS_Store"), "").unwrap();
    let result = run(root.path(), &Known::default());
    assert!(result.notes.is_empty(), "{:?}", result.notes);
    assert!(!empty.join(".longhand").exists());
}

#[test]
fn a_home_folder_two_levels_deep_or_in_a_hidden_folder_is_ignored() {
    let root = marked_root();
    note(&root.path().join("Pathfinder/Archive"), "Session 1", "A1");
    note(&root.path().join(".hidden"), "Session 2", "B1");
    // A folder inside a home folder is not a home folder.
    let home = note(root.path(), "Planning", "C1");
    note(&home, "Inner", "D1");
    let result = run(root.path(), &Known::default());
    assert_eq!(types(&result), [typed("C1", None)]);
}

// Covers R29 (KTD1).
#[test]
fn a_home_folder_that_a_person_moves_to_another_type_folder_gets_that_type() {
    let root = marked_root();
    let home = note(&root.path().join("Pathfinder"), "Session 12", "A1");
    let before = fs::read(home.join("session.md")).unwrap();
    fs::create_dir(root.path().join("One on One")).unwrap();
    let moved = root.path().join("One on One/Session 12");
    fs::rename(&home, &moved).unwrap();
    let result = run(root.path(), &Known::default());
    assert_eq!(types(&result), [typed("A1", Some("One on One"))]);
    assert_eq!(fs::read(moved.join("session.md")).unwrap(), before);
}

#[test]
fn a_root_without_its_marker_and_with_typed_rows_is_unavailable() {
    let root = tempfile::tempdir().unwrap();
    let mut known = Known::default();
    known.add("A1", &root.path().join("Pathfinder/Planning"));
    let result = run(root.path(), &known);
    assert_eq!(
        root_state(&result),
        &RootState::Unavailable(Unavailable::NoMarker)
    );
    assert!(!library::scan::marker(root.path()).exists());
}

#[test]
fn a_targeted_scan_reads_a_typed_home_folder() {
    let root = marked_root();
    let home = note(&root.path().join("Pathfinder"), "Session 12", "A1");
    note(&root.path().join("Pathfinder"), "Session 13", "B1");
    let (result, read) = scan_folders(
        root.path(),
        &folders(&[&home]),
        &Known::default(),
        &options(),
    )
    .unwrap();
    assert_eq!(types(&result), [typed("A1", Some("Pathfinder"))]);
    assert_eq!(read, folders(&[&home]));
}

#[test]
fn a_targeted_scan_of_a_type_folder_reads_its_notes() {
    let root = marked_root();
    let folder = root.path().join("D&D");
    let first = note(&folder, "Session 12", "A1");
    let second = note(&folder, "Session 13", "B1");
    let (result, read) = scan_folders(
        root.path(),
        &folders(&[&folder]),
        &Known::default(),
        &options(),
    )
    .unwrap();
    assert_eq!(
        types(&result),
        [typed("A1", Some("D&D")), typed("B1", Some("D&D"))]
    );
    assert_eq!(read, folders(&[&folder, &first, &second]));
    assert!(!folder.join(".longhand").exists());
}

/// A type change into a new type folder gives events on the type folder
/// and on the home folder in it. The targeted scan reads the home folder
/// one time, so the note is not another version of itself.
#[test]
fn a_targeted_scan_of_a_type_folder_and_its_home_folder_reads_the_note_once() {
    let root = marked_root();
    let folder = root.path().join("Work");
    let home = note(&folder, "Session 12", "A1");
    let mut known = Known::default();
    known.add("A1", &home);
    let (result, read) =
        scan_folders(root.path(), &folders(&[&folder, &home]), &known, &options()).unwrap();
    assert_eq!(types(&result), [typed("A1", Some("Work"))]);
    assert_eq!(result.notes[0].path, home);
    assert!(result.notes[0].duplicate_folders.is_empty());
    assert_eq!(read, folders(&[&folder, &home]));
}

/// A type folder whose own `session.md` is not valid still holds notes.
/// The scan reports the problem and reads the notes in it (KTD12).
#[test]
fn a_type_folder_with_an_invalid_session_md_still_lists_its_notes() {
    let root = marked_root();
    let folder = root.path().join("Pathfinder");
    note(&folder, "Session 12", "A1");
    fs::write(folder.join("session.md"), "not a session document").unwrap();
    let result = run(root.path(), &Known::default());
    assert_eq!(root_state(&result), &RootState::Available);
    assert_eq!(types(&result), [typed("A1", Some("Pathfinder"))]);
    assert_eq!(result.invalid.len(), 1);
    assert_eq!(result.invalid[0].path, folder);
}

/// As above, when the `session.md` of the type folder cannot be read.
#[cfg(unix)]
#[test]
fn a_type_folder_with_an_unreadable_session_md_still_lists_its_notes() {
    use std::os::unix::fs::PermissionsExt;
    let root = marked_root();
    let folder = root.path().join("Pathfinder");
    note(&folder, "Session 12", "A1");
    let session = folder.join("session.md");
    fs::write(&session, doc("X1").render()).unwrap();
    fs::set_permissions(&session, fs::Permissions::from_mode(0o000)).unwrap();
    let result = run(root.path(), &Known::default());
    fs::set_permissions(&session, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(types(&result), [typed("A1", Some("Pathfinder"))]);
    assert_eq!(result.unreadable.len(), 1);
    assert_eq!(result.unreadable[0].path, folder);
}

#[test]
fn a_targeted_scan_of_a_type_folder_with_an_invalid_session_md_reads_its_notes() {
    let root = marked_root();
    let folder = root.path().join("Pathfinder");
    let home = note(&folder, "Session 12", "A1");
    fs::write(folder.join("session.md"), "not a session document").unwrap();
    let (result, read) = scan_folders(
        root.path(),
        &folders(&[&folder]),
        &Known::default(),
        &options(),
    )
    .unwrap();
    assert_eq!(types(&result), [typed("A1", Some("Pathfinder"))]);
    assert_eq!(read, folders(&[&folder, &home]));
}

#[test]
fn the_place_of_a_home_folder_gives_its_root_and_type() {
    use library::scan::{Place, place_in, storage_place};
    let root = marked_root();
    let roots = [root.path().to_path_buf()];
    let typed_home = root.path().join("Pathfinder/Session 12");
    let untyped = root.path().join("Standup");
    let place = |note_type: Option<&str>| Place {
        root: root.path().to_path_buf(),
        note_type: note_type.map(str::to_string),
    };
    assert_eq!(
        place_in(&typed_home, &roots),
        Some(place(Some("Pathfinder")))
    );
    assert_eq!(place_in(&untyped, &roots), Some(place(None)));
    assert_eq!(place_in(&root.path().join("A/B/C"), &roots), None);
    assert_eq!(place_in(&root.path().join(".hidden/C"), &roots), None);
    assert_eq!(place_in(Path::new("/elsewhere/C"), &roots), None);
    // From the library marker, with no list of roots.
    assert_eq!(storage_place(&typed_home), Some(place(Some("Pathfinder"))));
    assert_eq!(storage_place(&untyped), Some(place(None)));
    let unmarked = tempfile::tempdir().unwrap();
    assert_eq!(storage_place(&unmarked.path().join("T/Session")), None);
    assert_eq!(storage_place(&unmarked.path().join("Session")), None);
}

// ---------------------------------------------------------------- removed audio (U2)

const AUDIO: &str = "audio/segment-01.opus";
const SECOND: &str = "audio/segment-02.opus";

/// Write the removal marker of `audio` in the note folder `dir`.
fn mark(dir: &Path, audio: &str) {
    fs::write(
        dir.join(session_core::folder::removal_marker(audio)),
        session_core::folder::REMOVAL_MARKER_TEXT,
    )
    .unwrap();
}

/// Add a second segment, with its audio, to the note in `dir`.
fn add_second_segment(dir: &Path) {
    let mut doc = session_core::note_folder::NoteFiles::read(dir).unwrap().doc;
    let mut second = doc.segments[0].clone();
    second.index = 1;
    second.offset_ms = 120_000;
    second.audio = SECOND.into();
    doc.segments.push(second);
    session_save::note_files::write_doc(dir, &doc).unwrap();
    fs::write(dir.join(SECOND), b"opus").unwrap();
}

// Covers R15.
#[test]
fn a_marker_and_no_audio_scans_as_complete_with_the_segment_removed() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    mark(&dir, AUDIO);
    fs::remove_file(dir.join(AUDIO)).unwrap();
    let result = run(root.path(), &Known::default());
    assert!(
        result.notes[0].waiting.is_empty(),
        "{:?}",
        result.notes[0].waiting
    );
    assert_eq!(result.notes[0].removed_audio, [AUDIO]);
}

// Covers KTD2.
#[test]
fn a_marker_with_the_audio_still_present_scans_as_removed() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    mark(&dir, AUDIO);
    let result = run(root.path(), &Known::default());
    assert!(result.notes[0].waiting.is_empty());
    assert_eq!(result.notes[0].removed_audio, [AUDIO]);
}

#[test]
fn a_missing_audio_file_with_no_marker_still_waits() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    fs::remove_file(dir.join(AUDIO)).unwrap();
    let result = run(root.path(), &Known::default());
    assert_eq!(result.notes[0].waiting, [Waiting::Audio(AUDIO.into())]);
    assert!(result.notes[0].removed_audio.is_empty());
}

#[test]
fn a_complete_folder_has_no_removed_audio() {
    let root = marked_root();
    note(root.path(), "Planning", "A1");
    let result = run(root.path(), &Known::default());
    assert!(result.notes[0].removed_audio.is_empty());
}

#[test]
fn only_the_marked_segment_stops_waiting() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    add_second_segment(&dir);
    mark(&dir, AUDIO);
    fs::remove_file(dir.join(AUDIO)).unwrap();
    fs::remove_file(dir.join(SECOND)).unwrap();
    let result = run(root.path(), &Known::default());
    // The sync has not delivered the second marker yet.
    assert_eq!(result.notes[0].waiting, [Waiting::Audio(SECOND.into())]);
    assert_eq!(result.notes[0].removed_audio, [AUDIO]);
}

#[test]
fn a_marker_is_not_a_conflict_copy() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    mark(&dir, AUDIO);
    let result = run(root.path(), &Known::default());
    assert!(result.notes[0].conflict_copies.is_empty());
}

#[test]
fn a_targeted_scan_reports_the_removed_audio() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    mark(&dir, AUDIO);
    fs::remove_file(dir.join(AUDIO)).unwrap();
    let (result, _) = scan_folders(
        root.path(),
        &folders(&[&dir]),
        &Known::default(),
        &options(),
    )
    .unwrap();
    assert!(result.notes[0].waiting.is_empty());
    assert_eq!(result.notes[0].removed_audio, [AUDIO]);
}

#[cfg(unix)]
#[test]
fn no_removal_marker_is_opened() {
    let root = marked_root();
    let dir = note(root.path(), "Planning", "A1");
    fs::remove_file(dir.join(AUDIO)).unwrap();
    // An open of a FIFO with no writer blocks, so a scan that opens the
    // marker does not finish.
    fifo(&dir.join(session_core::folder::removal_marker(AUDIO)));
    let path = root.path().to_path_buf();
    let result = within(move || scan(&[path], &Known::default(), &options()));
    assert!(result.notes[0].waiting.is_empty());
    assert_eq!(result.notes[0].removed_audio, [AUDIO]);
}

// ------------------------------------------- the early home folder (U3)

mod early_home {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use library::reconcile::{DELETE_WAIT, Reconciler, Target};
    use pipeline::early_home::{self, EarlyState};
    use session_core::Store;
    use session_save::RealFs;

    use super::{marked_root, options};

    fn never_claimed(_: &str) -> bool {
        false
    }

    fn at() -> chrono::DateTime<chrono::FixedOffset> {
        chrono::DateTime::parse_from_rfc3339("2026-09-25T09:00:00-05:00").unwrap()
    }

    fn shared() -> pipeline::SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().unwrap()))
    }

    fn recording(store: &pipeline::SharedStore, id: &str) {
        let mut s = store.lock().unwrap();
        s.create_session(id, at(), None).unwrap();
        early_home::begin(&s, id).unwrap();
    }

    #[test]
    fn this_computers_early_folder_is_not_adopted_as_a_second_row() {
        let root = marked_root();
        let store = shared();
        recording(&store, "placed");
        let home = early_home::write(&store, "placed", Some(root.path()), "run", &RealFs)
            .unwrap()
            .unwrap();
        // A crash after the rename and before its record: the folder is at
        // its name, and the row has no home path yet.
        recording(&store, "renamed");
        let state = early_home::state(&store.lock().unwrap(), "renamed")
            .unwrap()
            .unwrap();
        let filled =
            early_home::step(&store, "renamed", &state, Some(root.path()), "run", &RealFs).unwrap();
        early_home::record(&store, "renamed", &filled).unwrap();
        let placed = early_home::step(
            &store,
            "renamed",
            &filled,
            Some(root.path()),
            "run",
            &RealFs,
        )
        .unwrap();
        assert!(matches!(placed, EarlyState::Done { .. }));

        let mut reconciler = Reconciler::new(&options());
        reconciler
            .run(
                &store,
                &Target::Full(vec![root.path().to_path_buf()]),
                &never_claimed,
            )
            .unwrap();

        let s = store.lock().unwrap();
        assert_eq!(s.sessions().unwrap().len(), 2, "no row is adopted");
        let record = s.session("placed").unwrap().unwrap();
        assert_eq!(record.home_path.map(PathBuf::from), Some(home));
        assert_eq!(s.session("renamed").unwrap().unwrap().home_path, None);
    }

    #[test]
    fn a_row_whose_early_write_is_not_done_is_never_forgotten() {
        let root = marked_root();
        let store = shared();
        let name = "2026-09-25 0900 Longhand session";
        let missing = root.path().join(name);
        let set = |state: &EarlyState| {
            store
                .lock()
                .unwrap()
                .set_result_state(
                    "n",
                    early_home::STATE,
                    &serde_json::to_value(state).unwrap(),
                )
                .unwrap();
        };
        store
            .lock()
            .unwrap()
            .create_notes_only("n", at(), Some(&missing.to_string_lossy()))
            .unwrap();
        // The home path is recorded; the Done record is not (a crash).
        set(&EarlyState::Filled {
            root: root.path().to_path_buf(),
            folder_name: name.into(),
            note_type: None,
        });
        let target = Target::Full(vec![root.path().to_path_buf()]);
        let mut reconciler = Reconciler::new(&options());
        let start = Instant::now();
        let later = start + DELETE_WAIT + std::time::Duration::from_secs(1);
        reconciler
            .run_at(&store, &target, &never_claimed, start)
            .unwrap();
        let changes = reconciler
            .run_at(&store, &target, &never_claimed, later)
            .unwrap();

        assert!(changes.removed.is_empty());
        assert!(store.lock().unwrap().session("n").unwrap().is_some());

        // Once the early write is done, a folder that is gone is a delete.
        set(&EarlyState::Done { path: missing });
        let changes = reconciler
            .run_at(&store, &target, &never_claimed, later)
            .unwrap();
        assert_eq!(changes.removed, ["n"]);
    }
}

// ------------------------------- the result flag and the remote state (U4)

mod result_flag {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use chrono::{DateTime, FixedOffset, Local, TimeDelta};
    use library::scan::{Known, RemoteState, ScanOptions, Waiting, scan};
    use session_core::folder::{ResultFlag, SessionDoc};
    use session_save::in_use_marker::{FRESH_FOR, write_marker};

    use super::{marked_root, options};

    fn opened() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-09-25T09:00:00-05:00").unwrap()
    }

    /// The folder of a note with no result, as the early write leaves it:
    /// `session.md` and `notes.md` only.
    fn early_note(root: &Path, id: &str, result: Option<ResultFlag>) -> PathBuf {
        let dir = root.join(format!("2026-09-25 0900 {id}"));
        fs::create_dir_all(&dir).unwrap();
        let mut doc = SessionDoc::without_audio(id, opened(), None);
        doc.result = result;
        session_save::note_files::write_doc(&dir, &doc).unwrap();
        fs::write(dir.join("notes.md"), "Agenda\n").unwrap();
        dir
    }

    fn now() -> DateTime<FixedOffset> {
        Local::now().fixed_offset()
    }

    /// Make the file time of `longhand.json` in `dir` older than the time a
    /// marker stays fresh.
    fn age_records(dir: &Path) {
        let old = SystemTime::now() - FRESH_FOR.to_std().unwrap() - Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(dir.join("longhand.json"))
            .unwrap()
            .set_modified(old)
            .unwrap();
    }

    fn remote(root: &Path, own_run: Option<&str>) -> Option<RemoteState> {
        remote_with_runs(root, own_run.into_iter().collect())
    }

    fn remote_with_runs(root: &Path, own_runs: Vec<&str>) -> Option<RemoteState> {
        let options = ScanOptions {
            own_runs: own_runs.into_iter().map(str::to_string).collect(),
            ..options()
        };
        let result = scan(&[root.to_path_buf()], &Known::default(), &options);
        assert_eq!(result.notes.len(), 1);
        result.notes[0].remote
    }

    #[test]
    fn a_note_with_no_recording_waits_for_no_transcript_summary_or_audio() {
        let root = marked_root();
        early_note(root.path(), "n1", Some(ResultFlag::NoRecording));
        let result = scan(&[root.path().to_path_buf()], &Known::default(), &options());
        assert!(result.notes[0].waiting.is_empty(), "{:?}", result.notes[0]);
        assert_eq!(result.notes[0].remote, None);
    }

    #[test]
    fn a_note_with_no_field_no_segment_and_no_fingerprint_waits_for_nothing() {
        let root = marked_root();
        early_note(root.path(), "n1", None);
        let result = scan(&[root.path().to_path_buf()], &Known::default(), &options());
        assert!(result.notes[0].waiting.is_empty(), "{:?}", result.notes[0]);
    }

    #[test]
    fn a_note_whose_recording_is_not_saved_waits_for_no_result() {
        let root = marked_root();
        early_note(root.path(), "n1", Some(ResultFlag::NotSaved));
        let result = scan(&[root.path().to_path_buf()], &Known::default(), &options());
        assert!(result.notes[0].waiting.is_empty(), "{:?}", result.notes[0]);
    }

    #[test]
    fn a_note_with_no_result_still_waits_for_its_notes() {
        let root = marked_root();
        let dir = early_note(root.path(), "n1", Some(ResultFlag::NoRecording));
        fs::remove_file(dir.join("notes.md")).unwrap();
        let result = scan(&[root.path().to_path_buf()], &Known::default(), &options());
        assert_eq!(result.notes[0].waiting, [Waiting::Notes]);
    }

    #[test]
    fn a_fresh_marker_of_another_run_shows_the_note_in_use_whatever_the_flag() {
        let root = marked_root();
        let dir = early_note(root.path(), "n1", Some(ResultFlag::NoRecording));
        assert!(write_marker(&dir, "run-a", now()).unwrap());
        assert_eq!(remote(root.path(), Some("run-b")), Some(RemoteState::InUse));
        // The marker of this computer's own run does not count.
        assert_eq!(remote(root.path(), Some("run-a")), None);
    }

    #[test]
    fn a_saved_note_with_a_fresh_marker_of_another_run_is_in_use() {
        let root = marked_root();
        let dir = super::note(root.path(), "2026-09-25 0900 Planning", "A1");
        assert_eq!(remote(root.path(), Some("run-b")), None);
        assert!(write_marker(&dir, "run-a", now()).unwrap());
        assert_eq!(remote(root.path(), Some("run-b")), Some(RemoteState::InUse));
    }

    // A relaunch on the same computer: the run "run-a" quit while the note
    // processed, and its marker is still fresh. The next run "run-b"
    // finished the work. The note is not in use on another computer. A
    // fresh marker of another computer's run still counts.
    #[test]
    fn a_fresh_marker_of_an_earlier_run_on_this_computer_does_not_count() {
        let root = marked_root();
        let dir = super::note(root.path(), "2026-09-25 0900 Planning", "A1");
        assert!(write_marker(&dir, "run-a", now()).unwrap());
        assert_eq!(remote_with_runs(root.path(), vec!["run-b", "run-a"]), None);

        assert!(write_marker(&dir, "other-computer", now()).unwrap());
        assert_eq!(
            remote_with_runs(root.path(), vec!["run-b", "run-a"]),
            Some(RemoteState::InUse)
        );
    }

    #[test]
    fn a_new_note_that_is_not_saved_and_has_no_marker_yet_is_in_use() {
        let root = marked_root();
        early_note(root.path(), "n1", Some(ResultFlag::NotSaved));
        assert_eq!(remote(root.path(), None), Some(RemoteState::InUse));
    }

    // Covers AE13.
    #[test]
    fn an_old_note_that_is_not_saved_and_has_no_fresh_marker_is_not_finished() {
        let root = marked_root();
        let dir = early_note(root.path(), "n1", Some(ResultFlag::NotSaved));
        let old = now() - FRESH_FOR - TimeDelta::minutes(1);
        assert!(write_marker(&dir, "run-a", old).unwrap());
        age_records(&dir);
        assert_eq!(remote(root.path(), None), Some(RemoteState::NotFinished));

        // A fresh marker wins over the flag.
        assert!(write_marker(&dir, "run-a", now()).unwrap());
        assert_eq!(remote(root.path(), None), Some(RemoteState::InUse));
    }

    #[test]
    fn an_old_note_with_no_recording_and_no_fresh_marker_is_not_remote() {
        let root = marked_root();
        let dir = early_note(root.path(), "n1", Some(ResultFlag::NoRecording));
        let old = now() - FRESH_FOR - TimeDelta::minutes(1);
        assert!(write_marker(&dir, "run-a", old).unwrap());
        age_records(&dir);
        assert_eq!(remote(root.path(), None), None);
    }
}

// ------------------------------------------------- the new layout (U3 of the safe agent edits plan)

// Covers KTD1: the identity and the records come from `longhand.json`, and
// the title and the tags from `session.md`.
#[test]
fn a_note_takes_its_identity_from_longhand_json_and_its_title_from_session_md() {
    let root = marked_root();
    let dir = note(root.path(), "2026-09-25 0900 Planning", "A1");
    assert!(
        !fs::read_to_string(dir.join("session.md"))
            .unwrap()
            .contains("session_id")
    );
    fs::write(
        dir.join("session.md"),
        "---\ntitle: Agent title\nlonghand_tags:\n- follow-up\n---\nBody.\n",
    )
    .unwrap();
    let result = run(root.path(), &Known::default());
    let found = &result.notes[0];
    assert_eq!(found.doc.session_id, "A1");
    assert_eq!(found.layout, session_core::note_folder::Layout::New);
    assert_eq!(found.doc.title.as_deref(), Some("Agent title"));
    assert_eq!(found.doc.tags, vec!["follow-up".to_string()]);
    assert_eq!(found.written_title.as_ref().unwrap().text, "Planning");
    assert!(found.waiting.is_empty(), "{:?}", found.waiting);
}

// Covers KTD6.
#[test]
fn a_new_session_md_with_no_longhand_json_waits_for_a_sync_and_is_not_a_note() {
    let root = marked_root();
    let dir = note(root.path(), "2026-09-25 0900 Planning", "A1");
    fs::remove_file(dir.join("longhand.json")).unwrap();
    let result = run(root.path(), &Known::default());
    assert!(result.notes.is_empty());
    assert!(result.invalid.is_empty());
    assert_eq!(result.waiting_for_record.len(), 1);
    assert_eq!(result.waiting_for_record[0].path, dir);
    // A known folder keeps its id.
    let mut known = Known::default();
    known.add("A1", &dir);
    let result = run(root.path(), &known);
    assert_eq!(result.waiting_for_record[0].known_id.as_deref(), Some("A1"));
}

// Covers KTD1: a folder with only `longhand.json` is a note folder, not a
// type folder, in the full scan and in the targeted scan.
#[test]
fn a_folder_with_only_longhand_json_is_a_note_not_a_type_folder() {
    let root = marked_root();
    let dir = note(root.path(), "2026-09-25 0900 Planning", "A1");
    fs::remove_file(dir.join("session.md")).unwrap();
    // A folder inside it is not a note of a type.
    note(&dir, "Inner", "B1");
    let result = run(root.path(), &Known::default());
    assert_eq!(types(&result), [typed("A1", None)]);
    assert_eq!(result.notes[0].doc.title.as_deref(), Some("Planning"));
    let (targeted, _) = scan_folders(
        root.path(),
        &HashSet::from([dir.join("Inner")]),
        &Known::default(),
        &options(),
    )
    .unwrap();
    assert!(targeted.notes.is_empty(), "{:?}", targeted.notes);
}

// Covers KTD1: the scan reads only the exact name.
#[test]
fn a_conflict_copy_of_longhand_json_has_no_effect() {
    let root = marked_root();
    let dir = note(root.path(), "2026-09-25 0900 Planning", "A1");
    let mut other = doc("B1");
    other.title = Some("Other".into());
    fs::write(
        dir.join("longhand 2.json"),
        session_core::note_record::NoteRecord::from_doc(&other).render(),
    )
    .unwrap();
    let result = run(root.path(), &Known::default());
    assert_eq!(types(&result), [typed("A1", None)]);
    assert_eq!(
        result.notes[0].conflict_copies,
        [dir.join("longhand 2.json")]
    );
}

// Covers KTD6, R9: a `session.md` that holds `session_id` is the old
// layout, and its records are read, also with a `longhand.json`.
#[test]
fn an_old_layout_session_md_gives_the_records_also_with_a_longhand_json() {
    let root = marked_root();
    let dir = note(root.path(), "2026-09-25 0900 Planning", "A1");
    let mut old = doc("A1");
    old.title = Some("Old".into());
    old.transcript_fingerprint = Some(fingerprint(b"another transcript"));
    fs::write(dir.join("session.md"), old.render()).unwrap();
    let result = run(root.path(), &Known::default());
    let found = &result.notes[0];
    assert_eq!(found.layout, session_core::note_folder::Layout::Old);
    assert_eq!(found.doc.title.as_deref(), Some("Old"));
    assert_eq!(found.waiting, [Waiting::Transcript]);
}
