//! The working location's free-space floor (R7a).
//!
//! anarlog's `check_storage` less its second limit: anarlog also caps
//! `audio-recovery/` at 256 MB because it drains that directory as chunks
//! are acknowledged, while Longhand keeps every chunk until landing, so the
//! cap would stop a session at about 47 minutes (KTD4). Only the volume
//! floor remains. See PROVENANCE.md.

use std::path::Path;

/// Free space below which recording stops writing, as anarlog reserves.
pub const DISK_RESERVE_BYTES: u64 = 256 * 1024 * 1024;

/// Free bytes on the volume holding a path; `None` when unknown.
pub type SpaceProbe = Box<dyn Fn(&Path) -> Option<u64> + Send>;

pub fn system_probe() -> SpaceProbe {
    Box::new(available_space)
}

/// The deepest mount point containing `dir`, as anarlog resolves it.
pub fn available_space(dir: &Path) -> Option<u64> {
    let canonical = dir.canonicalize().ok()?;
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|disk| {
            disk.mount_point()
                .canonicalize()
                .is_ok_and(|mount| canonical.starts_with(mount))
        })
        .max_by_key(|disk| disk.mount_point().components().count())
        .map(|disk| disk.available_space())
}

/// `Err(available)` when a known figure is under the floor. An unknown
/// figure passes: refusing to record because a lookup failed would lose the
/// session to a guess.
pub fn check(available: Option<u64>, floor: u64) -> Result<(), u64> {
    match available {
        Some(bytes) if bytes < floor => Err(bytes),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_floor_is_a_strict_minimum() {
        assert_eq!(
            check(Some(DISK_RESERVE_BYTES - 1), DISK_RESERVE_BYTES),
            Err(DISK_RESERVE_BYTES - 1)
        );
        assert!(check(Some(DISK_RESERVE_BYTES), DISK_RESERVE_BYTES).is_ok());
        assert!(check(None, DISK_RESERVE_BYTES).is_ok());
    }

    #[test]
    fn the_system_probe_finds_the_temp_volume() {
        let dir = tempfile::tempdir().unwrap();
        assert!(available_space(dir.path()).is_some_and(|b| b > 0));
    }
}
