//! The early write of a note's home folder (U3, R1, R4, R5, KTD3, KTD4).
//!
//! Every new note gets its home folder in the storage folder at its start,
//! with `notes.md`, `session.md` and `longhand.json`, so the note exists and
//! syncs before it has audio. The audio and the processing files stay in
//! the working location. The first Save then publishes into this folder
//! (KTD3).
//!
//! The write follows the Save staging pattern, one [`EarlyState`] at a time,
//! and the store records each state before the next step (step, then
//! record):
//!
//! 1. [`EarlyState::Begun`]: [`begin`] records it when the row is made, so
//!    a crash before any write still gets one folder at the next launch.
//!    The step writes this run's in-use marker (a note that records),
//!    `notes.md`, `session.md`, then `longhand.json` into the hidden
//!    staging folder
//!    `.longhand-staging-<id>` in the storage folder.
//! 2. [`EarlyState::Filled`]: the staging folder is complete. The step
//!    moves it to the first free name candidate with `rename_new`.
//! 3. [`EarlyState::Done`]: the store records the home path, then this
//!    state. After that, [`sync`] keeps the flag and the notes of the
//!    folder up to date until the first Save.
//!
//! Every file in the folder is identifiable by the `session_id` in its
//! `longhand.json`. When the staging folder is gone in the Filled state, the
//! rename happened and its record did not: the step finds the folder at a
//! name candidate by `session_id` before it does anything again. So a crash
//! at any step leaves at most one folder (R4). When no candidate holds the
//! note, another computer or the user removed the folder, and the write
//! ends with no folder (R23, R24).
//!
//! The start of a recording never waits for this write: capture records
//! [`EarlyState::Begun`] with the row and starts the job of the note
//! ([`EarlyHomes`]), which writes on its own thread. While the storage
//! folder is not available, the job tries again (R5). The first Save of a
//! note whose early write is not done waits for it
//! ([`StageError::NoDestination`]) and never makes a second folder.
//!
//! One job for each note runs the early write and each later flag update
//! in order. The job, the first Save and removal hold the note guard
//! ([`guard`]) while they write, so a flag update never comes after the
//! `session.md` of the first Save.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard};
use std::time::Duration;

use chrono::Local;
use library::scan::{self, Known, NoteRead, READ_TIMEOUT, Read, ScanOptions, read_timed};
use serde::{Deserialize, Serialize};
use session_core::folder::{self, ResultFlag, SessionDoc};
use session_core::note_folder::{Layout, SESSION_FILE, written_fingerprint};
use session_core::note_record::{NOTE_RECORD_FILE, NoteRecord};
use session_core::{SessionRecord, SessionState, Store};
use session_save::{Fs, RealFs, fsync, home_write, naming, note_files, rename, rename_home};

use crate::save::{NOTES_FILE, note_started};
use crate::{SharedStore, StageError, in_use, lock, type_move};

/// The `result_state` key of the early write.
pub use session_core::jobs::EARLY_HOME as STATE;

/// Name candidates before the write gives up on the storage folder, as
/// Save does.
const MAX_NAMES: usize = 100;

/// How long a job waits before it tries again.
pub const RETRY_EVERY: Duration = Duration::from_secs(10);

/// The progress of the early write of a note's home folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum EarlyState {
    /// Nothing is at a final name. A staging folder of this note, if one is
    /// there, can be partial.
    Begun,
    /// The staging folder `.longhand-staging-<id>` in `root` is complete.
    /// The final name is `folder_name` or one of its candidates, in the
    /// folder of the type `note_type` in `root`, or directly in `root` for
    /// `None` (U7 of the note types plan).
    Filled {
        root: PathBuf,
        folder_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note_type: Option<String>,
    },
    /// The folder is at `path`, and the store records it as the home path.
    Done { path: PathBuf },
}

/// Record [`EarlyState::Begun`] for `id` when the note has no early write
/// yet. Call it in the same store lock that makes the row. Returns whether
/// it recorded the state.
pub fn begin(store: &Store, id: &str) -> Result<bool, StageError> {
    if store.result_state(id, STATE)?.is_some() {
        return Ok(false);
    }
    store.set_result_state(id, STATE, &serde_json::to_value(EarlyState::Begun)?)?;
    Ok(true)
}

/// The early write state of `id`. `None` for a note from before this
/// change, and for a note from another computer.
pub fn state(store: &Store, id: &str) -> Result<Option<EarlyState>, StageError> {
    Ok(store
        .result_state(id, STATE)?
        .map(serde_json::from_value)
        .transpose()?)
}

