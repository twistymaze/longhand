//! The in-use marker (U4 of the audio removal plan, KTD4): while this run
//! resumes a saved note, or queues or processes it (Transcribe Again,
//! Retry, processing after Resume), it keeps its own hidden file in the
//! home folder of the note. The removal of old audio on any computer skips
//! a note while any in-use marker of the note is fresh (R10, AE4).
//!
//! The file name is `.longhand-in-use-<run ID>`. The dot keeps the library
//! watcher and the library scan from reading it. The file holds the run ID
//! and the time of its last refresh. A marker is fresh while that time is
//! less than [`FRESH_FOR`] old. A time that is up to [`CLOCK_MARGIN`] in
//! the future is fresh, because the clock of the other computer can be
//! ahead. A marker whose time does not parse, or is further in the future,
//! is fresh until its file time is [`FRESH_FOR`] old.
//!
//! The write never makes a missing home folder: a rename on another
//! computer can move the folder, and a new folder at the old path would
//! be a second copy of the note. Each run removes only its own marker.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, FixedOffset, Local, TimeDelta};
use serde::{Deserialize, Serialize};
use session_save::home_write;

use crate::{SharedStore, lock};

/// The start of the name of each in-use marker.
pub const MARKER_PREFIX: &str = ".longhand-in-use-";

/// How long a marker stays fresh after its last refresh.
pub const FRESH_FOR: TimeDelta = TimeDelta::minutes(30);

/// How far in the future the time in a marker can be and stay fresh.
pub const CLOCK_MARGIN: TimeDelta = TimeDelta::minutes(5);

/// How often this run refreshes its markers.
pub const REFRESH_EVERY: Duration = Duration::from_secs(2 * 60);

/// The content of a marker.
#[derive(Debug, Serialize, Deserialize)]
struct Marker {
    run_id: String,
    refreshed_at: DateTime<FixedOffset>,
}

/// The file name of the marker of `run_id`. A character that is not a
/// letter, a digit, `-` or `_` becomes `_`, so the name is always one plain
/// file name.
pub fn marker_name(run_id: &str) -> String {
    let safe: String = run_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{MARKER_PREFIX}{safe}")
}

/// Whether `name` is the file name of an in-use marker.
pub fn is_marker_name(name: &str) -> bool {
    name.strip_prefix(MARKER_PREFIX)
        .is_some_and(|run| !run.is_empty())
}

/// Write or refresh the marker of `run_id` in `home`, with the time `now`.
/// Returns `false`, and writes nothing, when `home` is not a folder. The
/// write never makes a folder.
pub fn write_marker(home: &Path, run_id: &str, now: DateTime<FixedOffset>) -> io::Result<bool> {
    match home.symlink_metadata() {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    }
    let marker = Marker {
        run_id: run_id.to_string(),
        refreshed_at: now,
    };
    let text = serde_json::to_vec(&marker).map_err(io::Error::other)?;
    match home_write::write_atomic(home, &marker_name(run_id), &text) {
        Ok(()) => Ok(true),
        // The folder went away after the check.
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Remove the marker of `run_id` from `home`. A marker or a folder that is
/// not there is not an error.
pub fn remove_marker(home: &Path, run_id: &str) -> io::Result<()> {
    match std::fs::remove_file(home.join(marker_name(run_id))) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Whether the marker file at `path` is fresh at `now`. A file that cannot
/// be read counts by its file time. A file that is not there is not fresh.
pub fn marker_fresh(path: &Path, now: DateTime<FixedOffset>) -> bool {
    let refreshed = std::fs::read(path)
        .ok()
        .and_then(|text| serde_json::from_slice::<Marker>(&text).ok())
        .map(|marker| marker.refreshed_at);
    if let Some(at) = refreshed
        && at <= now + CLOCK_MARGIN
    {
        return now.signed_duration_since(at) < FRESH_FOR;
    }
    // The time does not parse, or is too far in the future to trust.
    let Ok(modified) = path.symlink_metadata().and_then(|m| m.modified()) else {
        return false;
    };
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO);
    TimeDelta::from_std(age).is_ok_and(|age| age < FRESH_FOR)
}

/// Whether any in-use marker in `home` is fresh at `now`: another run, on
/// this computer or another one, resumes or processes the note. An error
/// means that the folder cannot be read, and the caller must not remove
/// the audio. This reads the names in `home` and the small marker files
/// only. It opens no audio (KTD11).
pub fn in_use(home: &Path, now: DateTime<FixedOffset>) -> io::Result<bool> {
    for entry in std::fs::read_dir(home)? {
        let entry = entry?;
        let name = entry.file_name();
        if !is_marker_name(&name.to_string_lossy()) {
            continue;
        }
        if entry.file_type().is_ok_and(|t| t.is_file()) && marker_fresh(&entry.path(), now) {
            return Ok(true);
        }
    }
    Ok(false)
}

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
