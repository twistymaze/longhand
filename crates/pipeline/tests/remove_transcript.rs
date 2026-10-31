//! The transcript removal step (U4 of the transcript removal plan): it
//! removes the transcript of one saved note and any audio that is left, in
//! the order of KTD3, and a run after each partial effect finishes the
//! work.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, TimeDelta};
use pipeline::SharedStore;
use pipeline::remove_audio::{
    Eligibility, EligibilityInputs, FolderCheck, Guard, NoteSync, SegmentOutcome, StoreCheck,
    eligibility, no_guard, remove_audio, safe_audio_name,
};
use pipeline::remove_copies::RESULTS_FOLDER;
use pipeline::remove_transcript::{
    RemoveTranscriptError, TranscriptEligibility, TranscriptRemoval, finish_due,
    finish_transcript_removal, remove_transcript, remove_transcript_with, summary_stub_in,
    transcript_eligibility,
};
use session_core::folder::{
    SessionDoc, SummaryMeta, TRANSCRIPT_REMOVED_MARKER, TRANSCRIPT_REMOVED_TEXT, fingerprint,
    removal_marker,
};
use session_core::jobs::{Adoption, Claims};
use session_core::note_folder::{NoteFiles, READ_TIMEOUT};
use session_core::recognition::{ARTIFACT_FILE, ArtifactReference};
use session_core::{Segment, Stage};

const ID: &str = "20260601-0900-abcd";
const TRANSCRIPT: &str = "# Transcript\n\n- [00:00:01] Speaker 1: Hello.\n";
const SUMMARY: &str = "# Summary\n\nWe planned the week.\n";
const NOTES: &str = "My notes\n";
const RECOGNITION: &[u8] = b"{\"version\":1}\n";

fn time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

fn audio(index: u32) -> String {
    format!("audio/segment-{:02}.opus", index + 1)
}

/// Two segments of one minute. The last one ends on 1 June.
fn two_segments() -> Vec<Segment> {
    (0..2)
        .map(|i| {
            let end = time(&format!("2026-06-01T09:0{}:00-05:00", i + 1));
            Segment {
                index: i,
                offset_ms: u64::from(i) * 60_000,
                duration_ms: 60_000,
                wall_start: end - TimeDelta::minutes(1),
                wall_end: end,
                audio_file: audio(i),
                channels: 1,
                ends_in_pause: false,
            }
        })
        .collect()
}

fn doc(segments: &[Segment]) -> SessionDoc {
    let mut doc = SessionDoc::new(ID, None, segments).unwrap();
    doc.title = Some("Planning".into());
    doc.tags = vec!["work".into()];
    doc.transcript_fingerprint = Some(fingerprint(TRANSCRIPT.as_bytes()));
    doc.summary_fingerprint = Some(fingerprint(SUMMARY.as_bytes()));
    doc.recognition_snapshot = Some(serde_json::json!({"model": "parakeet"}));
    doc.recognition_artifact = Some(ArtifactReference {
        version: 1,
        filename: ARTIFACT_FILE.into(),
        sha256: "00".repeat(32),
    });
    doc.recognition_audio = Some([(audio(0), "11".repeat(32))].into());
    doc.found_speakers = Some(2);
    doc
}

struct Note {
    _root: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    store: SharedStore,
    claims: Claims,
}

/// A saved note folder in a storage folder with its library marker, as a
/// save leaves it: the documents, `recognition.jsonl`, one audio file for
/// each segment, and a complete summary in `longhand.json`. The store has
/// its row and a Transcribe row.
fn saved_note(segments: &[Segment]) -> Note {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    assert!(library::scan::write_marker(&root_path));
    let home = root_path.join("Planning");
    fs::create_dir_all(home.join("audio")).unwrap();
    let doc = doc(segments);
    session_save::note_files::write_doc(&home, &doc).unwrap();
    set_summary(
        &home,
        SummaryMeta::Complete {
            endpoint_host: "api.example.com".into(),
            model: "model".into(),
            prompt_name: None,
            prompt_id: None,
        },
    );
    fs::write(home.join("transcript.md"), TRANSCRIPT).unwrap();
    fs::write(home.join("summary.md"), SUMMARY).unwrap();
    fs::write(home.join("notes.md"), NOTES).unwrap();
    fs::write(home.join(ARTIFACT_FILE), RECOGNITION).unwrap();
    for segment in segments {
        if safe_audio_name(&segment.audio_file) {
            fs::write(home.join(&segment.audio_file), b"opus").unwrap();
        }
    }
    let store = common::store();
    let adoption = Adoption::from_doc(
        &doc,
        NOTES.into(),
        home.to_string_lossy().into_owned(),
        Some(fingerprint(TRANSCRIPT.as_bytes())),
    );
    {
        let mut store = store.lock().unwrap();
        assert!(store.adopt(&adoption).unwrap());
        store
            .put_checkpoint(ID, Stage::Transcribe, "window-0", "\"Hello.\"")
            .unwrap();
    }
    Note {
        _root: root,
        root: root_path,
        home,
        store,
        claims: Claims::default(),
    }
}

