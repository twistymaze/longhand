//! Move (KTD9, KTD10, R5, R6, R10): put every note of the old storage
//! folder in the new one. Each note is a step-then-record state machine
//! with its row in the Move journal (`Store::open_move`):
//!
//! - Pending: the note is in the old folder only. On one volume, a rename
//!   puts it in the new folder (Placed). On different volumes, a verified
//!   copy goes into a `.longhand-move-<id>` folder in the new folder
//!   (Copied).
//! - Copied: a rename that never replaces puts the staged copy in the new
//!   folder, with a free name on a clash (Placed).
//! - Placed: the Move holds the note, so note edits and renames wait. The
//!   final check compares the source with the placed copy. When they
//!   differ, the source is copied again (Copied, which replaces the placed
//!   copy). When they are equal, the store's home path changes (Homed).
//! - Homed: the source is removed with no further check (Done).
//!
//! A note in a type folder goes into the type folder with the same name in
//! the new folder, which the Move makes when it is not there (KTD14 of the
//! note types plan). Staging folders stay at the top of the new folder.
//!
//! The Move records each step before it takes the next one, and each step
//! first checks its own effect on disk, so a crash at any point resumes
//! with one complete copy of each note. A marker file in the moved folder
//! tells the Move's own copy apart from any other folder. It stays until
//! the home change is recorded.
//!
//! The Move staging prefix is not `.longhand-staging-`, so the launch
//! staging sweep never removes a staged copy. The Move removes its own
//! leftovers when it resumes.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::MutexGuard;

use session_core::folder;
use session_core::jobs::{Claim, Claims};
use session_core::note_folder::NoteFiles;
use session_core::{MoveJournal, MovePause, MoveStep, SessionState, Store, StoreError};
use session_save::{Fs, fsync, naming, rename};

use crate::{SharedStore, folder_rename, lock};

/// The hidden folder in the new folder that holds a verified copy.
pub const STAGING_PREFIX: &str = ".longhand-move-";

/// Written into a moved folder before it goes into the new folder, with
/// the note id. It stays through the rename into place, so a crash before
/// that step is recorded finds the folder. It is removed after the home
/// change.
pub const MARKER: &str = ".longhand-moving";

/// The folder that finds whether the two folders are on one volume.
const PROBE: &str = ".longhand-move-probe";

/// Name attempts in the new folder before the Move pauses.
const MAX_NAMES: usize = 100;

pub fn staging_name(note_id: &str) -> String {
    format!("{STAGING_PREFIX}{note_id}")
}

/// The copy of one note folder into a new staging folder:
/// [`copy_note`], or a copy that a test makes fail.
pub type CopyFn<'a> = dyn Fn(&Path, &Path) -> io::Result<u64> + Send + Sync + 'a;

/// Copy the note folder `from` into the new folder `to` and verify each
/// file (`stage::copy_verified`).
pub fn copy_note(from: &Path, to: &Path) -> io::Result<u64> {
    session_save::stage::copy_verified(from, to, &[])
}

/// What one step did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Taken {
    /// Record this step, then take the next one.
    Next(MoveStep),
    /// The source is gone: another computer or the user moved it away. The
    /// note leaves the journal.
    Dropped,
    /// The note is busy (recording, Save, processing, a summary
    /// regeneration). It moves later.
    Busy,
}

#[derive(Debug, thiserror::Error)]
pub enum MoveError {
    /// The Move pauses with this cause. Retry continues it.
    #[error("the move is paused: {0:?}")]
    Paused(MovePause),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// How a pass of the Move ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassEnd {
    /// Every note is in the new folder, and the journal is closed. Also
    /// when no journal is open.
    Finished,
    /// A note is busy, or a save puts a note in the old folder: a later
    /// pass moves it.
    Waiting,
    Paused(MovePause),
}

/// The progress of the Move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub moved: usize,
    pub total: usize,
}

/// The state of the open Move, for Settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub journal: MoveJournal,
    pub progress: Progress,
}

/// The work of the Move, with the filesystem operations that a test can
/// change.
pub struct Mover<'a> {
    pub store: &'a SharedStore,
    pub claims: &'a Claims,
    pub fs: &'a dyn Fs,
    pub copy: &'a CopyFn<'a>,
}

