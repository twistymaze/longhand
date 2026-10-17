//! The conversion of old notes (U5 of the safe agent edits plan, R9, R10,
//! R12, R13, KTD6). A note is saved through the real save stage and then
//! written in the old layout, as an older Longhand wrote it.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, FixedOffset, Local, TimeDelta};
use pipeline::SharedStore;
use pipeline::convert::{self, ConvertReport, FolderConversion, Step};
use session_core::folder::{SessionDoc, Summary, SummaryMeta, fingerprint};
use session_core::note_folder::{Layout, NoteFiles, written_fingerprint};
use session_core::note_record::NOTE_RECORD_FILE;
use session_core::{Event, Segment, Stage, Store};
use session_save::RealFs;

const TRANSCRIPT: &str = "# Transcript\n\n- [00:00:01] Speaker 1: We start with the budget.\n";

fn started() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-09-25T09:00:00-05:00").unwrap()
}

fn store() -> SharedStore {
    Arc::new(Mutex::new(Store::open_in_memory().unwrap()))
}

/// A storage folder with the library marker.
fn storage() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Notes");
    fs::create_dir(&root).unwrap();
    assert!(library::scan::write_marker(&root));
    (dir, root)
}

/// Record, process and save the note `id` into `root` through the real
/// save stage. Returns its folder.
fn save_note(store: &SharedStore, working: &Path, id: &str, root: &Path) -> PathBuf {
    let segment = Segment {
        index: 0,
        offset_ms: 0,
        duration_ms: 60_000,
        wall_start: started(),
        wall_end: started() + TimeDelta::minutes(1),
        audio_file: "audio/segment-01.opus".into(),
        channels: 1,
        ends_in_pause: false,
    };
    {
        let mut s = store.lock().unwrap();
        s.create_session(id, started(), None).unwrap();
        assert!(
            s.set_title(id, &format!("Note {id}"), session_core::TitleSource::User)
                .unwrap()
        );
        s.put_segment(id, &segment).unwrap();
        s.set_asr_model(id, "m").unwrap();
        s.apply(
            id,
            &Event::UserStop {
                at: segment.wall_end,
                retained_audio: true,
            },
        )
        .unwrap();
        s.dequeue_session(id, "test", segment.wall_end).unwrap();
    }
    let working = working.join(id);
    fs::create_dir_all(working.join("audio")).unwrap();
    fs::write(working.join(&segment.audio_file), b"opus").unwrap();
    fs::write(working.join("transcript.md"), TRANSCRIPT).unwrap();
    pipeline::checkpoint::mark_complete(store, id, Stage::Verify, 1).unwrap();
    pipeline::save::run(
        store,
        id,
        &working,
        &[segment],
        Some(root),
        &RealFs,
        &pipeline::Unattended,
    )
    .unwrap()
    .path
}

/// The text of the old-layout `session.md` of `doc`: the frontmatter that
/// an older Longhand wrote, with a key and a body line that an agent added,
/// and tags.
fn old_session_md(doc: &SessionDoc) -> String {
    let rendered = doc.render();
    let rendered = rendered.replacen("---\n", "---\nsource: agent\n", 1);
    format!("{rendered}A line that a person added.\n")
}

/// Write the note in `home` in the old layout, as an older Longhand wrote
/// it: the records in `session.md` and the summary data in the
/// frontmatter of `summary.md`. `longhand.json` is removed. Returns the
/// files of the note before the change.
fn write_old_layout(home: &Path) -> NoteFiles {
    let files = NoteFiles::read(home).unwrap();
    let mut doc = files.doc.clone();
    doc.tags = vec!["Person::Nathan".into(), "follow-up".into()];
    let body = fs::read_to_string(home.join("summary.md")).unwrap();
    let old_summary = Summary {
        meta: files.record.summary.clone().unwrap(),
        body: body.trim().to_string(),
    }
    .render();
    doc.summary_fingerprint = Some(fingerprint(old_summary.as_bytes()));
    fs::write(home.join("summary.md"), &old_summary).unwrap();
    fs::write(home.join("session.md"), old_session_md(&doc)).unwrap();
    fs::remove_file(home.join(NOTE_RECORD_FILE)).unwrap();
    files
}

