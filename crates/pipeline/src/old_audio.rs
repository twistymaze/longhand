//! The copy of the old audio of a resumed note (KTD8 of the audio removal
//! plan).
//!
//! A Resume of a saved note starts to record at once. Beside the recording,
//! a background task copies the audio of each segment that the note had
//! when the Resume started (an old segment) from the home folder into the
//! working location. Processing then reads the old audio from this copy, so
//! another computer can remove the old audio from the home folder while the
//! note records or processes. Save adds only the audio of the segments that
//! the Resume recorded, so it never puts old audio back into the home
//! folder.
//!
//! The list of the old segments and the state of each copy is a file in the
//! working location, [`LIST_FILE`]. The copy of a segment is at the same
//! relative name as its archive in the home folder. A removal marker or a
//! missing audio file in the home folder before the copy is complete gives
//! the segment the removed state (R16): the task deletes its partial copy.
//! Every end of the session deletes the copy (Saved, a deleted note, launch
//! recovery). A note that failed processing deletes only the copy of each
//! old segment whose home file has a removal marker.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use session_core::folder;
use session_core::{Segment, SessionState};
use session_save::home_write;

use crate::{SharedStore, StageError, lock};

/// The file in the working location that lists the old segments of a
/// resumed note and the state of each copy.
pub const LIST_FILE: &str = "old-audio.json";

/// The state of the copy of one old segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CopyState {
    /// The copy is not complete. Processing reads the home file.
    Pending,
    /// The copy is complete. Processing reads the copy.
    Copied,
    /// The audio was removed from the home folder before the copy was
    /// complete, or after a failed processing (R16). No copy is kept.
    Removed,
}

/// One segment that the note had when the Resume started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OldSegment {
    /// The audio name of the segment, relative to the note folder.
    pub audio_file: String,
    pub state: CopyState,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct List {
    segments: Vec<OldSegment>,
}

/// The lock of each working location. One copy of a note runs at a time,
/// and its list changes only under its lock. A copy that waits for an
/// online-only file does not stop the copy of another note. A reader reads
/// the list without the lock: each write replaces the whole file.
static LOCKS: Mutex<BTreeMap<PathBuf, Arc<Mutex<()>>>> = Mutex::new(BTreeMap::new());

fn copy_lock(working: &Path) -> Arc<Mutex<()>> {
    let mut locks = LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(locks.entry(working.to_path_buf()).or_default())
}