/// Whether the early write of `id` has begun and is not done. Until it is
/// done, the first Save waits, and the library never forgets the row.
pub fn unfinished(store: &Store, id: &str) -> Result<bool, StageError> {
    Ok(state(store, id)?.is_some_and(|s| !matches!(s, EarlyState::Done { .. })))
}

/// The notes whose early write the launch must start again: an early
/// write that is not done, and a note that opened with no recording and
/// has neither a home folder nor an early write (a crash lost its
/// [`begin`]). Sorted by id.
pub fn waiting(store: &Store) -> Result<Vec<String>, StageError> {
    let mut ids = Vec::new();
    for record in store.sessions()? {
        if record.state == SessionState::Discarded {
            continue;
        }
        let pending = match state(store, &record.id)? {
            Some(state) => !matches!(state, EarlyState::Done { .. }),
            None => notes_only_without_home(store, &record)?,
        };
        if pending {
            ids.push(record.id);
        }
    }
    ids.sort();
    Ok(ids)
}

/// A note that opened with no recording, and that has no home folder.
fn notes_only_without_home(store: &Store, record: &SessionRecord) -> Result<bool, StageError> {
    Ok(record.state == SessionState::Saved
        && record.home_path.is_none()
        && store.started_here(&record.id)?
        && store.segments(&record.id)?.is_empty())
}

/// The flag that `session.md` of `record` must carry now (R6, KTD1), or
/// `None` when this computer must not write the flag: the note has a
/// result, the first Save has begun, or the recording was discarded.
fn flag_for(store: &Store, record: &SessionRecord) -> Result<Option<ResultFlag>, StageError> {
    let segments = store.segments(&record.id)?;
    Ok(match record.state {
        SessionState::Discarded => None,
        SessionState::Saved if segments.is_empty() => Some(ResultFlag::NoRecording),
        SessionState::Saved => None,
        _ if crate::save::begun(store, &record.id)? => None,
        _ => Some(ResultFlag::NotSaved),
    })
}

/// The type of the home folder of `record`, from its row (U7 of the note
/// types plan). A name that fails the type name check never becomes a path
/// (KTD8): the row then has General as its type, and the folder goes
/// directly in the storage folder.
fn row_type(store: &Store, record: &SessionRecord) -> Result<Option<String>, StageError> {
    let Some(name) = &record.note_type else {
        return Ok(None);
    };
    match naming::check_type_name(name) {
        Ok(checked) if &checked == name => Ok(Some(checked)),
        _ => {
            log::warn!(
                "the type of {} is not a valid type name, so its folder goes in the storage folder",
                record.id
            );
            store.set_note_type(&record.id, None)?;
            Ok(None)
        }
    }
}

/// Whether the type folder cannot be made or used for a reason that a
/// retry does not fix.
fn is_refused(e: &rename_home::RenameError) -> bool {
    matches!(e, rename_home::RenameError::Io(io) if io.kind() == io::ErrorKind::InvalidInput)
}

/// The user changed the type of the note of `record` to the type of the
/// row after the early write read it, and the home folder is in the folder
/// of the type `placed` (U7 of the note types plan). The change becomes a type
/// move that this computer owes (KTD5), and the app shows the new type
/// while the move waits (KTD3). The caller holds the store lock.
fn owe_type_move(
    store: &Store,
    record: &SessionRecord,
    placed: Option<String>,
) -> Result<(), StageError> {
    let now = Local::now().fixed_offset();
    let to = record.note_type.clone();
    log::info!(
        "the type of {} changed while its folder was written, so the folder moves later",
        record.id
    );
    store.set_type_move(
        &record.id,
        &session_core::TypeMove {
            to: to.clone(),
            from: placed.clone(),
            changed_at: now,
            taken_over_at: None,
            remove_empty_from: false,
            field_written: false,
        },
    )?;
    store.set_pending_type(
        &record.id,
        Some(&folder::PendingType {
            to,
            from: placed.clone(),
            by: store.computer_id()?,
            at: now,
        }),
    )?;
    store.set_note_type(&record.id, placed.as_deref())?;
    Ok(())
}

// ------------------------------------------------------------ the note guard

static GUARDED: LazyLock<(Mutex<HashSet<String>>, Condvar)> =
    LazyLock::new(|| (Mutex::new(HashSet::new()), Condvar::new()));

/// The note guard of one note, held until it drops.
pub struct NoteGuard {
    id: String,
}

/// Take the note guard of `id`, and wait while another thread holds it. The
/// early write, each flag update and the first Save hold it while they
/// write. Removal of an empty note (U6) holds it while it checks and
/// removes. A thread must not take the guard of a note twice.
pub fn guard(id: &str) -> NoteGuard {
    let (held, signal) = &*GUARDED;
    let mut set = held.lock().unwrap_or_else(|e| e.into_inner());
    while set.contains(id) {
        set = signal.wait(set).unwrap_or_else(|e| e.into_inner());
    }
    set.insert(id.to_string());
    NoteGuard { id: id.to_string() }
}

