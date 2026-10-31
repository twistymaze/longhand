//! The removal pass (U6 of the audio removal plan): each computer that uses
//! the storage folder removes the audio of old saved notes (R5, R9, R11).
//!
//! The same pass also removes old transcripts (KTD6 of the transcript
//! removal plan).
//!
//! One pass does three things, in this order (KTD2, KTD5):
//!
//! 1. It finishes the delete of each segment that has its removal marker
//!    and still has its audio file, whether the setting is on or off. It
//!    finishes the transcript removal of each note that has the transcript
//!    marker and needs it ([`pipeline::remove_transcript::finish_due`]),
//!    whether the transcript setting is on or off. It writes no marker in
//!    this part. Then it deletes the old result copies of the note and
//!    `recognition.jsonl` of a note with no audio left, as
//!    [`pipeline::remove_copies`] describes (R13, KTD9 of the result copies
//!    plan). It skips a note that waits for a sync.
//! 2. While the setting is known and on (KTD3), it removes the audio of each
//!    saved note whose last recording end plus the delay is in the past.
//! 3. While the effective transcript delay is known and on (KTD7 of the
//!    transcript removal plan), it removes the transcript of each saved note
//!    whose last recording end plus that delay is in the past. It skips a
//!    note whose summary is the stub of a failed summary (R28).
//!
//! Before each note, the pass checks every guard again: the note is Saved
//! and has no claim, no Move is open, the storage folder and the note's
//! folder are available, no in-use marker of the note is fresh, the setting
//! is still known and on, and the delay has passed. The removal step checks
//! them once more after it takes its claim: the folder checks with no store
//! lock, then the store checks under the store lock. The pass
//! stops when the storage folder is not available or a Move is open, and it
//! stops writing new markers when the setting stops being known and on.
//!
//! The pass reads the store and file metadata only, and opens no audio file
//! (KTD11). It takes no capture lock.
//!
//! The pass runs at launch after the library scan, only in the single
//! running instance, then every [`PASS_EVERY`], after a confirmed change
//! that shortens the delay or turns the setting on (R17), and after the
//! shared settings change on disk (KTD6). One worker runs the passes one at
//! a time, on a blocking thread, so the UI never waits for a pass.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{DateTime, FixedOffset};
use pipeline::SharedStore;
use pipeline::remove_audio::{
    Eligibility, EligibilityInputs, Guard, NoteSync, Removal, RemoveAudioError, StoreCheck,
    eligibility, finish_removal, remove_audio, safe_audio_name,
};
use pipeline::remove_copies::{cleanup_due, remove_copies};
use pipeline::remove_transcript::{
    RemoveTranscriptError, TranscriptEligibility, TranscriptRemoval, finish_due,
    finish_transcript_removal, remove_transcript, summary_stub_in, transcript_eligibility,
};
use session_core::jobs::Claims;
use session_core::{SessionRecord, SessionState, Store, folder};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::commands::session::{removal_folder_checks, removal_store_checks};
use crate::managers::library::LibraryChanged;
use crate::managers::settings;
use crate::managers::shared::SharedSettingsManager;

/// The time between two timer passes (KTD6).
pub const PASS_EVERY: Duration = Duration::from_secs(15 * 60);

/// The reason that the guard gives when the storage folder or the note's
/// folder is not available.
const UNAVAILABLE: &str = "folder unavailable";

/// The reason that the guard gives when the setting is not known and on.
const SETTING_OFF: &str = "setting off";

/// The reason that the guard gives when the note is not eligible now.
const NOT_DUE: &str = "not due";

/// The reason that the guard gives when the summary of the note is the
/// stub of a failed summary (R28 of the transcript removal plan).
const SUMMARY_FAILED: &str = "summary failed";

/// What a pass reads from outside the store. The app gives the real
/// values, and a test gives its own.
pub struct PassInputs<'a> {
    pub store: &'a SharedStore,
    pub claims: &'a Claims,
    /// The storage folder, or `None` when this computer has none.
    pub storage: Option<&'a Path>,
    /// Whether the latest library scan can read the note with this id and
    /// home folder: its Destination is available and its `session.md` can
    /// be read.
    pub note_readable: &'a dyn Fn(&str, &Path) -> bool,
    /// Whether the note with this id waits for a sync ([`note_sync`]).
    /// It controls only the delete of `recognition.jsonl` (KTD8).
    pub note_sync: &'a dyn Fn(&str) -> NoteSync,
    /// The delay of the setting in whole days, or `None` when the setting
    /// is off, is not known, or the shared settings are not available
    /// (KTD3). The pass reads it again before each note.
    pub delay_days: &'a dyn Fn() -> Option<u32>,
    /// The effective delay of transcript removal in whole days, or `None`
    /// when no transcript goes automatically (KTD7 of the transcript
    /// removal plan). The pass reads it again before each note.
    pub transcript_delay_days: &'a dyn Fn() -> Option<u32>,
    /// The time now.
    pub now: &'a dyn Fn() -> DateTime<FixedOffset>,
    /// The row of the note with this id changed. The app sends a row change
    /// at once, so an open note page updates.
    pub row_changed: &'a dyn Fn(&str),
}

/// Why a pass ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PassEnd {
    /// The pass checked each note.
    #[default]
    Done,
    /// The storage folder is not available. The pass deleted nothing after
    /// the check that found this.
    StorageUnavailable,
    /// A Move runs or is paused. It changes the home folders of the notes.
    MoveOpen,
    /// The setting is off or not known. The pass writes no new marker.
    SettingOff,
}

/// The result of one pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassReport {
    pub end: PassEnd,
    /// The notes whose marked audio the pass deleted (part 1).
    pub finished: Vec<String>,
    /// The notes whose audio the pass removed (part 2).
    pub removed: Vec<String>,
    /// The notes whose old result copies the pass deleted (part 1).
    pub cleaned: Vec<String>,
    /// The notes whose marked transcript removal the pass finished (part
    /// 1).
    pub transcripts_finished: Vec<String>,
    /// The notes whose transcript the pass removed (part 3).
    pub transcripts_removed: Vec<String>,
    /// The notes whose row changed.
    pub changed: Vec<String>,
}

fn lock(store: &SharedStore) -> std::sync::MutexGuard<'_, Store> {
    store.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether `root` is a folder that has its library marker. A removed disk,
/// a lost share and a signed-out sync client fail this check.
fn root_reachable(root: &Path) -> bool {
    root.is_dir() && library::scan::marker(root).exists()
}

/// Whether the pass must stop now, and why.
fn stop_reason(inputs: &PassInputs) -> Option<PassEnd> {
    if !inputs.storage.is_some_and(root_reachable) {
        return Some(PassEnd::StorageUnavailable);
    }
    let store = lock(inputs.store);
    if !matches!(store.move_journal(), Ok(None)) {
        return Some(PassEnd::MoveOpen);
    }
    None
}

/// Whether the folder of the note is available now: the latest scan can
/// read it, and its Destination is a folder with its library marker. The
/// Destination is the parent folder, or the parent of a type folder (KTD4
/// of the note types plan).
fn note_available(inputs: &PassInputs, id: &str, home: &Path) -> bool {
    (inputs.note_readable)(id, home)
        && library::scan::storage_place(home).is_some_and(|place| root_reachable(&place.root))
}

/// The saved notes that have a home folder, sorted by id.
fn saved_notes(store: &SharedStore) -> Result<Vec<(String, PathBuf)>, String> {
    let rows = lock(store)
        .sessions_in(SessionState::Saved.name())
        .map_err(|e| e.to_string())?;
    let mut notes: Vec<(String, PathBuf)> = rows
        .into_iter()
        .filter_map(|r| Some((r.id, PathBuf::from(r.home_path?))))
        .collect();
    notes.sort();
    Ok(notes)
}

/// Whether the note `id` waits for a sync: the latest scan found that its
/// `transcript.md` or `summary.md` does not match `session.md` (R7). The
/// removal pass and Remove Audio use it for the delete of
/// `recognition.jsonl` only (KTD8).
pub(crate) fn note_sync(status: &library::status::LibraryStatus, id: &str) -> NoteSync {
    if status.note(id).result_waits() {
        NoteSync::Waiting
    } else {
        NoteSync::Synced
    }
}

/// Whether a segment of the note has its removal marker and still has its
/// audio file. This reads metadata only.
fn marked_audio_present(store: &SharedStore, id: &str, home: &Path) -> bool {
    let segments = match lock(store).segments(id) {
        Ok(segments) => segments,
        Err(e) => {
            log::warn!("audio removal: cannot read the segments of {id}: {e}");
            return false;
        }
    };
    segments.iter().any(|s| {
        safe_audio_name(&s.audio_file)
            && home.join(&s.audio_file).symlink_metadata().is_ok()
            && folder::audio_removed(home, &s.audio_file)
    })
}

