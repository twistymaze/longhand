//! The type move (U5 of the note types and tags plan): a type change puts
//! the home folder in the folder of the new type, on the computer that
//! owes the move, without loss (R5, R6, R27, R29, R35, KTD3, KTD5, KTD6,
//! KTD7, KTD13). Notes are saved through the real save stage into one
//! storage folder, which one or two computers use.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::{DateTime, FixedOffset, Local, TimeDelta};
use library::reconcile::{Coverage, Reconciler};
use library::scan::{Known, ScanOptions, scan};
use pipeline::SharedStore;
use pipeline::type_move::{
    self, Context, TAKEOVER_AFTER, TAKEOVER_WAIT, TypeChange, change_type_locked, run_owed,
    shown_type,
};
use session_core::folder::{PendingType, ResultFlag, SessionDoc};
use session_core::jobs::Claims;
use session_core::{BulkEdit, Event, RenameIntent, Segment, SessionRecord, Stage, Store};
use session_save::RealFs;
use session_save::in_use_marker::write_marker;

const TRANSCRIPT: &str = "# Transcript\n\n- [00:00:01] Speaker 1: We start with the budget.\n";

fn started() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-09-25T09:00:00-05:00").unwrap()
}

fn now() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

/// One computer: its store, its claims and its working location.
struct Computer {
    store: SharedStore,
    claims: Claims,
    working: tempfile::TempDir,
    reconciler: Reconciler,
    own_runs: Vec<String>,
}

fn computer() -> Computer {
    computer_with(Store::open_in_memory().unwrap(), "run-b")
}

fn computer_with(store: Store, run: &str) -> Computer {
    Computer {
        store: Arc::new(Mutex::new(store)),
        claims: Claims::default(),
        working: tempfile::tempdir().unwrap(),
        reconciler: Reconciler::new(&ScanOptions::default()),
        own_runs: vec![run.to_string()],
    }
}

