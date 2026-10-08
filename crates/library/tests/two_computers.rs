//! Two computers over one synced Destination (U4): each has its own store,
//! both save into one temporary folder, and each reconciles its store from
//! a scan of that folder. A note is saved through the real save stage.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, FixedOffset};
use library::reconcile::{Changes, Coverage, DELETE_WAIT, HAND_EDIT_WAIT, Reconciler, Target};
use library::refresh::{self, Refresh};
use library::scan::{Known, RemoteState, ScanOptions, Waiting, scan};
use library::status::NoteStatus;
use pipeline::SharedStore;
use session_core::folder::{AudioState, PendingType, ResultFlag, SessionDoc, fingerprint};
use session_core::{Event, Segment, SessionRecord, SessionState, Stage, Store, Title, TitleSource};
use session_save::in_use_marker::FRESH_FOR;

const TRANSCRIPT: &str = "# Transcript\n\n- [00:00:01] Speaker 1: We start with the budget.\n";

/// The options of the app, with its read time limit. A shorter limit fails
/// a read of a new file on a busy Windows runner, where the antivirus scans
/// the file when it is first opened.
fn options() -> ScanOptions {
    ScanOptions::default()
}

fn started() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-09-25T09:00:00-05:00").unwrap()
}

fn never_claimed(_: &str) -> bool {
    false
}

/// One computer: its store, its working location and its reconciler.
struct Computer {
    store: SharedStore,
    working: tempfile::TempDir,
    reconciler: Reconciler,
}

fn computer() -> Computer {
    Computer {
        store: Arc::new(Mutex::new(Store::open_in_memory().unwrap())),
        working: tempfile::tempdir().unwrap(),
        reconciler: Reconciler::new(&options()),
    }
}

impl Computer {
    /// Record, process and save the note `id` into `root` through the real
    /// save stage. Returns its folder.
    fn save_note(&self, id: &str, root: &Path, title: Option<Title>, notes: &str) -> PathBuf {
        {
            let mut s = self.store.lock().unwrap();
            s.create_session(id, started(), None).unwrap();
            s.set_notes(id, notes).unwrap();
            if let Some(title) = &title {
                assert!(s.set_title(id, &title.text, title.source).unwrap());
            }
        }
        self.save_recording(id, root)
    }

    /// Record one segment into the note `id`, which captures now, then
    /// process and save it into `root` through the real save stage.
    /// Returns its folder.
    fn save_recording(&self, id: &str, root: &Path) -> PathBuf {
        let segment = Segment {
            index: 0,
            offset_ms: 0,
            duration_ms: 60_000,
            wall_start: started(),
            wall_end: started() + chrono::TimeDelta::minutes(1),
            audio_file: "audio/segment-01.opus".into(),
            channels: 1,
        };
        {
            let mut s = self.store.lock().unwrap();
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
        let saved = pipeline::save::run(
            &self.store,
            id,
            &working,
            &[segment],
            Some(root),
            &session_save::RealFs,
            &pipeline::Unattended,
        )
        .unwrap();
        assert_eq!(self.state(id), SessionState::Saved);
        saved.path
    }

    /// Scan `root` and reconcile the store at `now`.
    fn sync(&mut self, root: &Path, now: Instant) -> Changes {
        self.sync_claimed(root, now, &never_claimed)
    }

    fn sync_claimed(
        &mut self,
        root: &Path,
        now: Instant,
        claimed: &dyn Fn(&str) -> bool,
    ) -> Changes {
        let known = Known::from_records(&self.store.lock().unwrap().sessions().unwrap());
        let result = scan(&[root.to_path_buf()], &known, &options());
        let mut store = self.store.lock().unwrap();
        self.reconciler
            .reconcile(&mut store, &result, &Coverage::Full, now, claimed)
            .unwrap()
    }

    fn record(&self, id: &str) -> Option<SessionRecord> {
        self.store.lock().unwrap().session(id).unwrap()
    }

    fn state(&self, id: &str) -> SessionState {
        self.record(id).unwrap().state
    }

    fn home(&self, id: &str) -> PathBuf {
        PathBuf::from(self.record(id).unwrap().home_path.unwrap())
    }

    fn notes(&self, id: &str) -> String {
        self.store.lock().unwrap().notes(id).unwrap()
    }
}

/// A Destination inside a temporary folder, so a test can move it away.
fn shared_root() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("OneDrive");
    fs::create_dir(&root).unwrap();
    (dir, root)
}

fn changed(ids: &[&str]) -> Changes {
    Changes {
        changed: ids.iter().map(|id| id.to_string()).collect(),
        removed: Vec::new(),
    }
}

fn removed(ids: &[&str]) -> Changes {
    Changes {
        changed: Vec::new(),
        removed: ids.iter().map(|id| id.to_string()).collect(),
    }
}

fn user_title(text: &str) -> Option<Title> {
    Some(Title {
        text: text.into(),
        source: TitleSource::User,
    })
}

/// Change `session.md` in `folder`, as another computer's write does.
fn edit_doc(folder: &Path, edit: impl FnOnce(&mut SessionDoc)) {
    let path = folder.join("session.md");
    let mut doc = SessionDoc::parse(&fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut doc);
    fs::write(&path, doc.render()).unwrap();
}

/// Move `folder` out of the Destination, as a sync does when it removes it.
fn take_away(folder: &Path, to: &Path) -> PathBuf {
    let away = to.join(folder.file_name().unwrap());
    fs::rename(folder, &away).unwrap();
    away
}

// Covers AE1, AE2.
#[test]
fn a_note_that_a_saved_is_adopted_by_b_with_its_title_date_and_notes() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "- bring snacks\n");

    assert_eq!(b.sync(&root, Instant::now()), changed(&["n1"]));
    let record = b.record("n1").unwrap();
    assert_eq!(record.state, SessionState::Saved);
    assert_eq!(record.title, user_title("Planning"));
    assert_eq!(record.created_at, started());
    assert_eq!(record.asr_model.as_deref(), Some("m"));
    assert_eq!(b.home("n1"), folder);
    assert_eq!(b.notes("n1"), "- bring snacks\n");
    let s = b.store.lock().unwrap();
    assert_eq!(s.segments("n1").unwrap().len(), 1);
    assert_eq!(
        s.transcript_fingerprint("n1").unwrap(),
        Some(fingerprint(TRANSCRIPT.as_bytes()))
    );
}

#[test]
fn a_folder_that_longhand_just_wrote_refreshes_to_no_change_and_emits_nothing() {
    let (_dir, root) = shared_root();
    let mut a = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "- bring snacks\n");
    assert_eq!(a.sync(&root, Instant::now()), Changes::default());
    let refreshed = refresh::refresh(
        &mut a.store.lock().unwrap(),
        "n1",
        &folder,
        options().read_timeout,
    )
    .unwrap();
    assert_eq!(refreshed, Refresh::default());
}

#[test]
fn a_refresh_with_an_unchanged_folder_changes_nothing() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    a.save_note("n1", &root, user_title("Planning"), "- bring snacks\n");
    let now = Instant::now();
    assert_eq!(b.sync(&root, now), changed(&["n1"]));
    let before = b.record("n1");
    assert_eq!(
        b.sync(&root, now + Duration::from_secs(1)),
        Changes::default()
    );
    let home = b.home("n1");
    let refreshed = refresh::refresh(
        &mut b.store.lock().unwrap(),
        "n1",
        &home,
        options().read_timeout,
    )
    .unwrap();
    assert_eq!(refreshed, Refresh::default());
    assert_eq!(b.record("n1"), before);
}

