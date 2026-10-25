//! The recording time of an audio file (R16).
//!
//! An m4a file records its time in the `mvhd` atom of `moov`, in seconds
//! since 1904-01-01 UTC. Symphonia does not give that value, so a small
//! atom reader gets it. An mp3 file records it in the ID3v2 `TDRC` frame,
//! which Symphonia gives as the recording date tag.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTag};

/// Seconds from 1904-01-01 to 1970-01-01.
const MAC_EPOCH_OFFSET: i64 = 2_082_844_800;
/// The atom reader stops after this many atoms, so a damaged file cannot
/// make it loop.
const MAX_ATOMS: usize = 4096;

/// The recording time that `path` records, or `None`. A time that is
/// zero, before 2000, or later than the file's modification time plus one
/// day is not a real recording time, so it counts as `None` and the caller
/// uses the modification time.
pub fn recording_time(path: &Path) -> Option<DateTime<Utc>> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let time = match extension.as_str() {
        "m4a" | "qta" => mvhd_time(path),
        "mp3" => tag_time(path),
        _ => None,
    }?;
    let modified: DateTime<Utc> = std::fs::metadata(path).ok()?.modified().ok()?.into();
    let earliest = Utc.with_ymd_and_hms(2000, 1, 1, 0, 0, 0).single()?;
    (time >= earliest && time <= modified + chrono::Duration::days(1)).then_some(time)
}

/// The creation time in the `mvhd` atom of an m4a file.
fn mvhd_time(path: &Path) -> Option<DateTime<Utc>> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let moov = find_atom(&mut file, 0, length, b"moov")?;
    let mvhd = find_atom(&mut file, moov.0, moov.1, b"mvhd")?;
    file.seek(SeekFrom::Start(mvhd.0)).ok()?;
    let mut version = [0u8; 4];
    file.read_exact(&mut version).ok()?;
    let seconds = if version[0] == 1 {
        let mut value = [0u8; 8];
        file.read_exact(&mut value).ok()?;
        i64::try_from(u64::from_be_bytes(value)).ok()?
    } else {
        let mut value = [0u8; 4];
        file.read_exact(&mut value).ok()?;
        i64::from(u32::from_be_bytes(value))
    };
    if seconds == 0 {
        return None;
    }
    Utc.timestamp_opt(seconds - MAC_EPOCH_OFFSET, 0).single()
}

/// The body (start, end) of the first atom of `kind` between `start` and
/// `end`, reading only the atom headers.
fn find_atom(file: &mut File, start: u64, end: u64, kind: &[u8; 4]) -> Option<(u64, u64)> {
    let mut position = start;
    for _ in 0..MAX_ATOMS {
        if position + 8 > end {
            return None;
        }
        file.seek(SeekFrom::Start(position)).ok()?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header).ok()?;
        let size = u64::from(u32::from_be_bytes(header[..4].try_into().ok()?));
        let (body, atom_end) = match size {
            0 => (position + 8, end),
            1 => {
                let mut large = [0u8; 8];
                file.read_exact(&mut large).ok()?;
                (
                    position + 16,
                    position.checked_add(u64::from_be_bytes(large))?,
                )
            }
            size if size < 8 => return None,
            size => (position + 8, position.checked_add(size)?),
        };
        if atom_end > end || atom_end <= position {
            return None;
        }
        if &header[4..] == kind {
            return Some((body, atom_end));
        }
        position = atom_end;
    }
    None
}

/// The recording date tag (ID3v2 `TDRC`) of a file, as local time. A tag
/// with no time of day counts as `None`.
fn tag_time(path: &Path) -> Option<DateTime<Utc>> {
    let file = File::open(path).ok()?;
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            source,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .ok()?;
    let mut metadata = format.metadata();
    let revision = metadata.skip_to_latest()?;
    revision.media.tags.iter().find_map(|tag| match &tag.std {
        Some(StandardTag::RecordingDate(text)) => parse_local(text),
        _ => None,
    })
}

fn parse_local(text: &str) -> Option<DateTime<Utc>> {
    let naive = [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ]
    .iter()
    .find_map(|format| NaiveDateTime::parse_from_str(text.trim(), format).ok())?;
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|local| local.with_timezone(&Utc))
}