fn set_summary(home: &Path, summary: SummaryMeta) {
    let mut record = NoteFiles::read_timed(home, READ_TIMEOUT).unwrap().record;
    record.summary = Some(summary);
    session_save::note_files::write_record(home, &record).unwrap();
}

fn run(note: &Note) -> TranscriptRemoval {
    remove_transcript(&note.store, &note.claims, ID, no_guard()).unwrap()
}

fn present(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

fn record(home: &Path) -> session_core::note_record::NoteRecord {
    NoteFiles::read_timed(home, READ_TIMEOUT).unwrap().record
}

/// Every file under `dir` with its content, sorted by path.
fn contents(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(contents(&path));
        } else {
            out.push((path.clone(), fs::read(&path).unwrap()));
        }
    }
    out.sort();
    out
}

/// The parts of the note that a removal keeps: `session.md` (title and
/// tags), `summary.md` and `notes.md`.
fn kept(home: &Path) -> Vec<Vec<u8>> {
    ["session.md", "summary.md", "notes.md"]
        .iter()
        .map(|name| fs::read(home.join(name)).unwrap())
        .collect()
}

/// Assert that the note has only what a removal keeps: no transcript
/// files, no transcript fields in `longhand.json`, and no transcript data
/// in the store.
fn assert_removed(note: &Note) {
    assert_eq!(
        fs::read_to_string(note.home.join(TRANSCRIPT_REMOVED_MARKER)).unwrap(),
        TRANSCRIPT_REMOVED_TEXT
    );
    assert!(!present(&note.home.join("transcript.md")));
    assert!(!present(&note.home.join(ARTIFACT_FILE)));
    let record = record(&note.home);
    assert_eq!(record.transcript_fingerprint, None);
    assert_eq!(record.recognition_snapshot, None);
    assert_eq!(record.recognition_artifact, None);
    assert_eq!(record.recognition_audio, None);
    // A count with no text stays.
    assert_eq!(record.found_speakers, Some(2));
    assert!(matches!(record.summary, Some(SummaryMeta::Complete { .. })));
    assert_eq!(
        record.summary_fingerprint,
        Some(fingerprint(SUMMARY.as_bytes()))
    );
    let store = note.store.lock().unwrap();
    assert!(store.transcript_removed(ID).unwrap());
    assert_eq!(store.transcript_fingerprint(ID).unwrap(), None);
    let row = store.session(ID).unwrap().unwrap();
    assert_eq!(row.recognition_snapshot, None);
    assert_eq!(row.found_speakers, Some(2));
    assert_eq!(row.title.map(|t| t.text), Some("Planning".to_string()));
    assert_eq!(row.tags, vec!["work".to_string()]);
    assert_eq!(store.notes(ID).unwrap(), NOTES);
    assert_eq!(
        store
            .checkpoint_value(ID, Stage::Transcribe, "window-0")
            .unwrap(),
        None
    );
}

/// Mark the audio of every segment removed, as the audio step leaves it.
fn remove_all_audio(note: &Note) {
    let removal =
        remove_audio(&note.store, &note.claims, ID, no_guard(), NoteSync::Synced).unwrap();
    assert!(removal.complete());
}

fn inputs(now: &str, delay_days: u32) -> EligibilityInputs {
    EligibilityInputs {
        now: time(now),
        delay_days,
        claimed: false,
        in_use: false,
    }
}

