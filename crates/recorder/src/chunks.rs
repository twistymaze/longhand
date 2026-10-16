//! Chunked capture on disk (KTD4).
//!
//! anarlog's `ChunkedSink` reshaped: 60 s chunks at the device's native
//! rate as 16-bit WAV instead of 16 kHz MP3; the 2 s lookback written first
//! into each new chunk so consecutive chunks overlap and a boundary never
//! loses a word; the file name is the metadata; `.part` while open,
//! `sync_all()` and a rename when closed. The segment index leads the name,
//! and there is no archive stream — the Opus archive is encoded after stop.
//! See PROVENANCE.md.
//!
//! A closed chunk is synced and renamed off the writing thread, as anarlog
//! does since its slow-disk fix: on macOS `sync_all` is a full cache flush
//! that can block for seconds while another session is saved, and a writer
//! blocked that long lets the capture ring overrun. At most one such sync is
//! outstanding; its failure surfaces at the next rotation or at finish, and a
//! chunk renamed only after its sync means an unsynced one is still a
//! `.part` that recovery repairs.
//!
//! Names, all times in milliseconds:
//! - open: `{segment}-{capture_started_at}-{start_ms}-{audio_start_ms}.part`
//! - closed: `{segment}-{capture_started_at}-{start_ms}-{end_ms}-{audio_start_ms}.wav`
//!
//! `start_ms` and `end_ms` are on the session timeline; `audio_start_ms` is
//! where the file's first sample sits, up to 2 s before `start_ms`.
//! `capture_started_at` is the segment's wall-clock start in Unix ms.

use std::collections::VecDeque;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

use crate::storage::{self, SpaceProbe};
use crate::wav::{self, WavFile};

pub const CHUNK_SECONDS: u64 = 60;
pub const LOOKBACK_SECONDS: u64 = 2;
/// Seconds of audio between free-space checks.
pub const STORAGE_CHECK_SECONDS: u64 = 5;
/// Seconds of audio between handing buffered bytes to the OS.
pub const FLUSH_SECONDS: u64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum RecorderError {
    #[error(
        "only {available} bytes free where audio is recorded; recording stopped at the floor of {floor} bytes"
    )]
    LowDisk { available: u64, floor: u64 },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkInfo {
    pub path: PathBuf,
    pub segment: u32,
    pub capture_started_at: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub audio_start_ms: u64,
}

pub struct SegmentArgs {
    /// The session's `audio-recovery/` directory.
    pub dir: PathBuf,
    pub segment: u32,
    pub capture_started_at: u64,
    /// Where the segment starts on the session timeline, so a resumed
    /// segment continues it rather than restarting at zero.
    pub offset_ms: u64,
    pub sample_rate: u32,
    /// 1 for a mono segment. 2 for a stereo segment: the microphone on the
    /// left and system audio on the right (R11, KTD6 of the system audio
    /// plan).
    pub channels: u16,
    pub floor_bytes: u64,
}

/// What a closed segment covered on the session timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentSpan {
    pub segment: u32,
    pub start_ms: u64,
    pub end_ms: u64,
}

impl SegmentSpan {
    pub fn duration_ms(&self) -> u64 {
        self.end_ms - self.start_ms
    }
}

pub struct ChunkedWriter {
    args: SegmentArgs,
    probe: SpaceProbe,
    /// Logical start of the open chunk on the session timeline.
    start_ms: u64,
    audio_start_ms: u64,
    /// New (non-lookback) frames in the open chunk. A frame is one sample
    /// of each channel.
    chunk_samples: u64,
    /// Every new frame of the segment, for its end on the timeline.
    segment_samples: u64,
    chunk: Option<WavFile>,
    /// The lookback, as interleaved samples.
    history: VecDeque<f32>,
    since_flush: u64,
    since_space_check: u64,
    /// The previous chunk's sync and rename, still running.
    pending: Option<JoinHandle<std::io::Result<()>>>,
    sync: fn(&File) -> std::io::Result<()>,
}