impl Drop for NoteGuard {
    fn drop(&mut self) {
        let (held, signal) = &*GUARDED;
        held.lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
        signal.notify_all();
    }
}

// ------------------------------------------------------------ the steps

pub(crate) fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

/// The storage folder `root` exists, is a folder and takes a new file.
fn check_root(root: &Path) -> Result<(), StageError> {
    session_save::check_destination(root).map_err(|e| StageError::NoDestination(e.to_string()))
}

/// An error in the storage folder: the write waits for the folder (R5).
fn storage(root: &Path, e: io::Error) -> StageError {
    StageError::NoDestination(format!(
        "writing the note's folder in {} failed: {e}",
        root.display()
    ))
}

/// Whether `path` is the folder of the note `id`: its `longhand.json`, or
/// an old-layout `session.md`, names `id`. The read has a time limit,
/// because a synced folder can hold a file that does not download. Such a
/// file gives [`NoteAt::Unknown`].
fn note_at(path: &Path, id: &str) -> NoteAt {
    match scan::read_note(path, READ_TIMEOUT) {
        NoteRead::Note(files) if files.doc.session_id == id => NoteAt::Yes,
        NoteRead::Unreadable(why) => NoteAt::Unknown(why),
        _ => NoteAt::No,
    }
}

/// Whether a folder is the folder of a note, from [`note_at`].
enum NoteAt {
    Yes,
    No,
    /// The files of the folder cannot be read now.
    Unknown(String),
}