#[test]
fn a_change_from_the_other_computer_refreshes_the_row() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, None, "- first\n");
    let now = Instant::now();
    b.sync(&root, now);

    fs::write(folder.join("notes.md"), "- first\n- second\n").unwrap();
    edit_doc(&folder, |doc| {
        doc.title = Some("Budget".into());
        doc.title_source = Some(TitleSource::User);
        doc.found_speakers = Some(2);
    });
    assert_eq!(b.sync(&root, now), changed(&["n1"]));
    let record = b.record("n1").unwrap();
    assert_eq!(record.title, user_title("Budget"));
    assert_eq!(record.found_speakers, Some(2));
    assert_eq!(b.notes("n1"), "- first\n- second\n");
}

// Covers AE4.
#[test]
fn a_rename_delivered_as_a_removed_folder_and_a_new_one_keeps_the_row_and_follows_the_path() {
    let (dir, root) = shared_root();
    let mut a = computer();
    let b = computer();
    let old = b.save_note("n1", &root, user_title("Planning"), "");
    let now = Instant::now();
    a.sync(&root, now);
    assert_eq!(a.home("n1"), old);

    // B renames the note; the sync delivers the removal first.
    let renamed = pipeline::folder_rename::rename_session(
        &b.store,
        "n1",
        "Budget review",
        &never_claimed,
        &session_save::RealFs,
    )
    .unwrap();
    let pipeline::folder_rename::FolderRename::Moved(new) = renamed else {
        panic!("the folder did not move: {renamed:?}");
    };
    let away = take_away(&new, dir.path());
    assert_eq!(a.sync(&root, now), Changes::default());
    assert!(a.reconciler.is_pending("n1"));

    fs::rename(&away, &new).unwrap();
    let later = now + Duration::from_secs(10);
    assert_eq!(a.sync(&root, later), changed(&["n1"]));
    assert!(!a.reconciler.is_pending("n1"));
    assert_eq!(a.home("n1"), new);
    assert_eq!(a.record("n1").unwrap().title, user_title("Budget review"));
    assert_eq!(a.sync(&root, now + DELETE_WAIT * 2), Changes::default());
    assert!(a.record("n1").is_some());
}

// Covers AE6.
#[test]
fn a_folder_removed_from_an_available_root_is_forgotten_after_30_seconds() {
    let (dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let gone = a.save_note("n1", &root, user_title("Planning"), "");
    let kept = a.save_note("n2", &root, user_title("Budget"), "- kept\n");
    let now = Instant::now();
    assert_eq!(b.sync(&root, now), changed(&["n1", "n2"]));

    let away = take_away(&gone, dir.path());
    assert_eq!(b.sync(&root, now), Changes::default());
    assert_eq!(b.reconciler.next_due(), Some(now + DELETE_WAIT));
    let almost = now + DELETE_WAIT - Duration::from_secs(1);
    assert_eq!(b.sync(&root, almost), Changes::default());
    assert!(b.record("n1").is_some());
    assert_eq!(b.sync(&root, now + DELETE_WAIT), removed(&["n1"]));
    assert!(b.record("n1").is_none());
    assert!(b.reconciler.next_due().is_none());

    // No folder changed: not the removed one, not the other note's.
    assert!(away.join("session.md").is_file());
    assert_eq!(
        fs::read_to_string(kept.join("notes.md")).unwrap(),
        "- kept\n"
    );
    assert!(b.record("n2").is_some());
    assert!(a.record("n1").is_some(), "the other store keeps its row");
}

// Covers AE6, KD7.
#[test]
fn a_root_that_goes_missing_leaves_every_row() {
    let (dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    a.save_note("n1", &root, user_title("Planning"), "");
    a.save_note("n2", &root, user_title("Budget"), "");
    let now = Instant::now();
    b.sync(&root, now);

    // The drive is unmounted.
    fs::rename(&root, dir.path().join("unmounted")).unwrap();
    assert_eq!(b.sync(&root, now), Changes::default());
    assert_eq!(b.sync(&root, now + DELETE_WAIT * 3), Changes::default());
    assert!(b.record("n1").is_some());
    assert!(b.record("n2").is_some());
    assert!(b.reconciler.next_due().is_none());
}

#[test]
fn transcribe_again_reads_the_speaker_count_that_another_computer_wrote() {
    use models::recognition::{RecognitionConfig, SpeakerSource};
    use pipeline::reprocess::{Guards, Readiness, Refusal};
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "");
    b.sync(&root, Instant::now());
    let settings = RecognitionConfig {
        speaker_source: SpeakerSource::Sortformer,
        ..Default::default()
    };
    let ready = |_: &models::recognition::ExecutionSpec| Readiness::Ready;
    let check = |b: &Computer| {
        pipeline::reprocess::check(
            &b.store.lock().unwrap(),
            "n1",
            Guards::default(),
            &settings,
            &ready,
        )
    };
    let model = check(&b).unwrap().config.asr_model;

    // A sets a count above the Sortformer limit while the dialog on B is open.
    edit_doc(&folder, |doc| {
        doc.speaker_count = Some(session_core::SpeakerCount::Exact(6));
    });
    assert!(check(&b).is_ok(), "the check does not read the folder");
    let refused = pipeline::reprocess::queue(
        &mut b.store.lock().unwrap(),
        "n1",
        Guards::default(),
        &settings,
        &model,
        &ready,
    )
    .unwrap_err();
    assert_eq!(
        refused,
        Refusal {
            message: "This note has 6 speakers. Sortformer supports up to 4. Choose another \
                      speaker source in Settings."
                .into(),
            in_settings: true,
        }
    );
    assert_eq!(b.state("n1"), SessionState::Saved);
    assert_eq!(
        b.record("n1").unwrap().speaker_count,
        Some(session_core::SpeakerCount::Exact(6))
    );
    assert!(b.store.lock().unwrap().queue().unwrap().is_empty());
}

#[test]
fn a_claimed_row_or_a_row_that_is_not_saved_follows_a_rename_but_is_never_refreshed_or_forgotten() {
    let (dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let claimed_folder = a.save_note("n1", &root, user_title("Planning"), "");
    let queued_folder = a.save_note("n2", &root, user_title("Budget"), "");
    let now = Instant::now();
    b.sync(&root, now);
    // On B, a summary regeneration claims n1, and Transcribe Again queues n2.
    let claimed = |id: &str| id == "n1";
    b.store
        .lock()
        .unwrap()
        .transcribe_again(
            "n2",
            "tdt",
            &Event::UserTranscribeAgain {
                at: started(),
                audio_intact: true,
                capture_active: false,
                regenerating: false,
            },
        )
        .unwrap();
    assert!(matches!(b.state("n2"), SessionState::Queued { .. }));

    // A renames both folders and changes both titles.
    let mut moved = Vec::new();
    for (folder, title) in [
        (&claimed_folder, "Planning v2"),
        (&queued_folder, "Budget v2"),
    ] {
        let to = root.join(title);
        fs::rename(folder, &to).unwrap();
        edit_doc(&to, |doc| doc.title = Some(title.into()));
        moved.push(to);
    }
    assert_eq!(b.sync_claimed(&root, now, &claimed), changed(&["n1", "n2"]));
    assert_eq!(b.home("n1"), moved[0]);
    assert_eq!(b.home("n2"), moved[1]);
    assert_eq!(b.record("n1").unwrap().title, user_title("Planning"));
    assert_eq!(b.record("n2").unwrap().title, user_title("Budget"));

    for folder in &moved {
        take_away(folder, dir.path());
    }
    assert_eq!(b.sync_claimed(&root, now, &claimed), Changes::default());
    let later = now + DELETE_WAIT * 2;
    assert_eq!(b.sync_claimed(&root, later, &claimed), Changes::default());
    assert!(b.record("n1").is_some());
    assert!(b.record("n2").is_some());
}

#[cfg(unix)]
#[test]
fn an_existing_row_whose_session_md_cannot_be_read_keeps_its_row() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "");
    let now = Instant::now();
    b.sync(&root, now);

    let session = folder.join("session.md");
    fs::set_permissions(&session, fs::Permissions::from_mode(0o000)).unwrap();
    let first = b.sync(&root, now);
    let after = b.sync(&root, now + DELETE_WAIT * 2);
    fs::set_permissions(&session, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(first, Changes::default());
    assert_eq!(after, Changes::default());
    assert!(!b.reconciler.is_pending("n1"));
    assert_eq!(b.record("n1").unwrap().title, user_title("Planning"));
}

/// A known row whose `session.md` is not valid is treated as unreadable: it
/// keeps its row, and nothing is refreshed from the file.
#[test]
fn an_existing_row_whose_session_md_is_not_valid_keeps_its_row() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "");
    let now = Instant::now();
    b.sync(&root, now);

    fs::write(folder.join("session.md"), "not a session document").unwrap();
    assert_eq!(b.sync(&root, now), Changes::default());
    assert_eq!(b.sync(&root, now + DELETE_WAIT * 2), Changes::default());
    assert_eq!(b.record("n1").unwrap().title, user_title("Planning"));
}