fn check(note: &Note, inputs: &EligibilityInputs) -> TranscriptEligibility {
    transcript_eligibility(&note.store.lock().unwrap(), ID, inputs).unwrap()
}

// ---------------------------------------------------------------- the step

#[test]
fn ae1_audio_goes_at_30_days_and_the_transcript_at_90_days() {
    // Covers AE1, R6, R7. The note stopped on 1 June. Audio removal is at
    // 30 days and transcript removal at 90 days.
    let note = saved_note(&two_segments());
    let before = kept(&note.home);
    let august = inputs("2026-08-01T12:00:00-05:00", 30);
    assert_eq!(
        eligibility(&note.store.lock().unwrap(), ID, &august).unwrap(),
        Eligibility::Eligible
    );
    let august = inputs("2026-08-01T12:00:00-05:00", 90);
    assert_eq!(
        check(&note, &august),
        TranscriptEligibility::NotYet {
            due: time("2026-08-30T09:02:00-05:00")
        }
    );
    remove_all_audio(&note);
    // On 1 August the note has its transcript and no audio.
    assert!(present(&note.home.join("transcript.md")));

    let september = inputs("2026-09-01T12:00:00-05:00", 90);
    assert_eq!(check(&note, &september), TranscriptEligibility::Eligible);
    let removal = run(&note);

    // On 1 September it has only its summary, notes, title and tags.
    assert!(removal.removed);
    assert!(removal.row_changed);
    assert_removed(&note);
    assert_eq!(kept(&note.home), before);
    assert_eq!(
        check(&note, &september),
        TranscriptEligibility::NoTranscript
    );
}

#[test]
fn ae6_a_note_with_audio_loses_its_audio_and_transcript_in_one_call() {
    // Covers AE6, R17, R18.
    let segments = two_segments();
    let note = saved_note(&segments);
    let before = kept(&note.home);

    let removal = run(&note);

    let outcomes: Vec<SegmentOutcome> = removal.audio.iter().map(|s| s.outcome.clone()).collect();
    assert_eq!(outcomes, vec![SegmentOutcome::Removed; 2]);
    for segment in &segments {
        assert!(!present(&note.home.join(&segment.audio_file)));
        assert!(present(
            &note.home.join(removal_marker(&segment.audio_file))
        ));
    }
    assert_eq!(
        note.store.lock().unwrap().removed_audio(ID).unwrap(),
        vec![audio(0), audio(1)]
    );
    assert_removed(&note);
    assert_eq!(kept(&note.home), before);
}

#[test]
fn a_hand_edited_transcript_goes() {
    // R9.
    let note = saved_note(&two_segments());
    fs::write(
        note.home.join("transcript.md"),
        "# Transcript\n\nEdited by hand.\n",
    )
    .unwrap();

    run(&note);

    assert_removed(&note);
}

#[test]
fn a_second_run_changes_nothing() {
    let note = saved_note(&two_segments());
    run(&note);
    let folder = contents(&note.home);

    let removal = run(&note);

    assert_eq!(contents(&note.home), folder);
    assert!(removal.removed);
    assert!(!removal.row_changed);
    assert!(removal.files.deleted.is_empty());
    assert!(!finish_due(&note.home, true));
}

#[test]
fn the_marker_comes_before_the_first_delete_of_a_transcript_file() {
    // KTD3: a delete that fails leaves the marker, so the transcript
    // counts as removed and the next run deletes it.
    let note = saved_note(&two_segments());

    let removal = remove_transcript_with(&note.store, &note.claims, ID, no_guard(), |path| {
        if path.ends_with("transcript.md") {
            assert!(
                present(&path.with_file_name(TRANSCRIPT_REMOVED_MARKER)),
                "the marker is missing at the delete"
            );
            Err(std::io::Error::other("injected"))
        } else {
            fs::remove_file(path)
        }
    })
    .unwrap();

    assert!(removal.removed);
    assert_eq!(removal.files.kept, vec![note.home.join("transcript.md")]);
    assert!(present(&note.home.join(TRANSCRIPT_REMOVED_MARKER)));
    assert!(present(&note.home.join("transcript.md")));
    assert!(finish_due(&note.home, true));

    finish_transcript_removal(&note.store, &note.claims, ID, no_guard()).unwrap();

    assert_removed(&note);
    assert!(!finish_due(&note.home, true));
}

