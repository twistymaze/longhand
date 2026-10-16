use std::io;
use std::path::{Path, PathBuf};

use session_core::folder::{PendingType, SessionDoc, fingerprint};
use session_core::note_folder::{NoteFiles, SESSION_FILE};
use session_core::note_record::{NOTE_RECORD_FILE, NoteRecord};
use session_core::recognition::{ARTIFACT_FILE, ARTIFACT_VERSION, ArtifactReference};
use session_core::{Title, TitleSource};
use session_save::note_files;
use session_save::publication::{self, OLD_LAYOUT, Outcome, Publication, WriteSet};

const RESULT_WRITES: [&str; 4] = [
    ARTIFACT_FILE,
    "transcript.md",
    "summary.md",
    NOTE_RECORD_FILE,
];

/// A `session.md` that an agent edited: a key that Longhand does not
/// know, a comment and a body.
const AGENT_SESSION_MD: &str =
    "---\ntitle: Title\n# an agent comment\nsource: agent\n---\n\nText that an agent wrote.\n";

fn reference(bytes: &[u8]) -> ArtifactReference {
    ArtifactReference {
        version: ARTIFACT_VERSION,
        filename: ARTIFACT_FILE.into(),
        sha256: fingerprint(bytes),
    }
}

fn started() -> chrono::DateTime<chrono::FixedOffset> {
    chrono::DateTime::parse_from_rfc3339("2026-09-27T00:00:00Z").unwrap()
}

/// The records of the result in `dir`.
fn result_record(dir: &Path) -> NoteRecord {
    let mut record = NoteRecord::new("s", started());
    record.written_title = Some(Title {
        text: "Title".into(),
        source: TitleSource::Proposed,
    });
    let bytes = |name: &str| std::fs::read(dir.join(name)).unwrap();
    record.transcript_fingerprint = Some(fingerprint(&bytes("transcript.md")));
    record.summary_fingerprint = Some(fingerprint(&bytes("summary.md")));
    if dir.join(ARTIFACT_FILE).exists() {
        record.recognition_artifact = Some(reference(&bytes(ARTIFACT_FILE)));
    }
    record
}

fn records(home: &Path) -> NoteRecord {
    NoteFiles::read(home).unwrap().record
}

/// A note folder with an old result, and a local staging folder with a
/// new result. Returns the publication, not recorded yet.
fn fixture(set: WriteSet) -> (tempfile::TempDir, PathBuf, PathBuf, Publication) {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let local = root.path().join("local");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&local).unwrap();
    std::fs::write(home.join("transcript.md"), "old transcript").unwrap();
    std::fs::write(home.join("summary.md"), "old summary").unwrap();
    std::fs::write(home.join(ARTIFACT_FILE), "old artifact").unwrap();
    std::fs::write(home.join("notes.md"), "user notes").unwrap();
    std::fs::write(home.join(SESSION_FILE), AGENT_SESSION_MD).unwrap();
    note_files::write_record(&home, &result_record(&home)).unwrap();
    let after = match set {
        WriteSet::Result => {
            std::fs::write(local.join("transcript.md"), "new transcript").unwrap();
            std::fs::write(local.join("summary.md"), "new summary").unwrap();
            std::fs::write(local.join(ARTIFACT_FILE), "new artifact").unwrap();
            result_record(&local)
        }
        WriteSet::Summary => {
            std::fs::write(local.join("summary.md"), "new summary").unwrap();
            let mut record = result_record(&home);
            record.summary_fingerprint = Some(fingerprint(b"new summary"));
            record
        }
    };
    let publication = Publication::new(&home, after, set).unwrap();
    (root, home, local, publication)
}

/// A hook that fails after the write number `stop`, as a crash does.
fn crash_after(stop: usize) -> impl FnMut(&str) -> io::Result<()> {
    let mut count = 0;
    move |_| {
        count += 1;
        if count == stop {
            Err(io::Error::other("crash"))
        } else {
            Ok(())
        }
    }
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap()
}

fn names(dir: &Path) -> Vec<PathBuf> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    names.sort();
    names
}