/// Take one step of the early write of `id` from `state`. `destination` is
/// the storage folder now, which only [`EarlyState::Begun`] uses: a later
/// step keeps the folder of its staging copy. `run_id` names this run's
/// in-use marker. The caller records the returned state with [`record`]
/// before the next step, and holds the note guard.
pub fn step(
    store: &SharedStore,
    id: &str,
    state: &EarlyState,
    destination: Option<&Path>,
    run_id: &str,
    fs: &dyn Fs,
) -> Result<EarlyState, StageError> {
    match state {
        EarlyState::Begun => {
            let root = destination
                .ok_or_else(|| StageError::NoDestination("no storage folder is set".into()))?;
            check_root(root)?;
            let (doc, notes, folder_name, records, note_type) = {
                let guard = lock(store);
                let record = guard.session(id)?.ok_or(StageError::Cancelled)?;
                let note_type = row_type(&guard, &record)?;
                let flag = flag_for(&guard, &record)?.ok_or(StageError::Cancelled)?;
                let segments = guard.segments(id)?;
                let started = note_started(&record, &segments);
                let mut doc = SessionDoc::without_audio(id, started, record.speaker_count);
                doc.result = Some(flag);
                doc.title = record.title.as_ref().map(|t| t.text.clone());
                doc.title_source = record.title.as_ref().map(|t| t.source);
                doc.tags = record.tags.clone();
                doc.pending_type = record.pending_type.clone();
                let title = record.title.as_ref().map(|t| t.text.as_str());
                let name = naming::folder_name(started, title);
                let notes = folder::render_notes(&guard.notes(id)?);
                (doc, notes, name, flag == ResultFlag::NotSaved, note_type)
            };
            let staging = root.join(naming::staging_name(id));
            match std::fs::create_dir(&staging) {
                Ok(()) => {}
                // A crash left this note's staging folder: the name holds
                // the note id. The writes below replace what it holds.
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists && staging.is_dir() => {}
                Err(e) => return Err(storage(root, e)),
            }
            // The marker first: the folder is in use by this run before its
            // records are anywhere (KTD4).
            if records {
                in_use::write_marker(&staging, run_id, Local::now().fixed_offset())
                    .map_err(|e| storage(root, e))?;
            }
            // `notes.md`, `session.md` with the title and the tags, then
            // `longhand.json` with the records and the identity, last
            // (KTD2 of the safe agent edits plan).
            let session = note_files::new_session_md(doc.title.as_deref(), &doc.tags);
            let record = NoteRecord::from_doc(&doc).render();
            home_write::write_atomic(&staging, NOTES_FILE, notes.as_bytes())
                .and_then(|()| home_write::write_atomic(&staging, SESSION_FILE, session.as_bytes()))
                .and_then(|()| {
                    home_write::write_atomic(&staging, NOTE_RECORD_FILE, record.as_bytes())
                })
                .and_then(|()| fsync::sync_dir(root))
                .map_err(|e| storage(root, e))?;
            {
                let guard = lock(store);
                guard.set_written_notes_fingerprint(
                    id,
                    Some(&folder::fingerprint(notes.as_bytes())),
                )?;
                guard.set_written_session_fingerprint(
                    id,
                    Some(&written_fingerprint(session.as_bytes(), record.as_bytes())),
                )?;
            }
            Ok(EarlyState::Filled {
                root: root.to_path_buf(),
                folder_name,
                note_type,
            })
        }
        EarlyState::Filled {
            root,
            folder_name,
            note_type,
        } => {
            // Only a storage folder that is there can tell a rename that
            // happened from a staging folder that is gone.
            check_root(root)?;
            let staging = root.join(naming::staging_name(id));
            let staged = exists(&staging);
            // The final name is in the type folder. The search after a
            // crash makes no type folder.
            let parent = match note_type {
                None => Some(root.clone()),
                Some(name) if staged => match type_move::type_folder(root, name) {
                    Ok(folder) => Some(folder),
                    Err(e) if is_refused(&e) => {
                        // A file has the name of the type folder: the note
                        // goes directly in the storage folder, in General.
                        log::warn!("the folder of {id} cannot go in the type folder {name}: {e}");
                        lock(store).set_note_type(id, None)?;
                        Some(root.clone())
                    }
                    Err(e) => return Err(storage(root, io::Error::other(e.to_string()))),
                },
                Some(name) => match type_move::existing_type_folder(root, name) {
                    Ok(folder) => folder,
                    // The rename went directly in the storage folder, as
                    // above.
                    Err(e) if is_refused(&e) => Some(root.clone()),
                    Err(e) => return Err(storage(root, io::Error::other(e.to_string()))),
                },
            };
            let candidates = parent.iter().flat_map(|parent| {
                (1..=MAX_NAMES).map(|attempt| parent.join(naming::candidate(folder_name, attempt)))
            });
            if !staged {
                // The rename happened and its record did not: the folder's
                // `session.md` names the note.
                let mut unreadable = None;
                for path in candidates.clone() {
                    match note_at(&path, id) {
                        NoteAt::Yes => return Ok(EarlyState::Done { path }),
                        NoteAt::No => {}
                        NoteAt::Unknown(why) => unreadable = Some(why),
                    }
                }
                // A folder that does not download yet can be the note: the
                // write tries again later.
                if let Some(why) = unreadable {
                    return Err(StageError::NoDestination(format!(
                        "a folder in {} cannot be read: {why}",
                        root.display()
                    )));
                }
                // Filled is recorded after the staging writes, so the
                // folder was at its final name, and another computer or the
                // user removed it. A folder that was removed is not made
                // again (R23, R24): the write ends, as `sync` does for a
                // home folder that is gone. The home path is the staging
                // name, which holds the note id, so no other note is ever
                // at it. A later scan finds the folder if it is at another
                // name ([`locate`]).
                log::warn!(
                    "the folder of {id} is gone from the storage folder, so it is not made again"
                );
                return Ok(EarlyState::Done { path: staging });
            }
            for target in candidates {
                match rename::rename_new_waiting(fs, &staging, &target) {
                    Ok(()) => {
                        fsync::sync_dir(root).map_err(|e| storage(root, e))?;
                        // Longhand saves to this storage folder now (KTD5
                        // of the library plan), as Save marks it.
                        crate::follow::mark_destination(root);
                        return Ok(EarlyState::Done { path: target });
                    }
                    Err(e) if rename::is_name_taken(&e) => continue,
                    // A folder in use is not a taken name (KTD8).
                    Err(e) if rename::is_in_use(&e) => {
                        return Err(storage(root, rename::in_use_error(&staging, &e)));
                    }
                    Err(e) => return Err(storage(root, e)),
                }
            }
            Err(StageError::NoDestination(format!(
                "the storage folder {} has no free name for {folder_name} after {MAX_NAMES} tries",
                root.display()
            )))
        }
        EarlyState::Done { path } => Ok(EarlyState::Done { path: path.clone() }),
    }
}

/// Record `state` as the early write state of `id`. For
/// [`EarlyState::Done`], the home path first, so a recorded Done always
/// has its home path. A row that is gone ends the write.
pub fn record(store: &SharedStore, id: &str, state: &EarlyState) -> Result<(), StageError> {
    let guard = lock(store);
    let Some(record) = guard.session(id)? else {
        return Err(StageError::Cancelled);
    };
    if let EarlyState::Done { path } = state {
        // A type change after the first step does not reach the folder:
        // the folder moves later.
        if let Some(EarlyState::Filled { root, .. }) = self::state(&guard, id)?
            && path.file_name() != Some(std::ffi::OsStr::new(&naming::staging_name(id)))
            && let Some(placed) = folder::note_type_in(&root, path)
            && !naming::same_optional_type(&placed, &record.note_type)
        {
            owe_type_move(&guard, &record, placed)?;
        }
        guard.set_home_path(id, &path.to_string_lossy())?;
    }
    guard.set_result_state(id, STATE, &serde_json::to_value(state)?)?;
    Ok(())
}