/// Open the Move journal from `old` to `new`. Returns false when a Move
/// runs or is paused (KTD11).
pub fn open(store: &SharedStore, old: &Path, new: &Path) -> Result<bool, StoreError> {
    let opened = lock(store).open_move(old, new)?;
    if opened {
        log::info!(
            "move: the notes go from {} to {}",
            old.display(),
            new.display()
        );
    }
    Ok(opened)
}

/// Continue a paused Move (Retry).
pub fn retry(store: &SharedStore) -> Result<(), StoreError> {
    lock(store).set_move_pause(None)
}

/// The open Move, or `None`.
pub fn status(store: &Store) -> Result<Option<Status>, StoreError> {
    let Some(journal) = store.move_journal()? else {
        return Ok(None);
    };
    Ok(Some(Status {
        journal,
        progress: progress(store)?,
    }))
}

fn progress(store: &Store) -> Result<Progress, StoreError> {
    let notes = store.move_notes()?;
    Ok(Progress {
        moved: notes.iter().filter(|(_, s)| *s == MoveStep::Done).count(),
        total: notes.len(),
    })
}

/// The folders that the library lists notes from: the storage folder and,
/// while the Move journal is open, the old folder (KTD10).
pub fn library_roots(store: &Store, storage: Option<PathBuf>) -> Vec<PathBuf> {
    let old = match store.move_journal() {
        Ok(journal) => journal.map(|j| j.old_folder),
        Err(e) => {
            log::error!("move: cannot read the journal: {e}");
            None
        }
    };
    let mut roots: Vec<PathBuf> = storage.into_iter().collect();
    if let Some(old) = old
        && !roots.contains(&old)
    {
        roots.push(old);
    }
    roots
}

/// The store lock, when the Move does not hold the note `note_id`. A
/// note edit or a rename takes the lock this way, so it waits for the
/// Move's final check and home change (KTD9). The Move takes its hold under
/// the store lock, so it cannot start while the caller holds the lock.
pub fn lock_unheld<'a>(
    store: &'a SharedStore,
    claims: &Claims,
    note_id: &str,
) -> MutexGuard<'a, Store> {
    loop {
        let guard = lock(store);
        if !claims.is_held(note_id) {
            return guard;
        }
        drop(guard);
        claims.wait_unheld(note_id);
    }
}

fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

/// Pause the Move when the old folder is not reachable. A signed-out sync
/// client, a removed disk or a lost share can remove the files and the
/// marker: the sources then look gone, and are not. Each step checks this
/// before it drops a note whose source is gone.
fn check_old(old: &Path) -> Result<(), MoveError> {
    if old.is_dir() && library::scan::marker(old).exists() {
        return Ok(());
    }
    Err(MoveError::Paused(MovePause::NotReachable {
        folder: old.to_string_lossy().into_owned(),
    }))
}

/// Copy each entry of the import record of the old folder into the new
/// folder (R11 of the audio import plan). The merge never replaces an
/// entry, so a second merge after a crash copies only the rest. An old
/// folder that is not reachable pauses the Move: its record is not empty,
/// only not readable now. An entry that cannot be read or does not parse
/// stays in the old folder, and the log names it.
fn merge_imports(journal: &MoveJournal) -> Result<(), MoveError> {
    let (old, new) = (&journal.old_folder, &journal.new_folder);
    check_old(old)?;
    let merged = import_record::merge(old, new).map_err(|e| pause_for(&e, new))?;
    for path in &merged.not_copied {
        log::warn!(
            "move: cannot copy the import record entry {}",
            path.display()
        );
    }
    if merged.copied > 0 {
        log::info!("move: copied {} import record entries", merged.copied);
    }
    Ok(())
}

/// The pause for an I/O error on `folder`.
fn pause_for(e: &io::Error, folder: &Path) -> MoveError {
    let text = folder.to_string_lossy().into_owned();
    let pause = if matches!(
        e.kind(),
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded
    ) {
        MovePause::DiskFull { folder: text }
    } else if !folder.is_dir() {
        MovePause::NotReachable { folder: text }
    } else {
        MovePause::Failed {
            message: e.to_string(),
        }
    };
    MoveError::Paused(pause)
}