#[test]
fn the_last_note_of_a_marked_root_removed_on_b_is_forgotten_by_a_after_30_seconds() {
    let (_dir, root) = shared_root();
    let mut a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "");
    let now = Instant::now();
    a.sync(&root, now);
    b.sync(&root, now);
    assert!(library::scan::marker(&root).is_file());

    // B moves the note to the Trash.
    fs::remove_dir_all(&folder).unwrap();
    assert_eq!(a.sync(&root, now), Changes::default());
    assert_eq!(a.sync(&root, now + DELETE_WAIT), removed(&["n1"]));
    assert!(a.record("n1").is_none());
}

#[test]
fn an_older_session_md_without_a_title_source_keeps_the_rows_title_source() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let proposed = Some(Title {
        text: "Planning".into(),
        source: TitleSource::Proposed,
    });
    let folder = a.save_note("n1", &root, proposed.clone(), "");
    let now = Instant::now();
    b.sync(&root, now);
    assert_eq!(b.record("n1").unwrap().title, proposed);

    // An older Longhand rewrites `session.md` with the same title.
    edit_doc(&folder, |doc| doc.title_source = None);
    assert_eq!(b.sync(&root, now), Changes::default());
    assert_eq!(b.record("n1").unwrap().title, proposed);

    // A new title with no source is the user's (R5).
    edit_doc(&folder, |doc| doc.title = Some("Budget".into()));
    assert_eq!(b.sync(&root, now), changed(&["n1"]));
    assert_eq!(b.record("n1").unwrap().title, user_title("Budget"));
}

#[test]
fn a_proposed_title_from_the_folder_replaces_a_users_title_in_the_row() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "");
    let now = Instant::now();
    b.sync(&root, now);

    // The other computer cleared the title and processing proposed one.
    edit_doc(&folder, |doc| {
        doc.title = Some("Budget".into());
        doc.title_source = Some(TitleSource::Proposed);
    });
    assert_eq!(b.sync(&root, now), changed(&["n1"]));
    assert_eq!(
        b.record("n1").unwrap().title,
        Some(Title {
            text: "Budget".into(),
            source: TitleSource::Proposed,
        })
    );
}

// Covers AE9.
#[test]
fn a_new_transcript_from_b_clears_as_recognition_results_for_that_note() {
    let (_dir, root) = shared_root();
    let mut a = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "");
    let other = a.save_note("n2", &root, user_title("Budget"), "");
    let now = Instant::now();
    {
        let s = a.store.lock().unwrap();
        for id in ["n1", "n2"] {
            for stage in [Stage::Assemble, Stage::Transcribe, Stage::Diarize] {
                s.put_checkpoint(id, stage, "window-0", "{}").unwrap();
            }
        }
    }
    assert_eq!(a.sync(&root, now), Changes::default());

    // B runs Transcribe Again with another model and saves. The sync
    // delivers transcript.md and session.md together.
    let transcript = "# Transcript\n\n- [00:00:01] Speaker 1: We start with the plan.\n";
    fs::write(folder.join("transcript.md"), transcript).unwrap();
    edit_doc(&folder, |doc| {
        doc.asr_model = Some("tdt".into());
        doc.transcript_fingerprint = Some(fingerprint(transcript.as_bytes()));
    });
    assert_eq!(a.sync(&root, now), changed(&["n1"]));
    assert!(a.reconciler.status().note("n1").waiting.is_empty());

    let s = a.store.lock().unwrap();
    for stage in [Stage::Assemble, Stage::Transcribe, Stage::Diarize] {
        assert!(s.checkpoints("n1", stage).unwrap().is_empty(), "{stage:?}");
        assert!(!s.checkpoints("n2", stage).unwrap().is_empty(), "{stage:?}");
    }
    assert_eq!(
        s.transcript_fingerprint("n1").unwrap(),
        Some(fingerprint(transcript.as_bytes()))
    );
    assert_eq!(
        s.session("n1").unwrap().unwrap().asr_model.as_deref(),
        Some("tdt")
    );
    assert!(other.is_dir());
}

// Covers AE1 (R6, R7).
#[test]
fn a_new_session_md_with_the_old_summary_waits_until_the_summary_arrives() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "");
    assert_eq!(b.sync_roots(&[&root]), changed(&["n1"]));

    // A saved a new transcript and summary. The first sync batch delivers
    // transcript.md and session.md, but not summary.md.
    let transcript = "# Transcript\n\n- [00:00:01] Speaker 1: We start with the plan.\n";
    let summary = "# Summary\n\nA new summary.\n";
    fs::write(folder.join("transcript.md"), transcript).unwrap();
    edit_doc(&folder, |doc| {
        doc.transcript_fingerprint = Some(fingerprint(transcript.as_bytes()));
        doc.summary_fingerprint = Some(fingerprint(summary.as_bytes()));
    });
    assert_eq!(b.sync_folders(&root, &[&folder]), changed(&["n1"]));
    assert_eq!(b.reconciler.status().note("n1").waiting, [Waiting::Summary]);
    // B follows the new transcript.
    assert_eq!(
        b.store
            .lock()
            .unwrap()
            .transcript_fingerprint("n1")
            .unwrap(),
        Some(fingerprint(transcript.as_bytes()))
    );

    // The second batch delivers summary.md. The wait clears, and the scan
    // reports the note, so an open note page reads it again.
    fs::write(folder.join("summary.md"), summary).unwrap();
    assert_eq!(b.sync_folders(&root, &[&folder]), changed(&["n1"]));
    assert_eq!(b.reconciler.status().note("n1"), NoteStatus::default());
}

impl Computer {
    /// A full scan of `root` at `now` (KTD7).
    fn sync_at(&mut self, root: &Path, now: Instant) -> Changes {
        let target = Target::Full(vec![root.to_path_buf()]);
        self.reconciler
            .run_at(&self.store, &target, &never_claimed, now)
            .unwrap()
    }

    fn waiting(&self, id: &str) -> Vec<Waiting> {
        self.reconciler.status().note(id).waiting
    }
}

const HAND_EDIT: &str = "# Transcript\n\n- [00:00:01] Speaker 1: A hand edit.\n";

