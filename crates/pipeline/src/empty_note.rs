//! Removal of empty notes (U6, R18, R19, KTD8).
//!
//! A note with no audio, no text in its notes and no title from the user
//! is removed, with its home folder, when the user leaves it. Only the
//! computer that started the note removes it. When the app quit first, the
//! launch pass ([`Removal::launch_pass`]) removes it at the next launch.
//!
//! The storage folder syncs across computers, so the home folder can hold
//! work from another computer. One check owns the conditions, and the note
//! stays when one of them fails:
//!
//! - The row: the note is Saved with zero segments, this computer started
//!   it, it did not exist before a discarded recording (R19), its notes
//!   have no text, and it has no title from the user. No delete, Move,
//!   summary or folder rename works on it.
//! - The folder: the storage folder is available, the home folder is a real
//!   folder, `session.md` and `notes.md` are the files that this computer
//!   last wrote (their fingerprints, KTD8), no in-use marker of another run
//!   is fresh, and the folder holds no other file.
//!
//! This run's own in-use marker goes first: after a discarded recording, it
//! can still be in the folder. A marker of an earlier run on this computer
//! is an old marker: the app quit or stopped before that run removed it.
//! Each other fresh marker counts as work on another computer.
//!
//! The names in the folder come from the file system and are compared
//! exactly with the names that Longhand writes. A name in another case, a
//! conflict copy, a subfolder or a link keeps the note. The removal deletes
//! only the files that the check accepted, one at a time, and then the
//! empty folder. It never deletes a folder recursively.
//!
//! Before the first file goes, the row records the folder ([`REMOVING`]).
//! When a removal stops after `notes.md` went, the next removal finds that
//! record, and a missing `notes.md` in that folder does not keep the note.
//! `session.md` must still be the file that this computer last wrote.
//!
//! The main window can show the note again while the removal runs. The
//! caller then gives a `shown` check, and the note stays while it is true.
//! The removal checks it first and again at the last step before it
//! deletes a file or the row.
//!
//! The early write job of the note stops before the check
//! ([`EarlyHomes::cancel`]), and the removal holds the note guard, a delete
//! claim and the store lock while it checks and removes. A notes save takes
//! the store lock, so it comes before the check or finds no row.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, FixedOffset};
use library::scan::{READ_TIMEOUT, Read, read_timed};
use session_core::folder;
use session_core::jobs::{Claims, MoveStep, RECORDED_INTO, TitleSource, WrittenFingerprints};
use session_core::{SessionState, Store};
use session_save::{RealFs, fsync, naming};

use crate::early_home::{self, EarlyHomes, EarlyState, Location};
use crate::in_use::{is_marker_name, marker_fresh, marker_name, remove_marker};
use crate::save::{NOTES_FILE, SESSION_FILE};
use crate::{SharedStore, StageError, lock};

/// The `result_state` key that records the folder whose files the removal
/// of an empty note started to delete. The row and this record go
/// together.
pub const REMOVING: &str = "empty-note-removal";

/// Why an empty note stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// The note has no row.
    NoNote,
    /// The note has audio, text in its notes, or a title from the user.
    NotEmpty,
    /// Another computer started the note.
    StartedElsewhere,
    /// The note existed before a recording that was discarded (R19).
    ExistedBefore,
    /// The note records or is processed, or a delete, a Move, a summary or
    /// a folder rename works on it.
    Busy,
    /// The storage folder is not available, or a file of the note cannot be
    /// read. The launch pass tries again at the next launch.
    Unavailable,
    /// `session.md` or `notes.md` is not the file that this computer last
    /// wrote: another computer can have changed it.
    Changed,
    /// An in-use marker of another run is fresh.
    InUse,
    /// The folder holds a file that is not a file of the note, or it is not
    /// a real folder.
    OtherFiles,
    /// The main window shows the note again.
    Shown,
}

/// What [`Removal::leave`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leave {
    /// The note and its home folder are gone.
    Removed,
    /// The note stays.
    Kept(Kept),
}

/// The removal of empty notes, with what it needs from the run.
pub struct Removal<'a> {
    pub store: &'a SharedStore,
    pub claims: &'a Claims,
    /// The early write jobs. The removal stops the job of the note first,
    /// and starts it again when the note stays.
    pub homes: Option<&'a Arc<EarlyHomes>>,
    /// The storage folder now. A note whose early write has not placed its
    /// staging folder yet has it there.
    pub destination: Option<&'a Path>,
    /// This run, which names its own in-use marker.
    pub run_id: &'a str,
    /// The earlier runs on this computer. Their markers are this
    /// computer's own work, so they do not keep the note.
    pub earlier_runs: &'a [String],
}