/// Each file of `dir` with its bytes, by name, without the in-use markers.
fn contents(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_file() && !name.starts_with('.') {
            out.insert(name, fs::read(entry.path()).unwrap());
        }
    }
    out
}

fn no_crash(_: Step) -> io::Result<()> {
    Ok(())
}

fn never_claimed(_: &str) -> bool {
    false
}

fn convert_all(store: &SharedStore, notes: &[(&str, &Path)]) -> ConvertReport {
    let notes: Vec<(String, PathBuf)> = notes
        .iter()
        .map(|(id, path)| (id.to_string(), path.to_path_buf()))
        .collect();
    convert::convert_notes(store, &notes, &[], &never_claimed)
}

// Covers R10: the conversion keeps every record, the tags, the title, the
// body and a key that an agent added.
#[test]
fn a_saved_note_converts_with_every_record_its_tags_title_and_body() {
    let (_dir, root) = storage();
    let working = tempfile::tempdir().unwrap();
    let store = store();
    let home = save_note(&store, working.path(), "n", &root);
    let new_summary = fs::read(home.join("summary.md")).unwrap();
    let before = write_old_layout(&home);

    let report = convert_all(&store, &[("n", &home)]);
    assert_eq!(report.converted, ["n"]);
    assert!(
        report.skipped.is_empty() && report.failed.is_empty(),
        "{report:?}"
    );

    let after = NoteFiles::read(&home).unwrap();
    assert_eq!(after.layout, Layout::New);
    assert_eq!(after.record, before.record);
    assert_eq!(after.doc.title.as_deref(), Some("Note n"));
    assert_eq!(after.doc.tags, ["Person::Nathan", "follow-up"]);
    // `summary.md` has the body only, as a new save writes it.
    assert_eq!(fs::read(home.join("summary.md")).unwrap(), new_summary);
    let session_md = fs::read_to_string(home.join("session.md")).unwrap();
    assert!(!session_md.contains("session_id"), "{session_md}");
    assert!(!session_md.contains("segments"), "{session_md}");
    assert!(
        session_md.starts_with("---\nsource: agent\n"),
        "{session_md}"
    );
    assert!(
        session_md.ends_with("A line that a person added.\n"),
        "{session_md}"
    );
    // The store records the files that this computer wrote.
    let written = store
        .lock()
        .unwrap()
        .written_fingerprints("n")
        .unwrap()
        .session;
    assert_eq!(
        written,
        Some(written_fingerprint(
            session_md.as_bytes(),
            &fs::read(home.join(NOTE_RECORD_FILE)).unwrap()
        ))
    );
}

// Covers R13: a crash after each write, then a second run, gives the same
// files as one run.
#[test]
fn a_crash_after_each_step_and_a_second_run_give_the_result_of_one_run() {
    let (_dir, root) = storage();
    let working = tempfile::tempdir().unwrap();
    let store = store();
    let home = save_note(&store, working.path(), "n", &root);
    write_old_layout(&home);
    let old = contents(&home);

    let clean = tempfile::tempdir().unwrap();
    for (name, bytes) in &old {
        fs::write(clean.path().join(name), bytes).unwrap();
    }
    assert!(matches!(
        convert::convert_folder(clean.path(), &mut no_crash).unwrap(),
        FolderConversion::Converted { .. }
    ));
    let expected = contents(clean.path());

    for crash in [Step::Summary, Step::SessionMd] {
        let folder = tempfile::tempdir().unwrap();
        for (name, bytes) in &old {
            fs::write(folder.path().join(name), bytes).unwrap();
        }
        let mut stop = |step: Step| {
            if step == crash {
                Err(io::Error::other("the computer stopped"))
            } else {
                Ok(())
            }
        };
        assert!(convert::convert_folder(folder.path(), &mut stop).is_err());
        // Until the last step, the note is in the old layout (KTD6).
        assert_eq!(
            NoteFiles::read(folder.path()).unwrap().layout,
            Layout::Old,
            "{crash:?}"
        );
        convert::convert_folder(folder.path(), &mut no_crash).unwrap();
        assert_eq!(contents(folder.path()), expected, "{crash:?}");
    }
}