impl ChunkedWriter {
    pub fn create(args: SegmentArgs) -> Result<Self, RecorderError> {
        Self::with_probe(args, storage::system_probe())
    }

    pub fn with_probe(args: SegmentArgs, probe: SpaceProbe) -> Result<Self, RecorderError> {
        if !matches!(args.channels, 1 | 2) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("a segment has 1 or 2 channels, not {}", args.channels),
            )
            .into());
        }
        std::fs::create_dir_all(&args.dir)?;
        storage::check(probe(&args.dir), args.floor_bytes).map_err(|available| {
            RecorderError::LowDisk {
                available,
                floor: args.floor_bytes,
            }
        })?;
        Ok(Self {
            start_ms: args.offset_ms,
            audio_start_ms: args.offset_ms,
            args,
            probe,
            chunk_samples: 0,
            segment_samples: 0,
            chunk: None,
            history: VecDeque::new(),
            since_flush: 0,
            since_space_check: 0,
            pending: None,
            sync: File::sync_all,
        })
    }

    /// Replace how a closed chunk is made durable, to test a slow or
    /// failing disk.
    pub fn with_sync(mut self, sync: fn(&File) -> std::io::Result<()>) -> Self {
        self.sync = sync;
        self
    }

    pub fn offset_ms(&self) -> u64 {
        self.args.offset_ms
    }

    pub fn sample_rate(&self) -> u32 {
        self.args.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.args.channels
    }

    fn rate(&self) -> u64 {
        self.args.sample_rate as u64
    }

    fn width(&self) -> usize {
        usize::from(self.args.channels)
    }

    fn ms(&self, frames: u64) -> u64 {
        frames * 1000 / self.rate()
    }

    fn partial_path(&self) -> PathBuf {
        self.args.dir.join(format!(
            "{}-{}-{}-{}.part",
            self.args.segment, self.args.capture_started_at, self.start_ms, self.audio_start_ms
        ))
    }

    /// Where the segment currently ends on the session timeline.
    pub fn end_ms(&self) -> u64 {
        self.args.offset_ms + self.ms(self.segment_samples)
    }

    /// Write samples. A stereo segment takes interleaved frames, and a part
    /// frame is refused, so the channels never move by a sample.
    pub fn write(&mut self, samples: &[f32]) -> Result<(), RecorderError> {
        let width = self.width();
        if !samples.len().is_multiple_of(width) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{} samples are not whole frames of {width} channels",
                    samples.len()
                ),
            )
            .into());
        }
        let chunk_len = CHUNK_SECONDS * self.rate();
        let mut rest = samples;
        // Split at the chunk boundary so every closed chunk holds exactly
        // 60 s of new audio and stems stay on whole-second marks.
        while !rest.is_empty() {
            let room = (chunk_len - self.chunk_samples) as usize * width;
            let (now, later) = rest.split_at(room.min(rest.len()));
            self.write_within_chunk(now)?;
            rest = later;
            if self.chunk_samples >= chunk_len {
                self.close_chunk(true)?;
            }
        }
        Ok(())
    }

    fn write_within_chunk(&mut self, samples: &[f32]) -> Result<(), RecorderError> {
        if self.since_space_check >= STORAGE_CHECK_SECONDS * self.rate() {
            self.since_space_check = 0;
            storage::check((self.probe)(&self.args.dir), self.args.floor_bytes).map_err(
                |available| RecorderError::LowDisk {
                    available,
                    floor: self.args.floor_bytes,
                },
            )?;
        }
        let width = self.width();
        if self.chunk.is_none() {
            self.audio_start_ms = self
                .start_ms
                .saturating_sub(self.ms((self.history.len() / width) as u64));
            let mut chunk = WavFile::create(
                &self.partial_path(),
                self.args.sample_rate,
                self.args.channels,
            )?;
            let (a, b) = self.history.as_slices();
            chunk.write(a)?;
            chunk.write(b)?;
            self.chunk = Some(chunk);
        }
        let rate = self.rate();
        let chunk = self.chunk.as_mut().expect("opened above");
        chunk.write(samples)?;
        let n = (samples.len() / width) as u64;
        self.chunk_samples += n;
        self.segment_samples += n;
        self.since_space_check += n;
        self.since_flush += n;

        let limit = (LOOKBACK_SECONDS * rate) as usize * width;
        self.history
            .extend(&samples[samples.len().saturating_sub(limit)..]);
        let excess = self.history.len().saturating_sub(limit);
        self.history.drain(..excess);

        if self.since_flush >= FLUSH_SECONDS * rate {
            self.since_flush = 0;
            chunk.flush()?;
        }
        Ok(())
    }

    /// Close the open chunk; `background` syncs and renames it on a thread
    /// of its own. The previous chunk's sync is waited for either way, so
    /// only one is ever outstanding and its failure stops this writer.
    fn close_chunk(&mut self, background: bool) -> Result<(), RecorderError> {
        let Some(chunk) = self.chunk.take() else {
            return self.join_pending();
        };
        let file = chunk.close()?;
        let end_ms = self.start_ms + self.ms(self.chunk_samples);
        let partial = self.partial_path();
        let closed = self.args.dir.join(format!(
            "{}-{}-{}-{}-{}.wav",
            self.args.segment,
            self.args.capture_started_at,
            self.start_ms,
            end_ms,
            self.audio_start_ms
        ));
        let dir = self.args.dir.clone();
        let sync = self.sync;
        let publish = move || {
            sync(&file)?;
            drop(file);
            std::fs::rename(partial, closed)?;
            sync_dir(&dir)
        };
        let previous = self.join_pending();
        let current = if background {
            self.pending = Some(std::thread::spawn(publish));
            Ok(())
        } else {
            publish().map_err(RecorderError::from)
        };
        self.start_ms = end_ms;
        self.chunk_samples = 0;
        previous?;
        current
    }

    fn join_pending(&mut self) -> Result<(), RecorderError> {
        if let Some(handle) = self.pending.take() {
            handle
                .join()
                .map_err(|_| std::io::Error::other("the chunk sync thread panicked"))??;
        }
        Ok(())
    }

    /// Close the last chunk and report the segment's span.
    pub fn finish(mut self) -> Result<SegmentSpan, RecorderError> {
        self.close_chunk(false)?;
        Ok(SegmentSpan {
            segment: self.args.segment,
            start_ms: self.args.offset_ms,
            end_ms: self.end_ms(),
        })
    }
}

