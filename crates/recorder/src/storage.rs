//! The working location's free-space floor (R7a).
//!
//! anarlog's `check_storage` less its second limit: anarlog also caps
//! `audio-recovery/` at 256 MB because it drains that directory as chunks
//! are acknowledged, while Longhand keeps every chunk until the save, so the
//! cap would stop a session at about 47 minutes (KTD4). Only the volume
//! floor remains. See PROVENANCE.md.

use std::path::Path;

/// Free space below which recording stops writing, as anarlog reserves.
pub const DISK_RESERVE_BYTES: u64 = 256 * 1024 * 1024;

/// Bytes of the 16 kHz mono f32 processing file for each second. A stereo
/// segment is mixed to mono, so this part has one channel always.
const TAP_BYTES_PER_SECOND: u64 = 16_000 * 4;

/// Working-location bytes each second of a mono session needs once
/// recording stops and processing begins (R7b): the 16 kHz mono f32 tap
/// plus the 64 kbit/s Opus archive, written while the chunks are still on
/// disk.
pub const PROCESSING_BYTES_PER_SECOND: u64 = TAP_BYTES_PER_SECOND + 64_000 / 8;

/// The session length the pre-record check plans for.
pub const PLANNED_SESSION_SECONDS: u64 = 4 * 60 * 60;

/// Chunk bytes per second at a native rate: 16-bit samples of each channel.
/// A stereo segment needs twice the bytes of a mono one (KTD6 of the system
/// audio plan).
pub fn chunk_bytes_per_second(sample_rate: u32, channels: u16) -> u64 {
    u64::from(sample_rate) * 2 * u64::from(channels)
}

/// Archive bytes per second: one 64 kbit/s Opus stream for each channel
/// (KTD7 of the system audio plan).
pub fn archive_bytes_per_second(channels: u16) -> u64 {
    64_000 / 8 * u64::from(channels)
}

/// [`PROCESSING_BYTES_PER_SECOND`] for a segment of `channels` channels:
/// the mono processing file and one archive stream for each channel.
pub fn processing_bytes_per_second(channels: u16) -> u64 {
    TAP_BYTES_PER_SECOND + archive_bytes_per_second(channels)
}

/// Peak working-location demand of a session of `seconds` at `sample_rate`
/// with `channels` channels: its chunks, with processing's files beside
/// them (R7b). About 2.4 GB for four hours at 48 kHz mono.
pub fn session_demand(sample_rate: u32, channels: u16, seconds: u64) -> u64 {
    (chunk_bytes_per_second(sample_rate, channels)
        + TAP_BYTES_PER_SECOND
        + archive_bytes_per_second(channels))
        * seconds
}

/// What processing `seconds` of recorded audio still needs, the chunks
/// being on disk already.
pub fn processing_demand(seconds: u64) -> u64 {
    PROCESSING_BYTES_PER_SECOND * seconds
}

/// `Err(available)` when a known figure cannot hold `needed` above the
/// floor. Unknown passes, as in [`check`].
pub fn check_headroom(available: Option<u64>, needed: u64, floor: u64) -> Result<(), u64> {
    match available {
        Some(bytes) if bytes < needed.saturating_add(floor) => Err(bytes),
        _ => Ok(()),
    }
}

/// Free bytes on the volume holding a path; `None` when unknown.
pub type SpaceProbe = Box<dyn Fn(&Path) -> Option<u64> + Send>;

pub fn system_probe() -> SpaceProbe {
    Box::new(available_space)
}

/// Free bytes on the volume that holds `dir`; `None` when unknown, for
/// example when `dir` does not exist.
///
/// It asks only that volume, with one system call. A list of all mounted
/// volumes asks each of them, and a network volume that does not answer can
/// then stop the caller, also the thread that writes the audio.
#[cfg(target_os = "macos")]
pub fn available_space(dir: &Path) -> Option<u64> {
    use objc2_foundation::{
        NSArray, NSNumber, NSString, NSURL, NSURLVolumeAvailableCapacityForImportantUsageKey,
        NSURLVolumeAvailableCapacityKey,
    };
    // The writer thread has no autorelease pool, so make one for each call.
    objc2::rc::autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(dir.to_str()?));
        // SAFETY: the keys are constant NSString values that Foundation
        // defines.
        let (important, available) = unsafe {
            (
                NSURLVolumeAvailableCapacityForImportantUsageKey,
                NSURLVolumeAvailableCapacityKey,
            )
        };
        let values = url
            .resourceValuesForKeys_error(&NSArray::from_slice(&[important, available]))
            .ok()?;
        let figure = |key| {
            values
                .objectForKey(key)
                .and_then(|value| value.downcast_ref::<NSNumber>().map(|n| n.longLongValue()))
                .and_then(|bytes| u64::try_from(bytes).ok())
        };
        capacity(figure(important), figure(available))
    })
}

/// The figure that Finder shows: the capacity for important usage, which
/// counts space that macOS can free (purgeable files), and the plain
/// available capacity when the first is missing or 0 (sysinfo's rule).
#[cfg(any(target_os = "macos", test))]
fn capacity(important: Option<u64>, available: Option<u64>) -> Option<u64> {
    important.filter(|&bytes| bytes != 0).or(available)
}

