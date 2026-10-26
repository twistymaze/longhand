//! The import worker: one audio file becomes a queued note, crash-safe,
//! with the recording wait and the disk check.
//!
//! Each "launch" in these tests is a new `Imports` over the same journal
//! file and store, as a new process has.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{TimeZone, Utc};
use import_record::{ComputerId, Identity, Lookup, State};
use pipeline::assemble;
use pipeline::import::{
    Entry, ImportEnvironment, ImportEvent, Imports, Next, Origin, Recognition, Request, Step,
};
use pipeline::{RECOVERY_DIR, SharedStore, Unattended, lock};
use session_core::jobs::Claims;
use session_core::{CaptureChoice, RecordingModel, SessionState, TitleSource};

/// The 5 s mono m4a fixture at 22.05 kHz. Its `mvhd` records 2026-10-20
/// 14:30 UTC.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/audio-import")
        .join(name)
}

const OTHER_COMPUTER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct Rig {
    store: SharedStore,
    working: PathBuf,
    storage: PathBuf,
    journal: PathBuf,
    /// The folder that holds the files to import.
    phone: PathBuf,
    suspended: Arc<AtomicBool>,
    free: Arc<AtomicU64>,
    /// True: the app has no storage folder.
    no_storage: Arc<AtomicBool>,
    /// True: the removal of a folder fails, as a Trash that refuses it.
    refuse_remove: Arc<AtomicBool>,
    events: Mutex<Receiver<ImportEvent>>,
    sender: Arc<Mutex<Sender<ImportEvent>>>,
    _dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (sender, events) = channel();
        let rig = Self {
            store: common::store(),
            working: dir.path().join("sessions"),
            storage: dir.path().join("storage"),
            journal: dir.path().join(pipeline::import::JOURNAL_FILE),
            phone: dir.path().join("phone"),
            suspended: Arc::default(),
            free: Arc::new(AtomicU64::new(u64::MAX)),
            no_storage: Arc::default(),
            refuse_remove: Arc::default(),
            events: Mutex::new(events),
            sender: Arc::new(Mutex::new(sender)),
            _dir: dir,
        };
        for folder in [&rig.working, &rig.storage, &rig.phone] {
            std::fs::create_dir_all(folder).unwrap();
        }
        rig
    }

    /// A new worker over the journal and the store: a launch.
    fn imports(&self) -> Arc<Imports> {
        let storage = self.storage.clone();
        let suspended = Arc::clone(&self.suspended);
        let free = Arc::clone(&self.free);
        let sender = Arc::clone(&self.sender);
        let no_storage = Arc::clone(&self.no_storage);
        let refuse_remove = Arc::clone(&self.refuse_remove);
        Imports::open(ImportEnvironment {
            store: Arc::clone(&self.store),
            working_root: self.working.clone(),
            journal: self.journal.clone(),
            computer_id: self.computer(),
            longhand_version: "1.4.0".into(),
            storage_folder: Box::new(move || {
                (!no_storage.load(Ordering::SeqCst)).then(|| storage.clone())
            }),
            suspended: Box::new(move || suspended.load(Ordering::SeqCst)),
            free_space: Arc::new(move |_: &Path| Some(free.load(Ordering::SeqCst))),
            remove: Box::new(move |path| {
                if refuse_remove.load(Ordering::SeqCst) {
                    return Err("the Trash refused the folder".into());
                }
                std::fs::remove_dir_all(path).map_err(|e| e.to_string())
            }),
            claims: Claims::default(),
            early_homes: None,
            events: Box::new(move |event| {
                let _ = sender.lock().unwrap().send(event);
            }),
            poll: Duration::from_millis(1),
        })
        .unwrap()
    }

    fn computer(&self) -> ComputerId {
        ComputerId::parse(&lock(&self.store).computer_id().unwrap()).unwrap()
    }

    /// Put the fixture `fixture` in the phone folder as `name`.
    fn file(&self, fixture_name: &str, name: &str) -> PathBuf {
        let path = self.phone.join(name);
        std::fs::copy(fixture(fixture_name), &path).unwrap();
        path
    }

    /// Wait for the first event that `matches`, and return it. The other
    /// events before it are dropped.
    fn wait_for(&self, what: &str, matches: impl Fn(&ImportEvent) -> bool) -> ImportEvent {
        let events = self.events.lock().unwrap();
        loop {
            let event = events
                .recv_timeout(Duration::from_secs(60))
                .unwrap_or_else(|_| panic!("no event: {what}"));
            if matches(&event) {
                return event;
            }
        }
    }

    fn events_now(&self) -> Vec<ImportEvent> {
        self.events.lock().unwrap().try_iter().collect()
    }

    fn state(&self, id: &str) -> Option<SessionState> {
        lock(&self.store).session(id).unwrap().map(|r| r.state)
    }
}