impl Computer {
    fn ctx(&self, now: DateTime<FixedOffset>) -> Context<'_> {
        Context {
            own_runs: &self.own_runs,
            claims: &self.claims,
            now,
            fs: &RealFs,
        }
    }

    /// Record, process and save the note `id` with the title `title` into
    /// `root` through the real save stage. Returns its folder.
    fn save_note(&self, id: &str, root: &Path, title: &str) -> PathBuf {
        let segment = Segment {
            index: 0,
            offset_ms: 0,
            duration_ms: 60_000,
            wall_start: started(),
            wall_end: started() + TimeDelta::minutes(1),
            audio_file: "audio/segment-01.opus".into(),
            channels: 1,
        };
        {
            let mut s = self.store.lock().unwrap();
            s.create_session(id, started(), None).unwrap();
            assert!(
                s.set_title(id, title, session_core::TitleSource::User)
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
        pipeline::save::run(
            &self.store,
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

    /// Save the note `id` into the type folder `note_type` of `root`.
    fn save_typed(&self, id: &str, root: &Path, note_type: &str, title: &str) -> PathBuf {
        let saved = self.save_note(id, root, title);
        let folder = root.join(note_type);
        fs::create_dir_all(&folder).unwrap();
        let home = folder.join(saved.file_name().unwrap());
        fs::rename(&saved, &home).unwrap();
        let s = self.store.lock().unwrap();
        s.set_home_path(id, &home.to_string_lossy()).unwrap();
        s.set_note_type(id, Some(note_type)).unwrap();
        home
    }

    fn change(&self, id: &str, to: Option<&str>) -> TypeChange {
        self.change_at(id, to, now())
    }

    fn change_at(&self, id: &str, to: Option<&str>, at: DateTime<FixedOffset>) -> TypeChange {
        let mut store = self.store.lock().unwrap();
        change_type_locked(&mut store, id, to, false, &self.ctx(at)).unwrap()
    }

    fn run(&self, at: DateTime<FixedOffset>) -> Vec<String> {
        run_owed(&self.store, &self.ctx(at)).unwrap()
    }

    /// Scan `root` and reconcile the store.
    fn sync(&mut self, root: &Path) {
        let known = Known::from_records(&self.store.lock().unwrap().sessions().unwrap());
        let result = scan(&[root.to_path_buf()], &known, &ScanOptions::default());
        let mut store = self.store.lock().unwrap();
        let claims = self.claims.clone();
        self.reconciler
            .reconcile(
                &mut store,
                &result,
                &Coverage::Full,
                Instant::now(),
                &|id| claims.busy(id),
            )
            .unwrap();
    }

    fn record(&self, id: &str) -> SessionRecord {
        self.store.lock().unwrap().session(id).unwrap().unwrap()
    }

    fn home(&self, id: &str) -> PathBuf {
        PathBuf::from(self.record(id).home_path.unwrap())
    }

    fn owes(&self, id: &str) -> bool {
        self.store.lock().unwrap().type_move(id).unwrap().is_some()
    }

    fn id(&self) -> String {
        self.store.lock().unwrap().computer_id().unwrap()
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
    SessionDoc::parse(&fs::read_to_string(home.join("session.md")).unwrap()).unwrap()
}

fn write_document(home: &Path, doc: &SessionDoc) {
    fs::write(home.join("session.md"), doc.render()).unwrap();
}

/// The visible entries of `dir`, sorted.
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names
}

fn name_of(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

/// Another computer records into the note: its in-use marker is fresh.
fn in_use_elsewhere(home: &Path) {
    assert!(write_marker(home, "run-a", now()).unwrap());
}

fn in_use_ended(home: &Path) {
    session_save::in_use_marker::remove_marker(home, "run-a").unwrap();
}

#[test]
fn an_idle_note_moves_into_the_new_type_folder_at_once() {
    // Covers F2 (the idle part) and R5.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("s", &root, "Standup");

    let change = b.change("s", Some("One on One"));
    let moved = root.join("One on One").join(name_of(&home));
    assert_eq!(change, TypeChange::Moved(moved.clone()));
    assert!(moved.join("session.md").exists());
    assert!(!home.exists());
    assert_eq!(b.home("s"), moved);
    assert_eq!(b.record("s").note_type.as_deref(), Some("One on One"));
    assert_eq!(b.record("s").pending_type, None);
    assert_eq!(document(&moved).pending_type, None);
    assert!(!b.owes("s"));
}

/// The watcher gives the new type folder and the home folder in it to one
/// targeted scan. The note has no other version, also after the wait.
#[test]
fn a_move_into_a_new_type_folder_shows_no_other_version() {
    use library::reconcile::Target;
    use library::status::OTHER_VERSION_WAIT;
    let (_dir, root) = storage();
    let mut b = computer();
    let home = b.save_note("s", &root, "Standup");
    b.change("s", Some("Work"));
    let folder = root.join("Work");
    let moved = folder.join(name_of(&home));

    let target = Target::Folders {
        root: root.clone(),
        folders: [folder, moved].into_iter().collect(),
    };
    let start = Instant::now();
    for at in [start, start + OTHER_VERSION_WAIT] {
        b.reconciler
            .run_at(&b.store, &target, &|_| false, at)
            .unwrap();
        let status = b.reconciler.status().note("s");
        assert!(status.other_versions.is_empty(), "{status:?}");
    }
}

#[test]
fn a_note_in_use_on_another_computer_moves_after_the_marker_is_gone() {
    // Covers AE1 and R5.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("s", &root, "Standup");
    in_use_elsewhere(&home);

    assert_eq!(b.change("s", Some("One on One")), TypeChange::Later);
    let field = document(&home).pending_type.unwrap();
    assert_eq!(field.to.as_deref(), Some("One on One"));
    assert_eq!(field.from, None);
    assert_eq!(field.by, b.id());
    assert!(b.owes("s"));
    assert_eq!(
        shown_type(&b.record("s")),
        (Some("One on One".into()), true)
    );

    // B does not move while the marker is fresh.
    assert!(b.run(now()).is_empty());
    assert!(home.exists());

    in_use_ended(&home);
    assert_eq!(b.run(now()), ["s"]);
    let moved = root.join("One on One").join(name_of(&home));
    assert_eq!(b.home("s"), moved);
    assert_eq!(document(&moved).pending_type, None);
    assert!(!b.owes("s"));
    assert_eq!(
        shown_type(&b.record("s")),
        (Some("One on One".into()), false)
    );
}

#[test]
fn a_note_that_this_computer_works_on_waits_for_the_work() {
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("s", &root, "Standup");
    let claim = b.claims.regenerate("s").unwrap();
    assert_eq!(b.change("s", Some("One on One")), TypeChange::Later);
    assert!(home.exists());
    drop(claim);
    assert_eq!(b.run(now()), ["s"]);
    assert!(root.join("One on One").join(name_of(&home)).exists());
}

#[test]
fn a_waiting_move_survives_a_quit_and_moves_after_the_next_launch() {
    let (dir, root) = storage();
    let db = dir.path().join("longhand.db");
    let home = {
        let b = computer_with(Store::open(&db).unwrap(), "run-b");
        let home = b.save_note("s", &root, "Standup");
        in_use_elsewhere(&home);
        assert_eq!(b.change("s", Some("One on One")), TypeChange::Later);
        home
    };
    in_use_ended(&home);

    let b = computer_with(Store::open(&db).unwrap(), "run-b2");
    assert_eq!(b.run(now()), ["s"]);
    assert!(root.join("One on One").join(name_of(&home)).exists());
    assert!(!b.owes("s"));
}

#[test]
fn a_crash_after_the_intent_and_before_the_move_moves_at_the_next_launch() {
    // The first crash-window test of the owed-rename entry.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("s", &root, "Standup");
    in_use_elsewhere(&home);
    assert_eq!(b.change("s", Some("One on One")), TypeChange::Later);
    in_use_ended(&home);
    let target = root.join("One on One").join(name_of(&home));
    fs::create_dir(root.join("One on One")).unwrap();
    b.store
        .lock()
        .unwrap()
        .set_rename_intent(
            "s",
            &RenameIntent {
                from: home.to_string_lossy().into(),
                to: target.to_string_lossy().into(),
                temp: None,
            },
        )
        .unwrap();

    pipeline::folder_rename::resolve_intents(&b.store, &RealFs).unwrap();
    assert_eq!(b.home("s"), home);
    b.run(now());
    assert_eq!(b.home("s"), target);
    assert_eq!(entries(&root.join("One on One")), [name_of(&home)]);
    assert!(!b.owes("s"));
}

#[test]
fn a_crash_after_the_move_and_before_the_row_is_cleared_creates_no_second_folder() {
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("s", &root, "Standup");
    in_use_elsewhere(&home);
    assert_eq!(b.change("s", Some("One on One")), TypeChange::Later);
    in_use_ended(&home);
    // The move happened and its record ended, but the row of the move was
    // not cleared.
    let target = root.join("One on One").join(name_of(&home));
    fs::create_dir(root.join("One on One")).unwrap();
    b.store
        .lock()
        .unwrap()
        .set_rename_intent(
            "s",
            &RenameIntent {
                from: home.to_string_lossy().into(),
                to: target.to_string_lossy().into(),
                temp: None,
            },
        )
        .unwrap();
    fs::rename(&home, &target).unwrap();

    pipeline::folder_rename::resolve_intents(&b.store, &RealFs).unwrap();
    b.run(now());
    assert_eq!(b.home("s"), target);
    assert_eq!(entries(&root.join("One on One")), [name_of(&home)]);
    assert_eq!(entries(&root), ["One on One"]);
    assert_eq!(document(&target).pending_type, None);
    assert!(!b.owes("s"));
}

#[test]
fn a_crash_after_the_work_and_before_the_waiting_move_moves_at_the_next_launch() {
    // The second crash-window test of the owed-rename entry: the work on
    // the note ended, and the process stopped before the move.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("s", &root, "Standup");
    let claim = b.claims.regenerate("s").unwrap();
    assert_eq!(b.change("s", None), TypeChange::Unchanged);
    assert_eq!(b.change("s", Some("Pathfinder")), TypeChange::Later);
    drop(claim);

    assert_eq!(b.run(now()), ["s"]);
    assert!(root.join("Pathfinder").join(name_of(&home)).exists());
}

#[test]
fn a_target_folder_with_the_same_name_gives_the_free_name() {
    // Covers R27.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("s", &root, "Standup");
    let taken = root.join("One on One").join(name_of(&home));
    fs::create_dir_all(&taken).unwrap();

    let suffixed = root
        .join("One on One")
        .join(format!("{}-2", name_of(&home)));
    assert_eq!(
        b.change("s", Some("One on One")),
        TypeChange::Moved(suffixed.clone())
    );
    assert!(taken.exists());
    assert!(suffixed.join("session.md").exists());
}

#[test]
fn a_target_type_folder_with_other_case_is_used() {
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("s", &root, "Standup");
    fs::create_dir(root.join("pathfinder")).unwrap();
    let change = b.change("s", Some("Pathfinder"));
    assert_eq!(
        change,
        TypeChange::Moved(root.join("pathfinder").join(name_of(&home)))
    );
    assert_eq!(entries(&root), ["pathfinder"]);
}

#[test]
fn an_unavailable_storage_folder_pauses_the_move_and_keeps_the_row() {
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("s", &root, "Standup");
    fs::remove_file(library::scan::marker(&root)).unwrap();

    assert_eq!(b.change("s", Some("One on One")), TypeChange::Later);
    assert!(b.owes("s"));
    assert_eq!(shown_type(&b.record("s")).0.as_deref(), Some("One on One"));
    assert!(b.run(now()).is_empty());
    assert!(home.exists());
    assert!(b.owes("s"));

    assert!(library::scan::write_marker(&root));
    assert_eq!(b.run(now()), ["s"]);
    assert!(root.join("One on One").join(name_of(&home)).exists());
}

#[test]
fn a_folder_that_a_person_moves_while_the_move_waits_wins() {
    // KTD3, R29.
    let (_dir, root) = storage();
    let mut b = computer();
    let home = b.save_note("s", &root, "Standup");
    in_use_elsewhere(&home);
    assert_eq!(b.change("s", Some("One on One")), TypeChange::Later);
    fs::create_dir(root.join("Board")).unwrap();
    let dragged = root.join("Board").join(name_of(&home));
    fs::rename(&home, &dragged).unwrap();
    b.sync(&root);
    assert_eq!(b.home("s"), dragged);

    b.run(now());
    assert!(!b.owes("s"));
    assert_eq!(document(&dragged).pending_type, None);
    assert_eq!(shown_type(&b.record("s")), (Some("Board".into()), false));
    assert!(dragged.exists());
}

#[test]
fn a_change_back_to_the_type_of_the_folder_clears_the_waiting_change() {
    // KTD3.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_typed("s", &root, "Pathfinder", "Session 12");
    in_use_elsewhere(&home);
    assert_eq!(b.change("s", Some("One on One")), TypeChange::Later);
    assert_eq!(b.change("s", Some("D&D")), TypeChange::Later);
    // A second change keeps `from`.
    let field = document(&home).pending_type.unwrap();
    assert_eq!(
        (field.to.as_deref(), field.from.as_deref()),
        (Some("D&D"), Some("Pathfinder"))
    );
    assert_eq!(b.change("s", Some("pathfinder")), TypeChange::Unchanged);
    assert_eq!(document(&home).pending_type, None);
    assert_eq!(b.record("s").pending_type, None);
    assert!(!b.owes("s"));
}

#[test]
fn a_change_to_a_bad_name_is_refused() {
    let (_dir, root) = storage();
    let b = computer();
    b.save_note("s", &root, "Standup");
    let mut store = b.store.lock().unwrap();
    assert!(change_type_locked(&mut store, "s", Some("a/b"), false, &b.ctx(now())).is_err());
    assert!(store.type_move("s").unwrap().is_none());
}

#[test]
fn deleting_a_type_moves_every_note_to_the_storage_folder_and_removes_the_folder() {
    // Covers AE2 (the moves) and R6.
    let (_dir, root) = storage();
    let b = computer();
    for i in 0..30 {
        b.save_typed(
            &format!("s{i:02}"),
            &root,
            "Pathfinder",
            &format!("Session {i}"),
        );
    }
    fs::write(root.join("Pathfinder/.DS_Store"), b"x").unwrap();
    let edit = BulkEdit::DeleteType {
        name: "Pathfinder".into(),
    };
    let pass = type_move::apply_edit(&b.store, &root, 1, &edit, &b.ctx(now())).unwrap();
    assert!(pass.done);
    assert_eq!(pass.changed.len(), 30);
    assert!(!root.join("Pathfinder").exists());
    assert_eq!(entries(&root).len(), 30);
    for i in 0..30 {
        let record = b.record(&format!("s{i:02}"));
        assert_eq!(record.note_type, None);
        assert_eq!(
            Path::new(record.home_path.as_deref().unwrap()).parent(),
            Some(root.as_path())
        );
    }
}

#[test]
fn a_folder_with_another_file_is_not_removed() {
    let (_dir, root) = storage();
    let b = computer();
    b.save_typed("s", &root, "Pathfinder", "Session 12");
    fs::write(root.join("Pathfinder/map.png"), b"x").unwrap();
    let edit = BulkEdit::DeleteType {
        name: "Pathfinder".into(),
    };
    type_move::apply_edit(&b.store, &root, 1, &edit, &b.ctx(now())).unwrap();
    assert_eq!(entries(&root.join("Pathfinder")), ["map.png"]);
}

#[test]
fn merging_a_type_moves_its_notes_into_the_existing_folder() {
    // R6: "1 on 1" merges into "One on One".
    let (_dir, root) = storage();
    let b = computer();
    let first = b.save_typed("a", &root, "1 on 1", "Nathan");
    b.save_typed("b", &root, "One on One", "Sarah");
    let edit = BulkEdit::RenameType {
        from: "1 on 1".into(),
        to: "One on One".into(),
    };
    let pass = type_move::apply_edit(&b.store, &root, 1, &edit, &b.ctx(now())).unwrap();
    assert!(pass.done);
    assert_eq!(pass.changed, ["a"]);
    assert_eq!(entries(&root), ["One on One"]);
    assert_eq!(b.home("a"), root.join("One on One").join(name_of(&first)));
    assert_eq!(b.record("a").note_type.as_deref(), Some("One on One"));
}

#[test]
fn a_rename_that_changes_only_the_case_renames_the_type_folder() {
    // KTD7.
    let (_dir, root) = storage();
    let mut b = computer();
    let home = b.save_typed("s", &root, "pathfinder", "Session 12");
    let edit = BulkEdit::RenameType {
        from: "pathfinder".into(),
        to: "Pathfinder".into(),
    };
    let pass = type_move::apply_edit(&b.store, &root, 7, &edit, &b.ctx(now())).unwrap();
    assert!(pass.done);
    assert_eq!(pass.changed, ["s"]);
    let fs_entries: Vec<String> = fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != ".longhand")
        .collect();
    assert_eq!(fs_entries, ["Pathfinder"]);
    let renamed = root.join("Pathfinder").join(name_of(&home));
    assert_eq!(b.home("s"), renamed);
    b.sync(&root);
    assert_eq!(b.record("s").note_type.as_deref(), Some("Pathfinder"));
    assert_eq!(b.home("s"), renamed);
}

/// The visible names in `root`, with the case of the disk.
fn type_folders(root: &Path) -> Vec<String> {
    entries(root)
}

fn case_rename() -> BulkEdit {
    BulkEdit::RenameType {
        from: "pathfinder".into(),
        to: "Pathfinder".into(),
    }
}

#[test]
fn a_case_rename_waits_while_a_note_in_the_folder_is_busy() {
    // KTD6, KTD7.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_typed("s", &root, "pathfinder", "Session 12");
    in_use_elsewhere(&home);
    let pass = type_move::apply_edit(&b.store, &root, 7, &case_rename(), &b.ctx(now())).unwrap();
    assert!(!pass.done);
    assert!(pass.changed.is_empty());
    assert_eq!(type_folders(&root), ["pathfinder"]);
    assert_eq!(b.home("s"), home);
    assert_eq!(b.record("s").note_type.as_deref(), Some("pathfinder"));

    in_use_ended(&home);
    let pass = type_move::apply_edit(&b.store, &root, 7, &case_rename(), &b.ctx(now())).unwrap();
    assert!(pass.done);
    assert_eq!(type_folders(&root), ["Pathfinder"]);
}

#[test]
fn a_case_rename_that_a_crash_stopped_at_the_temporary_name_is_finished() {
    // KTD7: a crash after the first rename leaves the type folder at the
    // temporary name, and the rows at the old name.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_typed("s", &root, "pathfinder", "Session 12");
    let temp = root.join(session_save::rename_home::temp_name("type-7"));
    fs::rename(root.join("pathfinder"), &temp).unwrap();

    let pass = type_move::apply_edit(&b.store, &root, 7, &case_rename(), &b.ctx(now())).unwrap();
    assert!(pass.done);
    assert_eq!(pass.changed, ["s"]);
    assert!(!temp.exists());
    assert_eq!(type_folders(&root), ["Pathfinder"]);
    assert_eq!(b.home("s"), root.join("Pathfinder").join(name_of(&home)));
    assert_eq!(b.record("s").note_type.as_deref(), Some("Pathfinder"));
}

/// A volume whose rename from the temporary name to the new name fails.
struct SecondRenameFails(std::sync::atomic::AtomicUsize);

impl session_save::Fs for SecondRenameFails {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        RealFs.rename(from, to)
    }
    fn rename_new(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            return Err(std::io::Error::other("the second rename failed"));
        }
        RealFs.rename_new(from, to)
    }
}

#[test]
fn a_case_rename_whose_second_rename_fails_gives_back_the_old_folder_name() {
    // KTD7: the folder never stays at the temporary name.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_typed("s", &root, "pathfinder", "Session 12");
    let fails = SecondRenameFails(std::sync::atomic::AtomicUsize::new(0));
    let ctx = Context {
        fs: &fails,
        ..b.ctx(now())
    };
    assert!(type_move::apply_edit(&b.store, &root, 7, &case_rename(), &ctx).is_err());
    assert_eq!(fails.0.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(type_folders(&root), ["pathfinder"]);
    assert!(
        !root
            .join(session_save::rename_home::temp_name("type-7"))
            .exists()
    );
    assert!(home.join("session.md").exists());
    assert_eq!(b.home("s"), home);
    assert_eq!(b.record("s").note_type.as_deref(), Some("pathfinder"));
}

#[test]
fn a_busy_note_of_a_renamed_type_stays_and_shows_the_new_type() {
    // R6, KTD7, KTD9.
    let (_dir, root) = storage();
    let mut b = computer();
    let busy = b.save_typed("busy", &root, "Pathfinder", "Session 12");
    b.save_typed("idle", &root, "Pathfinder", "Session 13");
    in_use_elsewhere(&busy);
    let edit = BulkEdit::RenameType {
        from: "Pathfinder".into(),
        to: "D&D".into(),
    };
    let pass = type_move::apply_edit(&b.store, &root, 1, &edit, &b.ctx(now())).unwrap();
    assert!(pass.done);
    assert_eq!(entries(&root.join("Pathfinder")), [name_of(&busy)]);
    assert_eq!(entries(&root.join("D&D")).len(), 1);
    b.sync(&root);
    let record = b.record("busy");
    assert_eq!(shown_type(&record), (Some("D&D".into()), true));
    // Every note left in the old folder leaves it: the folder is not a
    // type that comes back (KTD9).
    assert_eq!(
        record.pending_type.unwrap().from.as_deref(),
        Some("Pathfinder")
    );
    // A second pass changes nothing.
    let again = type_move::apply_edit(&b.store, &root, 1, &edit, &b.ctx(now())).unwrap();
    assert!(again.changed.is_empty());

    in_use_ended(&busy);
    assert_eq!(b.run(now()), ["busy"]);
    assert!(!root.join("Pathfinder").exists());
    assert_eq!(entries(&root.join("D&D")).len(), 2);
}

/// One pass of the recorded type edit `id` of the store of `b`, as a
/// launch or a scan runs it: the notes come from the record, and the
/// record keeps the notes that are not done.
fn continue_recorded(b: &Computer, root: &Path, id: i64) -> type_move::EditPass {
    let open = b.store.lock().unwrap().bulk_edits().unwrap();
    let open = open.into_iter().find(|o| o.id == id).unwrap();
    let pass =
        type_move::apply_recorded(&b.store, root, id, &open.edit, &open.notes, &b.ctx(now()))
            .unwrap();
    let store = b.store.lock().unwrap();
    if pass.done {
        store.remove_bulk_edit(id).unwrap();
    } else {
        store.set_bulk_edit_notes(id, &pass.left).unwrap();
    }
    pass
}

/// A type edit of "Pathfinder" waits for a note whose session.md cannot
/// be read. A note that the user puts in "Pathfinder" after the edit keeps
/// the type, also after a relaunch.
fn a_type_edit_that_waits_keeps_a_later_type_change(edit: BulkEdit) {
    let (dir, root) = storage();
    let db = dir.path().join("longhand.db");
    let mut b = computer_with(Store::open(&db).unwrap(), "run-b");
    let stuck = b.save_typed("stuck", &root, "Pathfinder", "Session 12");
    b.save_typed("later", &root, "Standup", "Standup");
    let text = fs::read(stuck.join("session.md")).unwrap();
    fs::write(stuck.join("session.md"), b"not a session file").unwrap();

    let id = {
        let store = b.store.lock().unwrap();
        let notes = type_move::matching_notes(&store, &edit).unwrap();
        assert_eq!(notes, ["stuck"]);
        store.add_bulk_edit(&edit, &notes).unwrap()
    };
    let pass = continue_recorded(&b, &root, id);
    assert!(!pass.done);
    assert_eq!(pass.left, ["stuck"]);

    // A relaunch.
    b.store = Arc::new(Mutex::new(Store::open(&db).unwrap()));
    let later = b.change("later", Some("Pathfinder"));
    assert!(matches!(later, TypeChange::Moved(_)), "{later:?}");
    assert!(!continue_recorded(&b, &root, id).done);
    assert_eq!(b.record("later").note_type.as_deref(), Some("Pathfinder"));
    assert_eq!(
        b.home("later").parent(),
        Some(root.join("Pathfinder").as_path())
    );

    fs::write(stuck.join("session.md"), &text).unwrap();
    let pass = continue_recorded(&b, &root, id);
    assert!(pass.done);
    assert_eq!(pass.changed, ["stuck"]);
    assert_ne!(b.record("stuck").note_type.as_deref(), Some("Pathfinder"));
    assert_eq!(b.record("later").note_type.as_deref(), Some("Pathfinder"));
    assert_eq!(
        b.home("later").parent(),
        Some(root.join("Pathfinder").as_path())
    );
    assert!(b.store.lock().unwrap().bulk_edits().unwrap().is_empty());
}

#[test]
fn a_type_delete_that_waits_keeps_a_later_type_change() {
    a_type_edit_that_waits_keeps_a_later_type_change(BulkEdit::DeleteType {
        name: "Pathfinder".into(),
    });
}

#[test]
fn a_type_rename_that_waits_keeps_a_later_type_change() {
    a_type_edit_that_waits_keeps_a_later_type_change(BulkEdit::RenameType {
        from: "Pathfinder".into(),
        to: "D&D".into(),
    });
}

/// The pending type of another computer, `age` old, on the note in `home`.
fn pending_of_another_computer(home: &Path, to: &str, age: TimeDelta) -> PendingType {
    let pending = PendingType {
        to: Some(to.into()),
        from: None,
        by: "computer-a".into(),
        at: now() - age,
    };
    let mut doc = document(home);
    doc.pending_type = Some(pending.clone());
    write_document(home, &doc);
    pending
}

#[test]
fn a_move_that_waited_for_8_days_is_taken_over_and_moves_10_minutes_later() {
    // Covers R35 and KTD13.
    let (_dir, root) = storage();
    let mut b = computer();
    let home = b.save_note("s", &root, "Standup");
    pending_of_another_computer(&home, "One on One", TimeDelta::days(8));
    b.sync(&root);
    assert!(!b.owes("s"));

    let at = now();
    assert_eq!(b.run(at), ["s"]);
    assert!(b.owes("s"));
    assert_eq!(document(&home).pending_type.unwrap().by, b.id());
    assert!(home.exists());

    // Not yet: the takeover waits.
    b.run(at + TimeDelta::minutes(5));
    assert!(home.exists());

    b.run(at + TAKEOVER_WAIT + TimeDelta::minutes(1));
    let moved = root.join("One on One").join(name_of(&home));
    assert!(moved.exists());
    assert_eq!(document(&moved).pending_type, None);
    assert!(!b.owes("s"));
}

#[test]
fn two_computers_that_take_over_at_once_leave_one_move() {
    // KTD13: only the computer that the field names after 10 minutes
    // moves the folder, and the other drops its row.
    let (_dir, root) = storage();
    let mut b = computer();
    let mut c = computer_with(Store::open_in_memory().unwrap(), "run-c");
    let home = b.save_note("s", &root, "Standup");
    c.sync(&root);
    let first = pending_of_another_computer(&home, "One on One", TimeDelta::days(8));
    b.sync(&root);
    c.sync(&root);

    let at = now();
    assert_eq!(b.run(at), ["s"]);
    // C reads the file before B's write reaches it.
    let mut doc = document(&home);
    doc.pending_type = Some(first);
    write_document(&home, &doc);
    assert_eq!(c.run(at), ["s"]);
    assert_eq!(document(&home).pending_type.unwrap().by, c.id());

    let later = at + TAKEOVER_WAIT + TimeDelta::minutes(1);
    assert!(b.run(later).is_empty());
    assert!(!b.owes("s"));
    assert!(home.exists());
    assert_eq!(c.run(later), ["s"]);
    assert!(root.join("One on One").join(name_of(&home)).exists());
    assert!(!c.owes("s"));
}

#[test]
fn a_waiting_move_with_result_pending_is_not_taken_over() {
    let (_dir, root) = storage();
    let mut b = computer();
    let home = b.save_note("s", &root, "Standup");
    pending_of_another_computer(&home, "One on One", TimeDelta::days(8));
    let mut doc = document(&home);
    doc.result = Some(ResultFlag::NotSaved);
    write_document(&home, &doc);
    b.sync(&root);
    assert!(b.run(now()).is_empty());
    assert!(!b.owes("s"));
    assert_eq!(document(&home).pending_type.unwrap().by, "computer-a");
}

#[test]
fn a_receiving_computer_never_moves_a_folder_for_a_young_pending_type() {
    let (_dir, root) = storage();
    let mut b = computer();
    let home = b.save_note("s", &root, "Standup");
    pending_of_another_computer(&home, "One on One", TAKEOVER_AFTER - TimeDelta::hours(1));
    b.sync(&root);
    assert!(b.run(now()).is_empty());
    assert!(!b.owes("s"));
    assert!(home.exists());
    assert_eq!(document(&home).pending_type.unwrap().by, "computer-a");
    assert_eq!(
        shown_type(&b.record("s")),
        (Some("One on One".into()), true)
    );
}

#[test]
fn a_waiting_move_is_dropped_when_another_computer_takes_it() {
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("s", &root, "Standup");
    in_use_elsewhere(&home);
    assert_eq!(b.change("s", Some("One on One")), TypeChange::Later);
    let mut doc = document(&home);
    doc.pending_type.as_mut().unwrap().by = "computer-c".into();
    write_document(&home, &doc);
    in_use_ended(&home);
    assert!(b.run(now()).is_empty());
    assert!(!b.owes("s"));
    assert!(home.exists());
}

#[test]
fn a_waiting_move_is_dropped_when_another_computer_clears_the_change() {
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("s", &root, "Standup");
    in_use_elsewhere(&home);
    assert_eq!(b.change("s", Some("One on One")), TypeChange::Later);
    let mut doc = document(&home);
    doc.pending_type = None;
    write_document(&home, &doc);
    in_use_ended(&home);
    assert!(b.run(now()).is_empty());
    assert!(!b.owes("s"));
    assert!(home.exists());
}

#[test]
fn a_waiting_move_of_a_deleted_note_is_dropped() {
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("s", &root, "Standup");
    in_use_elsewhere(&home);
    assert_eq!(b.change("s", Some("One on One")), TypeChange::Later);
    fs::remove_dir_all(&home).unwrap();
    assert!(b.run(now()).is_empty());
    assert!(!b.owes("s"));
    assert!(!root.join("One on One").exists());
}

#[test]
fn a_change_before_the_home_folder_is_written_changes_only_the_row() {
    let b = computer();
    b.store
        .lock()
        .unwrap()
        .create_session("s", started(), None)
        .unwrap();
    assert_eq!(b.change("s", Some("Pathfinder")), TypeChange::NoFolder);
    assert_eq!(b.record("s").note_type.as_deref(), Some("Pathfinder"));
    assert!(!b.owes("s"));
}

#[test]
fn on_two_computers_the_type_change_ends_with_one_folder_in_the_new_type_folder() {
    // Covers AE1 on two computers: B changes the type while A works on the
    // note, A shows the new type at once, and B moves the folder after A.
    let (_dir, root) = storage();
    let mut a = computer_with(Store::open_in_memory().unwrap(), "run-a");
    let mut b = computer();
    let home = a.save_note("s", &root, "Standup");
    b.sync(&root);
    in_use_elsewhere(&home);

    assert_eq!(b.change("s", Some("One on One")), TypeChange::Later);
    a.sync(&root);
    assert_eq!(
        shown_type(&a.record("s")),
        (Some("One on One".into()), true)
    );
    // A never moves the folder: it does not owe the move.
    assert!(a.run(now()).is_empty());
    assert!(home.exists());

    in_use_ended(&home);
    assert_eq!(b.run(now()), ["s"]);
    a.sync(&root);
    let moved = root.join("One on One").join(name_of(&home));
    for computer in [&a, &b] {
        let record = computer.record("s");
        assert_eq!(PathBuf::from(record.home_path.clone().unwrap()), moved);
        assert_eq!(shown_type(&record), (Some("One on One".into()), false));
        assert!(!computer.owes("s"));
        assert_eq!(computer.store.lock().unwrap().sessions().unwrap().len(), 1);
    }
}

#[test]
fn an_empty_type_folder_is_removed_only_with_system_files_in_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("Empty")).unwrap();
    fs::write(root.join("Empty/Thumbs.db"), b"x").unwrap();
    assert!(type_move::remove_empty_type_folder(root, "Empty"));
    fs::create_dir_all(root.join("Full/note")).unwrap();
    assert!(!type_move::remove_empty_type_folder(root, "Full"));
    assert!(!type_move::remove_empty_type_folder(root, "../x"));
    assert!(!type_move::remove_empty_type_folder(root, ".longhand"));
    assert!(root.join("Full/note").exists());
}