impl Drop for ChunkedWriter {
    /// A writer dropped without `finish` still lets its last sync end, so
    /// what is on disk is settled when the segment is.
    fn drop(&mut self) {
        let _ = self.join_pending();
    }
}

/// A rename is durable only once its directory is.
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    std::fs::File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

fn parse_numbers<const N: usize>(stem: &str) -> Option<[u64; N]> {
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() != N {
        return None;
    }
    let mut out = [0u64; N];
    for (slot, part) in out.iter_mut().zip(parts) {
        *slot = part.parse().ok()?;
    }
    Some(out)
}

/// Closed chunks in `dir`, ordered by segment and then timeline position.
pub fn list_chunks(dir: &Path) -> std::io::Result<Vec<ChunkInfo>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut chunks = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(stem) = name.strip_suffix(".wav") else {
            continue;
        };
        let Some([segment, capture, start, end, audio_start]) = parse_numbers::<5>(stem) else {
            continue;
        };
        let Ok(segment) = u32::try_from(segment) else {
            continue;
        };
        chunks.push(ChunkInfo {
            path: entry.path(),
            segment,
            capture_started_at: capture,
            start_ms: start,
            end_ms: end,
            audio_start_ms: audio_start,
        });
    }
    chunks.sort_by_key(|c| (c.segment, c.start_ms));
    Ok(chunks)
}