fn request(source: &Path, origin: Origin) -> Request {
    Request {
        source: source.to_path_buf(),
        origin,
        note_type: None,
        tags: Vec::new(),
        title: None,
        notes: String::new(),
    }
}

fn recognition() -> Recognition {
    Recognition {
        asr_model: "m".into(),
        snapshot: None,
        speaker_count: None,
    }
}

fn identity_of(path: &Path) -> Identity {
    Identity::parse(&audio_import::hash_file(path).unwrap()).unwrap()
}

fn entry(imports: &Imports, id: &str) -> Entry {
    imports
        .entries()
        .into_iter()
        .find(|e| e.note_id == id)
        .expect("a journal entry")
}

/// Take the next step of the entry of `id` and record it.
fn advance(imports: &Imports, id: &str) {
    match imports.step(&entry(imports, id)).unwrap() {
        Next::Step(step) => imports.record(id, step).unwrap(),
        other => panic!("the step of {id} gave {other:?}"),
    }
}

fn step_name(entry: &Entry) -> &'static str {
    match entry.step {
        Step::Journaled => "journaled",
        Step::Created => "created",
        Step::Copied { .. } => "copied",
        Step::Converted { .. } => "converted",
        Step::Recorded { .. } => "recorded",
        Step::Failed { .. } => "failed",
        Step::Duplicate => "duplicate",
    }
}

/// After a launch: exactly one note, queued with one segment, one
/// `imported` entry of this computer, an empty journal and no copy.
fn assert_one_queued_note(r: &Rig, id: &str, source: &Path) {
    let records = lock(&r.store).sessions().unwrap();
    assert_eq!(records.len(), 1, "{records:?}");
    assert!(
        matches!(r.state(id), Some(SessionState::Queued { .. })),
        "{:?}",
        r.state(id)
    );
    assert_eq!(lock(&r.store).segments(id).unwrap().len(), 1);
    let identity = identity_of(source);
    let own = import_record::entry_of(&r.storage, &identity, &r.computer());
    assert_eq!(own.map(|e| e.state), Some(State::Imported));
    assert!(r.imports().entries().is_empty());
    assert!(!r.working.join(id).join("import").exists());
}

#[test]
fn an_m4a_import_gives_a_queued_note_with_the_file_name_title_and_recording_time() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let imports = r.imports();
    let id = imports
        .import(request(&source, Origin::Manual), recognition())
        .unwrap();
    // R21: the note shows at once, before the conversion.
    assert_eq!(r.state(&id), Some(SessionState::Importing));
    imports.run_pending();

    assert_one_queued_note(&r, &id, &source);
    let record = lock(&r.store).session(&id).unwrap().unwrap();
    let title = record.title.clone().unwrap();
    assert_eq!(
        (title.text.as_str(), title.source),
        ("Team sync", TitleSource::Proposed)
    );
    let recorded = Utc.with_ymd_and_hms(2026, 10, 20, 14, 30, 0).unwrap();
    let segments = lock(&r.store).segments(&id).unwrap();
    assert_eq!(record.start(&segments), recorded);
    assert_eq!(segments[0].wall_start, recorded);
    assert_eq!(segments[0].channels, 1);
    assert_eq!(record.live_transcript, Some(false));
    let source_record = lock(&r.store).import_source(&id).unwrap().unwrap();
    assert_eq!(source_record.source_name, "Team sync.m4a");
    assert_eq!(
        source_record.sha256.as_deref(),
        Some(identity_of(&source).as_str())
    );
    // The file in the phone folder does not change (R7).
    assert_eq!(
        std::fs::read(&source).unwrap(),
        std::fs::read(fixture("tone-mono.m4a")).unwrap()
    );
    // KTD4: the stages run unchanged. Assemble gives 5 s at 16 kHz.
    let assembled = assemble::run(
        &r.store,
        &id,
        &r.working.join(&id),
        &segments,
        None,
        &Unattended,
    )
    .unwrap();
    let seconds = assembled[0].proc_samples as f64 / f64::from(assemble::PROC_RATE);
    assert!((seconds - 5.0).abs() < 0.1, "{seconds} s");
}

