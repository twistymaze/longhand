//! The file of the local settings, `settings.json` in the app data folder.
//! It is a JSON object; the local settings are under the key `settings`.
//!
//! Longhand reads and writes this file itself. A read that fails is never
//! taken for a missing file, so the defaults never replace a file that
//! holds the user's settings without the user's choice. A write goes to a
//! temporary file in the same folder and is then renamed over the file, so
//! a crash or a power loss never leaves a partly written file.

use std::fmt;
use std::fs::OpenOptions;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value};

/// The name of the file in the app data folder.
pub const FILE_NAME: &str = "settings.json";

/// The number of reads at launch before a file counts as unreadable.
pub const LAUNCH_TRIES: u32 = 5;
/// The time between two reads at launch.
pub const LAUNCH_DELAY: Duration = Duration::from_millis(200);

/// Why the file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreadable {
    /// The file system refused the read. The text is the error.
    Io(String),
    /// The file is not a JSON object. The text is the error.
    DoesNotParse(String),
}

impl fmt::Display for Unreadable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unreadable::Io(e) => write!(f, "the file can't be read: {e}"),
            Unreadable::DoesNotParse(e) => write!(f, "the file is damaged: {e}"),
        }
    }
}

/// Read the file once. A missing file gives `None`.
fn read_once(path: &Path) -> Result<Option<Map<String, Value>>, Unreadable> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Unreadable::Io(e.to_string())),
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(map)) => Ok(Some(map)),
        Ok(_) => Err(Unreadable::DoesNotParse("it is not a JSON object".into())),
        Err(e) => Err(Unreadable::DoesNotParse(e.to_string())),
    }
}

/// Read the file. A missing file gives `None`. A read that fails is tried
/// again `tries - 1` times after `delay`: another app can hold the file, or
/// write it in steps, for a short time.
pub fn read(
    path: &Path,
    tries: u32,
    delay: Duration,
) -> Result<Option<Map<String, Value>>, Unreadable> {
    read_with(path, tries, || std::thread::sleep(delay))
}

/// [`read`] with the wait between two tries.
fn read_with(
    path: &Path,
    tries: u32,
    mut wait: impl FnMut(),
) -> Result<Option<Map<String, Value>>, Unreadable> {
    let mut result = read_once(path);
    for _ in 1..tries {
        if result.is_ok() {
            break;
        }
        wait();
        result = read_once(path);
    }
    result
}

fn unpredictable() -> u64 {
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u32(std::process::id());
    if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        h.write_u128(t.as_nanos());
    }
    h.finish()
}

/// Replace the file with `map`. The bytes go to a new temporary file in the
/// same folder, which is synced and renamed over the file. An error before
/// the rename keeps the old file. The rename is the commit: after it, the
/// file holds `map`, so an error of the folder sync is logged and the write
/// counts as done.
pub fn write(path: &Path, map: &Map<String, Value>) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("the settings file has no folder"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("the settings file has no name"))?
        .to_string_lossy();
    let bytes = serde_json::to_vec_pretty(map).map_err(io::Error::other)?;
    std::fs::create_dir_all(dir)?;
    let temp = dir.join(format!(".{name}.{:016x}.tmp", unpredictable()));
    let written = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .and_then(|mut file| file.write_all(&bytes).and_then(|()| file.sync_all()))
        .and_then(|()| std::fs::rename(&temp, path));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    if let Err(e) = std::fs::File::open(dir).and_then(|d| d.sync_all()) {
        log::warn!("could not sync the folder of the settings file: {e}");
    }
    Ok(())
}

/// Move the file to `settings.unreadable-<UTC time>.json` in the same
/// folder, or that name with `-2`, `-3` and so on when it is taken. Returns
/// the new path.
pub fn set_aside(path: &Path) -> io::Result<PathBuf> {
    let time = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let stem = format!("settings.unreadable-{time}");
    let mut candidate = path.with_file_name(format!("{stem}.json"));
    let mut n = 2;
    while candidate.exists() {
        candidate = path.with_file_name(format!("{stem}-{n}.json"));
        n += 1;
    }
    std::fs::rename(path, &candidate)?;
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_missing_file_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read(&dir.path().join(FILE_NAME), 1, Duration::ZERO),
            Ok(None)
        );
    }

    #[test]
    fn a_file_that_does_not_parse_is_unreadable_and_does_not_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        for bytes in [&b""[..], b"{\"settings\": {\"storage_fol", b"[1, 2]"] {
            std::fs::write(&path, bytes).unwrap();
            let result = read(&path, 2, Duration::ZERO);
            assert!(
                matches!(result, Err(Unreadable::DoesNotParse(_))),
                "{result:?}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn a_file_that_another_app_finishes_reads_on_a_later_try() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, b"{\"settings\": ").unwrap();
        let mut waits = 0;
        let result = read_with(&path, LAUNCH_TRIES, || {
            waits += 1;
            // Another app ends its write during the second wait.
            if waits == 2 {
                std::fs::write(&path, b"{\"settings\": {}}").unwrap();
            }
        });
        assert_eq!(result, Ok(Some(map(json!({ "settings": {} })))));
        assert_eq!(waits, 2);
    }

    #[test]
    fn a_read_that_the_file_system_refuses_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        // A folder at the path: the read fails, and it is not a missing file.
        let path = dir.path().join(FILE_NAME);
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(
            read(&path, 2, Duration::ZERO),
            Err(Unreadable::Io(_))
        ));
    }

    #[test]
    fn a_write_reads_back_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, b"{\"settings\": {\"old\": true}}").unwrap();
        let new = map(json!({ "settings": { "storage_folder": "/notes" } }));
        write(&path, &new).unwrap();
        assert_eq!(read(&path, 1, Duration::ZERO), Ok(Some(new)));
        assert_eq!(names(dir.path()), [FILE_NAME]);
    }

    #[test]
    fn a_write_that_cannot_rename_returns_the_error_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("inside"), b"x").unwrap();
        assert!(write(&path, &map(json!({ "settings": {} }))).is_err());
        assert_eq!(names(dir.path()), [FILE_NAME]);
        assert_eq!(names(&path), ["inside"]);
    }

    #[test]
    fn the_file_of_an_earlier_build_reads() {
        // The store plugin wrote pretty JSON with the key `settings`.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let earlier =
            json!({ "settings": { "settings_schema_version": 9, "hide_from_dock": true } });
        std::fs::write(&path, serde_json::to_vec_pretty(&earlier).unwrap()).unwrap();
        assert_eq!(
            read(&path, 1, Duration::ZERO),
            Ok(Some(map(earlier.clone())))
        );
        write(&path, &map(earlier.clone())).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            serde_json::to_vec_pretty(&earlier).unwrap()
        );
    }

    #[test]
    fn set_aside_keeps_the_bytes_under_a_free_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, b"damaged 1").unwrap();
        let first = set_aside(&path).unwrap();
        std::fs::write(&path, b"damaged 2").unwrap();
        let second = set_aside(&path).unwrap();
        assert!(!path.exists());
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), b"damaged 1");
        assert_eq!(std::fs::read(&second).unwrap(), b"damaged 2");
        let name = first.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("settings.unreadable-") && name.ends_with(".json"));
    }
}
