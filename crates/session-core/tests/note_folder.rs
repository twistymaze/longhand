use chrono::{DateTime, FixedOffset};
use session_core::folder::{FolderError, FrontmatterPatch, PendingType, SessionDoc};
use session_core::note_folder::{Layout, NoteFiles, NoteFilesError, written_fingerprint};
use session_core::note_record::NoteRecord;
use session_core::{Segment, Title, TitleSource};

fn started() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-09-22T09:00:00-05:00").unwrap()
}

fn record() -> NoteRecord {
    NoteRecord {
        written_title: Some(Title {
            text: "Budget".into(),
            source: TitleSource::Proposed,
        }),
        transcript_fingerprint: Some("t1".into()),
        ..NoteRecord::new("01j8z", started())
    }
}

#[test]
fn a_new_layout_note_takes_its_identity_from_the_record_and_its_title_from_session_md() {
    let session_md = "---\ntitle: Budget\nlonghand_tags:\n- Person::Nathan\n- person :: nathan\n- Follow-up\nsource: agent\n---\nAgent text.\n";
    let files = NoteFiles::parse(Some(session_md), Some(&record().render())).unwrap();
    assert_eq!(files.layout, Layout::New);
    assert_eq!(files.doc.session_id, "01j8z");
    assert_eq!(files.doc.title.as_deref(), Some("Budget"));
    assert_eq!(files.doc.title_source, Some(TitleSource::Proposed));
    // R8: two tags with one key count as one tag.
    assert_eq!(files.doc.tags, vec!["Person::Nathan", "Follow-up"]);
    assert_eq!(files.doc.transcript_fingerprint.as_deref(), Some("t1"));
    assert!(!files.title_changed_outside());
}

#[test]
fn a_title_that_differs_from_the_last_written_title_is_the_users() {
    let files = NoteFiles::parse(
        Some("---\ntitle: Agent title\n---\n"),
        Some(&record().render()),
    )
    .unwrap();
    assert_eq!(files.doc.title_source, Some(TitleSource::User));
    assert!(files.title_changed_outside());
}

#[test]
fn a_session_md_that_holds_session_id_is_the_old_layout_also_with_a_record() {
    let mut doc = SessionDoc::at("01j8z", started(), None, &[]);
    doc.title = Some("Old".into());
    doc.title_source = Some(TitleSource::User);
    doc.transcript_fingerprint = Some("old".into());
    let files = NoteFiles::parse(Some(&doc.render()), Some(&record().render())).unwrap();
    assert_eq!(files.layout, Layout::Old);
    assert_eq!(files.doc.transcript_fingerprint.as_deref(), Some("old"));
    assert_eq!(files.written_title(), None);
    assert!(!files.title_changed_outside());
    assert_eq!(files.record.transcript_fingerprint.as_deref(), Some("old"));
}

#[test]
fn a_new_session_md_with_no_record_waits_for_a_sync() {
    let session_md = FrontmatterPatch::new().set("title", "Budget").create();
    assert_eq!(
        NoteFiles::parse(Some(&session_md), None),
        Err(NoteFilesError::WaitingForRecord)
    );
    assert_eq!(NoteFiles::parse(None, None), Err(NoteFilesError::NotANote));
}

#[test]
fn a_folder_with_only_the_record_is_a_note_with_the_last_written_title() {
    let files = NoteFiles::parse(None, Some(&record().render())).unwrap();
    assert_eq!(files.layout, Layout::New);
    assert_eq!(files.doc.title.as_deref(), Some("Budget"));
    assert_eq!(files.doc.title_source, Some(TitleSource::Proposed));
}

#[test]
fn a_record_that_does_not_parse_is_invalid() {
    assert!(matches!(
        NoteFiles::parse(Some("---\n---\n"), Some("{ not json")),
        Err(NoteFilesError::Invalid(_))
    ));
    assert!(matches!(
        NoteFiles::parse(Some("---\n---\n"), Some("{\"layout\": 9}")),
        Err(NoteFilesError::Newer { layout: 9 })
    ));
}

#[test]
fn the_run_fingerprint_ignores_the_title_the_pending_type_and_the_layout() {
    let base = record();
    let mut moved = base.clone();
    moved.written_title = Some(Title {
        text: "Other".into(),
        source: TitleSource::User,
    });
    moved.pending_type = Some(PendingType {
        to: Some("Team".into()),
        from: None,
        by: "b".into(),
        at: started(),
    });
    assert_eq!(base.run_fingerprint(), moved.run_fingerprint());
    let mut changed = base.clone();
    changed.transcript_fingerprint = Some("t2".into());
    assert_ne!(base.run_fingerprint(), changed.run_fingerprint());
    let mut resegmented = base.clone();
    resegmented.found_speakers = Some(2);
    assert_ne!(base.run_fingerprint(), resegmented.run_fingerprint());
}

#[test]
fn the_written_fingerprint_covers_both_files() {
    assert_ne!(
        written_fingerprint(b"a", b"b"),
        written_fingerprint(b"a", b"c")
    );
    assert_ne!(
        written_fingerprint(b"a", b"b"),
        written_fingerprint(b"x", b"b")
    );
}

/// An old-layout `session.md` with one segment whose audio name is
/// `audio`.
fn old_session_md(audio: &str) -> String {
    let segment = Segment {
        index: 0,
        offset_ms: 0,
        duration_ms: 60_000,
        wall_start: started(),
        wall_end: started() + chrono::TimeDelta::minutes(1),
        audio_file: audio.into(),
        channels: 1,
        ends_in_pause: false,
    };
    SessionDoc::new("01j8z", None, &[segment]).unwrap().render()
}

#[test]
fn an_old_layout_session_md_whose_segment_audio_is_not_a_path_in_the_note_folder_is_invalid() {
    for audio in [
        "/etc/passwd",
        "../segment-01.opus",
        "audio/../../segment-01.opus",
        "\\\\server\\share\\segment-01.opus",
        "//server/share/segment-01.opus",
        "C:\\Windows\\segment-01.opus",
    ] {
        // The assert does not print the result: its text holds the session id.
        assert!(
            matches!(
                NoteFiles::parse(Some(&old_session_md(audio)), None),
                Err(NoteFilesError::Invalid(FolderError::Yaml { file, .. })) if file == "session.md"
            ),
            "{audio:?}"
        );
    }
    let files = NoteFiles::parse(Some(&old_session_md("audio/segment-01.opus")), None).unwrap();
    assert_eq!(files.layout, Layout::Old);
    assert_eq!(files.doc.segments[0].audio, "audio/segment-01.opus");
}

#[test]
fn an_old_layout_session_md_with_a_path_as_its_id_is_invalid() {
    let doc = SessionDoc::at("../..", started(), None, &[]);
    assert!(matches!(
        NoteFiles::parse(Some(&doc.render()), None),
        Err(NoteFilesError::Invalid(_))
    ));
}