/// The folder checks that every removal in a pass does after the claim,
/// with no store lock: that of Remove Audio (no fresh in-use marker), and
/// the storage folder and the note's folder are available.
fn pass_folder_guard(inputs: &PassInputs, record: &SessionRecord) -> Result<(), &'static str> {
    removal_folder_checks(record, (inputs.now)())?;
    let available = inputs.storage.is_some_and(root_reachable)
        && record
            .home_path
            .as_deref()
            .is_some_and(|home| note_available(inputs, &record.id, Path::new(home)));
    if !available {
        return Err(UNAVAILABLE);
    }
    Ok(())
}

/// The guard of each step in part 1 of a pass: the folder checks of the
/// pass and the store checks of Remove Audio.
fn part_one_guard<'a>(
    inputs: &'a PassInputs,
) -> Guard<impl FnOnce(&SessionRecord) -> Result<(), &'static str> + 'a, StoreCheck> {
    Guard {
        folder: move |record: &SessionRecord| pass_folder_guard(inputs, record),
        store: |store: &Store, _: &SessionRecord| removal_store_checks(store),
    }
}

/// The store checks of a new removal in a pass, under the store lock: those
/// of Remove Audio (no open Move), the setting is still known and on, and
/// the note is eligible with the row that the step just brought up to date.
fn new_removal_store_checks(
    inputs: &PassInputs,
    store: &Store,
    record: &SessionRecord,
) -> Result<(), &'static str> {
    removal_store_checks(store)?;
    let Some(delay_days) = (inputs.delay_days)() else {
        return Err(SETTING_OFF);
    };
    // The step holds its own claim, and `pass_folder_guard` checks the
    // in-use markers.
    let check = EligibilityInputs {
        now: (inputs.now)(),
        delay_days,
        claimed: false,
        in_use: false,
    };
    match eligibility(store, &record.id, &check) {
        Ok(Eligibility::Eligible) => Ok(()),
        _ => Err(NOT_DUE),
    }
}

/// Whether a fresh in-use marker is in the home folder `home` at `now`. A
/// folder that cannot be read counts as in use. `None` when the folder is
/// missing: the note is skipped.
fn home_in_use(home: &Path, now: DateTime<FixedOffset>) -> Option<bool> {
    match pipeline::in_use::in_use(home, now) {
        Ok(in_use) => Some(in_use),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(_) => Some(true),
    }
}

/// Whether the removal of the audio of the note can start now with the
/// delay `delay_days`. The pass and [`count_eligible`] share this check. It
/// reads the store, the claims and file metadata only (KTD11).
fn eligible_now(inputs: &PassInputs, id: &str, home: &Path, delay_days: u32) -> bool {
    if !note_available(inputs, id, home) {
        return false;
    }
    let now = (inputs.now)();
    let Some(in_use) = home_in_use(home, now) else {
        return false;
    };
    let check = EligibilityInputs {
        now,
        delay_days,
        claimed: inputs.claims.busy(id),
        in_use,
    };
    let store = lock(inputs.store);
    matches!(eligibility(&store, id, &check), Ok(Eligibility::Eligible))
}

/// Log the result of the step for one note, and report its row change.
fn record_step(
    inputs: &PassInputs,
    id: &str,
    result: Result<Removal, RemoveAudioError>,
    done: &mut Vec<String>,
    changed: &mut Vec<String>,
) {
    match result {
        Ok(removal) => {
            if removal.row_changed {
                changed.push(id.to_string());
                (inputs.row_changed)(id);
            }
            if !removal.segments.is_empty() {
                done.push(id.to_string());
            }
            for segment in removal.segments.iter().filter(|s| {
                !matches!(
                    s.outcome,
                    pipeline::remove_audio::SegmentOutcome::Removed
                        | pipeline::remove_audio::SegmentOutcome::AlreadyRemoved
                )
            }) {
                log::warn!(
                    "audio removal: the audio {} of {id} was not removed: {:?}",
                    segment.audio_file,
                    segment.outcome
                );
            }
        }
        Err(RemoveAudioError::Busy(reason)) => {
            log::debug!("audio removal: skipped {id}: {reason}");
        }
        Err(RemoveAudioError::NoFolder) => {
            log::debug!("audio removal: skipped {id}: its folder is not available");
        }
        Err(RemoveAudioError::Failed(message)) => {
            log::warn!("audio removal: the removal of the audio of {id} failed: {message}");
        }
    }
}

/// Delete the old result copies of the note `id` (R13, KTD9 of the result
/// copies plan). The cleanup uses the guards of part 1, and it skips a note
/// that waits for a sync. A later pass tries again.
fn clean_note(inputs: &PassInputs, id: &str, home: &Path, cleaned: &mut Vec<String>) {
    let segments = match lock(inputs.store).segments(id) {
        Ok(segments) => segments,
        Err(e) => {
            log::warn!("audio removal: cannot read the segments of {id}: {e}");
            return;
        }
    };
    if !cleanup_due(home, &segments) {
        return;
    }
    if (inputs.note_sync)(id) == NoteSync::Waiting {
        log::debug!("audio removal: {id} waits for a sync, so its old result copies stay for now");
        return;
    }
    let guard = part_one_guard(inputs);
    match remove_copies(inputs.store, inputs.claims, id, guard) {
        Ok(cleanup) => {
            if !cleanup.deleted.is_empty() {
                log::info!("audio removal: deleted the old result copies of {id}");
                cleaned.push(id.to_string());
            }
        }
        Err(RemoveAudioError::Busy(reason)) => {
            log::debug!("audio removal: the cleanup skipped {id}: {reason}");
        }
        Err(RemoveAudioError::NoFolder) => {
            log::debug!("audio removal: the cleanup skipped {id}: its folder is not available");
        }
        Err(RemoveAudioError::Failed(message)) => {
            log::warn!("audio removal: the cleanup of {id} failed: {message}");
        }
    }
}

// ------------------------------------------------------- transcript removal

/// What the transcript part of a pass does with one note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranscriptCheck {
    /// The removal of the transcript can start now.
    Remove,
    /// The delay has passed, but the summary of the note is the stub of a
    /// failed summary, so the transcript stays (R28).
    SummaryFailed,
    /// The transcript stays for another reason.
    Keep,
}

/// How many notes a pass with a transcript delay would act on now (R19,
/// R28 of the transcript removal plan).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct TranscriptCount {
    /// The notes whose transcript the pass would remove.
    pub remove: u32,
    /// The notes whose delay has passed and whose transcript the pass
    /// keeps, because their summary failed (R28).
    pub skipped: u32,
}

/// Whether the last recording end of the note `id` plus `delay_days` is
/// at or before `now`.
fn delay_passed(
    store: &SharedStore,
    id: &str,
    delay_days: u32,
    now: DateTime<FixedOffset>,
) -> bool {
    let segments = match lock(store).segments(id) {
        Ok(segments) => segments,
        Err(e) => {
            log::warn!("transcript removal: cannot read the segments of {id}: {e}");
            return false;
        }
    };
    segments
        .iter()
        .map(|s| s.wall_end)
        .max()
        .is_some_and(|end| end + chrono::TimeDelta::days(i64::from(delay_days)) <= now)
}

/// What the transcript part of a pass with the effective delay
/// `delay_days` does with the note now. The pass and [`count_transcripts`]
/// share this check. It reads the store, the claims and file metadata, and
/// `longhand.json` of a note that is otherwise eligible (R28).
fn transcript_check(
    inputs: &PassInputs,
    id: &str,
    home: &Path,
    delay_days: u32,
) -> TranscriptCheck {
    if !note_available(inputs, id, home) {
        return TranscriptCheck::Keep;
    }
    let now = (inputs.now)();
    let Some(in_use) = home_in_use(home, now) else {
        return TranscriptCheck::Keep;
    };
    let check = EligibilityInputs {
        now,
        delay_days,
        claimed: inputs.claims.busy(id),
        in_use,
    };
    let eligibility = transcript_eligibility(&lock(inputs.store), id, &check);
    match eligibility {
        // A note that another computer saved has no failed-summary row
        // here. Its `longhand.json` records the stub.
        Ok(TranscriptEligibility::Eligible) if summary_stub_in(home) => {
            TranscriptCheck::SummaryFailed
        }
        Ok(TranscriptEligibility::Eligible) => TranscriptCheck::Remove,
        Ok(TranscriptEligibility::SummaryFailed)
            if delay_passed(inputs.store, id, delay_days, now) =>
        {
            TranscriptCheck::SummaryFailed
        }
        _ => TranscriptCheck::Keep,
    }
}

