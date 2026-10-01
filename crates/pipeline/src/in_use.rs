//! The in-use marker (U4 of the audio removal plan, KTD4): while this run
//! resumes a saved note, or queues or processes it (Transcribe Again,
//! Retry, processing after Resume), it keeps its own hidden file in the
//! home folder of the note. The removal of old audio on any computer skips
//! a note while any in-use marker of the note is fresh (R10, AE4).
//!
//! The marker file, its name and its freshness are in
//! [`session_save::in_use_marker`], so that the library scan can read the
//! markers too (KTD5 of the notes without a recording plan). This module
//! re-exports them and keeps the [`InUseKeeper`] of this run.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, FixedOffset, Local};

pub use session_save::in_use_marker::{
    CLOCK_MARGIN, FRESH_FOR, MARKER_PREFIX, in_use, in_use_except, is_marker_name, marker_fresh,
    marker_name, remove_marker, write_marker,
};

use crate::{SharedStore, lock};

/// How often this run refreshes its markers.
pub const REFRESH_EVERY: Duration = Duration::from_secs(2 * 60);

// ------------------------------------------------------------------ the keeper

/// The part of this run that keeps the in-use markers. Each refresh finds
/// the notes that this run has open: a row with a home folder that is not
/// Saved or Discarded (a Resume, the queue, processing, a failure). It
/// writes or refreshes the marker of each one, and removes the marker of
/// each note whose work ended (Saved, Discarded, or no row).
///
/// The keeper takes the store lock only to read the rows, and never while
/// it writes in a home folder. It takes no capture lock.
pub struct InUseKeeper {
    run_id: String,
    store: SharedStore,
    state: Mutex<KeeperState>,
    signal: Condvar,
}

#[derive(Default)]
struct KeeperState {
    /// The home folder of each note where this run wrote its marker.
    written: BTreeMap<String, PathBuf>,
    woken: bool,
    started: bool,
    stopped: bool,
}

impl InUseKeeper {
    /// A keeper for the run `run_id`, the claim owner of this run's
    /// processing ([`crate::Runner::owner`]).
    pub fn new(store: SharedStore, run_id: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            run_id: run_id.into(),
            store,
            state: Mutex::default(),
            signal: Condvar::new(),
        })
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Start the thread that refreshes the markers at once and then every
    /// [`REFRESH_EVERY`], and at each [`InUseKeeper::wake`]. A second call
    /// does nothing.
    pub fn start(self: &Arc<Self>) -> io::Result<()> {
        {
            let mut state = self.state();
            if state.started || state.stopped {
                return Ok(());
            }
            state.started = true;
        }
        let keeper = Arc::clone(self);
        std::thread::Builder::new()
            .name("in-use".into())
            .spawn(move || keeper.run())
            .map(|_| ())
    }

    /// Refresh the markers now: a note started a Resume or entered the
    /// queue, or its work ended.
    pub fn wake(&self) {
        self.state().woken = true;
        self.signal.notify_all();
    }

    /// Stop the thread. The markers of open work stay: the work is still
    /// open, and a marker that is not refreshed stops being fresh after
    /// [`FRESH_FOR`].
    pub fn stop(&self) {
        self.state().stopped = true;
        self.signal.notify_all();
    }

    fn run(&self) {
        loop {
            self.refresh(Local::now().fixed_offset());
            let mut state = self.state();
            if !state.woken && !state.stopped {
                state = self
                    .signal
                    .wait_timeout(state, REFRESH_EVERY)
                    .map(|(state, _)| state)
                    .unwrap_or_else(|e| e.into_inner().0);
            }
            if state.stopped {
                return;
            }
            state.woken = false;
        }
    }

    /// One refresh at `now`: write the marker of each open note, and
    /// remove the marker of each note whose work ended.
    pub fn refresh(&self, now: DateTime<FixedOffset>) {
        // The home folder of each row, and the IDs of the open notes. The
        // store lock is released before the keeper touches a folder.
        let rows = match lock(&self.store).sessions() {
            Ok(rows) => rows,
            Err(e) => {
                log::warn!("in-use: could not read the notes: {e}");
                return;
            }
        };
        let mut homes: BTreeMap<String, PathBuf> = BTreeMap::new();
        let mut open: BTreeMap<String, PathBuf> = BTreeMap::new();
        for row in rows {
            let Some(home) = row.home_path.map(PathBuf::from) else {
                continue;
            };
            if !row.state.is_final() {
                open.insert(row.id.clone(), home.clone());
            }
            homes.insert(row.id, home);
        }
        let written = std::mem::take(&mut self.state().written);
        let mut kept = BTreeMap::new();
        for (id, old) in written {
            match open.get(&id) {
                Some(home) if *home == old => {}
                // A rename moved the folder, and the marker with it.
                Some(_) => {
                    self.remove(&id, &old);
                }
                None => {
                    // Save can rename the folder when it ends the work, so
                    // the marker can be in the current home folder and not
                    // at the recorded path.
                    let removed_old = self.remove(&id, &old);
                    let removed = match homes.get(&id) {
                        Some(current) if *current != old => self.remove(&id, current),
                        _ => removed_old,
                    };
                    if removed {
                        log::info!("in-use: removed the marker of {id}");
                    } else {
                        // The next refresh tries again.
                        kept.insert(id, old);
                    }
                }
            }
        }
        for (id, home) in open {
            match write_marker(&home, &self.run_id, now) {
                Ok(true) => {
                    kept.insert(id, home);
                }
                Ok(false) => {
                    log::debug!("in-use: the folder of {id} is not there, so it has no marker")
                }
                Err(e) => log::warn!("in-use: could not write the marker of {id}: {e}"),
            }
        }
        self.state().written.extend(kept);
    }

    /// Remove this run's marker of `id` from `home`. Returns `false` when
    /// the remove failed.
    fn remove(&self, id: &str, home: &Path) -> bool {
        match remove_marker(home, &self.run_id) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("in-use: could not remove the marker of {id}: {e}");
                false
            }
        }
    }

    fn state(&self) -> MutexGuard<'_, KeeperState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}