// Covers AE2 (R8), KTD7.
#[test]
fn a_hand_edit_of_the_transcript_waits_for_a_minute_and_then_is_a_hand_edit() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "");
    let start = Instant::now();
    b.sync_at(&root, start);

    fs::write(folder.join("transcript.md"), HAND_EDIT).unwrap();
    b.sync_at(&root, start);
    assert_eq!(b.waiting("n1"), [Waiting::Transcript]);
    // The worker scans again at the deadline.
    assert_eq!(b.reconciler.next_due(), Some(start + HAND_EDIT_WAIT));

    let almost = start + HAND_EDIT_WAIT - Duration::from_secs(1);
    b.sync_at(&root, almost);
    assert_eq!(b.waiting("n1"), [Waiting::Transcript]);

    // At the deadline the note is a hand edit: it does not wait, and the
    // scan reports it, so the note page releases the actions.
    let changes = b.sync_at(&root, start + HAND_EDIT_WAIT);
    assert!(changes.changed.contains(&"n1".to_string()), "{changes:?}");
    assert_eq!(b.reconciler.status().note("n1"), NoteStatus::default());
    assert_eq!(b.reconciler.next_due(), None);
    b.sync_at(&root, start + HAND_EDIT_WAIT * 2);
    assert!(b.waiting("n1").is_empty());
}

// Covers KTD7: a new change restarts the clock.
#[test]
fn a_second_change_during_the_wait_restarts_the_clock() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "");
    let start = Instant::now();
    b.sync_at(&root, start);

    fs::write(folder.join("transcript.md"), HAND_EDIT).unwrap();
    b.sync_at(&root, start);
    let later = start + HAND_EDIT_WAIT / 2;
    fs::write(folder.join("summary.md"), "# Summary\n\nEdited.\n").unwrap();
    b.sync_at(&root, later);
    assert_eq!(b.reconciler.next_due(), Some(later + HAND_EDIT_WAIT));

    b.sync_at(&root, start + HAND_EDIT_WAIT);
    assert_eq!(b.waiting("n1"), [Waiting::Transcript, Waiting::Summary]);
    b.sync_at(&root, later + HAND_EDIT_WAIT);
    assert!(b.waiting("n1").is_empty());
}

// Covers R8: a file that exists but cannot be read is never a hand edit.
#[test]
fn a_document_that_cannot_be_read_never_expires() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "");
    let start = Instant::now();
    b.sync_at(&root, start);

    // A folder in place of the file: the read fails.
    fs::remove_file(folder.join("summary.md")).unwrap();
    fs::create_dir(folder.join("summary.md")).unwrap();
    b.sync_at(&root, start);
    assert_eq!(b.waiting("n1"), [Waiting::Summary]);
    assert_eq!(b.reconciler.next_due(), None);
    b.sync_at(&root, start + HAND_EDIT_WAIT * 10);
    assert_eq!(b.waiting("n1"), [Waiting::Summary]);
}

// Covers R8: a missing file expires, so that an action can make it again.
#[test]
fn a_missing_summary_expires_and_a_summary_that_arrives_clears_the_wait() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "");
    let summary = fs::read(folder.join("summary.md")).unwrap();
    let start = Instant::now();
    b.sync_at(&root, start);

    fs::remove_file(folder.join("summary.md")).unwrap();
    b.sync_at(&root, start);
    assert_eq!(b.waiting("n1"), [Waiting::Summary]);
    // The file arrives before the deadline: the note does not wait.
    fs::write(folder.join("summary.md"), &summary).unwrap();
    b.sync_at(&root, start + Duration::from_secs(5));
    assert!(b.waiting("n1").is_empty());
    assert_eq!(b.reconciler.next_due(), None);

    let later = start + Duration::from_secs(10);
    fs::remove_file(folder.join("summary.md")).unwrap();
    b.sync_at(&root, later);
    assert_eq!(b.waiting("n1"), [Waiting::Summary]);
    b.sync_at(&root, later + HAND_EDIT_WAIT);
    assert!(b.waiting("n1").is_empty());
}

// Covers R8: a missing transcript also expires. Regenerate Summary then
// refuses with "The transcript is missing." (pipeline tests).
#[test]
fn a_missing_transcript_expires_after_the_wait() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "");
    let start = Instant::now();
    b.sync_at(&root, start);

    fs::remove_file(folder.join("transcript.md")).unwrap();
    b.sync_at(&root, start);
    assert_eq!(b.waiting("n1"), [Waiting::Transcript]);
    assert_eq!(b.reconciler.next_due(), Some(start + HAND_EDIT_WAIT));
    b.sync_at(&root, start + HAND_EDIT_WAIT);
    assert!(b.waiting("n1").is_empty());
}

// Covers R7: the scan and the adoption do not refuse a note for a
// mismatch.
#[test]
fn a_new_transcript_with_the_old_session_md_waits_and_is_still_adopted() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "- bring snacks\n");

    // The sync delivers the new transcript.md of A before its session.md.
    fs::write(
        folder.join("transcript.md"),
        "# Transcript\n\n- [00:00:01] Speaker 1: We start with the plan.\n",
    )
    .unwrap();
    assert_eq!(b.sync_roots(&[&root]), changed(&["n1"]));
    assert_eq!(b.state("n1"), SessionState::Saved);
    assert_eq!(b.home("n1"), folder);
    assert_eq!(b.notes("n1"), "- bring snacks\n");
    assert_eq!(
        b.reconciler.status().note("n1").waiting,
        [Waiting::Transcript]
    );
}

// Covers R6, R7, KTD6.
#[test]
fn a_rename_on_b_during_a_sync_keeps_the_old_fingerprints_and_the_note_waits() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "");
    b.sync_roots(&[&root]);
    let written = SessionDoc::read(&folder).unwrap();

    // The sync delivers the new transcript.md of A before its session.md.
    fs::write(
        folder.join("transcript.md"),
        "# Transcript\n\n- [00:00:01] Speaker 1: We start with the plan.\n",
    )
    .unwrap();
    b.sync_roots(&[&root]);
    let rename = |title: &str| {
        pipeline::folder_rename::rename_session(
            &b.store,
            "n1",
            title,
            &never_claimed,
            &session_save::RealFs,
        )
        .unwrap()
    };

    // A title that keeps the folder name, then one that renames it.
    assert_eq!(
        rename("Planning?"),
        pipeline::folder_rename::FolderRename::Unchanged
    );
    let doc = SessionDoc::read(&folder).unwrap();
    assert_eq!(doc.title.as_deref(), Some("Planning?"));
    assert_eq!(doc.transcript_fingerprint, written.transcript_fingerprint);
    assert_eq!(doc.summary_fingerprint, written.summary_fingerprint);

    let pipeline::folder_rename::FolderRename::Moved(moved) = rename("Budget") else {
        panic!("the folder did not move");
    };
    let doc = SessionDoc::read(&moved).unwrap();
    assert_eq!(doc.title.as_deref(), Some("Budget"));
    assert_eq!(doc.transcript_fingerprint, written.transcript_fingerprint);
    assert_eq!(doc.summary_fingerprint, written.summary_fingerprint);
    b.sync_roots(&[&root]);
    assert_eq!(
        b.reconciler.status().note("n1").waiting,
        [Waiting::Transcript]
    );
}

// Covers KTD10.
#[test]
fn a_refresh_promotes_the_result_snapshot_only_when_both_documents_match() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "");
    b.sync_roots(&[&root]);
    let before = b.store.lock().unwrap().recognition_snapshot("n1").unwrap();

    // A Transcribe Again on A wrote a new session.md with a new snapshot.
    // Its transcript.md has not arrived.
    let transcript = "# Transcript\n\n- [00:00:01] Speaker 1: We start with the plan.\n";
    let snapshot = serde_json::json!({ "model": "tdt" });
    edit_doc(&folder, |doc| {
        doc.transcript_fingerprint = Some(fingerprint(transcript.as_bytes()));
        doc.recognition_snapshot = Some(snapshot.clone());
    });
    let refresh = |b: &Computer| {
        refresh::refresh(
            &mut b.store.lock().unwrap(),
            "n1",
            &folder,
            options().read_timeout,
        )
        .unwrap()
    };
    refresh(&b);
    assert_eq!(
        b.store.lock().unwrap().recognition_snapshot("n1").unwrap(),
        before
    );

    fs::write(folder.join("transcript.md"), transcript).unwrap();
    refresh(&b);
    assert_eq!(
        b.store.lock().unwrap().recognition_snapshot("n1").unwrap(),
        Some(snapshot)
    );
}