/// The folder checks of a new transcript removal in a pass, with no store
/// lock: those of [`pass_folder_guard`], and the summary of the note is not
/// the stub of a failed summary (R28).
fn transcript_folder_guard(
    inputs: &PassInputs,
    record: &SessionRecord,
) -> Result<(), &'static str> {
    pass_folder_guard(inputs, record)?;
    if record
        .home_path
        .as_deref()
        .is_some_and(|home| summary_stub_in(Path::new(home)))
    {
        return Err(SUMMARY_FAILED);
    }
    Ok(())
}

/// The store checks of a new transcript removal in a pass, under the store
/// lock: those of Remove Audio (no open Move), the effective delay is still
/// known and on, and the note is eligible with the row that the step just
/// brought up to date.
fn new_transcript_store_checks(
    inputs: &PassInputs,
    store: &Store,
    record: &SessionRecord,
) -> Result<(), &'static str> {
    removal_store_checks(store)?;
    let Some(delay_days) = (inputs.transcript_delay_days)() else {
        return Err(SETTING_OFF);
    };
    // The step holds its own claim, and `pass_folder_guard` checks the
    // in-use markers.
    let check = EligibilityInputs {
        now: (inputs.now)(),
        delay_days,
        claimed: false,
        in_use: false,
    };
    match transcript_eligibility(store, &record.id, &check) {
        Ok(TranscriptEligibility::Eligible) => Ok(()),
        Ok(TranscriptEligibility::SummaryFailed) => Err(SUMMARY_FAILED),
        _ => Err(NOT_DUE),
    }
}

/// Log the result of the transcript step for one note, and report its row
/// change.
fn record_transcript_step(
    inputs: &PassInputs,
    id: &str,
    result: Result<TranscriptRemoval, RemoveTranscriptError>,
    done: &mut Vec<String>,
    changed: &mut Vec<String>,
) {
    match result {
        Ok(removal) => {
            if removal.row_changed {
                changed.push(id.to_string());
                (inputs.row_changed)(id);
            }
            if removal.removed
                && (removal.row_changed
                    || !removal.audio.is_empty()
                    || !removal.files.deleted.is_empty())
            {
                done.push(id.to_string());
            }
            for path in &removal.files.kept {
                log::warn!(
                    "transcript removal: {} of {id} is not a regular file, so it stays",
                    path.display()
                );
            }
        }
        Err(RemoveTranscriptError::Busy(reason)) => {
            log::debug!("transcript removal: skipped {id}: {reason}");
        }
        Err(RemoveTranscriptError::NoFolder) => {
            log::debug!("transcript removal: skipped {id}: its folder is not available");
        }
        Err(RemoveTranscriptError::AudioLeft(segments)) => {
            for segment in &segments {
                log::warn!(
                    "transcript removal: the audio {} of {id} was not removed, so the transcript stays: {:?}",
                    segment.audio_file,
                    segment.outcome
                );
            }
        }
        Err(RemoveTranscriptError::Failed(message)) => {
            log::warn!(
                "transcript removal: the removal of the transcript of {id} failed: {message}"
            );
        }
    }
}

/// Whether one of the notes `ids` has the transcript marker and needs
/// [`finish_transcript_removal`] (KTD3, KTD6 of the transcript removal
/// plan). A scan that changed such a note wakes the pass, so a
/// `transcript.md` that a late write put back does not wait for the timer.
/// It reads the store and file metadata, and `longhand.json` of a note with
/// the marker.
pub fn needs_finish(store: &SharedStore, ids: &[String]) -> bool {
    ids.iter().any(|id| {
        let (home, flag) = {
            let store = lock(store);
            let Ok(Some(record)) = store.session(id) else {
                return false;
            };
            if record.state != SessionState::Saved {
                return false;
            }
            let Some(home) = record.home_path else {
                return false;
            };
            (
                PathBuf::from(home),
                store.transcript_removed(id).unwrap_or(false),
            )
        };
        finish_due(&home, flag)
    })
}

/// One removal pass (KTD5). See the module documentation.
pub fn run_pass(inputs: &PassInputs) -> PassReport {
    let mut report = PassReport::default();
    if let Some(end) = stop_reason(inputs) {
        report.end = end;
        return report;
    }
    let notes = match saved_notes(inputs.store) {
        Ok(notes) => notes,
        Err(e) => {
            log::error!("audio removal: cannot read the notes: {e}");
            return report;
        }
    };

    // Part 1: a marker makes the audio removed, whether the setting is on
    // or off. Delete each marked audio file that is still there (KTD2).
    // Then delete the old result copies of the note (KTD9 of the result
    // copies plan).
    for (id, home) in &notes {
        if let Some(end) = stop_reason(inputs) {
            report.end = end;
            return report;
        }
        if !note_available(inputs, id, home) || inputs.claims.busy(id) {
            continue;
        }
        if marked_audio_present(inputs.store, id, home) {
            let guard = part_one_guard(inputs);
            let result = finish_removal(
                inputs.store,
                inputs.claims,
                id,
                guard,
                (inputs.note_sync)(id),
            );
            record_step(
                inputs,
                id,
                result,
                &mut report.finished,
                &mut report.changed,
            );
        }
        // The transcript marker makes the transcript removed, whether the
        // transcript setting is on or off (KTD3 of the transcript removal
        // plan).
        let flag = lock(inputs.store).transcript_removed(id).unwrap_or(false);
        if finish_due(home, flag) {
            let result =
                finish_transcript_removal(inputs.store, inputs.claims, id, part_one_guard(inputs));
            record_transcript_step(
                inputs,
                id,
                result,
                &mut report.transcripts_finished,
                &mut report.changed,
            );
        }
        clean_note(inputs, id, home, &mut report.cleaned);
    }

    // Part 2: new markers, only while the setting is known and on (KTD3).
    for (id, home) in &notes {
        if let Some(end) = stop_reason(inputs) {
            report.end = end;
            return report;
        }
        let Some(delay_days) = (inputs.delay_days)() else {
            report.end = PassEnd::SettingOff;
            return report;
        };
        if !eligible_now(inputs, id, home, delay_days) {
            continue;
        }
        let guard = Guard {
            folder: |record: &SessionRecord| pass_folder_guard(inputs, record),
            store: |store: &Store, record: &SessionRecord| {
                new_removal_store_checks(inputs, store, record)
            },
        };
        let result = remove_audio(
            inputs.store,
            inputs.claims,
            id,
            guard,
            (inputs.note_sync)(id),
        );
        record_step(inputs, id, result, &mut report.removed, &mut report.changed);
    }

    // Part 3: new transcript markers, only while the effective transcript
    // delay is known and on (KTD6, KTD7 of the transcript removal plan).
    // The step removes any audio that is left first. The setting is off by
    // default (R2), so a pass that starts with no delay ends as done.
    if (inputs.transcript_delay_days)().is_none() {
        return report;
    }
    for (id, home) in &notes {
        if let Some(end) = stop_reason(inputs) {
            report.end = end;
            return report;
        }
        let Some(delay_days) = (inputs.transcript_delay_days)() else {
            report.end = PassEnd::SettingOff;
            return report;
        };
        if transcript_check(inputs, id, home, delay_days) != TranscriptCheck::Remove {
            continue;
        }
        let guard = Guard {
            folder: |record: &SessionRecord| transcript_folder_guard(inputs, record),
            store: |store: &Store, record: &SessionRecord| {
                new_transcript_store_checks(inputs, store, record)
            },
        };
        let result = remove_transcript(inputs.store, inputs.claims, id, guard);
        record_transcript_step(
            inputs,
            id,
            result,
            &mut report.transcripts_removed,
            &mut report.changed,
        );
    }
    report
}

/// How many notes a pass with the effective transcript delay `delay_days`
/// would remove the transcript of now, and how many it would keep because
/// their summary failed (R19, R28 of the transcript removal plan). This
/// uses the same per-note check as the pass. It is zero while the storage
/// folder is not available or a Move is open. The pass checks each note
/// again when it runs.
pub fn count_transcripts(inputs: &PassInputs, delay_days: u32) -> TranscriptCount {
    let mut count = TranscriptCount::default();
    if stop_reason(inputs).is_some() {
        return count;
    }
    let notes = match saved_notes(inputs.store) {
        Ok(notes) => notes,
        Err(e) => {
            log::error!("transcript removal: cannot read the notes: {e}");
            return count;
        }
    };
    for (id, home) in &notes {
        match transcript_check(inputs, id, home, delay_days) {
            TranscriptCheck::Remove => count.remove = count.remove.saturating_add(1),
            TranscriptCheck::SummaryFailed => count.skipped = count.skipped.saturating_add(1),
            TranscriptCheck::Keep => {}
        }
    }
    count
}