#[test]
fn a_crash_after_the_marker_is_finished_by_the_next_run() {
    // KTD3: another computer wrote the marker and stopped. The next run
    // deletes the files and writes the record.
    let note = saved_note(&two_segments());
    remove_all_audio(&note);
    fs::write(
        note.home.join(TRANSCRIPT_REMOVED_MARKER),
        TRANSCRIPT_REMOVED_TEXT,
    )
    .unwrap();
    assert!(finish_due(&note.home, true));

    let removal = finish_transcript_removal(&note.store, &note.claims, ID, no_guard()).unwrap();

    assert!(removal.removed);
    assert!(removal.audio.is_empty());
    assert_removed(&note);
    assert!(!finish_due(&note.home, true));
}

#[test]
fn a_crash_before_the_record_write_is_finished_by_the_next_run() {
    // KTD3: the files are gone, and `longhand.json` and the store still
    // have the transcript data.
    let note = saved_note(&two_segments());
    remove_all_audio(&note);
    fs::write(
        note.home.join(TRANSCRIPT_REMOVED_MARKER),
        TRANSCRIPT_REMOVED_TEXT,
    )
    .unwrap();
    fs::remove_file(note.home.join("transcript.md")).unwrap();
    fs::remove_file(note.home.join(ARTIFACT_FILE)).ok();
    // The record fields alone make the note due, also with the flag set.
    assert!(finish_due(&note.home, true));
    assert!(finish_due(&note.home, false));

    finish_transcript_removal(&note.store, &note.claims, ID, no_guard()).unwrap();

    assert_removed(&note);
    assert!(!finish_due(&note.home, true));
    // The store flag of this computer alone makes the note due.
    assert!(finish_due(&note.home, false));
}

#[test]
fn finish_keeps_the_transcript_of_a_note_with_no_marker() {
    let note = saved_note(&two_segments());
    let before = contents(&note.home);
    assert!(!finish_due(&note.home, false));

    let removal = finish_transcript_removal(&note.store, &note.claims, ID, no_guard()).unwrap();

    assert!(!removal.removed);
    assert_eq!(contents(&note.home), before);
    assert!(!note.store.lock().unwrap().transcript_removed(ID).unwrap());
}

#[test]
fn an_unsafe_audio_name_keeps_the_transcript_and_writes_no_marker() {
    // KTD3 (2): a segment that keeps its audio stops the step.
    let mut segments = two_segments();
    segments[1].audio_file = "transcript.md".into();
    let note = saved_note(&segments);

    let result = remove_transcript(&note.store, &note.claims, ID, no_guard());

    let Err(RemoveTranscriptError::AudioLeft(audio)) = result else {
        panic!("expected AudioLeft, got {result:?}");
    };
    assert_eq!(audio[1].outcome, SegmentOutcome::UnsafeName);
    assert!(!present(&note.home.join(TRANSCRIPT_REMOVED_MARKER)));
    assert_eq!(
        fs::read_to_string(note.home.join("transcript.md")).unwrap(),
        TRANSCRIPT
    );
    assert!(present(&note.home.join(ARTIFACT_FILE)));
    assert!(!note.store.lock().unwrap().transcript_removed(ID).unwrap());
}

#[test]
fn a_failed_audio_marker_write_keeps_the_transcript_and_writes_no_marker() {
    // The audio folder is gone, so no audio marker can be written.
    let note = saved_note(&two_segments());
    fs::remove_dir_all(note.home.join("audio")).unwrap();

    let result = remove_transcript(&note.store, &note.claims, ID, no_guard());

    let Err(RemoveTranscriptError::AudioLeft(audio)) = result else {
        panic!("expected AudioLeft, got {result:?}");
    };
    assert!(
        audio
            .iter()
            .all(|s| matches!(s.outcome, SegmentOutcome::Failed(_)))
    );
    assert!(!present(&note.home.join(TRANSCRIPT_REMOVED_MARKER)));
    assert!(present(&note.home.join("transcript.md")));
}