// Covers KTD3: a writer inside its own run.
#[test]
fn a_refresh_inside_a_run_takes_a_users_title_and_the_notes_and_keeps_the_runs_results() {
    let (_dir, root) = shared_root();
    let a = computer();
    let folder = a.save_note("n1", &root, user_title("Planning"), "- bring snacks\n");
    a.store
        .lock()
        .unwrap()
        .put_checkpoint("n1", Stage::Transcribe, "window-0", "{}")
        .unwrap();
    let stored = a
        .store
        .lock()
        .unwrap()
        .transcript_fingerprint("n1")
        .unwrap();

    // B renames the note, edits its notes and writes a new transcript.
    let transcript = "# Transcript\n\n- [00:00:01] Speaker 1: We start with the plan.\n";
    fs::write(folder.join("transcript.md"), transcript).unwrap();
    fs::write(folder.join("notes.md"), "- bring more snacks\n").unwrap();
    edit_doc(&folder, |doc| {
        doc.title = Some("Budget".into());
        doc.title_source = Some(TitleSource::User);
        doc.asr_model = Some("tdt".into());
        doc.found_speakers = Some(3);
        doc.transcript_fingerprint = Some(fingerprint(transcript.as_bytes()));
    });

    let refreshed = refresh::refresh_scoped(
        &mut a.store.lock().unwrap(),
        "n1",
        &folder,
        options().read_timeout,
        refresh::Scope::TitleAndNotes,
    )
    .unwrap();
    assert_eq!(
        refreshed,
        Refresh {
            changed: true,
            cleared_recognition_results: false,
        }
    );
    let record = a.record("n1").unwrap();
    assert_eq!(record.title, user_title("Budget"));
    assert_eq!(a.notes("n1"), "- bring more snacks\n");
    // The run's model, speakers, results and fingerprint stay.
    assert_eq!(record.asr_model.as_deref(), Some("m"));
    assert_eq!(record.found_speakers, None);
    let s = a.store.lock().unwrap();
    assert_eq!(s.checkpoints("n1", Stage::Transcribe).unwrap().len(), 1);
    assert_eq!(s.transcript_fingerprint("n1").unwrap(), stored);
}

// ---------------------------------------------------------------- user fields (KTD2 of the note types and tags plan)

fn pending_type() -> PendingType {
    PendingType {
        to: Some("One on One".into()),
        from: None,
        by: "computer-b".into(),
        at: started(),
    }
}

/// Write a tag and a pending type into `session.md` in `folder`, as the
/// other computer's tag and type edit does.
fn edit_user_fields(folder: &Path) {
    edit_doc(folder, |doc| {
        doc.tags = vec!["Topic::Security".into()];
        doc.pending_type = Some(pending_type());
    });
}

fn assert_user_fields(record: &SessionRecord) {
    assert_eq!(record.tags, vec!["Topic::Security".to_string()]);
    assert_eq!(record.pending_type, Some(pending_type()));
}

#[test]
fn an_adopted_note_has_the_tags_and_the_pending_type_of_its_session_md() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, None, "");
    edit_user_fields(&folder);
    assert_eq!(b.sync(&root, Instant::now()), changed(&["n1"]));
    let record = b.record("n1").unwrap();
    assert_user_fields(&record);
    // The scan sets the note type from the path (U3).
    assert_eq!(record.note_type, None);
}

#[test]
fn a_tag_edit_from_the_other_computer_refreshes_a_saved_row() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, None, "");
    let now = Instant::now();
    b.sync(&root, now);
    edit_user_fields(&folder);
    assert_eq!(b.sync(&root, now), changed(&["n1"]));
    assert_user_fields(&b.record("n1").unwrap());

    // A removal of the fields also refreshes the row.
    edit_doc(&folder, |doc| {
        doc.tags.clear();
        doc.pending_type = None;
    });
    assert_eq!(b.sync(&root, now), changed(&["n1"]));
    let record = b.record("n1").unwrap();
    assert!(record.tags.is_empty());
    assert_eq!(record.pending_type, None);
}

#[test]
fn a_busy_rows_reconcile_takes_a_tag_edit_from_the_other_computer() {
    let (_dir, root) = shared_root();
    let a = computer();
    let mut b = computer();
    let folder = a.save_note("n1", &root, None, "");
    let now = Instant::now();
    b.sync(&root, now);
    // On B, Transcribe Again queues the note: the row is busy.
    b.store
        .lock()
        .unwrap()
        .transcribe_again(
            "n1",
            "tdt",
            &Event::UserTranscribeAgain {
                at: started(),
                audio_intact: true,
                capture_active: false,
                regenerating: false,
            },
        )
        .unwrap();
    assert!(matches!(b.state("n1"), SessionState::Queued { .. }));

    edit_user_fields(&folder);
    assert_eq!(b.sync(&root, now), changed(&["n1"]));
    assert_user_fields(&b.record("n1").unwrap());
}

#[test]
fn a_refresh_inside_a_run_takes_the_tags_and_the_pending_type() {
    let (_dir, root) = shared_root();
    let a = computer();
    let folder = a.save_note("n1", &root, None, "");
    edit_user_fields(&folder);
    let refreshed = refresh::refresh_scoped(
        &mut a.store.lock().unwrap(),
        "n1",
        &folder,
        options().read_timeout,
        refresh::Scope::TitleAndNotes,
    )
    .unwrap();
    assert!(refreshed.changed);
    assert_user_fields(&a.record("n1").unwrap());
}

#[test]
fn a_refresh_inside_a_run_keeps_the_runs_title_over_a_proposal_in_the_folder() {
    let (_dir, root) = shared_root();
    let a = computer();
    let proposed = |text: &str| {
        Some(Title {
            text: text.into(),
            source: TitleSource::Proposed,
        })
    };
    let folder = a.save_note("n1", &root, proposed("Planning"), "");
    // The run proposed a new title with its new summary.
    assert!(
        a.store
            .lock()
            .unwrap()
            .set_title("n1", "Budget", TitleSource::Proposed)
            .unwrap()
    );

    let refreshed = refresh::refresh_scoped(
        &mut a.store.lock().unwrap(),
        "n1",
        &folder,
        options().read_timeout,
        refresh::Scope::TitleAndNotes,
    )
    .unwrap();
    assert_eq!(refreshed, Refresh::default());
    assert_eq!(a.record("n1").unwrap().title, proposed("Budget"));
}

impl Computer {
    /// A targeted scan of `folders` in `root`, as the watcher runs it (U6).
    fn sync_folders(&mut self, root: &Path, folders: &[&Path]) -> Changes {
        let target = Target::Folders {
            root: root.to_path_buf(),
            folders: folders.iter().map(|f| f.to_path_buf()).collect(),
        };
        self.reconciler
            .run(&self.store, &target, &never_claimed)
            .unwrap()
    }

    fn sync_roots(&mut self, roots: &[&Path]) -> Changes {
        let target = Target::Full(roots.iter().map(|r| r.to_path_buf()).collect());
        self.reconciler
            .run(&self.store, &target, &never_claimed)
            .unwrap()
    }
}