/// The channel count of a chunk, from its WAV header: 1 for mono, 2 for
/// stereo. After a crash the open segment has no row, so recovery reads it
/// here (KTD6 of the system audio plan).
pub fn chunk_channels(chunk: &ChunkInfo) -> std::io::Result<u16> {
    wav::read_format(&chunk.path).map(|(_, channels)| channels)
}

/// The segment index of a chunk file name: the first `-` field of a `.wav`
/// or `.part` name. Only that field is read, so a name with other fields
/// that do not parse still gives its segment. Returns `None` for another
/// file.
pub fn chunk_segment_of(file_name: &str) -> Option<u32> {
    let stem = file_name
        .strip_suffix(".wav")
        .or_else(|| file_name.strip_suffix(".part"))?;
    stem.split('-').next()?.parse().ok()
}

/// Remove every chunk of `segment`, closed or open (R35).
pub fn remove_segment(dir: &Path, segment: u32) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let prefix = format!("{segment}-");
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && (name.ends_with(".wav") || name.ends_with(".part")) {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Close every `.part` a crash left open: repair its header from its length
/// and give it the closed name its audio supports (R8). A partial holding
/// nothing past its lookback duplicates the previous chunk and is removed.
/// A partial no longer than a WAV header holds no samples and is removed.
/// Call only while no writer is running on `dir`.
///
/// An error on one file does not stop the others: the file stays in place,
/// a warning is logged, and the report names it in `left`. Only an error
/// that reads the folder itself is returned.
pub fn recover_partial_chunks(dir: &Path) -> std::io::Result<PartialRecovery> {
    let mut report = PartialRecovery::default();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                log::warn!("could not read an entry of {}: {e}", dir.display());
                continue;
            }
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(stem) = name.strip_suffix(".part") else {
            continue;
        };
        let Some([segment, capture, start, audio_start]) = parse_numbers::<4>(stem) else {
            continue;
        };
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_file() => {}
            Ok(_) => continue,
            Err(e) => {
                log::warn!("could not read the type of partial chunk {name} ({e}); left in place");
                report.left.push(path);
                continue;
            }
        }
        // A `.part` no longer than a header holds no samples. A crash before
        // the first flush leaves a 0-byte `.part`, because the writer buffers
        // the header.
        match entry.metadata() {
            Ok(meta) if meta.len() <= wav::HEADER_BYTES => {
                if let Err(e) = std::fs::remove_file(&path) {
                    log::warn!("could not remove empty partial chunk {name} ({e}); left in place");
                    report.left.push(path);
                }
                continue;
            }
            _ => {}
        }
        let repaired = match wav::repair(&path) {
            Ok(repaired) => repaired,
            Err(e) => {
                log::warn!("partial chunk {name} is unreadable ({e}); left in place");
                report.left.push(path);
                continue;
            }
        };
        let end = audio_start + repaired.frames * 1000 / repaired.sample_rate.max(1) as u64;
        if end <= start {
            if let Err(e) = std::fs::remove_file(&path) {
                log::warn!("could not remove partial chunk {name} ({e}); left in place");
                report.left.push(path);
            }
            continue;
        }
        let closed = dir.join(format!(
            "{segment}-{capture}-{start}-{end}-{audio_start}.wav"
        ));
        if let Err(e) = std::fs::rename(&path, &closed) {
            log::warn!("could not close partial chunk {name} ({e}); left in place");
            report.left.push(path);
            continue;
        }
        report.recovered.push(ChunkInfo {
            path: closed,
            segment: segment as u32,
            capture_started_at: capture,
            start_ms: start,
            end_ms: end,
            audio_start_ms: audio_start,
        });
    }
    if !report.recovered.is_empty()
        && let Err(e) = sync_dir(dir)
    {
        log::warn!("could not sync {} after chunk recovery: {e}", dir.display());
    }
    Ok(report)
}

/// What [`recover_partial_chunks`] did in one folder.
#[derive(Debug, Default)]
pub struct PartialRecovery {
    /// The chunks that it closed.
    pub recovered: Vec<ChunkInfo>,
    /// The `.part` files that it could not close. Each stays in place.
    pub left: Vec<PathBuf>,
}