#[test]
fn the_conflict_copies_of_the_transcript_and_recognition_go() {
    // KTD4.
    let note = saved_note(&two_segments());
    let copies = [
        "transcript (1).md",
        "transcript-LAPTOP.md",
        "recognition (Erv's conflicted copy 2026-06-02).jsonl",
        "recognition 2.jsonl",
    ];
    for name in copies {
        fs::write(note.home.join(name), "copy").unwrap();
    }
    // Names that are not copies of these two files stay.
    let others = ["transcript (1).txt", "recognition.json", "transcripts.md"];
    for name in others {
        fs::write(note.home.join(name), "other").unwrap();
    }

    let removal = run(&note);

    for name in copies {
        assert!(!present(&note.home.join(name)), "{name}");
        assert!(
            removal.files.deleted.contains(&note.home.join(name)),
            "{name}"
        );
    }
    for name in others {
        assert!(present(&note.home.join(name)), "{name}");
    }
    assert_removed(&note);
}

#[test]
fn a_name_with_another_case_a_link_and_a_folder_stay() {
    // KTD4: only regular files with the exact names go.
    let note = saved_note(&two_segments());
    fs::remove_file(note.home.join("transcript.md")).unwrap();
    fs::write(note.home.join("Transcript.MD"), "upper").unwrap();
    fs::create_dir(
        note.home
            .join(ARTIFACT_FILE.replace(".jsonl", " (1).jsonl")),
    )
    .unwrap();
    fs::remove_file(note.home.join(ARTIFACT_FILE)).unwrap();
    let outside = note.root.join("outside.md");
    fs::write(&outside, "outside").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, note.home.join(ARTIFACT_FILE)).unwrap();

    let removal = run(&note);

    assert!(removal.removed);
    assert_eq!(
        fs::read_to_string(note.home.join("Transcript.MD")).unwrap(),
        "upper"
    );
    assert!(note.home.join("recognition (1).jsonl").is_dir());
    assert_eq!(fs::read_to_string(&outside).unwrap(), "outside");
    #[cfg(unix)]
    assert!(
        note.home
            .join(ARTIFACT_FILE)
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn the_old_result_copies_go() {
    // KTD4: the `.longhand-results` copies hold transcript text.
    let note = saved_note(&two_segments());
    let child = note.home.join(RESULTS_FOLDER).join("committed-1");
    fs::create_dir_all(&child).unwrap();
    fs::write(child.join("transcript.md"), "old transcript").unwrap();
    fs::write(child.join("summary.md"), "old summary").unwrap();

    run(&note);

    assert!(!present(&note.home.join(RESULTS_FOLDER)));
    assert_removed(&note);
}

#[test]
fn a_storage_folder_that_is_not_reachable_stops_the_step() {
    let note = saved_note(&two_segments());
    remove_all_audio(&note);
    fs::remove_dir_all(note.root.join(".longhand")).unwrap();

    let result = remove_transcript(&note.store, &note.claims, ID, no_guard());

    assert!(matches!(result, Err(RemoveTranscriptError::NoFolder)));
    assert!(!present(&note.home.join(TRANSCRIPT_REMOVED_MARKER)));
    assert!(present(&note.home.join("transcript.md")));
}

// ---------------------------------------------------------------- guards

#[test]
fn a_note_that_is_not_saved_is_not_touched() {
    // R10.
    let dir = tempfile::tempdir().unwrap();
    let store = common::store();
    common::record_session(
        &store,
        dir.path(),
        ID,
        60,
        None,
        &[(16_000, vec![0.0; 16_000])],
    );
    let before = contents(dir.path());

    let result = remove_transcript(&store, &Claims::default(), ID, no_guard());

    assert!(matches!(
        result,
        Err(RemoveTranscriptError::Busy("capturing"))
    ));
    assert_eq!(contents(dir.path()), before);
}

#[test]
fn a_claimed_note_or_a_refused_guard_is_not_touched() {
    let note = saved_note(&two_segments());
    let before = contents(&note.home);
    {
        let _claim = note.claims.regenerate(ID).unwrap();
        let result = remove_transcript(&note.store, &note.claims, ID, no_guard());
        assert!(matches!(
            result,
            Err(RemoveTranscriptError::Busy("regenerating"))
        ));
    }
    // A fresh in-use marker of another run: the guard of the pass refuses.
    let in_use: Guard<FolderCheck, StoreCheck> = Guard {
        folder: |_| Err("in use"),
        store: |_, _| Ok(()),
    };
    let result = remove_transcript(&note.store, &note.claims, ID, in_use);
    assert!(matches!(result, Err(RemoveTranscriptError::Busy("in use"))));
    let result = finish_transcript_removal(
        &note.store,
        &note.claims,
        ID,
        Guard {
            folder: |_: &session_core::SessionRecord| Err("in use"),
            store: |_: &session_core::Store, _: &session_core::SessionRecord| Ok(()),
        },
    );
    assert!(matches!(result, Err(RemoveTranscriptError::Busy("in use"))));

    assert_eq!(contents(&note.home), before);
    assert!(!note.claims.busy(ID));
}

// ---------------------------------------------------------------- eligibility

#[test]
fn a_note_with_no_recording_or_no_transcript_is_not_eligible() {
    let late = inputs("2036-01-01T00:00:00+00:00", 1);
    let store = common::store();
    let home = tempfile::tempdir().unwrap();
    let mut adoption = Adoption::from_doc(
        &doc(&two_segments()),
        String::new(),
        home.path().to_string_lossy().into_owned(),
        None,
    );
    adoption.segments.clear();
    store.lock().unwrap().adopt(&adoption).unwrap();
    assert_eq!(
        transcript_eligibility(&store.lock().unwrap(), ID, &late).unwrap(),
        TranscriptEligibility::NoRecording
    );

    let note = saved_note(&two_segments());
    note.store
        .lock()
        .unwrap()
        .set_transcript_removed(ID, true)
        .unwrap();
    assert_eq!(check(&note, &late), TranscriptEligibility::NoTranscript);
}

#[test]
fn a_note_that_is_not_saved_claimed_or_in_use_is_not_eligible() {
    let dir = tempfile::tempdir().unwrap();
    let store = common::store();
    common::queue_session(
        &store,
        dir.path(),
        ID,
        60,
        None,
        &[(16_000, vec![0.0; 16_000])],
    );
    let late = inputs("2036-01-01T00:00:00+00:00", 1);
    assert_eq!(
        transcript_eligibility(&store.lock().unwrap(), ID, &late).unwrap(),
        TranscriptEligibility::NotSaved
    );

    let note = saved_note(&two_segments());
    let mut claimed = late.clone();
    claimed.claimed = true;
    assert_eq!(check(&note, &claimed), TranscriptEligibility::Claimed);
    let mut in_use = late;
    in_use.in_use = true;
    assert_eq!(check(&note, &in_use), TranscriptEligibility::InUse);
}

#[test]
fn the_eligibility_check_opens_no_file() {
    // The check uses the store only. A home folder that is gone does not
    // change it.
    let note = saved_note(&two_segments());
    fs::remove_dir_all(&note.home).unwrap();
    assert_eq!(
        check(&note, &inputs("2026-09-01T12:00:00-05:00", 90)),
        TranscriptEligibility::Eligible
    );
}

#[test]
fn ae14_a_note_whose_summary_failed_is_not_eligible_but_remove_transcript_works() {
    // Covers AE14, R28. A 100-day-old note, transcripts at 90 days.
    let note = saved_note(&two_segments());
    pipeline::save::record_summary_failure(&note.store, ID, "the endpoint refused").unwrap();
    set_summary(&note.home, SummaryMeta::stub());
    let now = inputs("2026-09-09T12:00:00-05:00", 90);

    assert_eq!(check(&note, &now), TranscriptEligibility::SummaryFailed);
    // A note that another computer saved has no failed-summary row here:
    // its `longhand.json` records the stub.
    assert!(summary_stub_in(&note.home));

    // R17: Remove Transcript on the note still removes it.
    let removal = run(&note);
    assert!(removal.removed);
    assert!(!present(&note.home.join("transcript.md")));
    assert!(note.store.lock().unwrap().transcript_removed(ID).unwrap());
}

#[test]
fn a_complete_summary_is_not_a_stub() {
    let note = saved_note(&two_segments());
    assert!(!summary_stub_in(&note.home));
}