// Covers R10, R12: a note with a fresh in-use marker of another computer
// is not converted now, with no failure, and converts on the next run.
#[test]
fn a_note_in_use_on_another_computer_is_skipped_and_converts_on_the_next_run() {
    let (_dir, root) = storage();
    let working = tempfile::tempdir().unwrap();
    let store = store();
    let home = save_note(&store, working.path(), "n", &root);
    write_old_layout(&home);
    assert!(
        session_save::in_use_marker::write_marker(&home, "run-a", Local::now().fixed_offset())
            .unwrap()
    );
    let old = contents(&home);

    let report = convert_all(&store, &[("n", &home)]);
    assert_eq!(report.skipped, ["n"]);
    assert!(report.converted.is_empty() && report.failed.is_empty());
    assert_eq!(contents(&home), old);

    session_save::in_use_marker::remove_marker(&home, "run-a").unwrap();
    assert_eq!(convert_all(&store, &[("n", &home)]).converted, ["n"]);
    assert_eq!(NoteFiles::read(&home).unwrap().layout, Layout::New);
}

// Covers AE4, R13: after the conversion, an older build writes one note in
// the old layout. A second conversion changes only that note.
#[test]
fn a_second_conversion_changes_only_the_note_that_an_older_build_wrote() {
    let (_dir, root) = storage();
    let working = tempfile::tempdir().unwrap();
    let store = store();
    let first = save_note(&store, working.path(), "n1", &root);
    let second = save_note(&store, working.path(), "n2", &root);
    write_old_layout(&first);
    write_old_layout(&second);
    convert_all(&store, &[("n1", &first), ("n2", &second)]);
    let converted = contents(&first);

    // An older build changes the title of n2 and writes the old layout.
    // Its `longhand.json` stays.
    let mut doc = NoteFiles::read(&second).unwrap().doc;
    doc.title = Some("Older build".into());
    fs::write(second.join("session.md"), doc.render()).unwrap();
    assert_eq!(NoteFiles::read(&second).unwrap().layout, Layout::Old);

    let report = convert_all(&store, &[("n1", &first), ("n2", &second)]);
    assert_eq!(report.converted, ["n2"]);
    assert_eq!(contents(&first), converted);
    let after = NoteFiles::read(&second).unwrap();
    assert_eq!(after.layout, Layout::New);
    assert_eq!(after.doc.title.as_deref(), Some("Older build"));
    assert_eq!(
        after.record.written_title.map(|t| t.text).as_deref(),
        Some("Older build")
    );
    // The summary data stays: the summary did not change.
    assert!(after.record.summary.is_some());
}

// Covers R10: two computers that convert one note write the same bytes, so
// the sync makes no conflict copy.
#[test]
fn two_computers_that_convert_one_note_write_identical_files() {
    let (_dir, root) = storage();
    let working = tempfile::tempdir().unwrap();
    let store = store();
    let home = save_note(&store, working.path(), "n", &root);
    write_old_layout(&home);
    let other = tempfile::tempdir().unwrap();
    for (name, bytes) in contents(&home) {
        fs::write(other.path().join(name), bytes).unwrap();
    }

    convert::convert_folder(&home, &mut no_crash).unwrap();
    convert::convert_folder(other.path(), &mut no_crash).unwrap();
    assert_eq!(contents(&home), contents(other.path()));
}

// Covers R10, F3: a note with no recording, with only `notes.md` and an
// old `session.md`, converts.
#[test]
fn a_note_with_only_notes_and_an_old_session_md_converts() {
    let (_dir, root) = storage();
    let home = root.join("2026-09-25 0900 Notes only");
    fs::create_dir(&home).unwrap();
    let mut doc = SessionDoc::without_audio("e", started(), None);
    doc.title = Some("Notes only".into());
    fs::write(home.join("session.md"), doc.render()).unwrap();
    fs::write(home.join("notes.md"), "- bring snacks\n").unwrap();

    let report = convert_all(&store(), &[("e", &home)]);
    assert_eq!(report.converted, ["e"]);
    let after = NoteFiles::read(&home).unwrap();
    assert_eq!(after.layout, Layout::New);
    assert_eq!(after.doc.result_flag(), doc.result_flag());
    assert_eq!(after.doc.title.as_deref(), Some("Notes only"));
    assert_eq!(
        fs::read_to_string(home.join("notes.md")).unwrap(),
        "- bring snacks\n"
    );
}