/// Whether `path` is this note's folder in the new folder: it has the
/// Move's marker with the note id, or it is the same note (its
/// `longhand.json`, or an old-layout `session.md`, has the note id), so a
/// second copy is never placed.
fn is_ours(path: &Path, note_id: &str) -> bool {
    moving_copy(path, note_id)
        || NoteFiles::read(path).is_ok_and(|files| files.doc.note_id == note_id)
}

/// Whether `path` has the Move's marker with the note id: it is a copy
/// that this Move made and has not homed yet.
fn moving_copy(path: &Path, note_id: &str) -> bool {
    std::fs::read_to_string(path.join(MARKER)).is_ok_and(|m| m.trim() == note_id)
}

/// The note's folder in the new folder, among the names that a rename
/// into place can give it.
fn find_placed(new: &Path, name: &str, note_id: &str) -> Option<PathBuf> {
    (1..=MAX_NAMES)
        .map(|attempt| new.join(naming::candidate(name, attempt)))
        .find(|path| path.is_dir() && is_ours(path, note_id))
}

/// The folder in `new` that takes the note whose folder is `source` in
/// `old`: the type folder with the same name, or `new` itself for a note
/// of General (KTD14 of the note types plan). `None` when `source` is
/// not in `old`, directly or in a type folder.
fn target_folder(old: &Path, new: &Path, source: &Path) -> Option<PathBuf> {
    folder::note_type_in(old, source).map(|note_type| match note_type {
        Some(name) => new.join(name),
        None => new.to_path_buf(),
    })
}

/// Make the type folder `target` in the new folder when it is not there.
/// The new folder itself is never made: a new folder that went away
/// fails here.
fn make_target(new: &Path, target: &Path) -> io::Result<()> {
    if target == new || target.is_dir() {
        return Ok(());
    }
    match std::fs::create_dir(target) {
        Ok(()) => fsync::sync_dir(new),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Remove a copy that this Move made, its marker last. A removal that
/// stops part way keeps the marker, so the next step still knows the
/// folder as this Move's copy and removes the rest; it is never taken for
/// a finished note.
fn remove_copy(dir: &Path) -> io::Result<()> {
    session_save::remove_marker_last(dir, MARKER)
}

fn write_marker(dir: &Path, note_id: &str) -> io::Result<()> {
    std::fs::write(dir.join(MARKER), note_id)?;
    fsync::sync_file(&dir.join(MARKER))?;
    fsync::sync_dir(dir)
}

fn remove_marker(dir: &Path) -> io::Result<()> {
    match std::fs::remove_file(dir.join(MARKER)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    fsync::sync_dir(dir)
}

/// Whether the folders `a` and `b` hold the same files with the same
/// bytes. The Move's marker and the in-use markers do not count: another
/// computer refreshes its in-use marker while the Move runs (KTD4 of the
/// audio removal plan). Links are not followed, as the copy does not
/// follow them.
pub fn same_contents(a: &Path, b: &Path) -> io::Result<bool> {
    fn listing(dir: &Path, top: bool) -> io::Result<Vec<(std::ffi::OsString, bool)>> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name();
            if top && (name == MARKER || crate::in_use::is_marker_name(&name.to_string_lossy())) {
                continue;
            }
            let kind = entry.file_type()?;
            if kind.is_dir() {
                out.push((name, true));
            } else if kind.is_file() {
                out.push((name, false));
            }
        }
        out.sort();
        Ok(out)
    }
    fn walk(a: &Path, b: &Path, top: bool) -> io::Result<bool> {
        let (left, right) = (listing(a, top)?, listing(b, top)?);
        if left != right {
            return Ok(false);
        }
        for (name, is_dir) in left {
            let (x, y) = (a.join(&name), b.join(&name));
            let same = if is_dir {
                walk(&x, &y, false)?
            } else {
                same_file(&x, &y)?
            };
            if !same {
                return Ok(false);
            }
        }
        Ok(true)
    }
    walk(a, b, true)
}

fn same_file(a: &Path, b: &Path) -> io::Result<bool> {
    use std::io::Read;
    if std::fs::metadata(a)?.len() != std::fs::metadata(b)?.len() {
        return Ok(false);
    }
    let (mut x, mut y) = (std::fs::File::open(a)?, std::fs::File::open(b)?);
    let (mut bx, mut by) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        let n = x.read(&mut bx)?;
        if n == 0 {
            return Ok(y.read(&mut by[..1])? == 0);
        }
        let mut read = 0;
        while read < n {
            let m = y.read(&mut by[read..n])?;
            if m == 0 {
                return Ok(false);
            }
            read += m;
        }
        if bx[..n] != by[..n] {
            return Ok(false);
        }
    }
}

