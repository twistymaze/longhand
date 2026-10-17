//! Tag edits and the tag list (U6 of the note types and tags plan, R9, R10,
//! R12, R13, R15, R18, R30, KTD10, KTD11). Notes are saved through the real
//! save stage into one storage folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, FixedOffset, Local, TimeDelta};
use pipeline::SharedStore;
use pipeline::early_home::{self, EarlyState};
use pipeline::tags::{
    self, CONTROL_IN_TAG, EMPTY_TAG, NOTE_UNAVAILABLE, TAGS_NOT_TEXT, TagCount, check_tag,
};
use session_core::folder::{ResultFlag, SessionDoc};
use session_core::jobs::Claims;
use session_core::{BulkEdit, Event, Segment, SessionRecord, SessionState, Stage, Store};
use session_save::RealFs;
use session_save::in_use_marker::write_marker;

const TRANSCRIPT: &str = "# Transcript\n\n- [00:00:01] Speaker 1: We start with the budget.\n";
const RUN: &str = "run-b";

fn started() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-09-25T09:00:00-05:00").unwrap()
}

fn now() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

fn texts(tags: &[&str]) -> Vec<String> {
    tags.iter().map(|t| t.to_string()).collect()
}

struct Computer {
    store: SharedStore,
    claims: Claims,
    working: tempfile::TempDir,
}

fn computer() -> Computer {
    Computer {
        store: Arc::new(Mutex::new(Store::open_in_memory().unwrap())),
        claims: Claims::default(),
        working: tempfile::tempdir().unwrap(),
    }
}

impl Computer {
    /// Record, process and save the note `id`, which started at `at`, into
    /// `root` through the real save stage, then give its `session.md` the
    /// tags `tags` and refresh the row. Returns its folder.
    fn save_note(
        &self,
        id: &str,
        root: &Path,
        at: DateTime<FixedOffset>,
        tags: &[&str],
    ) -> PathBuf {
        let segment = Segment {
            index: 0,
            offset_ms: 0,
            duration_ms: 60_000,
            wall_start: at,
            wall_end: at + TimeDelta::minutes(1),
            audio_file: "audio/segment-01.opus".into(),
            channels: 1,
            ends_in_pause: false,
        };
        {
            let mut s = self.store.lock().unwrap();
            s.create_session(id, at, None).unwrap();
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
        let working = self.working.path().join(id);
        fs::create_dir_all(working.join("audio")).unwrap();
        fs::write(working.join(&segment.audio_file), b"opus").unwrap();
        fs::write(working.join("transcript.md"), TRANSCRIPT).unwrap();
        pipeline::checkpoint::mark_complete(&self.store, id, Stage::Verify, 1).unwrap();
        let home = pipeline::save::run(
            &self.store,
            id,
            &working,
            &[segment],
            Some(root),
            &RealFs,
            &pipeline::Unattended,
        )
        .unwrap()
        .path;
        if !tags.is_empty() {
            edit_doc(&home, |doc| doc.tags = texts(tags));
            self.store
                .lock()
                .unwrap()
                .set_tags(id, &texts(tags))
                .unwrap();
        }
        home
    }

    /// Start a note that records, and write its home folder in `root` as
    /// the early write does. Returns the folder.
    fn start_recording(&self, id: &str, root: &Path) -> PathBuf {
        {
            let mut s = self.store.lock().unwrap();
            s.create_session(id, started(), None).unwrap();
            early_home::begin(&s, id).unwrap();
        }
        early_home::write(&self.store, id, Some(root), RUN, &RealFs)
            .unwrap()
            .unwrap()
    }

    fn set(&self, id: &str, add: &[&str], remove: &[&str]) -> Result<Vec<String>, String> {
        tags::set_note_tags(&self.store, &self.claims, id, &texts(add), &texts(remove))
            .map_err(|e| e.to_string())
    }

    fn apply(&self, edit: &BulkEdit) -> tags::TagPass {
        tags::apply_edit(&self.store, &self.claims, edit).unwrap()
    }

    fn list(&self) -> Vec<TagCount> {
        tags::tag_list(&self.store.lock().unwrap()).unwrap()
    }

    fn record(&self, id: &str) -> SessionRecord {
        self.store.lock().unwrap().session(id).unwrap().unwrap()
    }
}

/// A storage folder with the library marker.
fn storage() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Notes");
    fs::create_dir(&root).unwrap();
    assert!(library::scan::write_marker(&root));
    (dir, root)
}