#[test]
fn the_result_set_writes_the_timed_text_then_the_documents_then_longhand_json() {
    let (_root, home, local, publication) = fixture(WriteSet::Result);
    publication.record(&local).unwrap();
    let mut written = Vec::new();
    let outcome = publication
        .publish_with(&home, &local, || (), &mut |name| {
            written.push(name.to_string());
            Ok(())
        })
        .unwrap();
    assert_eq!(outcome, Outcome::Committed);
    assert_eq!(written, RESULT_WRITES);
    for name in ["transcript.md", "summary.md", ARTIFACT_FILE] {
        assert_eq!(read(&home, name), read(&local, name), "{name}");
    }
    assert_eq!(read(&home, "notes.md"), "user notes");
    let record = records(&home);
    assert_eq!(
        record.transcript_fingerprint,
        Some(fingerprint(b"new transcript"))
    );
    assert_eq!(
        record.summary_fingerprint,
        Some(fingerprint(b"new summary"))
    );
}

#[test]
fn a_save_keeps_the_body_and_the_unknown_keys_of_session_md() {
    let (_root, home, local, publication) = fixture(WriteSet::Result);
    publication.record(&local).unwrap();
    publication.publish(&home, &local, || ()).unwrap();
    assert_eq!(read(&home, SESSION_FILE), AGENT_SESSION_MD);
}