#[test]
fn a_sheet_title_is_the_users_title() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Memo 12.m4a");
    let imports = r.imports();
    let mut request = request(&source, Origin::Manual);
    request.title = Some("Budget review".into());
    request.note_type = Some("One on One".into());
    request.tags = vec!["phone".into()];
    let id = imports.import(request, recognition()).unwrap();
    let record = lock(&r.store).session(&id).unwrap().unwrap();
    let title = record.title.unwrap();
    assert_eq!(
        (title.text.as_str(), title.source),
        ("Budget review", TitleSource::User)
    );
    assert_eq!(record.note_type.as_deref(), Some("One on One"));
    assert_eq!(record.tags, ["phone"]);
}

#[test]
fn sheet_notes_are_the_notes_of_the_new_note() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Memo 12.m4a");
    let imports = r.imports();
    let mut request = request(&source, Origin::Manual);
    request.notes = "Weekly sync with Dana".into();
    let id = imports.import(request, recognition()).unwrap();
    assert_eq!(lock(&r.store).notes(&id).unwrap(), "Weekly sync with Dana");
}

#[test]
fn a_journal_request_of_an_older_build_has_no_notes() {
    let mut json = serde_json::to_value(request(Path::new("/a/Memo.m4a"), Origin::Manual)).unwrap();
    json.as_object_mut().unwrap().remove("notes");
    let request: Request = serde_json::from_value(json).unwrap();
    assert_eq!(request.notes, "");
}

// ---------------------------------------------------- crash windows (KTD6)

/// Record the steps of a watched-folder import up to the step `before`,
/// take one more step, and do not record it: the process stops between
/// the effect of a step and its record. Then launch and run. A watched
/// file makes the recovery read the import record again (state diagram).
fn crash_after_the_effect_of(before: &str) {
    crash_after_the_effect_of_then(before, |_, _| {});
}

/// [`crash_after_the_effect_of`], with `between` run after the crash and
/// before the launch.
fn crash_after_the_effect_of_then(before: &str, between: impl FnOnce(&Rig, &Path)) {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let id = {
        let imports = r.imports();
        let entry = imports
            .begin(request(&source, Origin::WatchedFolder), recognition())
            .unwrap();
        let id = entry.note_id.clone();
        while step_name(&self::entry(&imports, &id)) != before {
            advance(&imports, &id);
        }
        // The effect of the next step, with no record.
        let next = imports.step(&self::entry(&imports, &id)).unwrap();
        assert!(
            matches!(next, Next::Step(_) | Next::Done),
            "the step after {before} gave {next:?}"
        );
        id
    };
    between(&r, &source);
    let imports = r.imports();
    imports.launch();
    imports.run_pending();
    assert_one_queued_note(&r, &id, &source);
}

#[test]
fn a_crash_after_the_journal_entry_makes_one_note() {
    // Step 1 is recorded; the row (step 2) is not made.
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let id = r
        .imports()
        .begin(request(&source, Origin::WatchedFolder), recognition())
        .unwrap()
        .note_id;
    let imports = r.imports();
    imports.launch();
    imports.run_pending();
    assert_one_queued_note(&r, &id, &source);
}

#[test]
fn a_crash_after_the_row_is_made_makes_one_note() {
    crash_after_the_effect_of("journaled");
}

#[test]
fn a_crash_after_the_copy_makes_one_note() {
    crash_after_the_effect_of("created");
}

#[test]
fn a_crash_after_the_conversion_makes_one_note() {
    crash_after_the_effect_of("copied");
}