fn document(home: &Path) -> SessionDoc {
    session_core::note_folder::NoteFiles::read(home)
        .unwrap()
        .doc
}

fn edit_doc(home: &Path, edit: impl FnOnce(&mut SessionDoc)) {
    let mut doc = document(home);
    edit(&mut doc);
    session_save::note_files::write_doc(home, &doc).unwrap();
}

#[test]
fn a_tag_edit_changes_only_the_tag_lines_of_session_md() {
    // R4, KTD3: the agent's key, comment, quote style and body stay.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("n", &root, started(), &[]);
    let agent = "---\n# set by an agent\ntitle: 'Note n'\nlonghand_tags:\n- Person::Nathan\nsource: agent\n---\n\nThe agent's text.\n";
    fs::write(home.join("session.md"), agent).unwrap();
    let records = fs::read(home.join("longhand.json")).unwrap();

    b.set("n", &["follow-up"], &[]).unwrap();

    assert_eq!(
        fs::read_to_string(home.join("session.md")).unwrap(),
        "---\n# set by an agent\ntitle: 'Note n'\nlonghand_tags:\n- Person::Nathan\n- follow-up\nsource: agent\n---\n\nThe agent's text.\n"
    );
    // A tag edit changes no record.
    assert_eq!(fs::read(home.join("longhand.json")).unwrap(), records);
}

#[test]
fn a_tag_edit_writes_one_tag_for_each_tag_key() {
    // R8: an agent wrote one tag key two times. The second tag has
    // quotes: with no quotes, YAML reads `person :: nathan` as a mapping.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("n", &root, started(), &[]);
    fs::write(
        home.join("session.md"),
        "---\ntitle: Note n\nlonghand_tags:\n- Person::Nathan\n- 'person :: nathan'\n---\n",
    )
    .unwrap();

    assert_eq!(
        b.set("n", &["follow-up"], &[]),
        Ok(texts(&["Person::Nathan", "follow-up"]))
    );
    assert_eq!(
        fs::read_to_string(home.join("session.md")).unwrap(),
        "---\ntitle: Note n\nlonghand_tags:\n- Person::Nathan\n- follow-up\n---\n"
    );
}

#[test]
fn a_tag_edit_of_a_note_whose_session_md_does_not_parse_is_refused_with_no_write() {
    // R6: the broken file stays as it is, so the user can correct it.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("n", &root, started(), &[]);
    let broken = "---\ntitle: Note n\n  bad: indent\n---\n";
    fs::write(home.join("session.md"), broken).unwrap();
    let records = fs::read(home.join("longhand.json")).unwrap();

    assert!(b.set("n", &["follow-up"], &[]).is_err());
    assert_eq!(fs::read_to_string(home.join("session.md")).unwrap(), broken);
    assert_eq!(fs::read(home.join("longhand.json")).unwrap(), records);
}

#[test]
fn a_tag_edit_keeps_the_text_of_a_tag_that_reads_as_a_number() {
    // `1.50` stays `1.50`, not `1.5`.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("n", &root, started(), &[]);
    fs::write(
        home.join("session.md"),
        "---\ntitle: Note n\nlonghand_tags:\n- 1.50\n---\n",
    )
    .unwrap();

    assert_eq!(
        b.set("n", &["follow-up"], &[]),
        Ok(texts(&["1.50", "follow-up"]))
    );
    assert_eq!(document(&home).tags, texts(&["1.50", "follow-up"]));
}

#[test]
fn a_tag_edit_of_a_note_with_a_tag_item_that_is_not_text_is_refused_with_no_write() {
    // The write would remove the item, so the file stays as it is and the
    // message tells the user to fix it. A tag rename leaves the note for a
    // later pass, as for a `session.md` that does not parse.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("n", &root, started(), &["Customer::Mayo"]);
    for text in [
        "---\ntitle: Note n\nlonghand_tags:\n- Customer::Mayo\n- ~\n---\n",
        "---\ntitle: Note n\nlonghand_tags:\n- Customer::Mayo\n- [a]\n---\n",
    ] {
        fs::write(home.join("session.md"), text).unwrap();
        let records = fs::read(home.join("longhand.json")).unwrap();

        assert_eq!(
            b.set("n", &["follow-up"], &[]),
            Err(TAGS_NOT_TEXT.to_string())
        );
        let pass = b.apply(&BulkEdit::RenameTag {
            from: "Customer::Mayo".into(),
            to: "Customer::Mayo Clinic".into(),
        });
        assert_eq!((pass.matched, pass.done), (1, false), "{text}");
        assert_eq!(fs::read_to_string(home.join("session.md")).unwrap(), text);
        assert_eq!(fs::read(home.join("longhand.json")).unwrap(), records);
        assert_eq!(b.record("n").tags, texts(&["Customer::Mayo"]));
    }
}