impl Mover<'_> {
    /// Make the journal ready for a pass: both folders are reachable, the
    /// journal knows whether they are on different volumes, and the
    /// staging folders that no Copied step names are removed. Returns the
    /// journal. A folder that is not reachable pauses the Move.
    pub fn prepare(&self) -> Result<Option<MoveJournal>, MoveError> {
        let Some(mut journal) = lock(self.store).move_journal()? else {
            return Ok(None);
        };
        let (old, new) = (journal.old_folder.clone(), journal.new_folder.clone());
        if !lock(self.store).moving_notes()?.is_empty() {
            check_old(&old)?;
        }
        if let Err(e) = session_save::check_destination(&new) {
            log::warn!("move: {e}");
            return Err(MoveError::Paused(MovePause::NotReachable {
                folder: new.to_string_lossy().into_owned(),
            }));
        }
        if journal.cross_volume.is_none() {
            let cross = self.probe(&old, &new)?;
            lock(self.store).set_move_cross_volume(cross)?;
            journal.cross_volume = Some(cross);
        }
        self.remove_leftovers(&new)?;
        Ok(Some(journal))
    }

    /// Whether `old` and `new` are on different volumes: a rename of an
    /// empty folder from one to the other says so.
    fn probe(&self, old: &Path, new: &Path) -> Result<bool, MoveError> {
        let (from, to) = (old.join(PROBE), new.join(PROBE));
        for path in [&from, &to] {
            if exists(path) {
                std::fs::remove_dir_all(path).map_err(|e| pause_for(&e, old))?;
            }
        }
        std::fs::create_dir(&from).map_err(|e| pause_for(&e, old))?;
        let cross = match self.fs.rename_new(&from, &to) {
            Ok(()) => {
                let _ = std::fs::remove_dir(&to);
                false
            }
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                let _ = std::fs::remove_dir(&from);
                true
            }
            Err(e) => {
                let _ = std::fs::remove_dir(&from);
                return Err(pause_for(&e, new));
            }
        };
        log::info!(
            "move: the folders are on {}",
            if cross {
                "different volumes"
            } else {
                "one volume"
            }
        );
        Ok(cross)
    }

    /// Remove each Move staging folder in `new` that no Copied step names:
    /// a copy that a crash stopped, or a second copy after a final check.
    fn remove_leftovers(&self, new: &Path) -> Result<(), MoveError> {
        let kept: Vec<PathBuf> = lock(self.store)
            .move_notes()?
            .into_iter()
            .filter_map(|(_, step)| match step {
                MoveStep::Copied { staging, .. } => Some(staging),
                _ => None,
            })
            .collect();
        let entries = std::fs::read_dir(new).map_err(|e| pause_for(&e, new))?;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let ours = name.to_str().is_some_and(|n| n.starts_with(STAGING_PREFIX));
            let path = entry.path();
            if ours && !kept.contains(&path) {
                std::fs::remove_dir_all(&path).map_err(|e| pause_for(&e, new))?;
                log::info!("move: removed the leftover {}", path.display());
            }
        }
        Ok(())
    }

    /// Add a Pending row for each note whose home folder is in the old
    /// folder and has no row: a note that a resumed Save put there, or one
    /// that another computer added. Returns whether a row was added.
    fn rescan(&self, journal: &MoveJournal) -> Result<bool, MoveError> {
        let store = lock(self.store);
        let mut added = false;
        for record in store.sessions()? {
            let Some(home) = record.home_path.as_deref().map(Path::new) else {
                continue;
            };
            if folder::note_type_in(&journal.old_folder, home).is_some()
                && home.is_dir()
                && store.add_move_note(&record.id)?
            {
                log::info!("move: {} is in the old folder too", record.id);
                added = true;
            }
        }
        Ok(added)
    }

    /// One pass of the Move: every note that is not busy goes to the new
    /// folder. `progress` receives the note id and the progress after each
    /// note.
    pub fn pass(&self, progress: &mut dyn FnMut(&str, Progress)) -> Result<PassEnd, StoreError> {
        match self.pass_inner(progress) {
            Ok(end) => Ok(end),
            Err(MoveError::Paused(pause)) => {
                log::warn!("move: paused: {pause:?}");
                lock(self.store).set_move_pause(Some(&pause))?;
                Ok(PassEnd::Paused(pause))
            }
            Err(MoveError::Store(e)) => Err(e),
        }
    }

    fn pass_inner(&self, report: &mut dyn FnMut(&str, Progress)) -> Result<PassEnd, MoveError> {
        if let Some(journal) = lock(self.store).move_journal()?
            && let Some(pause) = journal.pause
        {
            return Ok(PassEnd::Paused(pause));
        }
        let Some(journal) = self.prepare()? else {
            return Ok(PassEnd::Finished);
        };
        loop {
            self.rescan(&journal)?;
            let mut waiting = false;
            let ids = lock(self.store).moving_notes()?;
            for id in ids {
                match self.note(&journal, &id)? {
                    Taken::Busy => waiting = true,
                    Taken::Next(_) | Taken::Dropped => {}
                }
                report(&id, progress(&lock(self.store))?);
            }
            // A note that a resumed Save put in the old folder during the
            // pass moves too (KTD10).
            if self.rescan(&journal)? && !waiting {
                continue;
            }
            if waiting || !self.all_moved(&journal)? {
                return Ok(PassEnd::Waiting);
            }
            // The import record goes with the notes (R11 of the audio
            // import plan), before the journal closes. A crash before the
            // close continues the merge at the next pass.
            merge_imports(&journal)?;
            let mut store = lock(self.store);
            if !store.moving_notes()?.is_empty()
                || !store.saves_into(&journal.old_folder)?.is_empty()
            {
                // A note came into the old folder during the merge.
                continue;
            }
            store.close_move()?;
            log::info!("move: every note is in {}", journal.new_folder.display());
            return Ok(PassEnd::Finished);
        }
    }

    /// Whether every note is in the new folder, and no save puts a note in
    /// the old folder.
    fn all_moved(&self, journal: &MoveJournal) -> Result<bool, MoveError> {
        let store = lock(self.store);
        Ok(store.moving_notes()?.is_empty() && store.saves_into(&journal.old_folder)?.is_empty())
    }

    /// Take the steps of one note until it is done, busy or dropped.
    fn note(&self, journal: &MoveJournal, id: &str) -> Result<Taken, MoveError> {
        let Some(_moving) = self.claims.moving(id) else {
            return Ok(Taken::Busy);
        };
        let mut hold: Option<Claim> = None;
        loop {
            let Some(step) = lock(self.store).move_step(id)? else {
                // The note was deleted.
                return Ok(Taken::Dropped);
            };
            if step == MoveStep::Done {
                return Ok(Taken::Next(MoveStep::Done));
            }
            if self.needs_hold(journal, &step) {
                if hold.is_none() {
                    hold = self.take_hold(id)?;
                    if hold.is_none() {
                        return Ok(Taken::Busy);
                    }
                }
            } else {
                hold = None;
            }
            match self.step(journal, id, &step)? {
                Taken::Next(next) => {
                    lock(self.store).set_move_step(id, &next)?;
                    if next == MoveStep::Done {
                        drop(hold.take());
                        self.rename_owed(id);
                        log::info!("move: {id} is in the new folder");
                    }
                }
                Taken::Dropped => {
                    lock(self.store).drop_move_note(id)?;
                    log::info!("move: the folder of {id} is gone, so it leaves the move");
                    return Ok(Taken::Dropped);
                }
                Taken::Busy => return Ok(Taken::Busy),
            }
        }
    }

    /// The steps that the Move takes while it holds the note: from the
    /// final check to the home change, and a rename on one volume, which
    /// moves the folder and changes the home path together.
    fn needs_hold(&self, journal: &MoveJournal, step: &MoveStep) -> bool {
        match step {
            MoveStep::Pending => journal.cross_volume == Some(false),
            MoveStep::Copied { replaces, .. } => replaces.is_some(),
            MoveStep::Placed { .. } => true,
            MoveStep::Homed { .. } | MoveStep::Done => false,
        }
    }

    /// Hold the note for the final check and the home change, when it is
    /// saved and free. Under the store lock, so an edit that holds the lock
    /// ends first, and a later edit waits.
    fn take_hold(&self, id: &str) -> Result<Option<Claim>, MoveError> {
        let store = lock(self.store);
        let free = store
            .session(id)?
            .is_some_and(|r| r.state == SessionState::Saved && r.home_path.is_some());
        Ok(if free { self.claims.hold(id) } else { None })
    }

    /// The folder rename that a rename during the Move owes the note: the
    /// folder takes the name its title gives now. A summary regeneration or
    /// a delete can claim the note after the Move releases its hold; the
    /// rename then waits for it (KTD8). The Move's own claim does not count.
    fn rename_owed(&self, id: &str) {
        let store = lock(self.store);
        let owed = store
            .owed_renames()
            .is_ok_and(|ids| ids.iter().any(|owed| owed == id));
        let claimed = self.claims.is_regenerating_or_deleting(id);
        if owed && let Err(e) = folder_rename::reconcile(&store, id, claimed, true, self.fs) {
            log::warn!("move: could not rename the folder of {id}: {e}");
        }
    }

    /// Take one step of the note `id` from `step`. Each step first checks
    /// whether its effect is on disk, so a step whose record a crash lost
    /// gives the same result again. The caller records the result.
    pub fn step(
        &self,
        journal: &MoveJournal,
        id: &str,
        step: &MoveStep,
    ) -> Result<Taken, MoveError> {
        let (old, new) = (&journal.old_folder, &journal.new_folder);
        match step {
            MoveStep::Pending => self.pending(journal, id),
            MoveStep::Copied {
                source,
                staging,
                replaces,
            } => {
                let name = folder_name(source);
                // A source that is not in the old folder came from a record
                // of a step before type folders: it is in General.
                let target = target_folder(old, new, source).unwrap_or_else(|| new.clone());
                if !exists(staging) {
                    // The rename into place happened and its record did not:
                    // the marker, kept through the rename, finds the folder.
                    if let Some(path) = find_placed(&target, &name, id) {
                        return Ok(Taken::Next(MoveStep::Placed {
                            source: Some(source.clone()),
                            path,
                        }));
                    }
                    if exists(source) {
                        return Ok(Taken::Next(MoveStep::Pending));
                    }
                    check_old(old)?;
                    return Ok(Taken::Dropped);
                }
                if let Some(stale) = replaces
                    && exists(stale)
                    && is_ours(stale, id)
                {
                    std::fs::remove_dir_all(stale).map_err(|e| pause_for(&e, new))?;
                    fsync::sync_dir(stale.parent().unwrap_or(new))
                        .map_err(|e| pause_for(&e, new))?;
                }
                make_target(new, &target).map_err(|e| pause_for(&e, new))?;
                let path = self.place(staging, &target, &name)?;
                Ok(Taken::Next(MoveStep::Placed {
                    source: Some(source.clone()),
                    path,
                }))
            }
            MoveStep::Placed { source, path } => {
                let store = lock(self.store);
                let Some(record) = store.session(id)? else {
                    return Ok(Taken::Dropped);
                };
                let homed = MoveStep::Homed {
                    source: source.clone(),
                    path: path.clone(),
                };
                if record.home_path.as_deref().map(Path::new) == Some(path.as_path()) {
                    return Ok(Taken::Next(homed));
                }
                drop(store);
                if !exists(path) {
                    // The placed copy went away: the source is the copy.
                    return Ok(match source {
                        Some(source) if exists(source) => Taken::Next(MoveStep::Pending),
                        Some(_) => {
                            check_old(old)?;
                            Taken::Dropped
                        }
                        None => Taken::Dropped,
                    });
                }
                if let Some(source) = source
                    && !exists(source)
                    && moving_copy(path, id)
                {
                    // The source went after the copy: a person or another
                    // computer deleted or moved it, and an edit after the
                    // copy can be gone with it. The copy is not homed
                    // without the final check. A folder with no marker is
                    // not this Move's copy: it is a finished note that
                    // another computer moved here, and it is homed.
                    check_old(old)?;
                    remove_copy(path).map_err(|e| pause_for(&e, new))?;
                    fsync::sync_dir(path.parent().unwrap_or(new))
                        .map_err(|e| pause_for(&e, new))?;
                    return Ok(Taken::Dropped);
                }
                if let Some(source) = source
                    && exists(source)
                    && !same_contents(source, path).map_err(|e| pause_for(&e, new))?
                {
                    // An edit reached the source after the copy: copy again.
                    log::info!("move: {id} changed after its copy, so it is copied again");
                    let staging = self.copy(old, source, new, id)?;
                    return Ok(Taken::Next(MoveStep::Copied {
                        source: source.clone(),
                        staging,
                        replaces: Some(path.clone()),
                    }));
                }
                let mut store = lock(self.store);
                let saved = store
                    .session(id)?
                    .is_some_and(|r| r.state == SessionState::Saved);
                if !saved {
                    return Ok(Taken::Busy);
                }
                store.set_move_home(id, path, &homed)?;
                Ok(Taken::Next(homed))
            }
            MoveStep::Homed { source, path } => {
                remove_marker(path).map_err(|e| pause_for(&e, new))?;
                if let Some(source) = source
                    && source != path
                    && exists(source)
                {
                    std::fs::remove_dir_all(source).map_err(|e| pause_for(&e, old))?;
                    // The old folder, or the type folder in it.
                    fsync::sync_dir(source.parent().unwrap_or(old))
                        .map_err(|e| pause_for(&e, old))?;
                }
                Ok(Taken::Next(MoveStep::Done))
            }
            MoveStep::Done => Ok(Taken::Next(MoveStep::Done)),
        }
    }

    fn pending(&self, journal: &MoveJournal, id: &str) -> Result<Taken, MoveError> {
        let new = &journal.new_folder;
        let record = lock(self.store).session(id)?;
        let Some(record) = record else {
            return Ok(Taken::Dropped);
        };
        let Some(source) = record.home_path.as_deref().map(PathBuf::from) else {
            return Ok(Taken::Dropped);
        };
        // Only a note in the old folder moves, with its type folder (KTD14
        // of the note types plan). A note whose home is in the new folder
        // already is done.
        let Some(target) = target_folder(&journal.old_folder, new, &source) else {
            return Ok(if folder::note_type_in(new, &source).is_some() {
                Taken::Next(MoveStep::Done)
            } else {
                Taken::Dropped
            });
        };
        let name = folder_name(&source);
        if !exists(&source) {
            // A rename on one volume happened and its record did not; or
            // another computer moved the note into the new folder.
            if let Some(path) = find_placed(&target, &name, id) {
                return Ok(Taken::Next(MoveStep::Placed { source: None, path }));
            }
            check_old(&journal.old_folder)?;
            return Ok(Taken::Dropped);
        }
        if record.state != SessionState::Saved {
            return Ok(Taken::Busy);
        }
        // The note is in the new folder already: it gets no second copy.
        if let Some(path) = find_placed(&target, &name, id) {
            return Ok(Taken::Next(MoveStep::Placed {
                source: Some(source),
                path,
            }));
        }
        if journal.cross_volume == Some(false) {
            make_target(new, &target).map_err(|e| pause_for(&e, new))?;
            write_marker(&source, id).map_err(|e| pause_for(&e, &journal.old_folder))?;
            match self.rename_into(&source, &target, &name) {
                Ok(path) => return Ok(Taken::Next(MoveStep::Placed { source: None, path })),
                Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                    log::info!("move: {id} is on another volume, so it is copied");
                }
                Err(e) => {
                    let _ = remove_marker(&source);
                    return Err(pause_for(&e, new));
                }
            }
        }
        let staging = self.copy(&journal.old_folder, &source, new, id)?;
        Ok(Taken::Next(MoveStep::Copied {
            source,
            staging,
            replaces: None,
        }))
    }

    /// Copy `source` into a new Move staging folder in `new`, verified and
    /// with the marker. A partial copy is removed.
    fn copy(&self, old: &Path, source: &Path, new: &Path, id: &str) -> Result<PathBuf, MoveError> {
        let staging = new.join(staging_name(id));
        if exists(&staging) {
            std::fs::remove_dir_all(&staging).map_err(|e| pause_for(&e, new))?;
        }
        let copied = (self.copy)(source, &staging)
            .and_then(|_| write_marker(&staging, id))
            .and_then(|()| fsync::sync_dir(new));
        if let Err(e) = copied {
            let _ = std::fs::remove_dir_all(&staging);
            log::warn!("move: the copy of {id} failed: {e}");
            // A read error from an old folder that went away names that
            // folder. The source can be in a type folder in it (KTD14).
            let folder = match source.parent() {
                _ if !old.is_dir() => old,
                Some(parent) if !parent.is_dir() => parent,
                _ => new,
            };
            return Err(pause_for(&e, folder));
        }
        Ok(staging)
    }

    /// Put the staged copy in `new` with the first free name.
    fn place(&self, staging: &Path, new: &Path, name: &str) -> Result<PathBuf, MoveError> {
        self.rename_into(staging, new, name)
            .map_err(|e| pause_for(&e, new))
    }

    /// Rename `from` into `new` with the first free name that `name` gives.
    /// A rename never replaces a folder.
    fn rename_into(&self, from: &Path, new: &Path, name: &str) -> io::Result<PathBuf> {
        for attempt in 1..=MAX_NAMES {
            let target = new.join(naming::candidate(name, attempt));
            match rename::rename_new_waiting(self.fs, from, &target) {
                Ok(()) => {
                    fsync::sync_dir(new)?;
                    return Ok(target);
                }
                Err(e) if rename::is_name_taken(&e) => continue,
                // A folder in use is not a taken name (KTD8).
                Err(e) if rename::is_in_use(&e) => return Err(rename::in_use_error(from, &e)),
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::other(format!(
            "the new folder has no free name for {name} after {MAX_NAMES} tries"
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A saved note "a" with no audio whose folder in a storage folder has
    /// the name of its first title, and a rename to "Review" that the Move
    /// owes it. Returns the store, the storage folder and the home folder.
    fn owed_rename() -> (tempfile::TempDir, SharedStore, PathBuf) {
        let dest = tempfile::tempdir().unwrap();
        let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        {
            let mut s = lock(&store);
            let at = chrono::DateTime::parse_from_rfc3339("2026-09-27T00:00:00Z").unwrap();
            s.create_notes_only("a", at, None).unwrap();
            s.set_title("a", "Budget", session_core::TitleSource::User)
                .unwrap();
            crate::early_home::begin(&s, "a").unwrap();
        }
        let home =
            crate::early_home::write(&store, "a", Some(dest.path()), "run", &session_save::RealFs)
                .unwrap()
                .unwrap();
        let s = lock(&store);
        s.set_title("a", "Review", session_core::TitleSource::User)
            .unwrap();
        s.set_rename_owed("a", true).unwrap();
        drop(s);
        (dest, store, home)
    }

    fn mover<'a>(store: &'a SharedStore, claims: &'a Claims) -> Mover<'a> {
        Mover {
            store,
            claims,
            fs: &session_save::RealFs,
            copy: &copy_note,
        }
    }

    /// A summary regeneration that started after the Move released its
    /// hold keeps the folder name: the rename waits for it (KTD8).
    #[test]
    fn the_owed_rename_waits_for_a_summary_regeneration() {
        let (_dest, store, home) = owed_rename();
        let claims = Claims::default();
        let _regenerating = claims.regenerate("a").unwrap();
        mover(&store, &claims).rename_owed("a");
        assert!(home.is_dir(), "the folder keeps its name");
        assert_eq!(lock(&store).owed_renames().unwrap(), ["a"]);
    }

    #[test]
    fn the_owed_rename_renames_a_free_note() {
        let (_dest, store, home) = owed_rename();
        let claims = Claims::default();
        mover(&store, &claims).rename_owed("a");
        assert!(!home.exists());
        let record = lock(&store).session("a").unwrap().unwrap();
        assert!(record.home_path.unwrap().ends_with("Review"));
    }
}