// Covers R10: a note that does not convert stays in the old layout and is
// named in the report.
#[test]
fn a_note_that_does_not_convert_stays_old_and_is_named() {
    let (_dir, root) = storage();
    let home = root.join("2026-09-25 0900 Flow");
    fs::create_dir(&home).unwrap();
    // The title is an alias of the `session_id` anchor: with no
    // `session_id`, the frontmatter does not parse.
    fs::write(
        home.join("session.md"),
        "---\nsession_id: &id f\nstarted: 2026-09-25T09:00:00-05:00\ntitle: *id\n---\n",
    )
    .unwrap();
    let report = convert_all(&store(), &[("f", &home)]);
    assert!(report.converted.is_empty());
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].0, "f");
    assert_eq!(NoteFiles::read(&home).unwrap().layout, Layout::Old);
}

// The summary data of an old note is in the frontmatter of `summary.md`.
#[test]
fn the_summary_data_of_an_old_note_moves_into_longhand_json() {
    let (_dir, root) = storage();
    let home = root.join("2026-09-25 0900 Plan");
    fs::create_dir(&home).unwrap();
    let meta = SummaryMeta::Complete {
        endpoint_host: "api.example.com".into(),
        model: "m".into(),
        prompt_name: Some("Team sync".into()),
        prompt_id: Some("b1".into()),
    };
    let summary = Summary {
        meta: meta.clone(),
        body: "# Summary\n\nWe agreed.".into(),
    };
    let mut doc = SessionDoc::without_audio("p", started(), None);
    doc.result = None;
    doc.summary_fingerprint = Some(fingerprint(summary.render().as_bytes()));
    fs::write(home.join("summary.md"), summary.render()).unwrap();
    fs::write(home.join("session.md"), doc.render()).unwrap();

    convert::convert_folder(&home, &mut no_crash).unwrap();
    let after = NoteFiles::read(&home).unwrap();
    assert_eq!(after.record.summary, Some(meta));
    assert_eq!(
        fs::read_to_string(home.join("summary.md")).unwrap(),
        summary.render_body()
    );
    assert_eq!(
        after.record.summary_fingerprint,
        Some(fingerprint(summary.render_body().as_bytes()))
    );
}

// The body of an old summary that starts with a
// rule is written by the rule of `render_body`, so that it shows in full.
#[test]
fn an_old_summary_whose_body_starts_with_a_rule_converts_to_the_body_in_full() {
    let (_dir, root) = storage();
    let home = root.join("2026-09-25 0900 Rule");
    fs::create_dir(&home).unwrap();
    let summary = Summary {
        meta: SummaryMeta::stub(),
        body: "---\nOwner: Anna\n---\n\n## Overview\n\nWe agreed.".into(),
    };
    let mut doc = SessionDoc::without_audio("r", started(), None);
    doc.result = None;
    doc.summary_fingerprint = Some(fingerprint(summary.render().as_bytes()));
    fs::write(home.join("summary.md"), summary.render()).unwrap();
    fs::write(home.join("session.md"), doc.render()).unwrap();

    convert::convert_folder(&home, &mut no_crash).unwrap();
    let after = NoteFiles::read(&home).unwrap();
    let text = fs::read_to_string(home.join("summary.md")).unwrap();
    assert_eq!(text, summary.render_body());
    assert_eq!(Summary::body_of(&text), summary.body);
    assert_eq!(
        after.record.summary_fingerprint,
        Some(fingerprint(summary.render_body().as_bytes()))
    );
}

/// Copy each file of `from` into a new folder.
fn copy_of(from: &BTreeMap<String, Vec<u8>>) -> tempfile::TempDir {
    let folder = tempfile::tempdir().unwrap();
    for (name, bytes) in from {
        fs::write(folder.path().join(name), bytes).unwrap();
    }
    folder
}

/// Set the summary fingerprint of the old-layout `session.md` in `home`.
fn set_summary_fingerprint(home: &Path, value: Option<String>) {
    let mut doc = NoteFiles::read(home).unwrap().doc;
    doc.summary_fingerprint = value;
    fs::write(home.join("session.md"), old_session_md(&doc)).unwrap();
}