#[test]
fn a_crash_after_the_imported_entry_makes_one_note_and_one_entry() {
    // Another computer's entry syncs before the launch. The entry of this
    // computer with the journal's time is this import's own entry, so the
    // note stays.
    crash_after_the_effect_of_then("converted", |r, source| {
        let other = ComputerId::parse(OTHER_COMPUTER).unwrap();
        let theirs =
            import_record::Entry::new(State::Imported, "Team sync.m4a", 1, other, common::at(0));
        import_record::write(&r.storage, &identity_of(source), &theirs).unwrap();
    });
}

#[test]
fn a_crash_after_the_note_is_queued_makes_one_note() {
    crash_after_the_effect_of("recorded");
}

// ---------------------------------------------------------------- launch

#[test]
fn a_launch_removes_an_importing_row_with_no_journal_entry_and_its_working_folder() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let id = r
        .imports()
        .import(request(&source, Origin::Manual), recognition())
        .unwrap();
    std::fs::create_dir_all(r.working.join(&id).join(RECOVERY_DIR)).unwrap();
    // The journal is lost.
    std::fs::remove_file(&r.journal).unwrap();
    let imports = r.imports();
    imports.launch();
    assert_eq!(r.state(&id), None);
    assert!(!r.working.join(&id).exists());
}

#[test]
fn launch_recovery_and_a_recording_start_leave_an_importing_note_unchanged() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let imports = r.imports();
    let id = imports
        .import(request(&source, Origin::Manual), recognition())
        .unwrap();
    // A partial conversion is in the working folder, as after a crash.
    advance(&imports, &id);
    common::write_segment(&r.working.join(&id), 0, 0, 16_000, &[0.1; 32_000], true);

    pipeline::recover::recover_crashed(&r.store, &r.working).unwrap();
    lock(&r.store)
        .start_session_with_recognition(
            "recording",
            common::at(0),
            None,
            &RecordingModel {
                asr_model: "m".into(),
                live_transcript: false,
            },
            &CaptureChoice::default(),
            None,
            &[],
        )
        .unwrap();

    assert_eq!(r.state(&id), Some(SessionState::Importing));
    assert!(lock(&r.store).segments(&id).unwrap().is_empty());
    assert_eq!(step_name(&entry(&imports, &id)), "copied");
}

// ------------------------------------------------------------ waits (KTD9)

#[test]
fn a_conversion_pauses_while_a_recording_runs_and_then_completes() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let imports = r.imports();
    let id = imports
        .import(request(&source, Origin::Manual), recognition())
        .unwrap();
    r.suspended.store(true, Ordering::SeqCst);
    let worker = {
        let imports = Arc::clone(&imports);
        std::thread::spawn(move || imports.run_pending())
    };
    r.wait_for("the pause", |e| matches!(e, ImportEvent::Paused { .. }));
    // The note waits while the recording runs.
    assert_eq!(r.state(&id), Some(SessionState::Importing));
    r.suspended.store(false, Ordering::SeqCst);
    r.wait_for(
        "the end of the import",
        |e| matches!(e, ImportEvent::Ended { note_id } if *note_id == id),
    );
    worker.join().unwrap();
    assert_one_queued_note(&r, &id, &source);
}

#[test]
fn an_import_waits_for_free_space_and_continues_when_it_returns() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    r.free.store(1_000_000, Ordering::SeqCst);
    let imports = r.imports();
    let id = imports
        .import(request(&source, Origin::Manual), recognition())
        .unwrap();
    imports.run_pending();
    // The copy did not start, and the disk notice is set (R22).
    assert!(!r.working.join(&id).join("import").exists());
    assert_eq!(step_name(&entry(&imports, &id)), "created");
    assert_eq!(imports.notices().disk, std::slice::from_ref(&id));
    assert!(
        r.events_now()
            .iter()
            .any(|e| matches!(e, ImportEvent::DiskWait { note_id } if *note_id == id))
    );

    r.free.store(u64::MAX, Ordering::SeqCst);
    imports.retry();
    imports.run_pending();
    assert_one_queued_note(&r, &id, &source);
    assert!(imports.notices().disk.is_empty());
    assert!(
        r.events_now()
            .iter()
            .any(|e| matches!(e, ImportEvent::DiskClear { note_id } if *note_id == id))
    );
}