#[test]
fn a_tag_edit_of_an_old_layout_note_goes_through_the_patcher() {
    // KTD6: before the conversion, the tags of an old note still change,
    // and the records stay in `session.md`.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("n", &root, started(), &[]);
    let doc = document(&home);
    fs::remove_file(home.join("longhand.json")).unwrap();
    let old = format!("{}A line that a person added.\n", doc.render());
    fs::write(home.join("session.md"), &old).unwrap();

    assert_eq!(b.set("n", &["follow-up"], &[]), Ok(texts(&["follow-up"])));
    let text = fs::read_to_string(home.join("session.md")).unwrap();
    assert!(text.ends_with("A line that a person added.\n"), "{text}");
    assert!(!home.join("longhand.json").exists());
    let files = session_core::note_folder::NoteFiles::read(&home).unwrap();
    assert_eq!(files.layout, session_core::note_folder::Layout::Old);
    assert_eq!(files.doc.tags, texts(&["follow-up"]));
    assert_eq!(files.doc.segments, doc.segments);
}

#[test]
fn a_tag_text_is_trimmed_and_must_not_be_empty_or_have_a_line_break() {
    assert_eq!(
        check_tag("  Person :: Nathan "),
        Ok("Person :: Nathan".into())
    );
    assert_eq!(
        check_tag("Topic::Security").as_deref(),
        Ok("Topic::Security")
    );
    for text in ["", "   ", "\t"] {
        assert_eq!(check_tag(text), Err(EMPTY_TAG.to_string()), "{text:?}");
    }
    for text in ["a\nb", "a\tb", "a\u{7}b"] {
        assert_eq!(check_tag(text), Err(CONTROL_IN_TAG.to_string()), "{text:?}");
    }
}

#[test]
fn a_tag_added_to_a_recording_note_is_written_and_the_recording_continues() {
    // Covers R10, R11.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.start_recording("r", &root);
    let state = b.record("r").state;

    assert_eq!(
        b.set("r", &["Topic::Security"], &[]),
        Ok(texts(&["Topic::Security"]))
    );
    let doc = document(&home);
    assert_eq!(doc.tags, texts(&["Topic::Security"]));
    assert_eq!(doc.result, Some(ResultFlag::NotSaved));
    assert_eq!(b.record("r").tags, texts(&["Topic::Security"]));
    assert_eq!(b.record("r").state, state);
    // The in-use marker of the recording stays.
    assert!(
        fs::read_dir(&home)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains(RUN))
    );
}

#[test]
fn a_second_tag_with_the_same_scope_is_kept() {
    // Covers AE4, R15.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("n", &root, started(), &["Person::Nathan"]);

    let tags = b.set("n", &["Person::Sarah"], &[]).unwrap();
    assert_eq!(tags, texts(&["Person::Nathan", "Person::Sarah"]));
    assert_eq!(document(&home).tags, tags);
    assert_eq!(b.record("n").tags, tags);
}

#[test]
fn a_tag_with_the_key_of_a_tag_of_the_note_changes_nothing() {
    // Covers R30.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("n", &root, started(), &["Person::Nathan"]);
    let before = fs::read(home.join("session.md")).unwrap();

    assert_eq!(
        b.set("n", &["person::nathan"], &[]),
        Ok(texts(&["Person::Nathan"]))
    );
    assert_eq!(
        b.set("n", &[" PERSON :: NATHAN "], &[]),
        Ok(texts(&["Person::Nathan"]))
    );
    assert_eq!(fs::read(home.join("session.md")).unwrap(), before);
}

#[test]
fn a_tag_is_removed_by_its_key() {
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("n", &root, started(), &["Person::Nathan", "follow-up"]);

    assert_eq!(
        b.set("n", &[], &["person :: nathan"]),
        Ok(texts(&["follow-up"]))
    );
    assert_eq!(document(&home).tags, texts(&["follow-up"]));
}