// Covers R10, R13: a summary that differs from its fingerprint, or that
// has no fingerprint, keeps its summary data after a stop between the
// `summary.md` write and the `session.md` write and a second run.
#[test]
fn a_crash_keeps_the_summary_data_of_a_summary_that_differs_from_its_fingerprint() {
    for fingerprint in [Some("another summary".to_string()), None] {
        let (_dir, root) = storage();
        let working = tempfile::tempdir().unwrap();
        let store = store();
        let home = save_note(&store, working.path(), "n", &root);
        let before = write_old_layout(&home);
        set_summary_fingerprint(&home, fingerprint.clone());
        let old = contents(&home);

        let clean = copy_of(&old);
        convert::convert_folder(clean.path(), &mut no_crash).unwrap();
        let expected = contents(clean.path());
        let one_run = NoteFiles::read(clean.path()).unwrap().record;
        assert_eq!(one_run.summary, before.record.summary, "{fingerprint:?}");

        // A stop before the `session.md` write.
        let folder = copy_of(&old);
        let mut stop = |step: Step| {
            if step == Step::SessionMd {
                Err(io::Error::other("the computer stopped"))
            } else {
                Ok(())
            }
        };
        assert!(convert::convert_folder(folder.path(), &mut stop).is_err());
        convert::convert_folder(folder.path(), &mut no_crash).unwrap();
        assert_eq!(contents(folder.path()), expected, "{fingerprint:?}");

        // A sync changes `session.md` after the `summary.md` write.
        let folder = copy_of(&old);
        let session_md = folder.path().join("session.md");
        let mut sync = |step: Step| {
            if step == Step::Summary {
                let text = fs::read_to_string(&session_md).unwrap();
                fs::write(&session_md, format!("{text}A line from a sync.\n")).unwrap();
            }
            Ok(())
        };
        assert_eq!(
            convert::convert_folder(folder.path(), &mut sync).unwrap(),
            FolderConversion::Changed
        );
        convert::convert_folder(folder.path(), &mut no_crash).unwrap();
        let after = NoteFiles::read(folder.path()).unwrap();
        assert_eq!(after.layout, Layout::New);
        assert_eq!(after.record.summary, one_run.summary, "{fingerprint:?}");
        assert_eq!(
            after.record.summary_fingerprint, one_run.summary_fingerprint,
            "{fingerprint:?}"
        );
    }
}

// Covers R10: the conversion removes only the summary data from the
// frontmatter of `summary.md`. A key and a comment that a person added
// stay, also after a stop and a second run.
#[test]
fn the_conversion_keeps_other_keys_and_comments_of_the_summary_frontmatter() {
    let (_dir, root) = storage();
    let working = tempfile::tempdir().unwrap();
    let store = store();
    let home = save_note(&store, working.path(), "n", &root);
    let body_only = fs::read_to_string(home.join("summary.md")).unwrap();
    let before = write_old_layout(&home);
    let old_summary = fs::read_to_string(home.join("summary.md")).unwrap();
    let edited = old_summary.replacen("---\n", "---\nreviewed_by: Alice\n# A comment.\n", 1);
    fs::write(home.join("summary.md"), &edited).unwrap();
    set_summary_fingerprint(&home, Some(fingerprint(edited.as_bytes())));
    let old = contents(&home);

    let report = convert_all(&store, &[("n", &home)]);
    assert_eq!(report.converted, ["n"], "{report:?}");
    let summary = fs::read_to_string(home.join("summary.md")).unwrap();
    assert_eq!(
        summary,
        format!("---\nreviewed_by: Alice\n# A comment.\n---\n\n{body_only}")
    );
    let after = NoteFiles::read(&home).unwrap();
    assert_eq!(after.layout, Layout::New);
    assert_eq!(after.record.summary, before.record.summary);
    assert_eq!(
        after.record.summary_fingerprint,
        Some(fingerprint(summary.as_bytes()))
    );
    // The note page shows the body only.
    assert_eq!(Summary::body_of(&summary), body_only.trim());
    let expected = contents(&home);

    for crash in [Step::Summary, Step::SessionMd] {
        let folder = copy_of(&old);
        let mut stop = |step: Step| {
            if step == crash {
                Err(io::Error::other("the computer stopped"))
            } else {
                Ok(())
            }
        };
        assert!(convert::convert_folder(folder.path(), &mut stop).is_err());
        convert::convert_folder(folder.path(), &mut no_crash).unwrap();
        assert_eq!(contents(folder.path()), expected, "{crash:?}");
    }
}