/// Take the early write of `id` to its end, then [`sync`] the folder.
/// Returns the home folder, or `None` when the note has no early write
/// (a note from before this change or from another computer, a discarded
/// recording, or a row that is gone). A note that opened with no
/// recording and has neither a home folder nor an early write begins one
/// now. Takes the note guard.
pub fn write(
    store: &SharedStore,
    id: &str,
    destination: Option<&Path>,
    run_id: &str,
    fs: &dyn Fs,
) -> Result<Option<PathBuf>, StageError> {
    let _guard = guard(id);
    write_guarded(store, id, destination, run_id, fs, &|| false)
}

fn write_guarded(
    store: &SharedStore,
    id: &str,
    destination: Option<&Path>,
    run_id: &str,
    fs: &dyn Fs,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<PathBuf>, StageError> {
    {
        let guard = lock(store);
        let Some(record) = guard.session(id)? else {
            return Ok(None);
        };
        if record.state == SessionState::Discarded {
            return Ok(None);
        }
        if state(&guard, id)?.is_none() {
            if !notes_only_without_home(&guard, &record)? {
                return Ok(None);
            }
            begin(&guard, id)?;
        }
    }
    loop {
        if cancelled() {
            return Err(StageError::Cancelled);
        }
        let Some(state) = state(&lock(store), id)? else {
            return Ok(None);
        };
        if let EarlyState::Done { .. } = state {
            break;
        }
        let next = step(store, id, &state, destination, run_id, fs)?;
        record(store, id, &next)?;
    }
    if cancelled() {
        return Err(StageError::Cancelled);
    }
    sync_guarded(store, id, run_id)?;
    Ok(lock(store)
        .session(id)?
        .and_then(|r| r.home_path)
        .map(PathBuf::from))
}

/// Bring the flag and the notes of the home folder of `id` up to date,
/// after its early write is done (R2, R6). A note that records gets this
/// run's in-use marker before its flag says "recording not saved". A note
/// whose flag says "no recording" loses this run's marker after the flag
/// write. The flag is written only while the note has no result and its first Save
/// has not begun: after that, the Save owns `session.md`. `notes.md` is
/// written when the store holds other notes and the file is still the one
/// this computer last wrote. Each write records its fingerprint (KTD8).
/// Takes the note guard.
pub fn sync(store: &SharedStore, id: &str, run_id: &str) -> Result<(), StageError> {
    let _guard = guard(id);
    sync_guarded(store, id, run_id)
}

fn sync_guarded(store: &SharedStore, id: &str, run_id: &str) -> Result<(), StageError> {
    let (home, flag) = {
        let guard = lock(store);
        let Some(record) = guard.session(id)? else {
            return Ok(());
        };
        if !matches!(state(&guard, id)?, Some(EarlyState::Done { .. })) {
            return Ok(());
        }
        let Some(home) = record.home_path.clone().map(PathBuf::from) else {
            return Ok(());
        };
        let Some(flag) = flag_for(&guard, &record)? else {
            return Ok(());
        };
        (home, flag)
    };
    let root = storage_root(&home);
    if !home.is_dir() {
        // A folder that another computer removed is not made again
        // (R23, R24). A storage folder that is not there waits.
        check_root(&root)?;
        log::warn!("the folder of {id} is gone from the storage folder, so it gets no update");
        return Ok(());
    }
    if flag == ResultFlag::NotSaved
        && !in_use::write_marker(&home, run_id, Local::now().fixed_offset())
            .map_err(|e| storage(&root, e))?
    {
        return Ok(());
    }
    // The flag write and the notes sync read these files again under the
    // store lock. A file that cannot be read in time (a cloud file that
    // does not download) stops the job here, before the lock, so the lock
    // does not wait for it.
    for name in [SESSION_FILE, NOTE_RECORD_FILE, NOTES_FILE] {
        if let Read::Failed(why) = read_timed(home.join(name), READ_TIMEOUT) {
            return Err(StageError::NoDestination(format!(
                "{} cannot be read: {why}",
                home.join(name).display()
            )));
        }
    }
    write_flag(&lock(store), id, &home, &root, flag)?;
    // The note does not record any more: a recording that kept no audio
    // went back to "no recording". This job wrote the marker, and the
    // in-use keeper removes only the markers that it wrote, so the job
    // removes it here. A recording that starts after the check queues this
    // job again, and that run writes the marker again.
    if flag == ResultFlag::NoRecording {
        in_use::remove_marker(&home, run_id).map_err(|e| storage(&root, e))?;
    }
    sync_notes(&lock(store), id, &home)
}

/// Write `flag` into the `longhand.json` of the note `id` in `home`, and
/// record the fingerprint of what this computer wrote. The caller holds the
/// store lock from the read to the write, so a title write does not undo
/// the flag, and the flag write does not undo the title.
fn write_flag(
    store: &Store,
    id: &str,
    home: &Path,
    root: &Path,
    flag: ResultFlag,
) -> Result<(), StageError> {
    let files = match scan::read_note(home, READ_TIMEOUT) {
        NoteRead::Note(files) => *files,
        NoteRead::Unreadable(why) => {
            return Err(StageError::NoDestination(format!(
                "the files of {} cannot be read: {why}",
                home.display()
            )));
        }
        _ => return Ok(()),
    };
    // A note in the old layout gets no write: its records stay in
    // `session.md` until the conversion (KTD6).
    if files.layout != Layout::New
        || files.record.session_id != id
        || files.record.result == Some(flag)
    {
        return Ok(());
    }
    // Only the flag changes: the title and the rest stay as the files have
    // them, also when another computer wrote them.
    let mut record = files.record;
    record.result = Some(flag);
    let text = note_files::write_record(home, &record).map_err(|e| storage(root, e))?;
    // The read is timed: the caller holds the store lock.
    let session = match read_timed(home.join(SESSION_FILE), READ_TIMEOUT) {
        Read::Bytes(bytes) => bytes,
        Read::Missing | Read::Failed(_) => Vec::new(),
    };
    store.set_written_session_fingerprint(id, Some(&written_fingerprint(&session, &text)))?;
    Ok(())
}

/// The storage folder of the home folder `home`, directly or through its
/// type folder (KTD4 of the note types plan), from the library marker.
/// With no marker, the parent folder, as before type folders.
fn storage_root(home: &Path) -> PathBuf {
    scan::storage_place(home).map_or_else(
        || home.parent().unwrap_or(home).to_path_buf(),
        |place| place.root,
    )
}

/// Write the notes of the store into `home` when the file differs and is
/// still the file that this computer last wrote. The caller holds the
/// store lock from the read of the store notes to the record of the
/// written fingerprint, so a notes save cannot write between them and then
/// be replaced by older notes.
fn sync_notes(store: &Store, id: &str, home: &Path) -> Result<(), StageError> {
    let root = storage_root(home);
    let notes = store.notes(id)?;
    let written = store.written_fingerprints(id)?.notes;
    let rendered = folder::render_notes(&notes);
    let wanted = folder::fingerprint(rendered.as_bytes());
    let current = match read_timed(home.join(NOTES_FILE), READ_TIMEOUT) {
        Read::Bytes(bytes) => Some(folder::fingerprint(&bytes)),
        Read::Missing => None,
        Read::Failed(why) => {
            return Err(StageError::NoDestination(format!(
                "{} cannot be read: {why}",
                home.join(NOTES_FILE).display()
            )));
        }
    };
    if current.as_ref() == Some(&wanted) {
        return Ok(());
    }
    // Another computer wrote the file: the library refresh takes it.
    if current.is_some() && current != written {
        return Ok(());
    }
    home_write::write_atomic(home, NOTES_FILE, rendered.as_bytes())
        .map_err(|e| storage(&root, e))?;
    store.set_written_notes_fingerprint(id, Some(&wanted))?;
    Ok(())
}

/// Whether another computer deleted the home folder of `id` while this
/// computer recorded into it (R23): the early write is done, the storage
/// folder is there and every note folder in it can be read, and no folder
/// in it holds the note. A folder that holds the note at another name
/// becomes the home path. Recovery then discards the recording.
pub fn home_deleted(store: &SharedStore, id: &str) -> Result<bool, StageError> {
    let home = {
        let guard = lock(store);
        if !matches!(state(&guard, id)?, Some(EarlyState::Done { .. })) {
            return Ok(false);
        }
        match guard.session(id)?.and_then(|r| r.home_path) {
            Some(home) => PathBuf::from(home),
            None => return Ok(false),
        }
    };
    Ok(locate(store, id, &home)? == Location::Gone)
}

/// Where the folder of a note is, from [`locate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Location {
    /// The folder is at its home path.
    There,
    /// The folder is at another name in the storage folder. The row now
    /// has that home path.
    Moved,
    /// The storage folder is there and every note folder in it can be
    /// read, and no folder in it holds the note.
    Gone,
    /// The storage folder is not available, or a note folder in it cannot
    /// be read: this computer cannot tell where the note is.
    Unknown,
}