/// The folder of the note that the removal checks.
enum Target {
    /// No file of the note is in the storage folder.
    Nothing,
    /// The staging folder or the home folder of the note.
    Folder(PathBuf),
}

impl Removal<'_> {
    /// The user left the note `id`: the main window shows another view or
    /// closes, or the user starts a new note. Remove the note and its home
    /// folder when it is empty and every check passes (R18). `now` is the
    /// time for the freshness of the in-use markers.
    pub fn leave(&self, id: &str, now: DateTime<FixedOffset>) -> Result<Leave, StageError> {
        self.leave_unless_shown(id, now, &|_| false)
    }

    /// [`Removal::leave`], and the note stays while `shown(id)` is true:
    /// the main window shows the note again.
    pub fn leave_unless_shown(
        &self,
        id: &str,
        now: DateTime<FixedOffset>,
        shown: &dyn Fn(&str) -> bool,
    ) -> Result<Leave, StageError> {
        if shown(id) {
            return Ok(Leave::Kept(Kept::Shown));
        }
        // A first look: a note that is not empty keeps its early write job.
        if let Some(kept) = self.kept_by_row(&lock(self.store), id)? {
            return Ok(Leave::Kept(kept));
        }
        // After this, the job writes nothing more (KTD4). It takes the note
        // guard, so it stops before the guard is taken here.
        if let Some(homes) = self.homes {
            homes.cancel(id);
        }
        let left = self.leave_stopped(id, now, shown);
        // A note that stays continues its early write and its flag update.
        if let Some(homes) = self.homes
            && !matches!(left, Ok(Leave::Removed))
        {
            homes.start(id);
        }
        left
    }

    /// At launch, before the early writes continue: [`Removal::leave`] for
    /// each empty note that this computer started before `opened_before`,
    /// the start of this run. A note that opened in this run can be on the
    /// page. Returns what the pass did for each note, sorted by id.
    pub fn launch_pass(
        &self,
        opened_before: DateTime<FixedOffset>,
        now: DateTime<FixedOffset>,
    ) -> Result<Vec<(String, Leave)>, StageError> {
        let mut ids = Vec::new();
        {
            let store = lock(self.store);
            for record in store.sessions()? {
                if record.state == SessionState::Saved
                    && record.created_at < opened_before
                    && self.kept_by_row(&store, &record.id)?.is_none()
                {
                    ids.push(record.id);
                }
            }
        }
        ids.sort();
        let mut done = Vec::new();
        for id in ids {
            match self.leave(&id, now) {
                Ok(left) => done.push((id, left)),
                Err(e) => log::warn!("could not check the empty note {id}: {e}"),
            }
        }
        Ok(done)
    }

    /// Why the row of `id` keeps the note, or `None` when the row allows
    /// the removal.
    fn kept_by_row(&self, store: &Store, id: &str) -> Result<Option<Kept>, StageError> {
        if self.claims.busy(id) {
            return Ok(Some(Kept::Busy));
        }
        kept_by_row(store, id)
    }

    fn leave_stopped(
        &self,
        id: &str,
        now: DateTime<FixedOffset>,
        shown: &dyn Fn(&str) -> bool,
    ) -> Result<Leave, StageError> {
        // A summary, a Move and a delete cannot start while the removal
        // holds this claim.
        let Ok(_claim) = self.claims.delete(id) else {
            return Ok(Leave::Kept(Kept::Busy));
        };
        let _guard = early_home::guard(id);
        let target = match self.target(id)? {
            Ok(target) => target,
            Err(kept) => return Ok(Leave::Kept(kept)),
        };
        let mut store = lock(self.store);
        // The row can change before the store lock: check it again.
        if let Some(kept) = kept_by_row(&store, id)? {
            return Ok(Leave::Kept(kept));
        }
        let checked = match &target {
            Target::Folder(folder) => {
                // A library refresh can give the row another home path
                // before the store lock. A staging folder has the note id in
                // its name.
                let home = store.session(id)?.and_then(|r| r.home_path);
                let staging = folder.file_name().and_then(|n| n.to_str())
                    == Some(naming::staging_name(id).as_str());
                if !staging && home.as_deref().map(Path::new) != Some(folder.as_path()) {
                    return Ok(Leave::Kept(Kept::Changed));
                }
                let written = store.written_fingerprints(id)?;
                let removing = store.result_state(id, REMOVING)?
                    == Some(serde_json::Value::from(folder.to_string_lossy()));
                match check_folder(
                    folder,
                    &written,
                    removing,
                    self.run_id,
                    self.earlier_runs,
                    now,
                ) {
                    Ok(files) => Some((folder, files)),
                    Err(kept) => {
                        log::info!("the empty note {id} stays: {kept:?}");
                        return Ok(Leave::Kept(kept));
                    }
                }
            }
            Target::Nothing => None,
        };
        // The last step before a file or the row goes.
        if shown(id) {
            return Ok(Leave::Kept(Kept::Shown));
        }
        if let Some((folder, files)) = checked {
            store.set_result_state(
                id,
                REMOVING,
                &serde_json::Value::from(folder.to_string_lossy()),
            )?;
            remove_folder(folder, &files)?;
        }
        // The early write state goes first: the store does not forget a row
        // whose early write is not done.
        store.clear_result_state(id, early_home::STATE)?;
        if !store.forget(id)? {
            log::error!("the empty note {id} has no folder now, and its row stays");
            return Err(StageError::Failed(format!(
                "the row of the empty note {id} could not be removed"
            )));
        }
        log::info!("removed the empty note {id}");
        Ok(Leave::Removed)
    }

    /// The folder of `id` in the storage folder, from its early write
    /// state. The caller holds the note guard, and the early write job of
    /// the note is stopped. This takes the store lock only for short reads,
    /// because the steps below take it themselves.
    fn target(&self, id: &str) -> Result<Result<Target, Kept>, StageError> {
        let (state, home) = {
            let store = lock(self.store);
            let Some(record) = store.session(id)? else {
                return Ok(Err(Kept::NoNote));
            };
            (
                early_home::state(&store, id)?,
                record.home_path.map(PathBuf::from),
            )
        };
        match state {
            Some(EarlyState::Begun) => {
                // A staging folder of this note, if one is there, is in the
                // storage folder now.
                let Some(root) = self.destination else {
                    return Ok(Err(Kept::Unavailable));
                };
                if session_save::check_destination(root).is_err() {
                    return Ok(Err(Kept::Unavailable));
                }
                let staging = root.join(naming::staging_name(id));
                Ok(Ok(if early_home::exists(&staging) {
                    Target::Folder(staging)
                } else {
                    Target::Nothing
                }))
            }
            Some(EarlyState::Filled {
                root,
                folder_name,
                note_type,
            }) => {
                if session_save::check_destination(&root).is_err() {
                    return Ok(Err(Kept::Unavailable));
                }
                let staging = root.join(naming::staging_name(id));
                if early_home::exists(&staging) {
                    return Ok(Ok(Target::Folder(staging)));
                }
                // The rename to the final name happened and its record did
                // not. This step only finds the folder; it writes nothing.
                let filled = EarlyState::Filled {
                    root,
                    folder_name,
                    note_type,
                };
                let next = early_home::step(
                    self.store,
                    id,
                    &filled,
                    self.destination,
                    self.run_id,
                    &RealFs,
                )?;
                early_home::record(self.store, id, &next)?;
                match next {
                    EarlyState::Done { path } => self.home(id, &path),
                    // No folder holds the note: nothing is in the storage
                    // folder.
                    _ => Ok(Ok(Target::Nothing)),
                }
            }
            Some(EarlyState::Done { .. }) | None => match home {
                Some(home) => self.home(id, &home),
                None => Ok(Ok(Target::Nothing)),
            },
        }
    }

    /// The home folder of `id` at `home`. A folder at another name becomes
    /// the home path. When no folder in the storage folder holds the note,
    /// nothing of it is there.
    fn home(&self, id: &str, home: &Path) -> Result<Result<Target, Kept>, StageError> {
        Ok(match early_home::locate(self.store, id, home)? {
            Location::There => Ok(Target::Folder(home.to_path_buf())),
            Location::Moved => match lock(self.store).session(id)?.and_then(|r| r.home_path) {
                Some(moved) => Ok(Target::Folder(PathBuf::from(moved))),
                None => Err(Kept::NoNote),
            },
            Location::Gone => Ok(Target::Nothing),
            Location::Unknown => Err(Kept::Unavailable),
        })
    }
}