/// Run `f` under the lock of `working`.
fn locked<T>(working: &Path, f: impl FnOnce() -> T) -> T {
    let lock = copy_lock(working);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

/// Record the segments that a saved note has when its Resume starts, all
/// with no copy. This reads no audio and waits for no lock, so a Resume
/// does not wait for an online-only file. A list from an earlier Resume is
/// replaced. No copy of the note runs then: the note was saved, so its
/// last copy ended.
pub fn begin(working: &Path, segments: &[Segment]) -> io::Result<()> {
    let list = List {
        segments: segments
            .iter()
            .map(|segment| OldSegment {
                audio_file: segment.audio_file.clone(),
                state: CopyState::Pending,
            })
            .collect(),
    };
    std::fs::create_dir_all(working)?;
    write(working, &list)
}

/// The old segments of the resumed note in `working`, or `None` when the
/// working location has no list: the session is not a resumed saved note.
pub fn old_segments(working: &Path) -> io::Result<Option<Vec<OldSegment>>> {
    Ok(read(working)?.map(|list| list.segments))
}

/// Whether `audio_file` is the audio of an old segment of the resumed note
/// in `working`. Save does not add the audio of an old segment to the home
/// folder.
pub fn is_old(working: &Path, audio_file: &str) -> io::Result<bool> {
    Ok(old_segments(working)?
        .is_some_and(|old| old.iter().any(|segment| segment.audio_file == audio_file)))
}

/// Copy the audio of each old segment that has no copy yet from the home
/// folder into `working`. `home` gives the home folder of the note now: a
/// sync can move the folder during the copy. The read waits for an
/// online-only file to download; call this on a background thread or in a
/// processing stage, never on the thread that starts the recording.
///
/// A removal marker, or a missing file in a home folder that is there,
/// gives the segment the removed state and deletes its partial copy. A home
/// folder that is not there, or another read error, keeps the segment
/// pending: processing then reads the home file, as for a note with no
/// copy.
pub fn copy(working: &Path, home: &dyn Fn() -> Option<PathBuf>) -> io::Result<()> {
    locked(working, || {
        let Some(mut list) = read(working)? else {
            return Ok(());
        };
        for index in 0..list.segments.len() {
            if list.segments[index].state != CopyState::Pending {
                continue;
            }
            let audio_file = list.segments[index].audio_file.clone();
            let state = copy_one(working, home, &audio_file);
            if state == CopyState::Removed {
                remove_copy(working, &audio_file)?;
            }
            if state != CopyState::Pending {
                list.segments[index].state = state;
                write(working, &list)?;
            }
        }
        Ok(())
    })
}

fn copy_one(working: &Path, home: &dyn Fn() -> Option<PathBuf>, audio_file: &str) -> CopyState {
    if !crate::remove_audio::safe_audio_name(audio_file) {
        log::warn!("the old audio {audio_file} has an unsafe name and is not copied");
        return CopyState::Pending;
    }
    let target = working.join(audio_file);
    let (Some(dir), Some(name)) = (target.parent(), target.file_name().and_then(|n| n.to_str()))
    else {
        return CopyState::Pending;
    };
    // One more attempt when the home folder moved during the read.
    let mut attempts = 2;
    loop {
        attempts -= 1;
        let Some(folder) = home().filter(|folder| folder.is_dir()) else {
            return CopyState::Pending;
        };
        if folder::audio_removed(&folder, audio_file) {
            return CopyState::Removed;
        }
        let copied = std::fs::create_dir_all(dir)
            .and_then(|()| home_write::copy_atomic(dir, name, &folder.join(audio_file)));
        match copied {
            Ok(()) => return CopyState::Copied,
            Err(e) => {
                let moved = home().is_some_and(|now| now != folder);
                if moved && attempts > 0 {
                    continue;
                }
                if folder::audio_removed(&folder, audio_file)
                    || (e.kind() == io::ErrorKind::NotFound && folder.is_dir())
                {
                    log::info!(
                        "the old audio {audio_file} left {} before its copy was complete",
                        folder.display()
                    );
                    return CopyState::Removed;
                }
                log::warn!("could not copy the old audio {audio_file}: {e}");
                return CopyState::Pending;
            }
        }
    }
}

/// [`copy`] with the home folder that the store records for `note_id`
/// now.
pub fn copy_for(store: &SharedStore, note_id: &str, working: &Path) -> io::Result<()> {
    let home = || {
        lock(store)
            .session(note_id)
            .ok()
            .flatten()
            .and_then(|record| record.home_path)
            .map(PathBuf::from)
    };
    copy(working, &home)
}

/// For a resumed note that failed processing: delete the copy of each old
/// segment whose home file has a removal marker, because Retry is then
/// unavailable (KTD8). The copies of the other segments stay for Retry.
/// Returns the audio names whose copy this call deleted.
pub fn drop_removed(working: &Path, home: &Path) -> io::Result<Vec<String>> {
    locked(working, || {
        let Some(mut list) = read(working)? else {
            return Ok(Vec::new());
        };
        let mut dropped = Vec::new();
        for segment in &mut list.segments {
            if segment.state == CopyState::Removed
                || !folder::audio_removed(home, &segment.audio_file)
            {
                continue;
            }
            remove_copy(working, &segment.audio_file)?;
            segment.state = CopyState::Removed;
            dropped.push(segment.audio_file.clone());
        }
        if !dropped.is_empty() {
            write(working, &list)?;
        }
        Ok(dropped)
    })
}

/// Delete the copy of every old segment and the list. The session ended,
/// and the working location keeps no copy of its old audio.
pub fn discard(working: &Path) -> io::Result<()> {
    locked(working, || {
        let Some(list) = read(working)? else {
            return Ok(());
        };
        for segment in &list.segments {
            remove_copy(working, &segment.audio_file)?;
        }
        match std::fs::remove_file(working.join(LIST_FILE)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    })
}

/// At launch, before anything records or processes: delete the old audio
/// copy of each session that is Saved or has no row, and the copies of the
/// removed old segments of each session that failed processing. The copy
/// of a session that records, is interrupted, waits or processes stays
/// (R8).
pub fn sweep(store: &SharedStore, working_root: &Path) -> Result<(), StageError> {
    let entries = match std::fs::read_dir(working_root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let entry = entry?;
        let working = entry.path();
        if !working.join(LIST_FILE).is_file() {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let record = lock(store).session(&id)?;
        let result = match &record {
            None
            | Some(session_core::SessionRecord {
                state: SessionState::Saved | SessionState::Discarded,
                ..
            }) => discard(&working),
            Some(record) if matches!(record.state, SessionState::ProcessingFailed { .. }) => {
                match record.home_path.as_deref() {
                    Some(home) => drop_removed(&working, Path::new(home)).map(|_| ()),
                    None => Ok(()),
                }
            }
            Some(_) => Ok(()),
        };
        if let Err(e) = result {
            log::warn!(
                "could not delete the old audio copy in {}: {e}",
                working.display()
            );
        }
    }
    Ok(())
}

fn remove_copy(working: &Path, audio_file: &str) -> io::Result<()> {
    if !crate::remove_audio::safe_audio_name(audio_file) {
        return Ok(());
    }
    match std::fs::remove_file(working.join(audio_file)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn read(working: &Path) -> io::Result<Option<List>> {
    match std::fs::read(working.join(LIST_FILE)) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn write(working: &Path, list: &List) -> io::Result<()> {
    home_write::write_atomic(working, LIST_FILE, &serde_json::to_vec(list)?)
}