/// How many notes a pass with the delay `delay_days` would remove the audio
/// of now (R17, KTD11). This uses the same per-note check as the pass, and
/// reads the store and file metadata only. It is zero while the storage
/// folder is not available or a Move is open. The pass checks each note
/// again when it runs.
pub fn count_eligible(inputs: &PassInputs, delay_days: u32) -> usize {
    if stop_reason(inputs).is_some() {
        return 0;
    }
    let notes = match saved_notes(inputs.store) {
        Ok(notes) => notes,
        Err(e) => {
            log::error!("audio removal: cannot read the notes: {e}");
            return 0;
        }
    };
    notes
        .iter()
        .filter(|(id, home)| eligible_now(inputs, id, home, delay_days))
        .count()
}

// -------------------------------------------------------------- the manager

/// Runs the removal passes of this app (KTD6).
pub struct AudioRemovalManager {
    app: AppHandle,
    store: SharedStore,
    claims: Claims,
    status: Arc<library::status::LibraryStatus>,
    wake: Arc<tokio::sync::Notify>,
    started: AtomicBool,
    stopped: AtomicBool,
}

impl AudioRemovalManager {
    pub fn new(
        app: &AppHandle,
        store: SharedStore,
        claims: Claims,
        status: Arc<library::status::LibraryStatus>,
    ) -> Arc<Self> {
        Arc::new(Self {
            app: app.clone(),
            store,
            claims,
            status,
            wake: Arc::default(),
            started: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        })
    }

    /// Start the worker: a pass at once when `at_once` is true, then one
    /// every [`PASS_EVERY`] and at each [`AudioRemovalManager::run_now`].
    /// Call it after the launch scan of the library. A second call does
    /// nothing.
    pub fn start(self: &Arc<Self>, at_once: bool) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        if at_once {
            self.run_now();
        }
        // A scan that changed a note that needs its transcript removal
        // finished wakes the pass (KTD6 of the transcript removal plan).
        // The check reads folders, so it runs on a blocking thread.
        let woken = Arc::downgrade(self);
        LibraryChanged::listen_any(&self.app, move |event| {
            let Some(manager) = woken.upgrade() else {
                return;
            };
            let changed = event.payload.changed;
            if changed.is_empty() {
                return;
            }
            tauri::async_runtime::spawn_blocking(move || {
                if needs_finish(&manager.store, &changed) {
                    manager.run_now();
                }
            });
        });
        let manager = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let start = tokio::time::Instant::now() + PASS_EVERY;
            let mut timer = tokio::time::interval_at(start, PASS_EVERY);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = timer.tick() => {}
                    _ = manager.wake.notified() => {}
                }
                if manager.stopped.load(Ordering::SeqCst) {
                    return;
                }
                // A pass reads folders that can be on a slow volume.
                let worker = Arc::clone(&manager);
                let _ = tauri::async_runtime::spawn_blocking(move || worker.pass()).await;
            }
        });
    }

    /// Run a pass soon: after a change that shortens the delay or turns
    /// the setting on (R17), or after the shared settings change on disk.
    /// This does not wait. Two calls before the pass starts give one pass.
    pub fn run_now(&self) {
        self.wake.notify_one();
    }

    /// Start no more passes. A pass that runs completes.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.wake.notify_one();
    }

    /// How many notes a pass with the delay `delay_days` would remove the
    /// audio of now ([`count_eligible`]). This reads folders: call it off
    /// the main thread.
    pub fn count_eligible(&self, delay_days: u32) -> usize {
        self.with_inputs(|inputs| count_eligible(inputs, delay_days))
    }

    /// How many notes a pass with the effective transcript delay
    /// `delay_days` would remove the transcript of now, and how many it
    /// would skip ([`count_transcripts`]). This reads folders: call it off
    /// the main thread.
    pub fn count_transcripts(&self, delay_days: u32) -> TranscriptCount {
        self.with_inputs(|inputs| count_transcripts(inputs, delay_days))
    }

    fn pass(&self) {
        if self.stopped.load(Ordering::SeqCst) {
            return;
        }
        let report = self.with_inputs(run_pass);
        if !report.finished.is_empty() || !report.removed.is_empty() {
            log::info!(
                "audio removal: finished the removal for {} note(s) and removed the audio of {} note(s)",
                report.finished.len(),
                report.removed.len()
            );
        }
        if !report.transcripts_finished.is_empty() || !report.transcripts_removed.is_empty() {
            log::info!(
                "transcript removal: finished the removal for {} note(s) and removed the transcript of {} note(s)",
                report.transcripts_finished.len(),
                report.transcripts_removed.len()
            );
        }
        if report.end != PassEnd::Done {
            log::debug!("audio removal: the pass stopped: {:?}", report.end);
        }
    }

    fn with_inputs<T>(&self, run: impl FnOnce(&PassInputs) -> T) -> T {
        let storage = settings::get_local_settings(&self.app).storage_folder();
        let shared = self.app.try_state::<Arc<SharedSettingsManager>>();
        let delay_days = || {
            shared
                .as_ref()
                .and_then(|shared| shared.snapshot().audio_removal_days())
        };
        let transcript_delay_days = || {
            shared
                .as_ref()
                .and_then(|shared| shared.snapshot().transcript_removal_days())
        };
        let note_readable =
            |id: &str, home: &Path| !self.status.storage_unavailable(id, Some(home));
        let sync = |id: &str| note_sync(&self.status, id);
        let now = || chrono::Local::now().fixed_offset();
        let row_changed = |id: &str| {
            let changes = LibraryChanged {
                changed: vec![id.to_string()],
                removed: Vec::new(),
            };
            if let Err(e) = changes.emit(&self.app) {
                log::warn!("could not emit a library change: {e}");
            }
        };
        let inputs = PassInputs {
            store: &self.store,
            claims: &self.claims,
            storage: storage.as_deref(),
            note_readable: &note_readable,
            note_sync: &sync,
            delay_days: &delay_days,
            transcript_delay_days: &transcript_delay_days,
            now: &now,
            row_changed: &row_changed,
        };
        run(&inputs)
    }
}