#[test]
fn a_missing_session_md_is_written_before_longhand_json() {
    let (_root, home, local, publication) = fixture(WriteSet::Result);
    std::fs::remove_file(home.join(SESSION_FILE)).unwrap();
    publication.record(&local).unwrap();
    let mut written = Vec::new();
    publication
        .publish_with(&home, &local, || (), &mut |name| {
            written.push(name.to_string());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        written[written.len() - 2..],
        [SESSION_FILE.to_string(), NOTE_RECORD_FILE.to_string()]
    );
    let files = NoteFiles::read(&home).unwrap();
    assert_eq!(files.doc.title.as_deref(), Some("Title"));
    assert_eq!(files.doc.title_source, Some(TitleSource::Proposed));
}

/// A guard that tells while it is held.
struct Held(std::rc::Rc<std::cell::Cell<bool>>);

impl Drop for Held {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

/// The caller's guard is held from the last read of `longhand.json` to
/// its write, so a title or type writer that holds the same lock cannot
/// write between them. The staged files are written without it.
#[test]
fn the_records_are_read_and_written_while_the_caller_holds_its_guard() {
    let (_root, home, local, publication) = fixture(WriteSet::Result);
    // With no `session.md`, a write happens between the read and the
    // write of `longhand.json`.
    std::fs::remove_file(home.join(SESSION_FILE)).unwrap();
    publication.record(&local).unwrap();
    let held = std::rc::Rc::new(std::cell::Cell::new(false));
    let mut seen = Vec::new();
    publication
        .publish_with(
            &home,
            &local,
            || {
                held.set(true);
                Held(held.clone())
            },
            &mut |name| {
                seen.push((name.to_string(), held.get()));
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(
        seen,
        [
            (ARTIFACT_FILE.to_string(), false),
            ("transcript.md".to_string(), false),
            ("summary.md".to_string(), false),
            (SESSION_FILE.to_string(), true),
            (NOTE_RECORD_FILE.to_string(), true),
        ]
    );
    assert!(!held.get(), "the guard is released after the write");
}

/// A writer that held the lock just before the publication took it wrote
/// a title and a pending type. The publication reads the records after it
/// takes the lock, so both stay.
#[test]
fn records_written_before_the_guard_is_taken_stay() {
    let (_root, home, local, publication) = fixture(WriteSet::Result);
    publication.record(&local).unwrap();
    let title = Title {
        text: "New title".into(),
        source: TitleSource::User,
    };
    let outcome = publication
        .publish(&home, &local, || {
            let mut record = records(&home);
            record.written_title = Some(title.clone());
            record.pending_type = Some(pending_type());
            note_files::write_record(&home, &record).unwrap();
        })
        .unwrap();
    assert_eq!(outcome, Outcome::Committed);
    let record = records(&home);
    assert_eq!(record.written_title, Some(title));
    assert_eq!(record.pending_type, Some(pending_type()));
    assert_eq!(
        record.transcript_fingerprint,
        Some(fingerprint(b"new transcript"))
    );
}

/// A type writer on another thread reads the records under the lock while
/// the publication writes its files, and writes them back with a pending
/// type when the publication waits for the lock. The publication reads
/// the records after the writer releases the lock, so the result and the
/// pending type both stay.
#[test]
fn a_record_writer_that_holds_the_lock_is_not_undone() {
    let (_root, home, local, publication) = fixture(WriteSet::Result);
    publication.record(&local).unwrap();
    let lock = std::sync::Arc::new(std::sync::Mutex::new(()));
    let (locking_tx, locking_rx) = std::sync::mpsc::channel::<()>();
    let (read_tx, read_rx) = std::sync::mpsc::channel::<()>();
    let writer = {
        let (lock, home) = (lock.clone(), home.clone());
        std::thread::spawn(move || {
            let _held = lock.lock().unwrap();
            let mut record = records(&home);
            read_tx.send(()).unwrap();
            // The publication is about to take the lock.
            locking_rx.recv().unwrap();
            record.pending_type = Some(pending_type());
            note_files::write_record(&home, &record).unwrap();
        })
    };
    read_rx.recv().unwrap();
    let outcome = publication
        .publish(&home, &local, || {
            locking_tx.send(()).unwrap();
            lock.lock().unwrap()
        })
        .unwrap();
    writer.join().unwrap();
    assert_eq!(outcome, Outcome::Committed);
    let record = records(&home);
    assert_eq!(record.pending_type, Some(pending_type()));
    assert_eq!(
        record.transcript_fingerprint,
        Some(fingerprint(b"new transcript"))
    );
}

#[test]
fn a_crash_before_longhand_json_leaves_the_note_waiting_for_a_sync() {
    let (_root, home, local, publication) = fixture(WriteSet::Result);
    publication.record(&local).unwrap();
    // The third write is `summary.md`: the stop comes before
    // `longhand.json`.
    assert!(
        publication
            .publish_with(&home, &local, || (), &mut crash_after(3))
            .is_err()
    );
    let record = records(&home);
    assert_eq!(
        record.transcript_fingerprint,
        Some(fingerprint(b"old transcript"))
    );
    assert_eq!(read(&home, "transcript.md"), "new transcript");
}

#[test]
fn a_rerun_after_each_boundary_reaches_the_same_end_state() {
    let (_root, home, local, publication) = fixture(WriteSet::Result);
    publication.record(&local).unwrap();
    publication.publish(&home, &local, || ()).unwrap();
    let files = [
        SESSION_FILE,
        NOTE_RECORD_FILE,
        "transcript.md",
        "summary.md",
        ARTIFACT_FILE,
    ];
    let complete: Vec<String> = files.iter().map(|name| read(&home, name)).collect();

    // 0: the record is written and nothing else. 1 to 3: after each
    // result file. 4: after `longhand.json`, before the record is deleted.
    for stop in 0..=RESULT_WRITES.len() {
        let (_root, home, local, publication) = fixture(WriteSet::Result);
        publication.record(&local).unwrap();
        if stop > 0 {
            assert!(
                publication
                    .publish_with(&home, &local, || (), &mut crash_after(stop))
                    .is_err()
            );
        }
        // The launch after the crash reads the record again.
        let recorded = Publication::load(&local).unwrap().expect("the record");
        assert_eq!(
            recorded.publish(&home, &local, || ()).unwrap(),
            Outcome::Committed,
            "stop {stop}"
        );
        let end: Vec<String> = files.iter().map(|name| read(&home, name)).collect();
        assert_eq!(end, complete, "stop {stop}");
        // A second rerun changes nothing.
        let mut written = Vec::new();
        recorded
            .publish_with(&home, &local, || (), &mut |name| {
                written.push(name.to_string());
                Ok(())
            })
            .unwrap();
        assert!(written.is_empty(), "stop {stop}: {written:?}");
        Publication::remove(&local).unwrap();
        assert!(Publication::load(&local).unwrap().is_none());
    }
}

#[test]
fn a_title_change_between_two_writes_stays() {
    let (_root, home, local, publication) = fixture(WriteSet::Result);
    publication.record(&local).unwrap();
    assert!(
        publication
            .publish_with(&home, &local, || (), &mut crash_after(2))
            .is_err()
    );
    // A rename writes the title and the last-written title.
    note_files::patch_session_md(&home, &note_files::title_patch(Some("Renamed"))).unwrap();
    let mut renamed = records(&home);
    renamed.written_title = Some(Title {
        text: "Renamed".into(),
        source: TitleSource::User,
    });
    note_files::write_record(&home, &renamed).unwrap();

    assert_eq!(
        publication.publish(&home, &local, || ()).unwrap(),
        Outcome::Committed
    );
    let files = NoteFiles::read(&home).unwrap();
    assert_eq!(files.doc.title.as_deref(), Some("Renamed"));
    assert_eq!(files.doc.title_source, Some(TitleSource::User));
    assert_eq!(
        files.record.summary_fingerprint,
        Some(fingerprint(b"new summary"))
    );
    assert_eq!(read(&home, "summary.md"), "new summary");
}

fn pending_type() -> PendingType {
    PendingType {
        to: Some("One on One".into()),
        from: None,
        by: "computer-b".into(),
        at: chrono::DateTime::parse_from_rfc3339("2026-09-27T01:00:00Z").unwrap(),
    }
}

/// Computer B writes the tags into `session.md` and a pending type into
/// `longhand.json` while computer A writes its result files. A's
/// publication keeps them, and reports no conflict.
fn user_fields_written_during_the_writes_stay(set: WriteSet) {
    let (_root, home, local, publication) = fixture(set);
    publication.record(&local).unwrap();
    let outcome = publication
        .publish_with(&home, &local, || (), &mut |name| {
            if name == "summary.md" {
                let tags = vec!["Topic::Security".to_string()];
                note_files::patch_session_md(
                    &home,
                    &note_files::with_tags(Default::default(), &tags),
                )?;
                let mut moved = records(&home);
                moved.pending_type = Some(pending_type());
                note_files::write_record(&home, &moved)?;
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(outcome, Outcome::Committed);
    let files = NoteFiles::read(&home).unwrap();
    assert_eq!(files.doc.tags, vec!["Topic::Security".to_string()]);
    assert_eq!(files.record.pending_type, Some(pending_type()));
    assert_eq!(
        files.record.summary_fingerprint,
        Some(fingerprint(b"new summary"))
    );
}

#[test]
fn user_fields_written_during_the_result_writes_stay() {
    user_fields_written_during_the_writes_stay(WriteSet::Result);
}

#[test]
fn user_fields_written_during_the_summary_writes_stay() {
    user_fields_written_during_the_writes_stay(WriteSet::Summary);
}

#[test]
fn a_pending_type_that_another_computer_removed_stays_removed() {
    let (_root, home, local, _) = fixture(WriteSet::Result);
    // The staged result was made while the note had a pending type.
    // Another computer then removed it.
    let mut after = result_record(&local);
    after.pending_type = Some(pending_type());
    let publication = Publication::new(&home, after, WriteSet::Result).unwrap();
    publication.record(&local).unwrap();
    assert_eq!(
        publication.publish(&home, &local, || ()).unwrap(),
        Outcome::Committed
    );
    assert_eq!(records(&home).pending_type, None);
}

#[test]
fn another_result_between_two_writes_is_a_conflict_and_nothing_is_written() {
    let (_root, home, local, publication) = fixture(WriteSet::Result);
    publication.record(&local).unwrap();
    assert!(
        publication
            .publish_with(&home, &local, || (), &mut crash_after(2))
            .is_err()
    );
    let mut other = records(&home);
    other.transcript_fingerprint = Some(fingerprint(b"another computer's transcript"));
    let other = other.render();
    std::fs::write(home.join(NOTE_RECORD_FILE), &other).unwrap();

    let mut written = Vec::new();
    let outcome = publication
        .publish_with(&home, &local, || (), &mut |name| {
            written.push(name.to_string());
            Ok(())
        })
        .unwrap();
    assert_eq!(outcome, Outcome::Conflict);
    assert!(written.is_empty(), "{written:?}");
    assert_eq!(read(&home, NOTE_RECORD_FILE), other);
    assert_eq!(read(&home, "summary.md"), "old summary");
}

#[test]
fn another_result_during_the_writes_stops_the_next_write() {
    let (_root, home, local, publication) = fixture(WriteSet::Result);
    publication.record(&local).unwrap();
    let other_home = home.clone();
    let mut written = Vec::new();
    let outcome = publication
        .publish_with(&home, &local, || (), &mut |name| {
            written.push(name.to_string());
            // The other computer's `longhand.json` arrives after the first
            // write.
            let mut other = records(&other_home);
            other.transcript_fingerprint = Some(fingerprint(b"another computer's transcript"));
            note_files::write_record(&other_home, &other).map(|_| ())
        })
        .unwrap();
    assert_eq!(outcome, Outcome::Conflict);
    assert_eq!(written, [ARTIFACT_FILE]);
    assert_eq!(read(&home, "transcript.md"), "old transcript");
    assert_eq!(read(&home, "summary.md"), "old summary");
}

#[test]
fn equal_result_fields_still_replace_a_hand_edited_file() {
    let (_root, home, local, _) = fixture(WriteSet::Result);
    // The run made the same transcript as the recorded one, and the user
    // edited `transcript.md` by hand.
    std::fs::copy(home.join("transcript.md"), local.join("transcript.md")).unwrap();
    std::fs::copy(home.join("summary.md"), local.join("summary.md")).unwrap();
    std::fs::copy(home.join(ARTIFACT_FILE), local.join(ARTIFACT_FILE)).unwrap();
    std::fs::write(home.join("transcript.md"), "hand edit").unwrap();
    let publication = Publication::new(&home, result_record(&local), WriteSet::Result).unwrap();
    publication.record(&local).unwrap();

    assert_eq!(
        publication.publish(&home, &local, || ()).unwrap(),
        Outcome::Committed
    );
    assert_eq!(read(&home, "transcript.md"), "old transcript");
}

#[test]
fn the_summary_set_writes_only_summary_md_and_longhand_json() {
    let (_root, home, local, publication) = fixture(WriteSet::Summary);
    publication.record(&local).unwrap();
    let mut written = Vec::new();
    let outcome = publication
        .publish_with(&home, &local, || (), &mut |name| {
            written.push(name.to_string());
            Ok(())
        })
        .unwrap();
    assert_eq!(outcome, Outcome::Committed);
    assert_eq!(written, ["summary.md", NOTE_RECORD_FILE]);
    assert_eq!(read(&home, "transcript.md"), "old transcript");
    assert_eq!(read(&home, ARTIFACT_FILE), "old artifact");
    let record = records(&home);
    assert_eq!(
        record.recognition_artifact,
        Some(reference(b"old artifact"))
    );
    assert_eq!(
        record.summary_fingerprint,
        Some(fingerprint(b"new summary"))
    );
}

#[test]
fn the_summary_set_drops_the_reference_to_a_timed_text_that_is_gone() {
    let (_root, home, local, publication) = fixture(WriteSet::Summary);
    publication.record(&local).unwrap();
    // The audio removal on another computer deletes `recognition.jsonl`
    // while the summary is made.
    std::fs::remove_file(home.join(ARTIFACT_FILE)).unwrap();

    assert_eq!(
        publication.publish(&home, &local, || ()).unwrap(),
        Outcome::Committed
    );
    assert!(!home.join(ARTIFACT_FILE).exists());
    let record = records(&home);
    assert_eq!(record.recognition_artifact, None);
    assert_eq!(record.recognition_audio, None);
    // The dropped reference does not make a rerun a conflict.
    assert_eq!(
        publication.publish(&home, &local, || ()).unwrap(),
        Outcome::Committed
    );
}

#[test]
fn a_note_in_the_old_layout_gets_no_publication_and_no_write() {
    let (_root, home, local, _) = fixture(WriteSet::Result);
    // The old layout: `session.md` holds `session_id` and the records.
    let mut doc = SessionDoc::at("s", started(), None, &[]);
    doc.transcript_fingerprint = Some(fingerprint(b"old transcript"));
    std::fs::write(home.join(SESSION_FILE), doc.render()).unwrap();
    std::fs::remove_file(home.join(NOTE_RECORD_FILE)).unwrap();
    let before = names(&home);
    let session_md = read(&home, SESSION_FILE);
    let error = Publication::new(&home, result_record(&local), WriteSet::Summary).unwrap_err();
    assert_eq!(error.to_string(), OLD_LAYOUT);
    assert_eq!(names(&home), before);
    assert_eq!(read(&home, SESSION_FILE), session_md);
}

#[test]
fn a_folder_with_no_record_has_no_publication() {
    let (_root, _home, local, _) = fixture(WriteSet::Result);
    assert!(Publication::load(&local).unwrap().is_none());
    Publication::remove(&local).unwrap();
    assert!(!local.join(publication::RECORD).exists());
}