/// Free bytes on the volume that holds `dir`; `None` when unknown, for
/// example when `dir` does not exist. See the macOS version.
#[cfg(windows)]
pub fn available_space(dir: &Path) -> Option<u64> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let name = directory_name(dir);
    let mut free = 0u64;
    // SAFETY: `name` is a NUL-terminated wide string that lives for the
    // call, and the out pointer is a valid u64; the API accepts null for the
    // two figures that are not read.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            name.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut free,
        )
    };
    (ok != 0).then_some(free)
}

/// `dir` as a NUL-terminated wide string that ends in a separator:
/// `GetDiskFreeSpaceExW` needs it for a UNC name (`\\server\share\`).
#[cfg(windows)]
fn directory_name(dir: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let mut name: Vec<u16> = dir.as_os_str().encode_wide().collect();
    if !matches!(name.last(), Some(&c) if c == u16::from(b'\\') || c == u16::from(b'/')) {
        name.push(u16::from(b'\\'));
    }
    name.push(0);
    name
}

/// No lookup on other systems (Longhand ships for macOS and Windows): the
/// figure is unknown, which the checks let pass.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn available_space(_dir: &Path) -> Option<u64> {
    None
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
    fn four_hours_at_48_khz_needs_about_two_and_a_half_gigabytes() {
        let demand = session_demand(48_000, 1, PLANNED_SESSION_SECONDS);
        // 1.38 GB of chunks, 0.92 GB of processing tap, 0.12 GB of archive.
        assert_eq!(demand, 2_419_200_000);
        assert!(check_headroom(Some(3_000_000_000), demand, DISK_RESERVE_BYTES).is_ok());
        assert!(check_headroom(Some(2_500_000_000), demand, DISK_RESERVE_BYTES).is_err());
    }

    #[test]
    fn a_stereo_estimate_doubles_the_chunk_and_archive_parts() {
        assert_eq!(
            chunk_bytes_per_second(48_000, 2),
            2 * chunk_bytes_per_second(48_000, 1)
        );
        assert_eq!(archive_bytes_per_second(2), 2 * archive_bytes_per_second(1));
        // The 16 kHz mono processing file stays one channel: processing
        // mixes a stereo segment to mono (KD4).
        let mono = session_demand(48_000, 1, 3_600);
        let stereo = session_demand(48_000, 2, 3_600);
        let doubled = (chunk_bytes_per_second(48_000, 1) + archive_bytes_per_second(1)) * 3_600;
        assert_eq!(stereo - mono, doubled);
        assert_eq!(
            mono,
            (chunk_bytes_per_second(48_000, 1) + PROCESSING_BYTES_PER_SECOND) * 3_600
        );
        assert_eq!(processing_bytes_per_second(1), PROCESSING_BYTES_PER_SECOND);
        assert_eq!(
            processing_bytes_per_second(2) - processing_bytes_per_second(1),
            archive_bytes_per_second(1)
        );
    }

    #[test]
    fn processing_needs_the_tap_and_archive_beside_the_chunks() {
        assert_eq!(processing_demand(3_600), 259_200_000);
        assert!(check_headroom(None, u64::MAX, DISK_RESERVE_BYTES).is_ok());
    }

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

    #[test]
    fn a_folder_that_does_not_exist_has_no_figure() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(available_space(&dir.path().join("gone")), None);
    }

    #[test]
    fn the_capacity_for_important_usage_comes_first_and_zero_means_missing() {
        assert_eq!(capacity(Some(50), Some(40)), Some(50));
        assert_eq!(capacity(Some(0), Some(40)), Some(40));
        assert_eq!(capacity(None, Some(40)), Some(40));
        assert_eq!(capacity(None, None), None);
    }

    /// Free space can change between the calls, so the figure is checked
    /// against a figure from before and after it, with a tolerance for
    /// writes by other processes.
    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn the_figure_is_the_one_that_sysinfo_gives_for_the_volume() {
        const TOLERANCE: u64 = 1 << 30;
        let dir = tempfile::tempdir().unwrap();
        let before = sysinfo_space(dir.path());
        let ours = available_space(dir.path()).unwrap();
        let after = sysinfo_space(dir.path());
        let (low, high) = (before.min(after), before.max(after));
        assert!(
            ours + TOLERANCE >= low && ours <= high + TOLERANCE,
            "{ours} is not between {low} and {high}"
        );
    }

    /// The lookup that the app used before: the deepest mount point that
    /// contains `dir`.
    #[cfg(any(target_os = "macos", windows))]
    fn sysinfo_space(dir: &Path) -> u64 {
        let canonical = dir.canonicalize().unwrap();
        sysinfo::Disks::new_with_refreshed_list()
            .list()
            .iter()
            .filter(|disk| {
                disk.mount_point()
                    .canonicalize()
                    .is_ok_and(|mount| canonical.starts_with(mount))
            })
            .max_by_key(|disk| disk.mount_point().components().count())
            .map(|disk| disk.available_space())
            .unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn a_windows_folder_gets_one_trailing_separator() {
        use std::os::windows::ffi::OsStrExt;
        let wide = |s: &str| std::ffi::OsStr::new(s).encode_wide().collect::<Vec<_>>();
        for (path, expected) in [
            (r"\\server\share", "\\\\server\\share\\\0"),
            (r"\\server\share\notes", "\\\\server\\share\\notes\\\0"),
            (r"C:\Users\a\", "C:\\Users\\a\\\0"),
            ("C:/Users/a/", "C:/Users/a/\0"),
        ] {
            assert_eq!(directory_name(Path::new(path)), wide(expected), "{path}");
        }
    }
}