// --------------------------------------------- the import record (R12, R13)

#[test]
fn another_computers_entry_during_the_conversion_removes_the_note() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let imports = r.imports();
    let id = imports
        .import(request(&source, Origin::WatchedFolder), recognition())
        .unwrap();
    advance(&imports, &id);
    // Another computer imported the same file, and its entry synced.
    let identity = identity_of(&source);
    let other = ComputerId::parse(OTHER_COMPUTER).unwrap();
    let theirs = import_record::Entry::new(
        State::Imported,
        "Team sync.m4a",
        std::fs::metadata(&source).unwrap().len(),
        other,
        common::at(0),
    );
    import_record::write(&r.storage, &identity, &theirs).unwrap();

    imports.run_pending();

    assert_eq!(r.state(&id), None);
    assert!(!r.working.join(&id).exists());
    assert!(imports.entries().is_empty());
    assert_eq!(
        import_record::entry_of(&r.storage, &identity, &r.computer()),
        None
    );
}

#[test]
fn a_manual_import_of_an_imported_file_makes_a_new_note() {
    // R3: the import record does not stop a manual import.
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let identity = identity_of(&source);
    let other = ComputerId::parse(OTHER_COMPUTER).unwrap();
    let theirs =
        import_record::Entry::new(State::Imported, "Team sync.m4a", 1, other, common::at(0));
    import_record::write(&r.storage, &identity, &theirs).unwrap();
    let imports = r.imports();
    let id = imports
        .import(request(&source, Origin::Manual), recognition())
        .unwrap();
    imports.run_pending();
    assert!(matches!(r.state(&id), Some(SessionState::Queued { .. })));
    assert!(import_record::entry_of(&r.storage, &identity, &r.computer()).is_some());
}

/// Delete the importing note `id` as the delete command does.
fn delete(r: &Rig, imports: &Imports, id: &str) {
    imports
        .delete_note(id, || {
            session_core::jobs::delete_session(
                &r.store,
                id,
                &r.working.join(id),
                &Claims::default(),
                |path: &Path| std::fs::remove_dir_all(path).map_err(|e| e.to_string()),
            )
        })
        .unwrap()
        .unwrap();
}

#[test]
fn a_delete_before_the_hash_is_known_still_records_the_file() {
    // Covers AE1.
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let imports = r.imports();
    let id = imports
        .import(request(&source, Origin::WatchedFolder), recognition())
        .unwrap();
    delete(&r, &imports, &id);
    imports.run_pending();

    assert_eq!(r.state(&id), None);
    assert!(imports.entries().is_empty());
    assert!(!r.working.join(&id).exists());
    // A later scan finds the file in the record, so it makes no note.
    assert!(matches!(
        import_record::lookup(&r.storage, &identity_of(&source)),
        Lookup::Imported(_)
    ));
}

#[test]
fn a_crash_after_a_delete_and_before_the_entry_writes_the_entry_at_launch() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let id = {
        let imports = r.imports();
        let id = imports
            .import(request(&source, Origin::WatchedFolder), recognition())
            .unwrap();
        delete(&r, &imports, &id);
        id
    };
    let imports = r.imports();
    imports.launch();
    imports.run_pending();

    assert!(lock(&r.store).sessions().unwrap().is_empty());
    assert!(!r.working.join(&id).exists());
    assert!(imports.entries().is_empty());
    assert!(matches!(
        import_record::lookup(&r.storage, &identity_of(&source)),
        Lookup::Imported(_)
    ));
}

#[test]
fn a_delete_during_the_conversion_stops_it_and_records_the_file() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let imports = r.imports();
    let id = imports
        .import(request(&source, Origin::WatchedFolder), recognition())
        .unwrap();
    r.suspended.store(true, Ordering::SeqCst);
    let worker = {
        let imports = Arc::clone(&imports);
        std::thread::spawn(move || imports.run_pending())
    };
    r.wait_for("the pause", |e| matches!(e, ImportEvent::Paused { .. }));
    delete(&r, &imports, &id);
    worker.join().unwrap();
    // The worker can stop before the row delete queues the entry again.
    imports.run_pending();

    assert_eq!(r.state(&id), None);
    assert!(!r.working.join(&id).exists());
    assert!(imports.entries().is_empty());
    assert!(matches!(
        import_record::lookup(&r.storage, &identity_of(&source)),
        Lookup::Imported(_)
    ));
}