#[test]
fn removing_the_last_follow_up_removes_it_from_the_tag_list() {
    // Covers AE6, R12.
    let (_dir, root) = storage();
    let b = computer();
    b.save_note("n", &root, started(), &["follow-up", "Topic::Security"]);
    b.save_note("m", &root, started(), &["Topic::Security"]);
    assert_eq!(
        b.list(),
        vec![
            TagCount {
                text: "follow-up".into(),
                key: "follow-up".into(),
                count: 1
            },
            TagCount {
                text: "Topic::Security".into(),
                key: "topic::security".into(),
                count: 2
            },
        ]
    );

    b.set("n", &[], &["follow-up"]).unwrap();
    let list = b.list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].key, "topic::security");
}

#[test]
fn the_tag_list_shows_the_spelling_of_the_note_with_the_earliest_start() {
    // Covers R30, KTD10.
    let (_dir, root) = storage();
    let b = computer();
    b.save_note(
        "late",
        &root,
        started() + TimeDelta::days(2),
        &["CUSTOMER::Mayo"],
    );
    b.save_note("early", &root, started(), &["customer :: mayo"]);
    b.save_note(
        "middle",
        &root,
        started() + TimeDelta::days(1),
        &["Customer::Mayo"],
    );

    assert_eq!(
        b.list(),
        vec![TagCount {
            text: "customer :: mayo".into(),
            key: "customer::mayo".into(),
            count: 3
        }]
    );
}

#[test]
fn a_tag_edit_keeps_a_tag_that_another_computer_added_and_that_has_synced() {
    // Covers R18: the edit reads the tags of the file, not of the row.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("n", &root, started(), &["Person::Nathan"]);
    // Another computer adds a tag. The sync delivers it before a scan.
    edit_doc(&home, |doc| doc.tags.push("Topic::Security".into()));

    let tags = b.set("n", &["follow-up"], &[]).unwrap();
    assert_eq!(
        tags,
        texts(&["Person::Nathan", "Topic::Security", "follow-up"])
    );
    assert_eq!(document(&home).tags, tags);
    assert_eq!(b.record("n").tags, tags);
}

#[test]
fn a_tag_edit_of_a_note_that_cannot_be_read_changes_nothing_and_says_why() {
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("n", &root, started(), &["Person::Nathan"]);
    // A `session.md` with no `longhand.json` waits for a sync (KTD6).
    fs::remove_file(home.join("longhand.json")).unwrap();

    assert_eq!(
        b.set("n", &["follow-up"], &[]),
        Err(NOTE_UNAVAILABLE.to_string())
    );
    assert_eq!(b.record("n").tags, texts(&["Person::Nathan"]));
}

#[test]
fn a_tag_of_a_note_with_no_home_folder_goes_into_the_early_write() {
    // Covers R10 before the home folder is written.
    let (_dir, root) = storage();
    let b = computer();
    {
        let mut s = b.store.lock().unwrap();
        s.create_session("r", started(), None).unwrap();
        early_home::begin(&s, "r").unwrap();
    }
    assert_eq!(
        b.set("r", &["Topic::Security"], &[]),
        Ok(texts(&["Topic::Security"]))
    );
    assert_eq!(b.record("r").tags, texts(&["Topic::Security"]));

    let home = early_home::write(&b.store, "r", Some(&root), RUN, &RealFs)
        .unwrap()
        .unwrap();
    assert_eq!(document(&home).tags, texts(&["Topic::Security"]));
}

#[test]
fn a_tag_of_a_note_whose_staging_folder_is_complete_goes_into_that_folder() {
    let (_dir, root) = storage();
    let b = computer();
    {
        let mut s = b.store.lock().unwrap();
        s.create_session("r", started(), None).unwrap();
        early_home::begin(&s, "r").unwrap();
    }
    let filled =
        early_home::step(&b.store, "r", &EarlyState::Begun, Some(&root), RUN, &RealFs).unwrap();
    assert!(matches!(filled, EarlyState::Filled { .. }));
    early_home::record(&b.store, "r", &filled).unwrap();

    b.set("r", &["Topic::Security"], &[]).unwrap();
    let home = early_home::write(&b.store, "r", Some(&root), RUN, &RealFs)
        .unwrap()
        .unwrap();
    assert_eq!(document(&home).tags, texts(&["Topic::Security"]));
}