// Covers R7 (U6).
#[test]
fn a_targeted_scan_adopts_a_new_folder_and_starts_no_delete_for_the_others() {
    let (dir, root) = shared_root();
    let mut a = computer();
    let b = computer();
    let first = b.save_note("n1", &root, user_title("Planning"), "");
    a.sync(&root, Instant::now());

    let second = b.save_note("n2", &root, user_title("Budget"), "");
    take_away(&first, dir.path());
    assert_eq!(a.sync_folders(&root, &[&second]), changed(&["n2"]));
    assert_eq!(a.home("n2"), second);
    assert!(!a.reconciler.is_pending("n1"), "n1 was not in the scan");
    // A targeted scan follows a file change in its folders, so it reports
    // the note in them again, and nothing else.
    assert_eq!(a.sync_folders(&root, &[&second]), changed(&["n2"]));
}

// Covers R10 (U6).
#[test]
fn a_targeted_scan_of_a_removed_folder_starts_its_pending_delete() {
    let (dir, root) = shared_root();
    let mut a = computer();
    let b = computer();
    let folder = b.save_note("n1", &root, user_title("Planning"), "");
    a.sync(&root, Instant::now());

    take_away(&folder, dir.path());
    assert_eq!(a.sync_folders(&root, &[&folder]), Changes::default());
    assert!(a.reconciler.is_pending("n1"));
    assert!(a.record("n1").is_some());
}

// Covers R8, R15 (U6).
#[test]
fn a_targeted_scan_of_a_duplicate_folder_keeps_the_home_folder() {
    let (_dir, root) = shared_root();
    let mut a = computer();
    let b = computer();
    let folder = b.save_note("n1", &root, user_title("Planning"), "");
    a.sync(&root, Instant::now());

    let name = folder.file_name().unwrap().to_string_lossy().into_owned();
    let copy = root.join(format!("{name} (1)"));
    fs::create_dir(&copy).unwrap();
    for entry in fs::read_dir(&folder).unwrap().flatten() {
        if entry.path().is_file() {
            fs::copy(entry.path(), copy.join(entry.file_name())).unwrap();
        }
    }
    // The row does not change, but the note now has another version (R15).
    assert_eq!(a.sync_folders(&root, &[&copy]), changed(&["n1"]));
    assert_eq!(a.home("n1"), folder);
    assert_eq!(
        a.reconciler.status().note("n1").other_versions,
        std::slice::from_ref(&copy)
    );
    assert_eq!(a.sync_folders(&root, &[&copy]), Changes::default());
}

// Covers R8 (U6).
#[test]
fn a_targeted_scan_reports_a_note_whose_summary_alone_changed() {
    let (_dir, root) = shared_root();
    let mut a = computer();
    let b = computer();
    let folder = b.save_note("n1", &root, user_title("Planning"), "");
    a.sync(&root, Instant::now());

    // Another computer regenerated the summary: the row does not change,
    // but an open note page must read the documents again.
    // The sync delivers summary.md and session.md together.
    let summary = "# Summary\n\nA new summary.\n";
    fs::write(folder.join("summary.md"), summary).unwrap();
    edit_doc(&folder, |doc| {
        doc.summary_fingerprint = Some(fingerprint(summary.as_bytes()));
    });
    assert_eq!(a.sync_folders(&root, &[&folder]), changed(&["n1"]));
    assert!(a.reconciler.status().note("n1").waiting.is_empty());
}

#[test]
fn a_conflict_copy_that_syncs_first_gives_way_to_the_real_folder() {
    let (dir, root) = shared_root();
    let mut a = computer();
    let b = computer();
    let folder = b.save_note("n1", &root, user_title("Planning"), "");
    let name = folder.file_name().unwrap().to_string_lossy().into_owned();

    // Only the conflict copy is in the storage folder at first, so A adopts
    // the note from it.
    let away = dir.path().join("real");
    fs::rename(&folder, &away).unwrap();
    let copy = root.join(format!("{name} (1)"));
    fs::create_dir(&copy).unwrap();
    copy_files(&away, &copy);
    a.sync(&root, Instant::now());
    assert_eq!(a.home("n1"), copy);

    // The real folder arrives: A moves to it and lists the copy as another
    // version.
    fs::rename(&away, &folder).unwrap();
    a.sync_roots(&[&root]);
    assert_eq!(a.home("n1"), folder);
    assert_eq!(
        a.reconciler.status().note("n1").other_versions,
        std::slice::from_ref(&copy)
    );
}

