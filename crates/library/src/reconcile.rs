//! Reconcile (KTD6): apply a library scan to the store.
//!
//! - A note id with no row is adopted (KTD1).
//! - A known note is refreshed from its folder (KTD3), but only when its row
//!   is Saved and no stage has claimed it.
//! - A note in a new folder moves the row's home path there, whatever the
//!   row's state or claim, so the home path follows a rename by id (R8). A
//!   row with a rename intent on this computer keeps its home path: this
//!   computer moves that folder itself.
//! - A row whose folder is gone from an available Destination gets a
//!   pending delete. After [`DELETE_WAIT`] with no folder for its id, the
//!   row is forgotten, if it is Saved, unclaimed and has no rename intent
//!   (R10). Forget changes no folder.
//! - Rows in an unavailable Destination stay as they are (KD7, R16).
//!
//! The scan runs without the store. Reconcile runs under the store lock,
//! and reads a folder again before it writes from it, so a change that
//! this computer made after the scan is not undone.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use session_core::{Adoption, SessionRecord, SessionState, Store, StoreError};

use crate::refresh::{self, FolderContents};
use crate::scan::{LibraryScan, NoteFolder, READ_TIMEOUT, ScanOptions};

/// How long a row stays after its folder is gone from an available
/// Destination (R10). A sync can deliver a rename as a removed folder and
/// then a new one (AE4).
pub const DELETE_WAIT: Duration = Duration::from_secs(30);

/// The note folders that a scan read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Coverage {
    /// Every note folder in each Destination of the scan.
    Full,
    /// Only these note folders: a targeted scan. A row whose home folder is
    /// not one of them gets no pending delete from the scan.
    Folders(HashSet<PathBuf>),
}

impl Coverage {
    fn covers(&self, home: &Path) -> bool {
        match self {
            Coverage::Full => true,
            Coverage::Folders(folders) => folders.contains(home),
        }
    }
}

/// The rows that a reconcile changed, for the `LibraryChanged` event
/// (KTD10). Each list is sorted and has no repeated id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Adopted rows, refreshed rows and rows with a new home path.
    pub changed: Vec<String>,
    /// Forgotten rows.
    pub removed: Vec<String>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty()
    }
}

/// Applies scans to the store, and keeps the pending deletes between two
/// scans. The pending deletes stay in memory only (KTD8).
#[derive(Debug, Clone)]
pub struct Reconciler {
    /// When each pending delete started.
    pending: HashMap<String, Instant>,
    read_timeout: Duration,
}

impl Default for Reconciler {
    fn default() -> Self {
        Self {
            pending: HashMap::new(),
            read_timeout: READ_TIMEOUT,
        }
    }
}

impl Reconciler {
    /// A reconciler whose folder reads use the time limit of `options`.
    pub fn new(options: &ScanOptions) -> Self {
        Self {
            pending: HashMap::new(),
            read_timeout: options.read_timeout,
        }
    }

    /// Whether the row `id` has a pending delete.
    pub fn is_pending(&self, id: &str) -> bool {
        self.pending.contains_key(id)
    }

    /// The earliest time at which a pending delete can complete. A
    /// reconcile at or after this time, with a new scan, forgets that row.
    pub fn next_due(&self) -> Option<Instant> {
        self.pending
            .values()
            .min()
            .map(|since| *since + DELETE_WAIT)
    }