fn mayo_notes(b: &Computer, root: &Path, count: usize) -> Vec<PathBuf> {
    (0..count)
        .map(|i| {
            b.save_note(
                &format!("n{i:02}"),
                root,
                started() + TimeDelta::hours(i as i64),
                &["Customer::Mayo", "Topic::Security"],
            )
        })
        .collect()
}

#[test]
fn a_rename_changes_every_note_including_one_that_another_computer_processes() {
    // Covers AE3, R13.
    let (_dir, root) = storage();
    let b = computer();
    let homes = mayo_notes(&b, &root, 12);
    // Another computer processes the first note.
    assert!(write_marker(&homes[0], "run-a", now()).unwrap());
    edit_doc(&homes[0], |doc| doc.result = Some(ResultFlag::NotSaved));

    let edit = BulkEdit::RenameTag {
        from: "customer::mayo".into(),
        to: "Customer::Mayo Clinic".into(),
    };
    let pass = b.apply(&edit);
    assert_eq!(pass.matched, 12);
    assert_eq!(pass.changed.len(), 12);
    assert!(pass.done);
    for (i, home) in homes.iter().enumerate() {
        let want = texts(&["Customer::Mayo Clinic", "Topic::Security"]);
        assert_eq!(document(home).tags, want, "note {i}");
        assert_eq!(b.record(&format!("n{i:02}")).tags, want, "note {i}");
    }
    assert_eq!(document(&homes[0]).result, Some(ResultFlag::NotSaved));
    // A second pass finds nothing to do.
    assert_eq!(
        b.apply(&edit),
        tags::TagPass {
            matched: 0,
            changed: Vec::new(),
            left: Vec::new(),
            done: true
        }
    );
}

#[test]
fn a_rename_onto_an_existing_tag_merges_and_a_note_with_both_keeps_one() {
    // Covers R13.
    let (_dir, root) = storage();
    let b = computer();
    let both = b.save_note(
        "both",
        &root,
        started(),
        &["Customer::Mayo Clinic", "follow-up", "Customer::Mayo"],
    );
    let old = b.save_note("old", &root, started(), &["Customer::Mayo"]);

    let pass = b.apply(&BulkEdit::RenameTag {
        from: "Customer::Mayo".into(),
        to: "Customer::Mayo Clinic".into(),
    });
    assert_eq!(pass.matched, 2);
    assert_eq!(
        document(&both).tags,
        texts(&["Customer::Mayo Clinic", "follow-up"])
    );
    assert_eq!(document(&old).tags, texts(&["Customer::Mayo Clinic"]));
    let list = b.list();
    assert_eq!(list.iter().map(|t| t.count).collect::<Vec<_>>(), vec![2, 1]);
    assert_eq!(list[0].key, "customer::mayo clinic");
}

#[test]
fn a_rename_that_changes_only_the_case_gives_every_note_the_new_spelling() {
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("n", &root, started(), &["customer::mayo"]);

    let edit = BulkEdit::RenameTag {
        from: "customer::mayo".into(),
        to: "Customer::Mayo".into(),
    };
    assert!(b.apply(&edit).done);
    assert_eq!(document(&home).tags, texts(&["Customer::Mayo"]));
    assert_eq!(b.apply(&edit).matched, 0);
}

#[test]
fn a_delete_counts_the_notes_and_removes_the_tag_from_every_note() {
    let (_dir, root) = storage();
    let b = computer();
    let homes = mayo_notes(&b, &root, 3);
    let other = b.save_note("other", &root, started(), &["Topic::Security"]);
    let before = fs::read(other.join("session.md")).unwrap();

    let pass = b.apply(&BulkEdit::DeleteTag {
        name: "CUSTOMER::MAYO".into(),
    });
    assert_eq!((pass.matched, pass.done), (3, true));
    for home in &homes {
        assert_eq!(document(home).tags, texts(&["Topic::Security"]));
    }
    assert_eq!(fs::read(other.join("session.md")).unwrap(), before);
    assert!(b.list().iter().all(|t| t.key != "customer::mayo"));
}