fn copy_files(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            fs::create_dir_all(&target).unwrap();
            copy_files(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

// Covers AE7, R6 (U7).
#[test]
fn a_part_that_arrives_later_clears_its_wait_and_reports_the_note() {
    let (dir, root) = shared_root();
    let mut a = computer();
    let b = computer();
    let folder = b.save_note("n1", &root, user_title("Planning"), "");
    let audio = folder.join("audio/segment-01.opus");
    let away = dir.path().join("segment-01.opus");
    fs::rename(&audio, &away).unwrap();

    assert_eq!(a.sync_roots(&[&root]), changed(&["n1"]));
    assert_eq!(
        a.reconciler.status().note("n1").waiting,
        [Waiting::Audio("audio/segment-01.opus".into())]
    );
    assert_eq!(a.sync_roots(&[&root]), Changes::default());

    fs::rename(&away, &audio).unwrap();
    assert_eq!(a.sync_folders(&root, &[&folder]), changed(&["n1"]));
    assert_eq!(a.reconciler.status().note("n1"), NoteStatus::default());
}

// Covers AE8, R15 (U7).
#[test]
fn a_conflict_copy_reports_the_note_and_names_the_file() {
    let (_dir, root) = shared_root();
    let mut a = computer();
    let b = computer();
    let folder = b.save_note("n1", &root, user_title("Planning"), "");
    a.sync_roots(&[&root]);

    let copy = folder.join("notes (1).md");
    fs::write(&copy, "Other notes\n").unwrap();
    assert_eq!(a.sync_folders(&root, &[&folder]), changed(&["n1"]));
    assert_eq!(
        a.reconciler.status().note("n1").other_versions,
        std::slice::from_ref(&copy)
    );

    fs::remove_file(&copy).unwrap();
    assert_eq!(a.sync_roots(&[&root]), changed(&["n1"]));
    assert!(a.reconciler.status().note("n1").other_versions.is_empty());
}

// Covers AE6, R16 (U7).
#[test]
fn a_root_that_goes_away_and_comes_back_reports_its_notes() {
    let (dir, root) = shared_root();
    let mut a = computer();
    let b = computer();
    let folder = b.save_note("n1", &root, user_title("Planning"), "");
    a.sync_roots(&[&root]);
    let status = Arc::clone(a.reconciler.status());
    assert!(!status.storage_unavailable("n1", Some(&folder)));

    let unmounted = dir.path().join("unmounted");
    fs::rename(&root, &unmounted).unwrap();
    assert_eq!(a.sync_roots(&[&root]), changed(&["n1"]));
    assert!(status.storage_unavailable("n1", Some(&folder)));
    assert_eq!(a.sync_roots(&[&root]), Changes::default());

    fs::rename(&unmounted, &root).unwrap();
    assert_eq!(a.sync_roots(&[&root]), changed(&["n1"]));
    assert!(!status.storage_unavailable("n1", Some(&folder)));
}

// Covers KD7, R10 (U6).
#[test]
fn a_scan_of_one_root_keeps_the_pending_deletes_of_another() {
    let (dir, root) = shared_root();
    let other = dir.path().join("Dropbox");
    fs::create_dir(&other).unwrap();
    let away = dir.path().join("away");
    fs::create_dir(&away).unwrap();
    let mut a = computer();
    let b = computer();
    let folder = b.save_note("n1", &root, user_title("Planning"), "");
    b.save_note("n2", &other, user_title("Budget"), "");
    assert_eq!(a.sync_roots(&[&root, &other]), changed(&["n1", "n2"]));

    take_away(&folder, &away);
    a.sync_roots(&[&root]);
    assert!(a.reconciler.is_pending("n1"));
    a.sync_roots(&[&other]);
    assert!(a.reconciler.is_pending("n1"), "a scan of another root");
}

// ---------------------------------------------------------------- removed audio (U2)

const AUDIO: &str = "audio/segment-01.opus";

impl Computer {
    fn removed_audio(&self, id: &str) -> Vec<String> {
        self.store.lock().unwrap().removed_audio(id).unwrap()
    }

    fn audio_state(&self, id: &str) -> AudioState {
        pipeline::save::audio_state(&self.store.lock().unwrap(), id).unwrap()
    }
}

/// The removal marker of `audio` arrives in the note folder `folder`.
fn deliver_marker(folder: &Path, audio: &str) {
    fs::write(
        folder.join(session_core::folder::removal_marker(audio)),
        session_core::folder::REMOVAL_MARKER_TEXT,
    )
    .unwrap();
}

#[test]
fn a_saved_note_has_its_audio_present() {
    let (_dir, root) = shared_root();
    let mut a = computer();
    let b = computer();
    b.save_note("n1", &root, user_title("Planning"), "");
    a.sync_roots(&[&root]);
    assert_eq!(a.audio_state("n1"), AudioState::Present);
    assert_eq!(b.audio_state("n1"), AudioState::Present);
    assert!(a.removed_audio("n1").is_empty());
}

// Covers AE5, R15.
#[test]
fn a_removal_whose_delete_arrives_before_its_marker_waits_then_completes() {
    let (_dir, root) = shared_root();
    let mut a = computer();
    let b = computer();
    let folder = b.save_note("n1", &root, user_title("Planning"), "");
    a.sync_roots(&[&root]);

    // B removed the audio. The sync delivers the delete first.
    fs::remove_file(folder.join(AUDIO)).unwrap();
    assert_eq!(a.sync_folders(&root, &[&folder]), changed(&["n1"]));
    assert_eq!(
        a.reconciler.status().note("n1").waiting,
        [Waiting::Audio(AUDIO.into())]
    );
    assert_eq!(a.audio_state("n1"), AudioState::Waiting);
    assert!(a.removed_audio("n1").is_empty());

    // The marker arrives: the note is complete, and the watcher's scan
    // reports it.
    deliver_marker(&folder, AUDIO);
    assert_eq!(a.sync_folders(&root, &[&folder]), changed(&["n1"]));
    assert_eq!(a.reconciler.status().note("n1"), NoteStatus::default());
    assert_eq!(a.removed_audio("n1"), [AUDIO]);
    assert_eq!(a.audio_state("n1"), AudioState::Removed);
    assert_eq!(a.sync_roots(&[&root]), Changes::default());
}

// Covers R15.
#[test]
fn a_removal_whose_marker_arrives_before_its_delete_is_removed_at_once() {
    let (_dir, root) = shared_root();
    let mut a = computer();
    let b = computer();
    let folder = b.save_note("n1", &root, user_title("Planning"), "");
    a.sync_roots(&[&root]);

    deliver_marker(&folder, AUDIO);
    // A full scan reports only the notes whose row or status changed: the
    // removed flag in the row makes the report, so an open note page reads
    // the note again.
    assert_eq!(a.sync_roots(&[&root]), changed(&["n1"]));
    assert_eq!(a.reconciler.status().note("n1"), NoteStatus::default());
    assert_eq!(a.removed_audio("n1"), [AUDIO]);
    assert_eq!(a.audio_state("n1"), AudioState::Removed);

    fs::remove_file(folder.join(AUDIO)).unwrap();
    assert_eq!(a.sync_roots(&[&root]), Changes::default());
    assert_eq!(a.reconciler.status().note("n1"), NoteStatus::default());
    assert_eq!(a.removed_audio("n1"), [AUDIO]);
    assert_eq!(a.audio_state("n1"), AudioState::Removed);
}

#[test]
fn a_note_adopted_after_its_audio_was_removed_is_complete_and_removed() {
    let (_dir, root) = shared_root();
    let mut a = computer();
    let b = computer();
    let folder = b.save_note("n1", &root, user_title("Planning"), "");
    deliver_marker(&folder, AUDIO);
    fs::remove_file(folder.join(AUDIO)).unwrap();

    assert_eq!(a.sync_roots(&[&root]), changed(&["n1"]));
    assert_eq!(a.state("n1"), SessionState::Saved);
    assert_eq!(a.reconciler.status().note("n1"), NoteStatus::default());
    assert_eq!(a.removed_audio("n1"), [AUDIO]);
    assert_eq!(a.audio_state("n1"), AudioState::Removed);
}

#[test]
fn a_refresh_sets_the_removed_flag_from_the_folder() {
    let (_dir, root) = shared_root();
    let mut a = computer();
    let b = computer();
    let folder = b.save_note("n1", &root, user_title("Planning"), "");
    a.sync_roots(&[&root]);

    deliver_marker(&folder, AUDIO);
    let refreshed = {
        let mut s = a.store.lock().unwrap();
        refresh::refresh(&mut s, "n1", &folder, Duration::from_secs(2)).unwrap()
    };
    assert!(refreshed.changed);
    assert_eq!(a.removed_audio("n1"), [AUDIO]);
    let again = {
        let mut s = a.store.lock().unwrap();
        refresh::refresh(&mut s, "n1", &folder, Duration::from_secs(2)).unwrap()
    };
    assert_eq!(again, Refresh::default());
}

// Covers R16.
#[test]
fn the_recording_computer_also_sees_its_own_note_as_removed() {
    let (_dir, root) = shared_root();
    let mut a = computer();
    let b = computer();
    let folder = b.save_note("n1", &root, user_title("Planning"), "");
    a.sync_roots(&[&root]);
    // A removes the audio of the note that B recorded. B scans.
    deliver_marker(&folder, AUDIO);
    fs::remove_file(folder.join(AUDIO)).unwrap();
    let mut b = b;
    assert_eq!(b.sync_roots(&[&root]), changed(&["n1"]));
    assert_eq!(b.removed_audio("n1"), [AUDIO]);
    assert_eq!(b.audio_state("n1"), AudioState::Removed);
}

// ------------------------------------- a note that records elsewhere (U4)

const RUN_A: &str = "run-a";
const RUN_B: &str = "run-b";

/// A computer whose run has the ID `run`: the in-use markers of that run
/// are its own, and its scans do not count them.
fn computer_run(run: &str) -> Computer {
    Computer {
        reconciler: Reconciler::new(&options()).with_own_runs([run.to_string()]),
        ..computer()
    }
}

impl Computer {
    /// Start a note that records, and write its home folder in `root` as
    /// the early write does: this run's in-use marker, `notes.md`, and
    /// `session.md` with `result: pending`. Returns the folder.
    fn start_recording(&self, id: &str, root: &Path, run: &str) -> PathBuf {
        {
            let mut s = self.store.lock().unwrap();
            s.create_session(id, started(), None).unwrap();
            pipeline::early_home::begin(&s, id).unwrap();
        }
        pipeline::early_home::write(&self.store, id, Some(root), run, &session_save::RealFs)
            .unwrap()
            .unwrap()
    }

    /// Open a note that does not record, and write its home folder in
    /// `root` with `result: none` and no in-use marker. Returns the folder.
    fn open_note(&self, id: &str, root: &Path) -> PathBuf {
        self.store
            .lock()
            .unwrap()
            .create_notes_only(id, started(), None)
            .unwrap();
        pipeline::early_home::write(&self.store, id, Some(root), RUN_A, &session_save::RealFs)
            .unwrap()
            .unwrap()
    }

    /// The remote state that the note page and Home show for `id`.
    fn remote(&self, id: &str) -> Option<RemoteState> {
        self.reconciler.status().remote(&self.record(id).unwrap())
    }
}

/// Make the file time of `session.md` in `folder` older than the time an
/// in-use marker stays fresh.
fn age_session_md(folder: &Path) {
    let old = SystemTime::now() - FRESH_FOR.to_std().unwrap() - Duration::from_secs(60);
    fs::File::options()
        .write(true)
        .open(folder.join("session.md"))
        .unwrap()
        .set_modified(old)
        .unwrap();
}

fn now() -> DateTime<FixedOffset> {
    chrono::Local::now().fixed_offset()
}

// Covers R12, R20.
#[test]
fn b_shows_a_note_that_a_records_as_in_use_and_reads_its_notes() {
    let (_dir, root) = shared_root();
    let mut a = computer_run(RUN_A);
    let mut b = computer_run(RUN_B);
    let folder = a.start_recording("n1", &root, RUN_A);
    fs::write(folder.join("notes.md"), "Agenda\n").unwrap();

    assert_eq!(b.sync_roots(&[&root]), changed(&["n1"]));
    assert_eq!(b.state("n1"), SessionState::Saved);
    assert_eq!(b.home("n1"), folder);
    assert_eq!(b.notes("n1"), "Agenda\n");
    assert!(b.reconciler.status().note("n1").waiting.is_empty());
    assert_eq!(b.remote("n1"), Some(RemoteState::InUse));

    // On A, the note records here: its own marker and flag are not work on
    // another computer.
    a.sync_roots(&[&root]);
    assert_eq!(a.remote("n1"), None);
    assert!(a.reconciler.status().note("n1").waiting.is_empty());
}

// Covers R23.
#[test]
fn b_shows_a_new_note_that_is_not_saved_as_in_use_before_its_marker_syncs() {
    let (_dir, root) = shared_root();
    let a = computer_run(RUN_A);
    let mut b = computer_run(RUN_B);
    let folder = a.start_recording("n1", &root, RUN_A);
    pipeline::in_use::remove_marker(&folder, RUN_A).unwrap();

    b.sync_roots(&[&root]);
    assert_eq!(b.remote("n1"), Some(RemoteState::InUse));
}

// Covers AE13, R23.
#[test]
fn b_shows_a_note_that_a_did_not_finish_as_not_finished() {
    let (_dir, root) = shared_root();
    let a = computer_run(RUN_A);
    let mut b = computer_run(RUN_B);
    let folder = a.start_recording("n1", &root, RUN_A);
    // A crashed an hour ago: its marker is old, and so is `session.md`.
    let old = now() - chrono::TimeDelta::hours(1);
    assert!(pipeline::in_use::write_marker(&folder, RUN_A, old).unwrap());
    age_session_md(&folder);

    assert_eq!(b.sync_roots(&[&root]), changed(&["n1"]));
    assert_eq!(b.remote("n1"), Some(RemoteState::NotFinished));
    assert!(b.reconciler.status().note("n1").waiting.is_empty());

    // A launches again and refreshes its marker: the note is in use.
    assert!(pipeline::in_use::write_marker(&folder, RUN_A, now()).unwrap());
    assert_eq!(b.sync_roots(&[&root]), changed(&["n1"]));
    assert_eq!(b.remote("n1"), Some(RemoteState::InUse));
}

// Covers R12, R21.
#[test]
fn a_fresh_marker_on_a_note_with_no_recording_makes_it_in_use_on_b() {
    let (_dir, root) = shared_root();
    let a = computer_run(RUN_A);
    let mut b = computer_run(RUN_B);
    let folder = a.open_note("n1", &root);

    b.sync_roots(&[&root]);
    assert_eq!(b.remote("n1"), None);
    assert!(b.reconciler.status().note("n1").waiting.is_empty());

    // A starts to record into the note: its marker comes before the flag.
    assert!(pipeline::in_use::write_marker(&folder, RUN_A, now()).unwrap());
    assert_eq!(b.sync_folders(&root, &[&folder]), changed(&["n1"]));
    assert_eq!(b.remote("n1"), Some(RemoteState::InUse));
}

// Covers R22.
#[test]
fn a_notes_edit_from_b_reaches_a_while_a_records() {
    let (_dir, root) = shared_root();
    let mut a = computer_run(RUN_A);
    let mut b = computer_run(RUN_B);
    let folder = a.start_recording("n1", &root, RUN_A);
    b.sync_roots(&[&root]);

    // B's notes save writes `notes.md`.
    fs::write(folder.join("notes.md"), "From B\n").unwrap();
    assert_eq!(a.sync_folders(&root, &[&folder]), changed(&["n1"]));
    assert_eq!(a.notes("n1"), "From B\n");
    assert_eq!(a.state("n1"), SessionState::Capturing);
    assert!(a.store.lock().unwrap().segments("n1").unwrap().is_empty());
}

// Covers R6, R20.
#[test]
fn the_result_of_a_reaches_the_open_note_on_b_and_ends_its_remote_state() {
    let (_dir, root) = shared_root();
    let a = computer_run(RUN_A);
    let mut b = computer_run(RUN_B);
    let folder = a.start_recording("n1", &root, RUN_A);
    b.sync_roots(&[&root]);
    assert_eq!(b.remote("n1"), Some(RemoteState::InUse));

    // A saves the result into the folder, and its keeper removes the
    // marker. The sync delivers the whole result.
    let saved = a.save_recording("n1", &root);
    assert_eq!(saved, folder);
    pipeline::in_use::remove_marker(&folder, RUN_A).unwrap();

    assert_eq!(b.sync_folders(&root, &[&folder]), changed(&["n1"]));
    assert_eq!(b.remote("n1"), None);
    assert_eq!(b.reconciler.status().note("n1"), NoteStatus::default());
    assert_eq!(b.store.lock().unwrap().segments("n1").unwrap().len(), 1);
    let doc = SessionDoc::parse(&fs::read_to_string(folder.join("session.md")).unwrap()).unwrap();
    assert_eq!(doc.result_flag(), None);
}

// Covers KTD9.
#[test]
fn a_rename_on_b_of_a_note_that_a_records_changes_only_the_title() {
    let (_dir, root) = shared_root();
    let a = computer_run(RUN_A);
    let mut b = computer_run(RUN_B);
    let folder = a.start_recording("n1", &root, RUN_A);
    b.sync_roots(&[&root]);

    let renamed = pipeline::folder_rename::rename_session(
        &b.store,
        "n1",
        "Budget",
        &never_claimed,
        &session_save::RealFs,
    )
    .unwrap();
    assert_eq!(renamed, pipeline::folder_rename::FolderRename::Later);
    assert!(folder.is_dir(), "the folder keeps its name");
    let doc = SessionDoc::parse(&fs::read_to_string(folder.join("session.md")).unwrap()).unwrap();
    assert_eq!(doc.title.as_deref(), Some("Budget"));
    assert_eq!(doc.result_flag(), Some(ResultFlag::NotSaved));
    assert!(b.store.lock().unwrap().owed_renames().unwrap().is_empty());

    // A's Save takes the title from the folder and renames the folder.
    let saved = a.save_recording("n1", &root);
    assert_eq!(
        saved.file_name().unwrap().to_string_lossy(),
        session_save::naming::folder_name(started(), Some("Budget"))
    );
    assert!(!folder.exists());
}