    /// Apply `scan` to `store`. `now` is the time of the scan. `claimed`
    /// tells whether a stage holds a claim on a session (a summary
    /// regeneration or a delete).
    pub fn reconcile(
        &mut self,
        store: &mut Store,
        scan: &LibraryScan,
        coverage: &Coverage,
        now: Instant,
        claimed: &dyn Fn(&str) -> bool,
    ) -> Result<Changes, StoreError> {
        let records: HashMap<String, SessionRecord> = store
            .sessions()?
            .into_iter()
            .map(|record| (record.id.clone(), record))
            .collect();
        let intents: HashSet<String> = store
            .rename_intents()?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        self.pending.retain(|id, _| records.contains_key(id));

        let mut changed = Vec::new();
        for note in &scan.notes {
            let id = &note.doc.session_id;
            self.pending.remove(id);
            let updated = match records.get(id) {
                None => adopt(store, note)?,
                Some(_) if intents.contains(id) => false,
                Some(record) => self.follow(store, record, note, claimed)?,
            };
            if updated {
                changed.push(id.clone());
            }
        }

        // A folder that exists but gives no note is not a delete (KTD5).
        let present: HashSet<&str> = scan
            .notes
            .iter()
            .map(|note| note.doc.session_id.as_str())
            .chain(
                scan.unreadable
                    .iter()
                    .chain(&scan.invalid)
                    .filter_map(|problem| problem.known_id.as_deref()),
            )
            .collect();
        let mut removed = Vec::new();
        for record in records.values() {
            let Some(home) = record.home_path.as_deref().map(Path::new) else {
                continue;
            };
            if present.contains(record.id.as_str()) || !coverage.covers(home) {
                continue;
            }
            if !home.parent().is_some_and(|root| scan.is_available(root)) {
                self.pending.remove(&record.id);
                continue;
            }
            let since = *self.pending.entry(record.id.clone()).or_insert(now);
            let due = now.saturating_duration_since(since) >= DELETE_WAIT;
            if due
                && record.state == SessionState::Saved
                && !claimed(&record.id)
                && !intents.contains(&record.id)
                && store.forget(&record.id)?
            {
                log::info!(
                    "library: the folder of {} is gone, so its row is removed",
                    record.id
                );
                self.pending.remove(&record.id);
                removed.push(record.id.clone());
            }
        }

        changed.sort();
        changed.dedup();
        removed.sort();
        Ok(Changes { changed, removed })
    }

    /// Follow the note's folder for a known row: a new home path, then the
    /// refresh. Returns whether the row changed.
    fn follow(
        &self,
        store: &mut Store,
        record: &SessionRecord,
        note: &NoteFolder,
        claimed: &dyn Fn(&str) -> bool,
    ) -> Result<bool, StoreError> {
        let id = record.id.as_str();
        let moved = record
            .home_path
            .as_deref()
            .is_some_and(|home| Path::new(home) != note.path);
        let refreshable =
            record.state == SessionState::Saved && record.home_path.is_some() && !claimed(id);
        let stale = refreshable && refresh::differs(store, id, &FolderContents::from_scan(note))?;
        if !moved && !stale {
            return Ok(false);
        }
        // Read the folder again: the scan ran without the store lock.
        let contents = match FolderContents::read(&note.path, self.read_timeout) {
            Ok(contents) if contents.doc.session_id == id => contents,
            Ok(_) => return Ok(false),
            Err(e) => {
                log::warn!("library: cannot refresh {id}: {e}");
                return Ok(false);
            }
        };
        if moved {
            log::info!("library: the folder of {id} is now {}", note.path.display());
            store.set_home_path(id, &note.path.to_string_lossy())?;
        }
        if !refreshable {
            return Ok(moved);
        }
        match refresh::apply(store, id, &contents) {
            Ok(refreshed) => Ok(moved || refreshed.changed),
            Err(refresh::RefreshError::Store(e)) => Err(e),
            Err(e) => {
                log::warn!("library: cannot refresh {id}: {e}");
                Ok(moved)
            }
        }
    }
}

/// Adopt a note that has no row (KTD1). A note whose `notes.md` has not
/// arrived starts with empty notes, and a later refresh reads them.
fn adopt(store: &mut Store, note: &NoteFolder) -> Result<bool, StoreError> {
    let adoption = Adoption::from_doc(
        &note.doc,
        note.notes.clone().unwrap_or_default(),
        note.path.to_string_lossy().into_owned(),
        note.transcript_fingerprint.clone(),
    );
    let adopted = store.adopt(&adoption)?;
    if adopted {
        log::info!(
            "library: adopted {} from {}",
            adoption.id,
            note.path.display()
        );
    }
    Ok(adopted)
}
