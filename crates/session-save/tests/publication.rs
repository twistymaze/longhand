use std::io;
use std::path::{Path, PathBuf};

use session_core::folder::{SessionDoc, fingerprint};
use session_core::recognition::{ARTIFACT_FILE, ARTIFACT_VERSION, ArtifactReference};
use session_save::publication::{self, Outcome, Publication, WriteSet};

const RESULT_WRITES: [&str; 4] = [ARTIFACT_FILE, "transcript.md", "summary.md", "session.md"];

fn reference(bytes: &[u8]) -> ArtifactReference {
    ArtifactReference {
        version: ARTIFACT_VERSION,
        filename: ARTIFACT_FILE.into(),
        sha256: fingerprint(bytes),
    }
}

/// The result of `dir` as `session.md` records it.
fn result_doc(dir: &Path, title: &str) -> SessionDoc {
    let mut doc =
        SessionDoc::parse("---\nsession_id: s\nstarted: 2026-09-27T00:00:00Z\nsegments: []\n---\n")
            .unwrap();
    doc.title = Some(title.into());
    let bytes = |name: &str| std::fs::read(dir.join(name)).unwrap();
    doc.transcript_fingerprint = Some(fingerprint(&bytes("transcript.md")));
    doc.summary_fingerprint = Some(fingerprint(&bytes("summary.md")));
    if dir.join(ARTIFACT_FILE).exists() {
        doc.recognition_artifact = Some(reference(&bytes(ARTIFACT_FILE)));
    }
    doc
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
    std::fs::write(home.join("session.md"), result_doc(&home, "Title").render()).unwrap();
    let after = match set {
        WriteSet::Result => {
            std::fs::write(local.join("transcript.md"), "new transcript").unwrap();
            std::fs::write(local.join("summary.md"), "new summary").unwrap();
            std::fs::write(local.join(ARTIFACT_FILE), "new artifact").unwrap();
            result_doc(&local, "Title")
        }
        WriteSet::Summary => {
            std::fs::write(local.join("summary.md"), "new summary").unwrap();
            let mut doc = result_doc(&home, "Title");
            doc.summary_fingerprint = Some(fingerprint(b"new summary"));
            doc
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

#[test]
fn the_result_set_writes_the_timed_text_then_the_documents_then_session_md() {
    let (_root, home, local, publication) = fixture(WriteSet::Result);
    publication.record(&local).unwrap();
    let mut written = Vec::new();
    let outcome = publication
        .publish_with(&home, &local, &mut |name| {
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
    let doc = SessionDoc::read(&home).unwrap();
    assert_eq!(
        doc.transcript_fingerprint,
        Some(fingerprint(b"new transcript"))
    );
    assert_eq!(doc.summary_fingerprint, Some(fingerprint(b"new summary")));
    assert!(!home.join(".longhand-results").exists());
    assert!(!home.join(".longhand-result-transition.json").exists());
}

#[test]
fn a_rerun_after_each_boundary_reaches_the_same_end_state() {
    let (_root, home, local, publication) = fixture(WriteSet::Result);
    publication.record(&local).unwrap();
    publication.publish(&home, &local).unwrap();
    let complete: Vec<String> = ["session.md", "transcript.md", "summary.md", ARTIFACT_FILE]
        .iter()
        .map(|name| read(&home, name))
        .collect();

    // 0: the record is written and nothing else. 1 to 3: after each
    // result file. 4: after `session.md`, before the record is deleted.
    for stop in 0..=RESULT_WRITES.len() {
        let (_root, home, local, publication) = fixture(WriteSet::Result);
        publication.record(&local).unwrap();
        if stop > 0 {
            assert!(
                publication
                    .publish_with(&home, &local, &mut crash_after(stop))
                    .is_err()
            );
        }
        // The launch after the crash reads the record again.
        let recorded = Publication::load(&local).unwrap().expect("the record");
        assert_eq!(
            recorded.publish(&home, &local).unwrap(),
            Outcome::Committed,
            "stop {stop}"
        );
        let end: Vec<String> = ["session.md", "transcript.md", "summary.md", ARTIFACT_FILE]
            .iter()
            .map(|name| read(&home, name))
            .collect();
        assert_eq!(end, complete, "stop {stop}");
        // A second rerun changes nothing.
        let mut written = Vec::new();
        recorded
            .publish_with(&home, &local, &mut |name| {
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
            .publish_with(&home, &local, &mut crash_after(2))
            .is_err()
    );
    let mut renamed = SessionDoc::read(&home).unwrap();
    renamed.title = Some("Renamed".into());
    std::fs::write(home.join("session.md"), renamed.render()).unwrap();

    assert_eq!(
        publication.publish(&home, &local).unwrap(),
        Outcome::Committed
    );
    let doc = SessionDoc::read(&home).unwrap();
    assert_eq!(doc.title.as_deref(), Some("Renamed"));
    assert_eq!(doc.summary_fingerprint, Some(fingerprint(b"new summary")));
    assert_eq!(read(&home, "summary.md"), "new summary");
}

#[test]
fn another_result_between_two_writes_is_a_conflict_and_nothing_is_written() {
    let (_root, home, local, publication) = fixture(WriteSet::Result);
    publication.record(&local).unwrap();
    assert!(
        publication
            .publish_with(&home, &local, &mut crash_after(2))
            .is_err()
    );
    let mut other = SessionDoc::read(&home).unwrap();
    other.transcript_fingerprint = Some(fingerprint(b"another computer's transcript"));
    let other = other.render();
    std::fs::write(home.join("session.md"), &other).unwrap();

    let mut written = Vec::new();
    let outcome = publication
        .publish_with(&home, &local, &mut |name| {
            written.push(name.to_string());
            Ok(())
        })
        .unwrap();
    assert_eq!(outcome, Outcome::Conflict);
    assert!(written.is_empty(), "{written:?}");
    assert_eq!(read(&home, "session.md"), other);
    assert_eq!(read(&home, "summary.md"), "old summary");
}

#[test]
fn another_result_during_the_writes_stops_the_next_write() {
    let (_root, home, local, publication) = fixture(WriteSet::Result);
    publication.record(&local).unwrap();
    let other_home = home.clone();
    let mut written = Vec::new();
    let outcome = publication
        .publish_with(&home, &local, &mut |name| {
            written.push(name.to_string());
            // The other computer's `session.md` arrives after the first
            // write.
            let mut other = SessionDoc::read(&other_home)?;
            other.transcript_fingerprint = Some(fingerprint(b"another computer's transcript"));
            std::fs::write(other_home.join("session.md"), other.render())
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
    let publication =
        Publication::new(&home, result_doc(&local, "Title"), WriteSet::Result).unwrap();
    publication.record(&local).unwrap();

    assert_eq!(
        publication.publish(&home, &local).unwrap(),
        Outcome::Committed
    );
    assert_eq!(read(&home, "transcript.md"), "old transcript");
}

#[test]
fn the_summary_set_writes_only_summary_md_and_session_md() {
    let (_root, home, local, publication) = fixture(WriteSet::Summary);
    publication.record(&local).unwrap();
    let mut written = Vec::new();
    let outcome = publication
        .publish_with(&home, &local, &mut |name| {
            written.push(name.to_string());
            Ok(())
        })
        .unwrap();
    assert_eq!(outcome, Outcome::Committed);
    assert_eq!(written, ["summary.md", "session.md"]);
    assert_eq!(read(&home, "transcript.md"), "old transcript");
    assert_eq!(read(&home, ARTIFACT_FILE), "old artifact");
    let doc = SessionDoc::read(&home).unwrap();
    assert_eq!(doc.recognition_artifact, Some(reference(b"old artifact")));
    assert_eq!(doc.summary_fingerprint, Some(fingerprint(b"new summary")));
}

#[test]
fn the_summary_set_drops_the_reference_to_a_timed_text_that_is_gone() {
    let (_root, home, local, publication) = fixture(WriteSet::Summary);
    publication.record(&local).unwrap();
    // The audio removal on another computer deletes `recognition.jsonl`
    // while the summary is made, and does not write `session.md`.
    std::fs::remove_file(home.join(ARTIFACT_FILE)).unwrap();

    assert_eq!(
        publication.publish(&home, &local).unwrap(),
        Outcome::Committed
    );
    assert!(!home.join(ARTIFACT_FILE).exists());
    let doc = SessionDoc::read(&home).unwrap();
    assert_eq!(doc.recognition_artifact, None);
    assert_eq!(doc.recognition_audio, None);
    // The dropped reference does not make a rerun a conflict.
    assert_eq!(
        publication.publish(&home, &local).unwrap(),
        Outcome::Committed
    );
}

#[test]
fn a_folder_with_no_record_has_no_publication() {
    let (_root, _home, local, _) = fixture(WriteSet::Result);
    assert!(Publication::load(&local).unwrap().is_none());
    Publication::remove(&local).unwrap();
    assert!(!local.join(publication::RECORD).exists());
}
