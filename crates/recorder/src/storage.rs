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
}
