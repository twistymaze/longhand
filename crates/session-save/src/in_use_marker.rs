//! The in-use marker (U4 of the audio removal plan, KTD4): while a run
//! records into a note, or queues or processes it, it keeps its own hidden
//! file in the home folder of the note. The removal of old audio on any
//! computer skips a note while any in-use marker of the note is fresh (R10,
//! AE4), and the library scan shows such a note as in use on another
//! computer (KTD5 of the notes without a recording plan).
//!
//! The file name is `.longhand-in-use-<run ID>`. The dot keeps the library
//! watcher and the library scan from reading it as a note file. The file
//! holds the run ID and the time of its last refresh. A marker is fresh
//! while that time is less than [`FRESH_FOR`] old. A time that is up to
//! [`CLOCK_MARGIN`] in the future is fresh, because the clock of the other
//! computer can be ahead. A marker whose time does not parse, or is further
//! in the future, is fresh until its file time is [`FRESH_FOR`] old.
//!
//! The write never makes a missing home folder: a rename on another
//! computer can move the folder, and a new folder at the old path would
//! be a second copy of the note. Each run removes only its own marker.
//!
//! The keeper that refreshes the markers of a run is
//! `pipeline::in_use::InUseKeeper`. These functions are here so that the
//! library, which `pipeline` depends on, can read the markers too.

use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, FixedOffset, TimeDelta};
use serde::{Deserialize, Serialize};

use crate::home_write;

/// The start of the name of each in-use marker.
pub const MARKER_PREFIX: &str = ".longhand-in-use-";

/// How long a marker stays fresh after its last refresh.
pub const FRESH_FOR: TimeDelta = TimeDelta::minutes(30);

/// How far in the future the time in a marker can be and stay fresh.
pub const CLOCK_MARGIN: TimeDelta = TimeDelta::minutes(5);

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
/// this computer or another one, records into or processes the note. An
/// error means that the folder cannot be read, and the caller must not
/// remove the audio. This reads the names in `home` and the small marker
/// files only. It opens no audio (KTD11).
pub fn in_use(home: &Path, now: DateTime<FixedOffset>) -> io::Result<bool> {
    in_use_except(home, &[], now)
}

/// [`in_use`], but the markers of the runs in `except` do not count. The
/// library scan gives this computer's runs here (this run and its earlier
/// runs), so that its own work does not show as work on another computer
/// (KTD5). A marker of any other run counts.
pub fn in_use_except(
    home: &Path,
    except: &[String],
    now: DateTime<FixedOffset>,
) -> io::Result<bool> {
    let own: Vec<String> = except.iter().map(|run| marker_name(run)).collect();
    for entry in std::fs::read_dir(home)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !is_marker_name(&name) || own.iter().any(|own| *own == *name) {
            continue;
        }
        if entry.file_type().is_ok_and(|t| t.is_file()) && marker_fresh(&entry.path(), now) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-09-30T12:00:00-05:00").unwrap()
    }

    #[test]
    fn the_marker_of_the_excepted_run_does_not_count() {
        let home = tempfile::tempdir().unwrap();
        assert!(write_marker(home.path(), "mine", now()).unwrap());
        assert!(in_use(home.path(), now()).unwrap());
        assert!(!in_use_except(home.path(), &runs(&["mine"]), now()).unwrap());

        assert!(write_marker(home.path(), "theirs", now()).unwrap());
        assert!(in_use_except(home.path(), &runs(&["mine"]), now()).unwrap());
    }

    // A relaunch: the marker of this computer's earlier run is still fresh,
    // and it does not count. A marker of another computer still counts.
    #[test]
    fn the_markers_of_all_excepted_runs_do_not_count() {
        let home = tempfile::tempdir().unwrap();
        assert!(write_marker(home.path(), "earlier", now()).unwrap());
        let own = runs(&["now", "earlier"]);
        assert!(!in_use_except(home.path(), &own, now()).unwrap());

        assert!(write_marker(home.path(), "theirs", now()).unwrap());
        assert!(in_use_except(home.path(), &own, now()).unwrap());
    }

    fn runs(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn a_marker_older_than_the_fresh_time_does_not_count() {
        let home = tempfile::tempdir().unwrap();
        let old = now() - FRESH_FOR - TimeDelta::minutes(1);
        assert!(write_marker(home.path(), "theirs", old).unwrap());
        assert!(!in_use_except(home.path(), &runs(&["mine"]), now()).unwrap());
    }
}