/// Find the folder of the note `id`, whose home path is `home`. A folder
/// that holds the note at another name becomes the home path. This scans
/// the storage folder with no store lock held.
pub(crate) fn locate(store: &SharedStore, id: &str, home: &Path) -> Result<Location, StageError> {
    if exists(home) {
        return Ok(Location::There);
    }
    // The home folder can be in a type folder (KTD4 of the note types
    // plan). With no marker, the storage folder is not available.
    let Some(root) = scan::storage_place(home).map(|place| place.root) else {
        return Ok(Location::Unknown);
    };
    // With the known rows, a storage folder with no library marker and no
    // note counts as not available (an empty mount point), not as empty.
    let known = Known::from_records(&lock(store).sessions()?);
    let found = scan::scan(std::slice::from_ref(&root), &known, &ScanOptions::default());
    // A folder of this note that cannot be read can be the real one, and a
    // readable folder with the same id then a copy of it.
    let unreadable_here = found
        .unreadable
        .iter()
        .any(|folder| folder.known_id.as_deref() == Some(id));
    if !found.is_available(&root) || unreadable_here {
        return Ok(Location::Unknown);
    }
    // A folder that holds the note shows where it is, also when the
    // folder of another note cannot be read.
    if let Some(note) = found.notes.iter().find(|note| note.doc.session_id == id) {
        let guard = lock(store);
        guard.set_home_path(id, &note.path.to_string_lossy())?;
        guard.set_note_type(id, note.note_type.as_deref())?;
        return Ok(Location::Moved);
    }
    // A folder that cannot be read can hold the note.
    if !found.unreadable.is_empty() {
        return Ok(Location::Unknown);
    }
    Ok(Location::Gone)
}