#[test]
fn a_rename_while_the_storage_folder_is_unavailable_continues_later() {
    // Covers KTD11: the failed notes are done by a later pass.
    let (dir, root) = storage();
    let b = computer();
    let homes = mayo_notes(&b, &root, 3);
    let away = dir.path().join("Away");
    fs::rename(&root, &away).unwrap();

    let edit = BulkEdit::RenameTag {
        from: "Customer::Mayo".into(),
        to: "Customer::Mayo Clinic".into(),
    };
    let pass = b.apply(&edit);
    assert_eq!(pass.matched, 3);
    assert!(pass.changed.is_empty());
    assert!(!pass.done);
    assert_eq!(
        b.record("n00").tags,
        texts(&["Customer::Mayo", "Topic::Security"])
    );

    fs::rename(&away, &root).unwrap();
    let pass = b.apply(&edit);
    assert_eq!((pass.changed.len(), pass.done), (3, true));
    for home in &homes {
        assert_eq!(
            document(home).tags,
            texts(&["Customer::Mayo Clinic", "Topic::Security"])
        );
    }
}

/// One pass of the recorded edit `id` of the store of `b`, as a launch or
/// a scan runs it: the notes come from the record, and the record keeps
/// the notes that are not done.
fn continue_recorded(b: &Computer, id: i64) -> tags::TagPass {
    let open = b.store.lock().unwrap().bulk_edits().unwrap();
    let open = open.into_iter().find(|o| o.id == id).unwrap();
    let pass = tags::apply_recorded(&b.store, &b.claims, &open.edit, &open.notes).unwrap();
    let store = b.store.lock().unwrap();
    if pass.done {
        store.remove_bulk_edit(id).unwrap();
    } else {
        store.set_bulk_edit_notes(id, &pass.left).unwrap();
    }
    pass
}

#[test]
fn a_tag_edit_that_waits_keeps_the_tag_that_the_user_adds_after_the_edit() {
    // A tag delete waits for a note whose session.md cannot be read. A tag
    // that the user adds to another note after the delete stays, also
    // after a relaunch.
    let (dir, root) = storage();
    let db = dir.path().join("longhand.db");
    let mut b = computer();
    b.store = Arc::new(Mutex::new(Store::open(&db).unwrap()));
    let stuck = b.save_note("stuck", &root, started(), &["follow-up"]);
    b.save_note("later", &root, started(), &[]);
    let text = fs::read(stuck.join("longhand.json")).unwrap();
    fs::remove_file(stuck.join("longhand.json")).unwrap();

    let edit = BulkEdit::DeleteTag {
        name: "follow-up".into(),
    };
    let id = {
        let store = b.store.lock().unwrap();
        let notes = tags::matching_notes(&store, &edit).unwrap();
        assert_eq!(notes, ["stuck"]);
        store.add_bulk_edit(&edit, &notes).unwrap()
    };
    let pass = continue_recorded(&b, id);
    assert_eq!((pass.matched, pass.done), (1, false));
    assert_eq!(pass.left, ["stuck"]);

    // A relaunch.
    b.store = Arc::new(Mutex::new(Store::open(&db).unwrap()));
    assert_eq!(
        b.set("later", &["follow-up"], &[]),
        Ok(texts(&["follow-up"]))
    );
    let pass = continue_recorded(&b, id);
    assert!(!pass.done);
    assert_eq!(b.record("later").tags, texts(&["follow-up"]));

    fs::write(stuck.join("longhand.json"), &text).unwrap();
    let pass = continue_recorded(&b, id);
    assert_eq!(
        (pass.changed.as_slice(), pass.done),
        (["stuck".to_string()].as_slice(), true)
    );
    assert!(b.record("stuck").tags.is_empty());
    assert_eq!(b.record("later").tags, texts(&["follow-up"]));
    assert!(b.store.lock().unwrap().bulk_edits().unwrap().is_empty());
}

#[test]
fn a_discarded_note_is_not_in_the_tag_list_or_a_tag_edit() {
    let b = computer();
    {
        let mut s = b.store.lock().unwrap();
        s.create_session("d", started(), None).unwrap();
        s.set_tags("d", &texts(&["follow-up"])).unwrap();
        s.apply(
            "d",
            &Event::UserStop {
                at: started() + TimeDelta::minutes(1),
                retained_audio: false,
            },
        )
        .unwrap();
        assert_eq!(
            s.session("d").unwrap().unwrap().state,
            SessionState::Discarded
        );
    }
    assert!(b.list().is_empty());
    assert_eq!(
        b.apply(&BulkEdit::DeleteTag {
            name: "follow-up".into()
        })
        .matched,
        0
    );
}

#[test]
fn a_type_edit_is_not_a_tag_edit() {
    let b = computer();
    assert!(
        tags::apply_edit(
            &b.store,
            &b.claims,
            &BulkEdit::DeleteType {
                name: "Pathfinder".into()
            }
        )
        .is_err()
    );
}