#[test]
fn a_file_that_does_not_decode_gets_a_failed_entry_and_its_note_is_removed() {
    let r = Rig::new();
    let source = r.phone.join("Notes.m4a");
    std::fs::write(&source, "These are notes, not audio.\n").unwrap();
    let imports = r.imports();
    let id = imports
        .import(request(&source, Origin::WatchedFolder), recognition())
        .unwrap();
    imports.run_pending();

    assert_eq!(r.state(&id), None);
    assert!(!r.working.join(&id).exists());
    assert!(imports.entries().is_empty());
    let identity = identity_of(&source);
    let Lookup::Failed(entry) = import_record::lookup(&r.storage, &identity) else {
        panic!("no failed entry");
    };
    assert_eq!(entry.longhand_version.as_deref(), Some("1.4.0"));
    assert_eq!(
        entry.decoder_name.as_deref(),
        Some(audio_import::DECODER_NAME)
    );
    assert_eq!(
        entry.decoder_version.as_deref(),
        Some(audio_import::DECODER_VERSION)
    );
    // The notices on Home read the failure: the identity, the file name and the time.
    let failed = imports.notices().failed;
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].identity, identity.as_str());
    assert_eq!(failed[0].file_name, "Notes.m4a");
    assert_eq!(failed[0].at, entry.time);
}

// Covers R20: Dismiss removes the failure. A new manual import of the
// same file is a new attempt, so it gives a new failure.
#[test]
fn a_dismissed_failure_goes_and_a_new_attempt_gives_a_new_failure() {
    let r = Rig::new();
    let source = r.phone.join("Notes.m4a");
    std::fs::write(&source, "These are notes, not audio.\n").unwrap();
    let imports = r.imports();
    let first = imports
        .import(request(&source, Origin::Manual), recognition())
        .unwrap();
    imports.run_pending();
    assert_eq!(imports.notices().failed.len(), 1);

    assert!(imports.dismiss(&first));
    assert!(imports.notices().failed.is_empty());
    assert!(!imports.dismiss(&first), "a dismissed failure is gone");

    let second = imports
        .import(request(&source, Origin::Manual), recognition())
        .unwrap();
    imports.run_pending();
    let failed = imports.notices().failed;
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].note_id, second);
    assert_eq!(failed[0].file_name, "Notes.m4a");
}

// ------------------------------------------- a delete that does not complete

#[test]
fn a_delete_mark_that_cannot_be_saved_refuses_the_delete() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let imports = r.imports();
    let id = imports
        .import(request(&source, Origin::WatchedFolder), recognition())
        .unwrap();
    // A folder in the place of the journal file: no save succeeds.
    std::fs::remove_file(&r.journal).unwrap();
    std::fs::create_dir(&r.journal).unwrap();

    let mut called = false;
    let result = imports.delete_note(&id, || {
        called = true;
        Ok::<(), ()>(())
    });

    assert!(result.is_err(), "{result:?}");
    assert!(!called, "the row delete must not start");
    assert_eq!(r.state(&id), Some(SessionState::Importing));
    assert!(!entry(&imports, &id).deleted);
}

#[test]
fn a_failed_delete_of_an_importing_note_lets_the_import_continue() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let imports = r.imports();
    let id = imports
        .import(request(&source, Origin::WatchedFolder), recognition())
        .unwrap();
    let result = imports.delete_note(&id, || {
        // The worker runs while the row delete runs.
        imports.run_pending();
        assert!(
            matches!(
                import_record::lookup(&r.storage, &identity_of(&source)),
                Lookup::New
            ),
            "no entry while the row exists"
        );
        Err::<(), _>("the note is moving")
    });
    assert_eq!(result.unwrap(), Err("the note is moving"));
    assert!(!entry(&imports, &id).deleted);

    imports.run_pending();
    assert_one_queued_note(&r, &id, &source);
}