// ------------------------------------------------------------ the jobs

/// The jobs of the early writes: one job for each note, on its own
/// thread. A job takes the early write to its end, then syncs the flag and
/// the notes, and tries again while the storage folder is not available
/// (R5). The start of a recording only queues work here and never waits.
pub struct EarlyHomes {
    store: SharedStore,
    run_id: String,
    /// The storage folder now.
    destination: Box<dyn Fn() -> Option<PathBuf> + Send + Sync>,
    /// Called after each job run that succeeded: the home path can be new,
    /// and a note that records can have a new in-use marker. The caller
    /// wakes the in-use markers and the notes that wait for a folder.
    done: Box<dyn Fn(&str) + Send + Sync>,
    retry: Duration,
    jobs: Mutex<HashMap<String, Arc<NoteJob>>>,
    stopped: AtomicBool,
}

#[derive(Default)]
struct NoteJob {
    state: Mutex<JobState>,
    signal: Condvar,
}

#[derive(Default)]
struct JobState {
    /// Work is queued: the early write, or a flag update.
    queued: bool,
    woken: bool,
    cancelled: bool,
    /// The thread ended. A new request makes a new job.
    ended: bool,
    /// Why the last run failed, while the job tries again.
    problem: Option<String>,
}

impl NoteJob {
    fn state(&self) -> MutexGuard<'_, JobState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl EarlyHomes {
    /// The jobs of the run `run_id`, the claim owner of this run's
    /// processing, which names its in-use markers.
    pub fn new(
        store: SharedStore,
        run_id: impl Into<String>,
        destination: Box<dyn Fn() -> Option<PathBuf> + Send + Sync>,
        done: Box<dyn Fn(&str) + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            run_id: run_id.into(),
            destination,
            done,
            retry: RETRY_EVERY,
            jobs: Mutex::default(),
            stopped: AtomicBool::new(false),
        })
    }

    /// The same jobs with another wait between two tries, for tests.
    pub fn with_retry(self: Arc<Self>, retry: Duration) -> Arc<Self> {
        match Arc::try_unwrap(self) {
            Ok(mut homes) => {
                homes.retry = retry;
                Arc::new(homes)
            }
            Err(homes) => homes,
        }
    }

    /// Start the early write of the new note `id` in the background (R1).
    /// Capture calls it after it records the row and [`begin`]. U5 calls it
    /// for a note that opens with no recording.
    pub fn start(self: &Arc<Self>, id: &str) {
        self.queue(id);
    }

    /// Queue a flag update to "recording not saved" for `id`: a recording
    /// started into a note that opened with no recording (U5). The job
    /// writes this run's in-use marker, then the flag, after the early
    /// write if that is not done yet. The update writes nothing once the
    /// first Save has begun.
    pub fn mark_pending(self: &Arc<Self>, id: &str) {
        self.queue(id);
    }

    /// At launch: start the early write of each note that waits for one
    /// ([`waiting`]).
    pub fn resume(self: &Arc<Self>) {
        let waiting = match waiting(&lock(&self.store)) {
            Ok(ids) => ids,
            Err(e) => {
                log::error!("could not list the notes that wait for their folder: {e}");
                return;
            }
        };
        for id in waiting {
            log::info!("the folder of {id} is not written yet; it is written now");
            self.queue(&id);
        }
    }

    /// The storage folder came back or changed: each job tries again now.
    pub fn wake(&self) {
        for job in self.jobs().values() {
            job.state().woken = true;
            job.signal.notify_all();
        }
    }

    /// Stop the job of `id` and wait until its thread ends. A step in
    /// progress completes first; after the return, the job writes nothing
    /// more. The early write state in the store then tells what is in the
    /// storage folder. Removal of an empty note (U6) calls it before it
    /// checks the folder. A note with no job returns at once.
    pub fn cancel(&self, id: &str) {
        let Some(job) = self.jobs().get(id).cloned() else {
            return;
        };
        let mut state = job.state();
        state.cancelled = true;
        job.signal.notify_all();
        while !state.ended {
            state = job.signal.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Wait until the job of `id` has no more work, or ends. A note with
    /// no job returns at once. While the storage folder is not available,
    /// this waits until it is back.
    pub fn wait(&self, id: &str) {
        let Some(job) = self.jobs().get(id).cloned() else {
            return;
        };
        let mut state = job.state();
        while !state.ended {
            state = job.signal.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Why the job of `id` waits, while it tries again: the storage
    /// folder is not available, for example. `None` when the job has no
    /// problem or the note has no job.
    pub fn problem(&self, id: &str) -> Option<String> {
        self.jobs()
            .get(id)
            .and_then(|job| job.state().problem.clone())
    }

    /// Before the process exits: every job stops after its step in
    /// progress. The early writes continue at the next launch.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        for job in self.jobs().values() {
            job.state().cancelled = true;
            job.signal.notify_all();
        }
    }

    fn jobs(&self) -> MutexGuard<'_, HashMap<String, Arc<NoteJob>>> {
        self.jobs.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn queue(self: &Arc<Self>, id: &str) {
        if self.stopped.load(Ordering::SeqCst) {
            return;
        }
        let mut jobs = self.jobs();
        if let Some(job) = jobs.get(id) {
            let mut state = job.state();
            if !state.ended && !state.cancelled {
                state.queued = true;
                state.woken = true;
                job.signal.notify_all();
                return;
            }
        }
        let job = Arc::new(NoteJob::default());
        job.state().queued = true;
        jobs.insert(id.to_string(), Arc::clone(&job));
        drop(jobs);
        let homes = Arc::clone(self);
        let note = id.to_string();
        let spawned = std::thread::Builder::new()
            .name("early-home".into())
            .spawn(move || homes.run(&note, &job));
        if let Err(e) = spawned {
            log::error!("could not start the folder write of {id}: {e}");
            if let Some(job) = self.jobs().remove(id) {
                job.state().ended = true;
                job.signal.notify_all();
            }
        }
    }

    fn run(&self, id: &str, job: &Arc<NoteJob>) {
        loop {
            {
                let mut state = job.state();
                if state.cancelled {
                    break;
                }
                state.queued = false;
                state.woken = false;
            }
            let outcome = {
                let _guard = guard(id);
                let destination = (self.destination)();
                let cancelled = || job.state().cancelled;
                write_guarded(
                    &self.store,
                    id,
                    destination.as_deref(),
                    &self.run_id,
                    &RealFs,
                    &cancelled,
                )
            };
            match outcome {
                Ok(_) => {
                    job.state().problem = None;
                    (self.done)(id);
                    // The job ends unless more work came during the run.
                    // The map lock comes first, as in `queue`.
                    let mut jobs = self.jobs();
                    let mut state = job.state();
                    if !state.queued || state.cancelled {
                        state.ended = true;
                        if jobs.get(id).is_some_and(|j| Arc::ptr_eq(j, job)) {
                            jobs.remove(id);
                        }
                        job.signal.notify_all();
                        return;
                    }
                }
                Err(StageError::Cancelled) => break,
                Err(e) => {
                    log::warn!("the folder of {id} is not written yet: {e}");
                    let mut state = job.state();
                    state.problem = Some(e.to_string());
                    if !state.woken && !state.cancelled {
                        state = job
                            .signal
                            .wait_timeout(state, self.retry)
                            .map(|(state, _)| state)
                            .unwrap_or_else(|e| e.into_inner().0);
                    }
                }
            }
        }
        let mut jobs = self.jobs();
        let mut state = job.state();
        state.ended = true;
        if jobs.get(id).is_some_and(|j| Arc::ptr_eq(j, job)) {
            jobs.remove(id);
        }
        job.signal.notify_all();
    }
}