/// Why the row of `id` keeps the note, or `None` when the row allows the
/// removal (R18, R19).
fn kept_by_row(store: &Store, id: &str) -> Result<Option<Kept>, StageError> {
    let Some(record) = store.session(id)? else {
        return Ok(Some(Kept::NoNote));
    };
    if record.state != SessionState::Saved {
        return Ok(Some(Kept::Busy));
    }
    if !store.segments(id)?.is_empty() {
        return Ok(Some(Kept::NotEmpty));
    }
    if !store.started_here(id)? {
        return Ok(Some(Kept::StartedElsewhere));
    }
    if store.result_state(id, RECORDED_INTO)?.is_some() {
        return Ok(Some(Kept::ExistedBefore));
    }
    let titled = record
        .title
        .as_ref()
        .is_some_and(|t| t.source == TitleSource::User);
    if titled || !store.notes(id)?.trim().is_empty() {
        return Ok(Some(Kept::NotEmpty));
    }
    let moving = !matches!(store.move_step(id)?, None | Some(MoveStep::Done));
    if moving || store.rename_intent(id)?.is_some() {
        return Ok(Some(Kept::Busy));
    }
    Ok(None)
}

/// Check the folder of an empty note (R18). Returns the files to delete:
/// the old in-use markers, then `notes.md`, then `session.md`. This run's
/// own marker goes first. A marker of one of the `earlier_runs` on this
/// computer is an old marker, also when it is fresh. `removing` is true
/// when an earlier removal of this note started to delete the files of
/// this folder: a missing `notes.md` then does not keep the note.
fn check_folder(
    folder: &Path,
    written: &WrittenFingerprints,
    removing: bool,
    run_id: &str,
    earlier_runs: &[String],
    now: DateTime<FixedOffset>,
) -> Result<Vec<PathBuf>, Kept> {
    let earlier: Vec<String> = earlier_runs.iter().map(|run| marker_name(run)).collect();
    // `parent` is the storage folder, or a type folder in it (KTD4 of the
    // note types plan). The check of the storage folder goes to the
    // storage folder itself, so it writes nothing in a type folder.
    let Some(parent) = folder.parent() else {
        return Err(Kept::OtherFiles);
    };
    let storage = library::scan::storage_place(folder).map_or(parent.to_path_buf(), |p| p.root);
    if session_save::check_destination(&storage).is_err() {
        return Err(Kept::Unavailable);
    }
    // A real folder at its path in the storage folder. A link, for example
    // to another folder, resolves to another path.
    match folder.symlink_metadata() {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(Kept::OtherFiles),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Kept::Changed),
        Err(_) => return Err(Kept::Unavailable),
    }
    let (Ok(real), Ok(real_parent)) = (folder.canonicalize(), parent.canonicalize()) else {
        return Err(Kept::Unavailable);
    };
    if folder.file_name().map(|name| real_parent.join(name)) != Some(real) {
        return Err(Kept::OtherFiles);
    }
    remove_marker(folder, run_id).map_err(|_| Kept::Unavailable)?;
    let mut files = Vec::new();
    let mut session = false;
    let mut notes = false;
    let entries = std::fs::read_dir(folder).map_err(|_| Kept::Unavailable)?;
    for entry in entries {
        let entry = entry.map_err(|_| Kept::Unavailable)?;
        // The type of the entry itself: a link is not a file here.
        let is_file = entry.file_type().map_err(|_| Kept::Unavailable)?.is_file();
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|_| is_file) else {
            return Err(Kept::OtherFiles);
        };
        if name == SESSION_FILE {
            session = true;
        } else if name == NOTES_FILE {
            notes = true;
        } else if is_marker_name(name) {
            if !earlier.iter().any(|own| own == name) && marker_fresh(&entry.path(), now) {
                return Err(Kept::InUse);
            }
            files.push(entry.path());
        } else {
            return Err(Kept::OtherFiles);
        }
    }
    if !session || !(notes || removing) {
        return Err(Kept::Changed);
    }
    let mut checked = vec![(SESSION_FILE, &written.session)];
    if notes {
        checked.insert(0, (NOTES_FILE, &written.notes));
    }
    for (name, last) in checked {
        let current = match read_timed(folder.join(name), READ_TIMEOUT) {
            Read::Bytes(bytes) => folder::fingerprint(&bytes),
            Read::Missing => return Err(Kept::Changed),
            Read::Failed(_) => return Err(Kept::Unavailable),
        };
        if last.as_deref() != Some(current.as_str()) {
            return Err(Kept::Changed);
        }
    }
    if notes {
        files.push(folder.join(NOTES_FILE));
    }
    files.push(folder.join(SESSION_FILE));
    Ok(files)
}

/// Delete `files` from `folder`, then the empty folder. `session.md` is the
/// last file: until it goes, the folder is still a note. A file that came
/// after the check stays, with the folder. When a file cannot be deleted,
/// the row keeps its [`REMOVING`] record, and the next removal continues.
fn remove_folder(folder: &Path, files: &[PathBuf]) -> Result<(), StageError> {
    for file in files {
        match std::fs::remove_file(file) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                return Err(StageError::NoDestination(format!(
                    "could not remove {}: {e}",
                    file.display()
                )));
            }
            _ => {}
        }
    }
    if let Err(e) = std::fs::remove_dir(folder) {
        log::warn!(
            "the folder {} is not empty after the removal of its note: {e}",
            folder.display()
        );
    }
    // The folder that held the note: the storage folder or a type folder.
    if let Some(parent) = folder.parent()
        && let Err(e) = fsync::sync_dir(parent)
    {
        log::warn!("could not sync {}: {e}", parent.display());
    }
    Ok(())
}