#[test]
fn a_crash_between_the_delete_mark_and_the_row_delete_continues_the_import() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let id = {
        let imports = r.imports();
        let id = imports
            .import(request(&source, Origin::WatchedFolder), recognition())
            .unwrap();
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            imports.delete_note(&id, || -> Result<(), ()> { panic!("the process stops") })
        }));
        assert!(crashed.is_err());
        id
    };
    let imports = r.imports();
    imports.launch();
    imports.run_pending();
    assert_one_queued_note(&r, &id, &source);
}

// -------------------------------------------------- a source that goes away

#[test]
fn a_source_that_goes_before_the_copy_removes_the_note_and_writes_no_entry() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let identity = identity_of(&source);
    let imports = r.imports();
    let id = imports
        .import(request(&source, Origin::WatchedFolder), recognition())
        .unwrap();
    std::fs::remove_file(&source).unwrap();
    imports.run_pending();

    assert_eq!(r.state(&id), None);
    assert!(imports.entries().is_empty());
    assert!(!r.working.join(&id).exists());
    assert!(matches!(
        import_record::lookup(&r.storage, &identity),
        Lookup::New
    ));
    assert!(
        r.events_now()
            .iter()
            .any(|e| matches!(e, ImportEvent::Removed { note_id } if *note_id == id))
    );
}

#[test]
fn a_delete_while_the_source_is_gone_waits_until_the_source_returns() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    let identity = identity_of(&source);
    let away = r.phone.join("away.m4a");
    let imports = r.imports();
    let id = imports
        .import(request(&source, Origin::WatchedFolder), recognition())
        .unwrap();
    // The removable volume goes before the hash is known.
    std::fs::rename(&source, &away).unwrap();
    delete(&r, &imports, &id);
    imports.run_pending();

    assert_eq!(r.state(&id), None);
    assert_eq!(imports.open_sources(), std::slice::from_ref(&source));
    assert!(matches!(
        import_record::lookup(&r.storage, &identity),
        Lookup::New
    ));

    std::fs::rename(&away, &source).unwrap();
    imports.retry();
    imports.run_pending();

    assert!(lock(&r.store).sessions().unwrap().is_empty());
    assert!(imports.entries().is_empty());
    assert!(matches!(
        import_record::lookup(&r.storage, &identity),
        Lookup::Imported(_)
    ));
}

// ------------------------------------------------------------ other waits

#[test]
fn an_import_waits_for_the_storage_folder_and_completes_after_a_retry() {
    let r = Rig::new();
    let source = r.file("tone-mono.m4a", "Team sync.m4a");
    r.no_storage.store(true, Ordering::SeqCst);
    let imports = r.imports();
    let id = imports
        .import(request(&source, Origin::Manual), recognition())
        .unwrap();
    imports.run_pending();
    assert_eq!(step_name(&entry(&imports, &id)), "converted");
    assert_eq!(r.state(&id), Some(SessionState::Importing));

    r.no_storage.store(false, Ordering::SeqCst);
    imports.retry();
    imports.run_pending();
    // One queued note and one `imported` entry of this computer.
    assert_one_queued_note(&r, &id, &source);
}

#[test]
fn a_failure_shows_its_notice_when_the_note_cannot_be_removed() {
    let r = Rig::new();
    let source = r.phone.join("Notes.m4a");
    std::fs::write(&source, "These are notes, not audio.\n").unwrap();
    r.refuse_remove.store(true, Ordering::SeqCst);
    let imports = r.imports();
    let id = imports
        .import(request(&source, Origin::Manual), recognition())
        .unwrap();
    imports.run_pending();
    assert_eq!(imports.notices().failed.len(), 1);
    assert_eq!(r.state(&id), Some(SessionState::Importing));

    r.refuse_remove.store(false, Ordering::SeqCst);
    imports.retry();
    imports.run_pending();
    assert_eq!(r.state(&id), None);
    assert!(imports.entries().is_empty());
    assert_eq!(imports.notices().failed.len(), 1);
}