#[cfg(unix)]
#[test]
fn a_move_whose_session_file_does_not_download_waits_and_does_not_block() {
    // A named pipe blocks its read until a writer opens it, as an
    // online-only file blocks its read until it downloads.
    let (_dir, root) = storage();
    let b = computer();
    let home = b.save_note("s", &root, "Standup");
    in_use_elsewhere(&home);
    assert_eq!(b.change("s", Some("One on One")), TypeChange::Later);
    in_use_ended(&home);
    let file = home.join("session.md");
    let text = fs::read(&file).unwrap();
    fs::remove_file(&file).unwrap();
    let status = std::process::Command::new("mkfifo")
        .arg(&file)
        .status()
        .unwrap();
    assert!(status.success());

    let (store, own_runs, claims) = (b.store.clone(), b.own_runs.clone(), b.claims.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let ctx = Context {
            own_runs: &own_runs,
            claims: &claims,
            now: now(),
            fs: &RealFs,
        };
        let _ = tx.send(run_owed(&store, &ctx).unwrap());
    });
    let moved = rx.recv_timeout(std::time::Duration::from_secs(30));
    // The file downloads: this also ends the read that waits.
    fs::write(&file, &text).unwrap();
    assert_eq!(
        moved.expect("the move waits on the read"),
        Vec::<String>::new()
    );
    assert!(b.owes("s"));
    assert!(home.exists());

    // The next run moves the folder.
    fs::remove_file(&file).unwrap();
    fs::write(&file, &text).unwrap();
    assert_eq!(b.run(now()), ["s"]);
}