/// Whether a change of the delay from `before` to `after` can remove audio
/// at once: the setting turns on, or the delay gets shorter (R17). `None`
/// is off or not known.
pub fn removes_more(before: Option<u32>, after: Option<u32>) -> bool {
    match (before, after) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(before), Some(after)) => after < before,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use chrono::TimeDelta;
    use pipeline::remove_copies::{RESULTS_FOLDER, TRANSITION_FILE};
    use session_core::folder::{REMOVAL_MARKER_TEXT, SessionDoc, fingerprint, removal_marker};
    use session_core::jobs::Adoption;
    use session_core::{Event, Segment, Stage};

    use super::*;

    const AUDIO: &str = "audio/segment-01.opus";

    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-10-01T09:00:00-05:00").unwrap()
    }

    /// A storage folder with its library marker, and a store.
    struct Library {
        _dir: tempfile::TempDir,
        root: PathBuf,
        store: SharedStore,
        claims: Claims,
    }

    fn library() -> Library {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        assert!(library::scan::write_marker(&root));
        Library {
            _dir: dir,
            root,
            store: Arc::new(std::sync::Mutex::new(Store::open_in_memory().unwrap())),
            claims: Claims::default(),
        }
    }

    impl Library {
        /// A saved note whose one segment ended `days_ago` days before
        /// [`now`], with its audio file. Returns its home folder.
        fn saved_note(&self, id: &str, name: &str, days_ago: i64) -> PathBuf {
            let home = self.root.join(name);
            std::fs::create_dir_all(home.join("audio")).unwrap();
            let end = now() - TimeDelta::days(days_ago);
            let segments = [Segment {
                index: 0,
                offset_ms: 0,
                duration_ms: 60_000,
                wall_start: end - TimeDelta::minutes(1),
                wall_end: end,
                audio_file: AUDIO.into(),
                channels: 1,
                ends_in_pause: false,
            }];
            let transcript = "# Transcript\n\n- [00:00:01] Speaker 1: Hello.\n";
            let summary = "---\nstatus: stub\nfailed_stage: summarize\n---\n\nNo summary.\n";
            let mut doc = SessionDoc::new(id, None, &segments).unwrap();
            doc.transcript_fingerprint = Some(fingerprint(transcript.as_bytes()));
            doc.summary_fingerprint = Some(fingerprint(summary.as_bytes()));
            session_save::note_files::write_doc(&home, &doc).unwrap();
            std::fs::write(home.join("transcript.md"), transcript).unwrap();
            std::fs::write(home.join("summary.md"), summary).unwrap();
            std::fs::write(home.join("notes.md"), "My notes\n").unwrap();
            std::fs::write(home.join(AUDIO), b"opus").unwrap();
            let adoption = Adoption::from_doc(
                &doc,
                "My notes\n".into(),
                home.to_string_lossy().into_owned(),
                Some(fingerprint(transcript.as_bytes())),
            );
            assert!(self.store.lock().unwrap().adopt(&adoption).unwrap());
            home
        }

        /// Move the saved note `id` through Transcribe Again to the state
        /// that `events` give after Queued.
        fn transcribe_again(&self, id: &str, events: &[Event]) {
            let mut store = self.store.lock().unwrap();
            let again = Event::UserTranscribeAgain {
                at: now(),
                audio_intact: true,
                capture_active: false,
                regenerating: false,
            };
            store.apply(id, &again).unwrap();
            for event in events {
                store.apply(id, event).unwrap();
            }
        }
    }

    /// The inputs of a pass, with a setting that `delay` gives.
    struct Env<'a> {
        library: &'a Library,
        delay: Cell<Option<u32>>,
        /// The effective delay of transcript removal (KTD7).
        transcript_delay: Cell<Option<u32>>,
        readable: Cell<bool>,
        /// The status of the library that [`Env::scan`] gives.
        status: Arc<library::status::LibraryStatus>,
        changed: RefCell<Vec<String>>,
        /// Called after each row change, to change the world during a pass.
        after_change: Box<dyn Fn(&Env)>,
    }

    impl<'a> Env<'a> {
        fn new(library: &'a Library, delay: Option<u32>) -> Self {
            Self {
                library,
                delay: Cell::new(delay),
                transcript_delay: Cell::new(None),
                readable: Cell::new(true),
                status: Arc::default(),
                changed: RefCell::default(),
                after_change: Box::new(|_| {}),
            }
        }

        fn run<T>(&self, run: impl FnOnce(&PassInputs) -> T) -> T {
            // The folder checks run with no store lock, so a slow folder
            // does not stop the other users of the store.
            let note_readable = |_: &str, _: &Path| {
                assert!(
                    self.library.store.try_lock().is_ok(),
                    "a folder check runs under the store lock"
                );
                self.readable.get()
            };
            let sync = |id: &str| note_sync(&self.status, id);
            let delay_days = || self.delay.get();
            let transcript_delay_days = || self.transcript_delay.get();
            let row_changed = |id: &str| {
                self.changed.borrow_mut().push(id.to_string());
                (self.after_change)(self);
            };
            let inputs = PassInputs {
                store: &self.library.store,
                claims: &self.library.claims,
                storage: Some(&self.library.root),
                note_readable: &note_readable,
                note_sync: &sync,
                delay_days: &delay_days,
                transcript_delay_days: &transcript_delay_days,
                now: &now,
                row_changed: &row_changed,
            };
            run(&inputs)
        }

        fn pass(&self) -> PassReport {
            self.run(run_pass)
        }

        /// Scan the storage folder into the status, as the library worker
        /// does.
        fn scan(&self) {
            library::reconcile::Reconciler::default()
                .with_status(Arc::clone(&self.status))
                .run(
                    &self.library.store,
                    &library::reconcile::Target::Full(vec![self.library.root.clone()]),
                    &|_| false,
                )
                .unwrap();
        }
    }

    fn has_recognition(home: &Path) -> bool {
        home.join(session_core::recognition::ARTIFACT_FILE)
            .symlink_metadata()
            .is_ok()
    }

    fn write_recognition(home: &Path) {
        std::fs::write(
            home.join(session_core::recognition::ARTIFACT_FILE),
            b"{\"version\":1}\n",
        )
        .unwrap();
    }

    // Covers R11: the pass deletes `recognition.jsonl` with the last audio,
    // and does not write `session.md`.
    #[test]
    fn a_pass_deletes_recognition_jsonl_with_the_last_audio() {
        let library = library();
        let home = library.saved_note("20260831-0900-aaaa", "Planning", 31);
        write_recognition(&home);
        let before = documents(&home);
        let env = Env::new(&library, Some(30));
        env.scan();

        let report = env.pass();

        assert_eq!(report.removed, vec!["20260831-0900-aaaa"]);
        assert!(!has_audio(&home));
        assert!(!has_recognition(&home));
        assert_eq!(documents(&home), before);
    }

    // KTD8: a note that waits for a sync loses its audio. Only the delete
    // of `recognition.jsonl` waits.
    #[test]
    fn a_note_that_waits_for_a_sync_loses_its_audio_and_keeps_recognition_jsonl() {
        let library = library();
        let id = "20260831-0900-aaaa";
        let home = library.saved_note(id, "Planning", 31);
        write_recognition(&home);
        // The new transcript.md arrives before its session.md.
        std::fs::write(
            home.join("transcript.md"),
            "# Transcript\n\n- [00:00:01] Speaker 1: Hello again.\n",
        )
        .unwrap();
        let env = Env::new(&library, Some(30));
        env.scan();
        assert_eq!(note_sync(&env.status, id), NoteSync::Waiting);

        let report = env.pass();

        assert_eq!(report.removed, vec![id]);
        assert!(!has_audio(&home));
        assert!(has_marker(&home));
        assert!(has_recognition(&home));
    }

    #[test]
    fn only_a_transcript_or_a_summary_that_waits_holds_the_recognition_delete() {
        let library = library();
        let id = "20260831-0900-aaaa";
        let home = library.saved_note(id, "Planning", 31);
        let env = Env::new(&library, Some(30));
        env.scan();
        assert_eq!(note_sync(&env.status, id), NoteSync::Synced);

        // A notes.md that differs does not hold the delete.
        std::fs::write(home.join("notes.md"), "Other notes\n").unwrap();
        env.scan();
        assert_eq!(note_sync(&env.status, id), NoteSync::Synced);

        std::fs::write(home.join("summary.md"), "Another summary\n").unwrap();
        env.scan();
        assert_eq!(note_sync(&env.status, id), NoteSync::Waiting);
    }

    fn has_audio(home: &Path) -> bool {
        home.join(AUDIO).exists()
    }

    fn has_marker(home: &Path) -> bool {
        home.join(removal_marker(AUDIO)).exists()
    }

    fn documents(home: &Path) -> Vec<Vec<u8>> {
        [
            "session.md",
            "longhand.json",
            "transcript.md",
            "summary.md",
            "notes.md",
        ]
        .iter()
        .map(|name| std::fs::read(home.join(name)).unwrap())
        .collect()
    }

    // Covers AE1: with the default delay, a note that stopped 31 days ago
    // loses its audio at the pass, and its documents stay.
    #[test]
    fn ae1_a_note_that_stopped_31_days_ago_loses_its_audio() {
        let library = library();
        let home = library.saved_note("20260831-0900-aaaa", "Planning", 31);
        let before = documents(&home);
        let env = Env::new(&library, Some(30));

        let report = env.pass();

        assert_eq!(report.end, PassEnd::Done);
        assert_eq!(report.removed, vec!["20260831-0900-aaaa"]);
        assert!(!has_audio(&home));
        assert!(has_marker(&home));
        assert_eq!(documents(&home), before);
        assert_eq!(*env.changed.borrow(), vec!["20260831-0900-aaaa"]);
    }

    // KTD4: a note in a type folder loses its old audio like any note.
    #[test]
    fn the_old_audio_of_a_note_in_a_type_folder_goes() {
        let library = library();
        let home = library.saved_note("20260831-0900-aaaa", "Pathfinder/Planning", 31);
        let before = documents(&home);
        let env = Env::new(&library, Some(30));

        let report = env.pass();

        assert_eq!(report.removed, vec!["20260831-0900-aaaa"]);
        assert!(!has_audio(&home));
        assert!(has_marker(&home));
        assert_eq!(documents(&home), before);
    }

    #[test]
    fn a_note_whose_delay_has_not_passed_keeps_its_audio() {
        let library = library();
        let home = library.saved_note("20260910-0900-aaaa", "Planning", 20);
        let env = Env::new(&library, Some(30));

        let report = env.pass();

        assert!(report.removed.is_empty());
        assert!(has_audio(&home));
        assert!(!has_marker(&home));
        assert!(env.changed.borrow().is_empty());
    }

    // Covers AE3: on computer B, a note that computer A recorded 40 days
    // ago loses its audio. B's store knows the note from the library scan
    // only (an adoption), and A's in-use marker is old.
    #[test]
    fn ae3_the_audio_of_a_note_that_another_computer_recorded_goes() {
        let library = library();
        let home = library.saved_note("20260822-0900-aaaa", "Planning", 40);
        let old = now() - TimeDelta::days(39);
        assert!(pipeline::in_use::write_marker(&home, "computer-a", old).unwrap());
        let env = Env::new(&library, Some(30));

        let report = env.pass();

        assert_eq!(report.removed, vec!["20260822-0900-aaaa"]);
        assert!(!has_audio(&home));
    }

    #[test]
    fn a_fresh_in_use_marker_keeps_the_audio() {
        // R10, AE4.
        let library = library();
        let home = library.saved_note("20260822-0900-aaaa", "Planning", 40);
        assert!(pipeline::in_use::write_marker(&home, "computer-a", now()).unwrap());
        let env = Env::new(&library, Some(30));

        let report = env.pass();

        assert!(report.removed.is_empty());
        assert!(has_audio(&home));
        assert!(!has_marker(&home));
    }

    #[test]
    fn a_note_that_is_not_saved_keeps_its_audio() {
        // R8: Queued, Processing, ProcessingFailed and AwaitingDestination.
        let library = library();
        let paths: [&[Event]; 4] = [
            &[],
            &[Event::Dequeued],
            &[Event::Dequeued, Event::StageFailed(Stage::Transcribe)],
            &[Event::Dequeued, Event::SaveFailed("no folder".into())],
        ];
        let mut homes = Vec::new();
        for (i, events) in paths.iter().enumerate() {
            let id = format!("20260822-0900-000{i}");
            homes.push(library.saved_note(&id, &format!("Note {i}"), 40));
            library.transcribe_again(&id, events);
        }
        let names: Vec<&str> = library
            .store
            .lock()
            .unwrap()
            .sessions()
            .unwrap()
            .iter()
            .map(|r| r.state.name())
            .collect();
        assert_eq!(
            names,
            [
                "queued",
                "processing",
                "processing_failed",
                "awaiting_destination"
            ]
        );
        let env = Env::new(&library, Some(30));

        let report = env.pass();

        assert!(report.removed.is_empty());
        for home in &homes {
            assert!(has_audio(home), "{}", home.display());
            assert!(!has_marker(home), "{}", home.display());
        }
    }

    #[test]
    fn a_note_with_a_claim_keeps_its_audio() {
        // KTD5: a Delete Note or a summary regeneration holds a claim.
        let library = library();
        let deleting = library.saved_note("20260822-0900-aaaa", "Deleting", 40);
        let regenerating = library.saved_note("20260822-0900-bbbb", "Regenerating", 40);
        let _delete = library.claims.delete("20260822-0900-aaaa").unwrap();
        let _regenerate = library.claims.regenerate("20260822-0900-bbbb").unwrap();
        let env = Env::new(&library, Some(30));

        let report = env.pass();

        assert!(report.removed.is_empty());
        assert!(has_audio(&deleting));
        assert!(has_audio(&regenerating));
        assert!(!has_marker(&deleting));
    }

    #[test]
    fn an_open_move_keeps_all_audio_until_the_move_ends() {
        let library = library();
        let marked = library.saved_note("20260822-0900-aaaa", "Marked", 40);
        std::fs::write(marked.join(removal_marker(AUDIO)), REMOVAL_MARKER_TEXT).unwrap();
        let old = library.saved_note("20260822-0900-bbbb", "Old", 40);
        library
            .store
            .lock()
            .unwrap()
            .open_move(Path::new("/old"), Path::new("/new"))
            .unwrap();
        let env = Env::new(&library, Some(30));

        let report = env.pass();

        assert_eq!(report.end, PassEnd::MoveOpen);
        assert!(has_audio(&marked));
        assert!(has_audio(&old));
        assert!(!has_marker(&old));

        library.store.lock().unwrap().close_move().unwrap();
        let report = env.pass();
        assert_eq!(report.end, PassEnd::Done);
        assert!(!has_audio(&marked));
        assert!(!has_audio(&old));
    }

    #[test]
    fn with_the_setting_off_a_marked_segment_with_audio_loses_the_audio() {
        // KTD2: a crash between the marker and the delete, or a sync that
        // delivered the marker before the delete.
        let library = library();
        let marked = library.saved_note("20260822-0900-aaaa", "Marked", 40);
        std::fs::write(marked.join(removal_marker(AUDIO)), REMOVAL_MARKER_TEXT).unwrap();
        let plain = library.saved_note("20260822-0900-bbbb", "Plain", 40);
        let env = Env::new(&library, None);

        let report = env.pass();

        assert_eq!(report.end, PassEnd::SettingOff);
        assert_eq!(report.finished, vec!["20260822-0900-aaaa"]);
        assert!(!has_audio(&marked));
        // The setting is off: no new marker.
        assert!(has_audio(&plain));
        assert!(!has_marker(&plain));
        assert_eq!(*env.changed.borrow(), vec!["20260822-0900-aaaa"]);
    }

    #[test]
    fn a_marked_segment_of_a_note_in_use_keeps_its_audio() {
        let library = library();
        let marked = library.saved_note("20260822-0900-aaaa", "Marked", 40);
        std::fs::write(marked.join(removal_marker(AUDIO)), REMOVAL_MARKER_TEXT).unwrap();
        assert!(pipeline::in_use::write_marker(&marked, "computer-a", now()).unwrap());
        let env = Env::new(&library, None);

        env.pass();

        assert!(has_audio(&marked));
    }

    #[test]
    fn a_setting_that_turns_off_during_a_pass_stops_the_next_delete() {
        let library = library();
        let first = library.saved_note("20260822-0900-aaaa", "First", 40);
        let second = library.saved_note("20260822-0900-bbbb", "Second", 40);
        let mut env = Env::new(&library, Some(30));
        env.after_change = Box::new(|env| env.delay.set(None));

        let report = env.pass();

        assert_eq!(report.end, PassEnd::SettingOff);
        assert_eq!(report.removed, vec!["20260822-0900-aaaa"]);
        assert!(!has_audio(&first));
        assert!(has_audio(&second));
        assert!(!has_marker(&second));
    }

    #[test]
    fn an_unavailable_storage_folder_stops_the_pass_with_no_deletes() {
        let library = library();
        let home = library.saved_note("20260822-0900-aaaa", "Planning", 40);
        std::fs::write(home.join(removal_marker(AUDIO)), REMOVAL_MARKER_TEXT).unwrap();
        // A signed-out sync client removes the library marker.
        std::fs::remove_file(library::scan::marker(&library.root)).unwrap();
        let env = Env::new(&library, Some(30));

        let report = env.pass();

        assert_eq!(report.end, PassEnd::StorageUnavailable);
        assert!(has_audio(&home));
        assert_eq!(env.run(|inputs| count_eligible(inputs, 30)), 0);
    }

    #[test]
    fn a_storage_folder_that_goes_away_during_a_pass_stops_the_next_delete() {
        let library = library();
        let first = library.saved_note("20260822-0900-aaaa", "First", 40);
        let second = library.saved_note("20260822-0900-bbbb", "Second", 40);
        let mut env = Env::new(&library, Some(30));
        env.after_change = Box::new(|env| {
            let marker = library::scan::marker(&env.library.root);
            if marker.exists() {
                std::fs::remove_file(marker).unwrap();
            }
        });

        let report = env.pass();

        assert_eq!(report.end, PassEnd::StorageUnavailable);
        assert!(!has_audio(&first));
        assert!(has_audio(&second));
        assert!(!has_marker(&second));
    }

    #[test]
    fn a_note_that_the_scan_cannot_read_keeps_its_audio() {
        let library = library();
        let home = library.saved_note("20260822-0900-aaaa", "Planning", 40);
        let env = Env::new(&library, Some(30));
        env.readable.set(false);

        let report = env.pass();

        assert!(report.removed.is_empty());
        assert!(has_audio(&home));
    }

    #[test]
    fn a_second_pass_changes_nothing() {
        let library = library();
        let home = library.saved_note("20260822-0900-aaaa", "Planning", 40);
        write_copies(&home);
        let env = Env::new(&library, Some(30));
        env.pass();
        env.changed.borrow_mut().clear();
        let before = documents(&home);

        let report = env.pass();

        assert!(report.removed.is_empty());
        assert!(report.finished.is_empty());
        assert!(report.cleaned.is_empty());
        assert!(env.changed.borrow().is_empty());
        assert_eq!(documents(&home), before);
    }

    // ------------------------------------------ the cleanup of result copies

    /// Write the result copies and the transition file that an older build
    /// left in `home`.
    fn write_copies(home: &Path) {
        let generation = home.join(RESULTS_FOLDER).join("committed-1");
        std::fs::create_dir_all(&generation).unwrap();
        std::fs::write(generation.join("transcript.md"), "old transcript").unwrap();
        std::fs::write(generation.join("summary.md"), "old summary").unwrap();
        std::fs::write(home.join(TRANSITION_FILE), b"{}").unwrap();
    }

    fn has_copies(home: &Path) -> bool {
        home.join(RESULTS_FOLDER).symlink_metadata().is_ok()
            || home.join(TRANSITION_FILE).symlink_metadata().is_ok()
    }

    // Covers AE5: the first pass deletes the copies and the transition
    // files, whether the setting is on or off, and the documents stay.
    #[test]
    fn ae5_a_pass_deletes_the_copies_and_the_documents_stay() {
        for delay in [None, Some(30)] {
            let library = library();
            let old = library.saved_note("20260822-0900-aaaa", "Old", 40);
            let recent = library.saved_note("20260926-0900-bbbb", "Recent", 5);
            write_copies(&old);
            write_copies(&recent);
            let before = [documents(&old), documents(&recent)];
            let env = Env::new(&library, delay);
            env.scan();

            let report = env.pass();

            assert!(!has_copies(&old), "{delay:?}");
            assert!(!has_copies(&recent), "{delay:?}");
            assert_eq!([documents(&old), documents(&recent)], before);
            assert_eq!(
                report.cleaned,
                vec!["20260822-0900-aaaa", "20260926-0900-bbbb"]
            );
            // The recent note keeps its audio.
            assert!(has_audio(&recent));
        }
    }

    // R14: the copies can hold the only result of the note.
    #[test]
    fn a_note_with_no_transcript_keeps_its_copies() {
        let library = library();
        let home = library.saved_note("20260926-0900-aaaa", "Planning", 5);
        write_copies(&home);
        std::fs::remove_file(home.join("transcript.md")).unwrap();
        let env = Env::new(&library, None);

        let report = env.pass();

        assert!(home.join(RESULTS_FOLDER).is_dir());
        assert!(home.join(TRANSITION_FILE).is_file());
        assert!(report.cleaned.is_empty());
    }

    #[test]
    fn a_note_that_another_computer_uses_keeps_its_copies() {
        let library = library();
        let home = library.saved_note("20260926-0900-aaaa", "Planning", 5);
        write_copies(&home);
        assert!(pipeline::in_use::write_marker(&home, "computer-a", now()).unwrap());
        let env = Env::new(&library, None);

        let report = env.pass();

        assert!(has_copies(&home));
        assert!(report.cleaned.is_empty());
    }

    #[test]
    fn a_note_with_a_claim_keeps_its_copies() {
        let library = library();
        let home = library.saved_note("20260926-0900-aaaa", "Planning", 5);
        write_copies(&home);
        let _claim = library.claims.regenerate("20260926-0900-aaaa").unwrap();
        let env = Env::new(&library, None);

        env.pass();

        assert!(has_copies(&home));
    }

    #[test]
    fn a_note_that_waits_for_a_sync_keeps_its_copies_until_a_later_pass() {
        let library = library();
        let id = "20260926-0900-aaaa";
        let home = library.saved_note(id, "Planning", 5);
        write_copies(&home);
        let transcript = std::fs::read(home.join("transcript.md")).unwrap();
        // The new transcript.md arrives before its session.md.
        std::fs::write(home.join("transcript.md"), "# Transcript\n\nNew.\n").unwrap();
        let env = Env::new(&library, None);
        env.scan();
        assert_eq!(note_sync(&env.status, id), NoteSync::Waiting);

        let report = env.pass();

        assert!(has_copies(&home));
        assert!(report.cleaned.is_empty());

        // The sync ends, and the next pass deletes the copies.
        std::fs::write(home.join("transcript.md"), transcript).unwrap();
        env.scan();
        assert_eq!(note_sync(&env.status, id), NoteSync::Synced);
        let report = env.pass();
        assert!(!has_copies(&home));
        assert_eq!(report.cleaned, vec![id]);
    }

    // KTD8: an older build removed the audio and left `recognition.jsonl`.
    #[test]
    fn a_note_whose_audio_is_already_removed_loses_recognition_jsonl() {
        let library = library();
        let home = library.saved_note("20260822-0900-aaaa", "Planning", 40);
        std::fs::write(home.join(removal_marker(AUDIO)), REMOVAL_MARKER_TEXT).unwrap();
        std::fs::remove_file(home.join(AUDIO)).unwrap();
        write_recognition(&home);
        let before = documents(&home);
        let env = Env::new(&library, None);

        let report = env.pass();

        assert!(!has_recognition(&home));
        assert_eq!(documents(&home), before);
        assert_eq!(report.cleaned, vec!["20260822-0900-aaaa"]);
    }

    #[test]
    fn a_storage_folder_that_goes_away_during_a_pass_keeps_the_later_copies() {
        let library = library();
        // The first note has a marked audio file. Its delete changes the
        // row, and the storage folder then goes away.
        let first = library.saved_note("20260822-0900-aaaa", "First", 40);
        std::fs::write(first.join(removal_marker(AUDIO)), REMOVAL_MARKER_TEXT).unwrap();
        write_copies(&first);
        let second = library.saved_note("20260822-0900-bbbb", "Second", 40);
        write_copies(&second);
        let mut env = Env::new(&library, None);
        env.after_change = Box::new(|env| {
            let marker = library::scan::marker(&env.library.root);
            if marker.exists() {
                std::fs::remove_file(marker).unwrap();
            }
        });

        let report = env.pass();

        assert_eq!(report.end, PassEnd::StorageUnavailable);
        assert_eq!(report.finished, vec!["20260822-0900-aaaa"]);
        assert!(report.cleaned.is_empty());
        assert!(has_copies(&first));
        assert!(has_copies(&second));
    }

    #[test]
    fn the_count_uses_the_given_delay_and_the_same_guards() {
        // R17: the count before a change uses the new delay.
        let library = library();
        library.saved_note("20260822-0900-aaaa", "Forty", 40);
        let busy = library.saved_note("20260822-0900-bbbb", "Busy", 40);
        library.saved_note("20260911-0900-cccc", "Twenty", 20);
        assert!(pipeline::in_use::write_marker(&busy, "computer-a", now()).unwrap());
        let env = Env::new(&library, Some(90));

        assert_eq!(env.run(|inputs| count_eligible(inputs, 30)), 1);
        assert_eq!(env.run(|inputs| count_eligible(inputs, 10)), 2);
        assert_eq!(env.run(|inputs| count_eligible(inputs, 90)), 0);
        // The count changes no file.
        assert!(has_audio(&busy));
    }

    #[test]
    fn only_a_shorter_delay_or_turning_on_removes_more() {
        assert!(removes_more(Some(90), Some(30)));
        assert!(removes_more(None, Some(30)));
        assert!(!removes_more(Some(30), Some(30)));
        assert!(!removes_more(Some(30), Some(90)));
        assert!(!removes_more(Some(30), None));
        assert!(!removes_more(None, None));
    }

    // ------------------------------------------------- transcript removal

    fn has_transcript(home: &Path) -> bool {
        home.join("transcript.md").symlink_metadata().is_ok()
    }

    fn has_transcript_marker(home: &Path) -> bool {
        session_core::folder::transcript_removed(home)
    }

    /// The documents that transcript removal keeps (R7).
    fn kept_documents(home: &Path) -> Vec<Vec<u8>> {
        ["session.md", "summary.md", "notes.md"]
            .iter()
            .map(|name| std::fs::read(home.join(name)).unwrap())
            .collect()
    }

    /// Record in `longhand.json` that the summary of the note in `home` is
    /// the stub of a failed summary, as another computer saves it (R28).
    fn record_summary_stub(home: &Path) {
        let mut record = session_core::note_folder::NoteFiles::read(home)
            .unwrap()
            .record;
        record.summary = Some(session_core::folder::SummaryMeta::Stub {
            failed_stage: "summarize".into(),
        });
        session_save::note_files::write_record(home, &record).unwrap();
    }

    /// The state that a crash after step (2) of the transcript step leaves
    /// (KTD3): no audio, the transcript marker, and every transcript file.
    fn crash_after_the_marker(home: &Path) {
        std::fs::write(home.join(removal_marker(AUDIO)), REMOVAL_MARKER_TEXT).unwrap();
        std::fs::remove_file(home.join(AUDIO)).unwrap();
        std::fs::write(
            home.join(session_core::folder::TRANSCRIPT_REMOVED_MARKER),
            session_core::folder::TRANSCRIPT_REMOVED_TEXT,
        )
        .unwrap();
    }

    fn flag(library: &Library, id: &str) -> bool {
        library
            .store
            .lock()
            .unwrap()
            .transcript_removed(id)
            .unwrap()
    }

    // Covers R6, R7, R10: with audio at 30 days and transcripts at 90
    // days, a note that stopped 100 days ago loses its transcript and a
    // note that stopped 50 days ago keeps it.
    #[test]
    fn a_pass_removes_a_due_transcript_and_keeps_one_that_is_not_due() {
        let library = library();
        let old = library.saved_note("20260623-0900-aaaa", "Old", 100);
        let recent = library.saved_note("20260812-0900-bbbb", "Recent", 50);
        let before = [kept_documents(&old), kept_documents(&recent)];
        let env = Env::new(&library, Some(30));
        env.transcript_delay.set(Some(90));

        let report = env.pass();

        assert_eq!(report.end, PassEnd::Done);
        assert_eq!(report.transcripts_removed, vec!["20260623-0900-aaaa"]);
        assert!(has_transcript_marker(&old));
        assert!(!has_transcript(&old));
        assert!(!has_audio(&old));
        assert!(flag(&library, "20260623-0900-aaaa"));
        // The recent note loses only its audio.
        assert!(has_transcript(&recent));
        assert!(!has_transcript_marker(&recent));
        assert!(!has_audio(&recent));
        assert_eq!([kept_documents(&old), kept_documents(&recent)], before);
        assert!(
            env.changed
                .borrow()
                .contains(&"20260623-0900-aaaa".to_string())
        );
    }

    // KTD6: part 1 finishes a marked note whether the setting is on, off
    // or not known, and writes no new marker.
    #[test]
    fn a_pass_with_no_known_setting_finishes_a_marked_note_and_removes_no_new_transcript() {
        let library = library();
        let marked = library.saved_note("20260623-0900-aaaa", "Marked", 100);
        crash_after_the_marker(&marked);
        let plain = library.saved_note("20260623-0900-bbbb", "Plain", 100);
        let env = Env::new(&library, None);

        let report = env.pass();

        assert_eq!(report.end, PassEnd::SettingOff);
        assert_eq!(report.transcripts_finished, vec!["20260623-0900-aaaa"]);
        assert!(report.transcripts_removed.is_empty());
        assert!(!has_transcript(&marked));
        assert!(flag(&library, "20260623-0900-aaaa"));
        assert!(has_transcript(&plain));
        assert!(!has_transcript_marker(&plain));
        assert!(has_audio(&plain));

        // A second pass changes nothing.
        let report = env.pass();
        assert!(report.transcripts_finished.is_empty());
    }

    #[test]
    fn a_transcript_setting_that_turns_off_during_a_pass_stops_the_next_removal() {
        let library = library();
        let first = library.saved_note("20260623-0900-aaaa", "First", 100);
        let second = library.saved_note("20260623-0900-bbbb", "Second", 100);
        let mut env = Env::new(&library, Some(30));
        env.transcript_delay.set(Some(90));
        let marked = first.clone();
        env.after_change = Box::new(move |env| {
            if has_transcript_marker(&marked) {
                env.transcript_delay.set(None);
            }
        });

        let report = env.pass();

        assert_eq!(report.end, PassEnd::SettingOff);
        assert_eq!(report.transcripts_removed, vec!["20260623-0900-aaaa"]);
        assert!(!has_transcript(&first));
        assert!(has_transcript(&second));
        assert!(!has_transcript_marker(&second));
    }

    // R11: the in-use guard and an open Move keep the transcript.
    #[test]
    fn a_fresh_in_use_marker_or_an_open_move_keeps_the_transcript() {
        let library = library();
        let used = library.saved_note("20260623-0900-aaaa", "Used", 100);
        assert!(pipeline::in_use::write_marker(&used, "computer-a", now()).unwrap());
        let env = Env::new(&library, Some(30));
        env.transcript_delay.set(Some(90));

        let report = env.pass();

        assert!(report.transcripts_removed.is_empty());
        assert!(has_transcript(&used));
        assert!(!has_transcript_marker(&used));

        let other = self::library();
        let moving = other.saved_note("20260623-0900-bbbb", "Moving", 100);
        other
            .store
            .lock()
            .unwrap()
            .open_move(Path::new("/old"), Path::new("/new"))
            .unwrap();
        let env = Env::new(&other, Some(30));
        env.transcript_delay.set(Some(90));

        let report = env.pass();

        assert_eq!(report.end, PassEnd::MoveOpen);
        assert!(has_transcript(&moving));
        assert!(!has_transcript_marker(&moving));
        assert_eq!(
            env.run(|inputs| count_transcripts(inputs, 90)),
            TranscriptCount::default()
        );
    }

    // R11: a computer that did not record the note removes its transcript.
    #[test]
    fn the_transcript_of_a_note_that_another_computer_recorded_goes() {
        let library = library();
        let home = library.saved_note("20260623-0900-aaaa", "Planning", 100);
        let old = now() - TimeDelta::days(99);
        assert!(pipeline::in_use::write_marker(&home, "computer-a", old).unwrap());
        let env = Env::new(&library, Some(30));
        env.transcript_delay.set(Some(90));

        let report = env.pass();

        assert_eq!(report.transcripts_removed, vec!["20260623-0900-aaaa"]);
        assert!(!has_transcript(&home));
    }

    // Covers AE14, R28: a note whose summary failed keeps its transcript,
    // on the computer that saved it (the store row) and on another
    // computer (`longhand.json`). The count names it as skipped.
    #[test]
    fn ae14_a_note_whose_summary_failed_keeps_its_transcript_and_the_count_names_it() {
        let library = library();
        let here = library.saved_note("20260623-0900-aaaa", "Here", 100);
        library
            .store
            .lock()
            .unwrap()
            .put_checkpoint(
                "20260623-0900-aaaa",
                Stage::Summarize,
                pipeline::save::SUMMARY_FAILED,
                "\"failed\"",
            )
            .unwrap();
        let elsewhere = library.saved_note("20260623-0900-bbbb", "Elsewhere", 100);
        record_summary_stub(&elsewhere);
        let plain = library.saved_note("20260623-0900-cccc", "Plain", 100);
        let env = Env::new(&library, Some(30));
        env.transcript_delay.set(Some(90));

        assert_eq!(
            env.run(|inputs| count_transcripts(inputs, 60)),
            TranscriptCount {
                remove: 1,
                skipped: 2
            }
        );

        let report = env.pass();

        assert_eq!(report.transcripts_removed, vec!["20260623-0900-cccc"]);
        assert!(!has_transcript(&plain));
        for home in [&here, &elsewhere] {
            assert!(has_transcript(home), "{}", home.display());
            assert!(!has_transcript_marker(home), "{}", home.display());
        }
        // A note whose delay has not passed is not counted as skipped.
        assert_eq!(
            env.run(|inputs| count_transcripts(inputs, 365)),
            TranscriptCount::default()
        );
    }

    // Covers AE5: the count with 90 days counts the notes that are 100 days
    // old, and changes no file.
    #[test]
    fn ae5_the_transcript_count_uses_the_given_delay_and_the_same_guards() {
        let library = library();
        let first = library.saved_note("20260623-0900-aaaa", "First", 100);
        library.saved_note("20260623-0900-bbbb", "Second", 100);
        let busy = library.saved_note("20260623-0900-cccc", "Busy", 100);
        assert!(pipeline::in_use::write_marker(&busy, "computer-a", now()).unwrap());
        library.saved_note("20260911-0900-dddd", "Recent", 20);
        let env = Env::new(&library, Some(30));
        env.transcript_delay.set(Some(365));

        assert_eq!(
            env.run(|inputs| count_transcripts(inputs, 90)),
            TranscriptCount {
                remove: 2,
                skipped: 0
            }
        );
        assert_eq!(
            env.run(|inputs| count_transcripts(inputs, 365)),
            TranscriptCount::default()
        );
        assert!(has_transcript(&first));
        assert!(has_audio(&first));
    }

    // KTD6: a scan that finds a note that needs finishing wakes the pass.
    #[test]
    fn only_a_note_that_needs_finishing_wakes_the_pass() {
        let library = library();
        let marked = library.saved_note("20260623-0900-aaaa", "Marked", 100);
        library.saved_note("20260623-0900-bbbb", "Plain", 100);
        let plain = vec!["20260623-0900-bbbb".to_string()];
        let both = vec![
            "20260623-0900-aaaa".to_string(),
            "20260623-0900-bbbb".to_string(),
        ];
        assert!(!needs_finish(&library.store, &both));

        crash_after_the_marker(&marked);
        assert!(needs_finish(&library.store, &both));
        assert!(!needs_finish(&library.store, &plain));

        let env = Env::new(&library, None);
        env.pass();
        assert!(!needs_finish(&library.store, &both));
    }
}
