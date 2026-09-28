use session_core::{
    folder::{SessionDoc, fingerprint},
    generation,
};
use session_save::{home_write, result};

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let work = root.path().join("work");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&work).unwrap();
    for (dir, text) in [(&home, "old"), (&work, "new")] {
        std::fs::write(dir.join("transcript.md"), format!("{text} transcript")).unwrap();
        std::fs::write(dir.join("summary.md"), format!("{text} summary")).unwrap();
    }
    std::fs::write(home.join("notes.md"), "user notes").unwrap();
    let mut doc =
        SessionDoc::parse("---\nsession_id: s\nstarted: 2026-09-27T00:00:00Z\nsegments: []\n---\n")
            .unwrap();
    doc.result_generation = Some(result::stage(&home, &home).unwrap());
    std::fs::write(home.join("session.md"), doc.render()).unwrap();
    (root, home, work)
}

fn prepare(home: &std::path::Path, work: &std::path::Path) -> result::Publication {
    let mut doc = generation::manifest(home).unwrap();
    doc.transcript_fingerprint = Some(fingerprint(b"new transcript"));
    doc.summary_fingerprint = Some(fingerprint(b"new summary"));
    let expected = fingerprint(&std::fs::read(home.join("session.md")).unwrap());
    result::prepare(home, work, work, doc, &expected).unwrap()
}

#[test]
fn readers_get_one_complete_result_at_each_effect_before_its_acknowledgement() {
    let (_root, home, work) = fixture();
    let publication = prepare(&home, &work);
    for step in 0..4 {
        let result = generation::resolve(&home, None).unwrap();
        let expected = if publication.committed() {
            "new"
        } else {
            "old"
        };
        assert_eq!(
            result.transcript,
            format!("{expected} transcript").as_bytes()
        );
        assert_eq!(result.summary, format!("{expected} summary").as_bytes());
        // Reload the owner journal as a restarted process would.
        let recovered = result::load(&work).unwrap();
        let done = recovered.step().unwrap();
        if step < 2 {
            assert!(!done);
        }
    }
    assert!(publication.committed());
    publication.rollback().unwrap();
    assert_eq!(
        generation::resolve(&home, None).unwrap().transcript,
        b"new transcript"
    );
    assert_eq!(std::fs::read(home.join("notes.md")).unwrap(), b"user notes");
}

#[test]
fn rollback_preserves_an_external_edit_and_recovery_files() {
    let (_root, home, work) = fixture();
    let publication = prepare(&home, &work);
    publication.step().unwrap();
    publication.step().unwrap();
    home_write::write_atomic(&home, "summary.md", b"external edit").unwrap();
    assert!(publication.rollback().is_err());
    assert_eq!(
        std::fs::read(home.join("summary.md")).unwrap(),
        b"external edit"
    );
    assert!(work.join(result::JOURNAL).is_file());
    assert_eq!(
        std::fs::read(home.join("transcript.md")).unwrap(),
        b"old transcript"
    );
    assert!(generation::resolve(&home, None).is_err());
}

#[test]
fn a_title_edit_after_manifest_commit_does_not_change_commit_status() {
    let (_root, home, work) = fixture();
    let publication = prepare(&home, &work);
    while !publication.step().unwrap() {}
    let mut doc = generation::manifest(&home).unwrap();
    doc.title = Some("User title after commit".into());
    home_write::write_atomic(&home, "session.md", doc.render().as_bytes()).unwrap();
    assert!(publication.committed());
    publication.rollback().unwrap();
    publication.acknowledge(&work).unwrap();
    assert_eq!(generation::manifest(&home).unwrap().title, doc.title);
    assert_eq!(
        generation::resolve(&home, None).unwrap().transcript,
        b"new transcript"
    );
}

#[test]
fn legacy_upgrade_and_each_unacknowledged_effect_recover_without_partial_reads() {
    let (_root, home, work) = fixture();
    let mut doc = generation::manifest(&home).unwrap();
    doc.result_generation = None;
    home_write::write_atomic(&home, "session.md", doc.render().as_bytes()).unwrap();
    let publication = prepare(&home, &work);
    while !publication.committed() {
        let resolved = generation::resolve(&home, None).unwrap();
        assert_eq!(resolved.transcript, b"old transcript");
        assert_eq!(resolved.summary, b"old summary");
        result::load(&work).unwrap().step().unwrap();
    }
    assert_eq!(
        generation::resolve(&home, None).unwrap().summary,
        b"new summary"
    );
}

#[test]
fn a_new_adopter_waits_but_a_receiver_keeps_its_verified_result() {
    let (_root, home, work) = fixture();
    let previous = generation::manifest(&home).unwrap();
    let publication = prepare(&home, &work);
    while !publication.step().unwrap() {}
    std::fs::remove_file(
        publication
            .next
            .directory(&home)
            .unwrap()
            .join("summary.md"),
    )
    .unwrap();
    assert!(generation::resolve(&home, None).is_err());
    let old = generation::resolve(&home, Some(&previous)).unwrap();
    assert_eq!(old.transcript, b"old transcript");
    assert_eq!(old.summary, b"old summary");
}

#[test]
fn moved_or_deleted_folders_and_notes_edits_refuse_publication() {
    for change in ["move", "delete", "notes"] {
        let (root, home, work) = fixture();
        let publication = prepare(&home, &work);
        match change {
            "move" => std::fs::rename(&home, root.path().join("moved")).unwrap(),
            "delete" => std::fs::remove_dir_all(&home).unwrap(),
            _ => std::fs::write(home.join("notes.md"), "changed notes").unwrap(),
        }
        assert!(publication.step().is_err());
        assert!(work.join(result::JOURNAL).exists());
        if change != "notes" {
            assert!(!home.exists());
        }
    }
}
