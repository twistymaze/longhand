//! The speech and level timeline of a segment (KTD1 to KTD4 and KTD6 of the
//! forgotten-recording plan).
//!
//! The segment thread gives each channel to one speech detector and
//! measures its level. It divides the new audio of a segment into cells of
//! 250 ms. Each cell holds, for each recorded channel, the voiced fraction
//! of its detector frames and its RMS level. The chunk writer writes the
//! cells of each closed chunk to a sidecar file beside the chunk:
//! `{segment}-{capture_started_at}-{start_ms}.speech`. The sidecar has no
//! cells for the 2 s lookback, so the cells of a segment do not overlap.
//!
//! Sidecar format, version 1: one byte for the version, one byte for the
//! channel count, then for each cell and each channel one byte for the
//! voiced fraction (0 to 255 for 0.0 to 1.0) and one byte for the level
//! ([`ChannelCell::level_dbfs`]).
//!
//! A cell with no data, for example in the open chunk after a crash or in a
//! chunk that an older build wrote, counts as speech.
//!
//! The per-note counters ([`GuardMeter`]) are atomics. The segment thread
//! adds the time of each cell, and sets the silence time to zero at speech.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use audio::resampler::FrameResampler;
use vad::{EarshotVad, VAD_SAMPLE_RATE, VoiceActivityDetector};

use crate::chunks::{CHUNK_SECONDS, ChunkInfo};

/// The time of one cell.
pub const CELL_MS: u64 = 250;
/// The cells of one full chunk.
pub const CELLS_PER_CHUNK: u64 = CHUNK_SECONDS * 1000 / CELL_MS;
/// A channel of a cell is voiced when at least this fraction of its
/// detector frames is voiced (KTD3).
pub const VOICED_CELL_FRACTION: f32 = 0.5;
/// Speech is at least this number of voiced cells on one channel ...
pub const SPEECH_CELLS: u32 = 4;
/// ... within this number of cells in a row (KTD3): 1 s within 2 s.
pub const SPEECH_WINDOW_CELLS: u32 = 8;
/// A chunk is quiet when the level at [`QUIET_PERCENTILE`] of its cells is
/// at or below this level on every recorded channel (KTD4).
pub const QUIET_LEVEL_DBFS: f32 = -50.0;
/// The percentile of the cell levels that the quiet test uses (KTD4).
pub const QUIET_PERCENTILE: f32 = 0.95;
/// The Earshot threshold (KTD1).
pub const EARSHOT_THRESHOLD: f32 = 0.5;
/// The sidecar format version.
pub const SIDECAR_VERSION: u8 = 1;

const SIDECAR_EXTENSION: &str = "speech";
const TEMPORARY_SUFFIX: &str = ".speech.tmp";
/// The level byte of a cell with only digital silence.
const SILENT_LEVEL: u8 = u8::MAX;
/// The time of one Earshot frame: 256 samples at 16 kHz (KTD1).
const EARSHOT_FRAME: Duration = Duration::from_millis(16);

/// One channel of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelCell {
    /// The voiced fraction of the detector frames, 0 to 255 for 0.0 to 1.0.
    pub voiced: u8,
    /// The RMS level: minus the level in whole dBFS, from 0 to 254. 255 is
    /// digital silence.
    pub level: u8,
}

impl ChannelCell {
    /// A channel cell from the voiced detector frames of `frames`, and the
    /// sum of the squares of `samples` samples.
    pub fn from_counts(voiced_frames: u32, frames: u32, square_sum: f64, samples: u64) -> Self {
        let voiced = if frames == 0 {
            0
        } else {
            (f64::from(voiced_frames) * 255.0 / f64::from(frames)).round() as u8
        };
        let rms = if samples == 0 {
            0.0
        } else {
            (square_sum / samples as f64).sqrt()
        };
        let level = if rms > 0.0 {
            (-20.0 * rms.log10()).round().clamp(0.0, 254.0) as u8
        } else {
            SILENT_LEVEL
        };
        Self { voiced, level }
    }

    pub fn voiced_fraction(&self) -> f32 {
        f32::from(self.voiced) / 255.0
    }

    /// Whether at least [`VOICED_CELL_FRACTION`] of the frames is voiced.
    pub fn is_voiced(&self) -> bool {
        self.voiced_fraction() >= VOICED_CELL_FRACTION
    }

    /// The RMS level in dBFS. Digital silence is negative infinity.
    pub fn level_dbfs(&self) -> f32 {
        if self.level == SILENT_LEVEL {
            f32::NEG_INFINITY
        } else {
            -f32::from(self.level)
        }
    }
}

/// 250 ms of new audio of a segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// The microphone: the only channel of a mono segment, or the left
    /// channel of a stereo segment.
    pub mic: ChannelCell,
    /// The app audio channel of a stereo segment. `None` for a mono segment.
    pub app: Option<ChannelCell>,
}

impl Cell {
    /// The recorded channels: the microphone, then the app audio.
    pub fn channels(&self) -> impl Iterator<Item = &ChannelCell> {
        std::iter::once(&self.mic).chain(self.app.as_ref())
    }
}

/// A speech detector for one channel.
pub trait VoiceDetector: Send {
    /// Take native-rate samples of the channel. Call `verdict` once for each
    /// detector frame that the samples complete: true for a voiced frame.
    fn push(&mut self, samples: &[f32], verdict: &mut dyn FnMut(bool));
}

/// Earshot on one channel (KTD1): a resampler to 16 kHz, then a bare
/// [`EarshotVad`] on frames of 256 samples. A frame's verdict goes to the
/// cell that receives the sample that completes the frame. The resampler
/// delay moves a verdict by some tens of milliseconds, which is small
/// against a cell.
pub struct EarshotDetector {
    resampler: FrameResampler,
    vad: EarshotVad,
}

impl EarshotDetector {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            resampler: FrameResampler::new(
                sample_rate as usize,
                VAD_SAMPLE_RATE as usize,
                EARSHOT_FRAME,
            ),
            vad: EarshotVad::new(EARSHOT_THRESHOLD).expect("the threshold is in range"),
        }
    }
}

impl VoiceDetector for EarshotDetector {
    fn push(&mut self, samples: &[f32], verdict: &mut dyn FnMut(bool)) {
        let vad = &mut self.vad;
        self.resampler.push(samples, |frame| {
            // A frame that Earshot refuses (a sample that is not finite) is
            // not voiced.
            verdict(vad.is_voice(frame).unwrap_or(false));
        });
    }
}

/// The sums of one channel in the open cell.
#[derive(Default, Clone, Copy)]
struct ChannelSums {
    square_sum: f64,
    frames: u32,
    voiced_frames: u32,
}

/// Divides the new audio of a segment into cells (KTD2).
pub struct CellBuilder {
    rate: u64,
    detectors: Vec<Box<dyn VoiceDetector>>,
    sums: Vec<ChannelSums>,
    /// The index of the open cell in the segment.
    index: u64,
    /// The sample frames in the open cell. A sample frame is one sample of
    /// each channel.
    filled: u64,
    scratch: Vec<f32>,
}

impl CellBuilder {
    /// A builder with one detector for each channel: 1 for a mono segment,
    /// 2 for a stereo segment.
    pub fn new(sample_rate: u32, detectors: Vec<Box<dyn VoiceDetector>>) -> Self {
        assert!(
            matches!(detectors.len(), 1 | 2),
            "a segment has 1 or 2 channels"
        );
        Self {
            rate: u64::from(sample_rate.max(1)),
            sums: vec![ChannelSums::default(); detectors.len()],
            detectors,
            index: 0,
            filled: 0,
            scratch: Vec::new(),
        }
    }

    pub fn channels(&self) -> u16 {
        self.detectors.len() as u16
    }

    /// The sample frames of the open cell. The cell ends are rounded on the
    /// segment's sample count, so a full chunk holds exactly
    /// [`CELLS_PER_CHUNK`] cells at every rate.
    fn cell_frames(&self) -> u64 {
        let end = |i: u64| i * self.rate * CELL_MS / 1000;
        end(self.index + 1) - end(self.index)
    }

    /// Add interleaved sample frames. `on_cell` gets each cell that the
    /// samples complete.
    pub fn push(&mut self, mut samples: &[f32], on_cell: &mut impl FnMut(Cell)) {
        let width = self.detectors.len();
        while samples.len() >= width {
            let room = (self.cell_frames() - self.filled) as usize;
            let take = room.min(samples.len() / width);
            let (now, later) = samples.split_at(take * width);
            for (channel, (detector, sums)) in self
                .detectors
                .iter_mut()
                .zip(self.sums.iter_mut())
                .enumerate()
            {
                let one: &[f32] = if width == 1 {
                    now
                } else {
                    self.scratch.clear();
                    self.scratch
                        .extend(now.iter().skip(channel).step_by(width).copied());
                    &self.scratch
                };
                sums.square_sum += one
                    .iter()
                    .map(|&s| f64::from(s) * f64::from(s))
                    .sum::<f64>();
                detector.push(one, &mut |voiced| {
                    sums.frames += 1;
                    sums.voiced_frames += u32::from(voiced);
                });
            }
            self.filled += take as u64;
            samples = later;
            if self.filled == self.cell_frames() {
                on_cell(self.close_cell());
            }
        }
    }

    /// Close the open cell when it has audio: the last cell of a segment
    /// can be shorter than 250 ms.
    pub fn finish(&mut self, on_cell: &mut impl FnMut(Cell)) {
        if self.filled > 0 {
            on_cell(self.close_cell());
        }
    }

    fn close_cell(&mut self) -> Cell {
        let filled = self.filled;
        let mut cells = self.sums.iter().map(|sums| {
            ChannelCell::from_counts(sums.voiced_frames, sums.frames, sums.square_sum, filled)
        });
        let cell = Cell {
            mic: cells.next().expect("one channel at least"),
            app: cells.next(),
        };
        self.sums.fill(ChannelSums::default());
        self.index += 1;
        self.filled = 0;
        cell
    }
}

/// The speech test of KTD3 on one channel: at least [`SPEECH_CELLS`]
/// voiced cells within [`SPEECH_WINDOW_CELLS`] cells in a row.
#[derive(Debug, Default, Clone, Copy)]
pub struct SpeechWindow {
    /// The last cells, the newest in the lowest bit.
    recent: u32,
}

impl SpeechWindow {
    /// Add the next cell. Returns whether the window holds speech now.
    pub fn push(&mut self, voiced: bool) -> bool {
        let mask = (1u32 << SPEECH_WINDOW_CELLS) - 1;
        self.recent = ((self.recent << 1) | u32::from(voiced)) & mask;
        self.recent.count_ones() >= SPEECH_CELLS
    }
}

/// The per-note counters of the silence check and the time limit (KTD6).
/// The segment threads of the note add to them. Only Continue and Keep
/// Recording set the silence time to zero from outside. A new note,
/// Resume and Record into a note start a new meter.
#[derive(Debug, Default)]
pub struct GuardMeter {
    silence_ms: AtomicU64,
    run_ms: AtomicU64,
}

impl GuardMeter {
    /// The recording time since the last speech.
    pub fn silence_ms(&self) -> u64 {
        self.silence_ms.load(Ordering::Relaxed)
    }

    /// The recording time of the current recording.
    pub fn run_ms(&self) -> u64 {
        self.run_ms.load(Ordering::Relaxed)
    }

    /// Start the silence check again from zero.
    pub fn reset_silence(&self) {
        self.silence_ms.store(0, Ordering::Relaxed);
    }

    /// Add `ms` of recording time. Speech sets the silence time to zero.
    pub fn add(&self, ms: u64, speech: bool) {
        self.run_ms.fetch_add(ms, Ordering::Relaxed);
        if speech {
            self.silence_ms.store(0, Ordering::Relaxed);
        } else {
            self.silence_ms.fetch_add(ms, Ordering::Relaxed);
        }
    }
}

/// Gives the cells of one segment to the note's [`GuardMeter`].
pub struct GuardFeed {
    meter: Arc<GuardMeter>,
    mic: SpeechWindow,
    app: SpeechWindow,
}

impl GuardFeed {
    pub fn new(meter: Arc<GuardMeter>) -> Self {
        Self {
            meter,
            mic: SpeechWindow::default(),
            app: SpeechWindow::default(),
        }
    }

    /// Add one cell: speech on either channel sets the silence time to zero.
    pub fn cell(&mut self, cell: &Cell) {
        let mic = self.mic.push(cell.mic.is_voiced());
        let app = cell.app.is_some_and(|app| self.app.push(app.is_voiced()));
        self.meter.add(CELL_MS, mic || app);
    }
}

/// The sidecar of the chunk that starts at `start_ms`.
pub fn sidecar_path(dir: &Path, segment: u32, capture_started_at: u64, start_ms: u64) -> PathBuf {
    dir.join(format!(
        "{segment}-{capture_started_at}-{start_ms}.{SIDECAR_EXTENSION}"
    ))
}

/// The sidecar of `chunk`.
pub fn sidecar_of(chunk: &ChunkInfo) -> PathBuf {
    let dir = chunk.path.parent().unwrap_or(Path::new(""));
    sidecar_path(dir, chunk.segment, chunk.capture_started_at, chunk.start_ms)
}

/// Whether `file_name` is a sidecar of `segment`, or its temporary file.
pub(crate) fn is_sidecar_of(file_name: &str, segment: u32) -> bool {
    let Some(rest) = file_name.strip_prefix(&format!("{segment}-")) else {
        return false;
    };
    rest.ends_with(&format!(".{SIDECAR_EXTENSION}")) || rest.ends_with(TEMPORARY_SUFFIX)
}

/// The bytes of a sidecar.
fn encode(channels: u16, cells: &[Cell]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(2 + cells.len() * 2 * usize::from(channels));
    bytes.push(SIDECAR_VERSION);
    bytes.push(channels as u8);
    for cell in cells {
        for channel in cell.channels() {
            bytes.push(channel.voiced);
            bytes.push(channel.level);
        }
    }
    bytes
}

/// Write a sidecar through a temporary file and a rename, so a sidecar is
/// never a part file. `sync` makes the temporary file durable before the
/// rename.
pub fn write_sidecar(
    path: &Path,
    channels: u16,
    cells: &[Cell],
    sync: fn(&File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let written = (|| {
        let mut file = File::create(&temporary)?;
        file.write_all(&encode(channels, cells))?;
        sync(&file)?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// The cells of a sidecar file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sidecar {
    pub channels: u16,
    pub cells: Vec<Cell>,
}

/// Read a sidecar. `None` when the file does not exist. A file in another
/// format version or with a part cell is an error.
pub fn read_sidecar(path: &Path) -> std::io::Result<Option<Sidecar>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let invalid = |why: String| std::io::Error::new(std::io::ErrorKind::InvalidData, why);
    let [version, channels, body @ ..] = bytes.as_slice() else {
        return Err(invalid("the sidecar has no header".into()));
    };
    if *version != SIDECAR_VERSION {
        return Err(invalid(format!("sidecar format version {version}")));
    }
    if !matches!(channels, 1 | 2) {
        return Err(invalid(format!("a sidecar with {channels} channels")));
    }
    let width = usize::from(*channels) * 2;
    if !body.len().is_multiple_of(width) {
        return Err(invalid("the sidecar ends in a part cell".into()));
    }
    let cells = body
        .chunks_exact(width)
        .map(|cell| Cell {
            mic: ChannelCell {
                voiced: cell[0],
                level: cell[1],
            },
            app: (width == 4).then(|| ChannelCell {
                voiced: cell[2],
                level: cell[3],
            }),
        })
        .collect();
    Ok(Some(Sidecar {
        channels: u16::from(*channels),
        cells,
    }))
}

/// The cells of the closed chunk `chunk`, one for each 250 ms of its new
/// audio. A cell with no data is `None`: the chunk has no sidecar (the open
/// chunk at a crash, or a chunk that an older build wrote), the sidecar
/// cannot be read, or it has fewer cells. A `None` cell counts as speech.
///
/// Capture writes the sidecar before it renames the chunk to its closed
/// name, so a chunk that a reader lists has its sidecar.
pub fn chunk_cells(chunk: &ChunkInfo) -> Vec<Option<Cell>> {
    let count = chunk
        .end_ms
        .saturating_sub(chunk.start_ms)
        .div_ceil(CELL_MS) as usize;
    let path = sidecar_of(chunk);
    let cells = match read_sidecar(&path) {
        Ok(Some(sidecar)) => sidecar.cells,
        Ok(None) => Vec::new(),
        Err(e) => {
            log::warn!(
                "could not read the sidecar {} ({e}); its cells count as speech",
                path.display()
            );
            Vec::new()
        }
    };
    (0..count).map(|i| cells.get(i).copied()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_cell_keeps_the_fraction_and_the_level() {
        // Half of 16 frames, at an RMS of 0.01 (-40 dBFS).
        let cell = ChannelCell::from_counts(8, 16, 0.0001 * 100.0, 100);
        assert!(cell.is_voiced());
        assert_eq!(cell.level_dbfs(), -40.0);
        let below = ChannelCell::from_counts(7, 16, 0.0, 100);
        assert!(!below.is_voiced());
        assert_eq!(below.level_dbfs(), f32::NEG_INFINITY);
    }

    #[test]
    fn speech_is_four_voiced_cells_within_eight() {
        let mut window = SpeechWindow::default();
        // Voiced, unvoiced in turn: the fourth voiced cell is the seventh.
        let results: Vec<bool> = [true, false, true, false, true, false, true]
            .into_iter()
            .map(|v| window.push(v))
            .collect();
        assert_eq!(results, [false, false, false, false, false, false, true]);
        // Four voiced cells 8 cells apart are not speech.
        let mut window = SpeechWindow::default();
        let spread = [
            true, false, false, true, false, false, true, false, false, true,
        ];
        assert!(!spread.into_iter().any(|v| window.push(v)));
    }

    #[test]
    fn a_sidecar_keeps_its_cells() {
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar_path(dir.path(), 3, 1_700_000_000_000, 60_000);
        let cells = vec![
            Cell {
                mic: ChannelCell {
                    voiced: 255,
                    level: 30,
                },
                app: Some(ChannelCell {
                    voiced: 0,
                    level: SILENT_LEVEL,
                }),
            };
            3
        ];
        write_sidecar(&path, 2, &cells, File::sync_all).unwrap();
        assert_eq!(
            read_sidecar(&path).unwrap(),
            Some(Sidecar { channels: 2, cells })
        );
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            "3-1700000000000-60000.speech"
        );
        // No temporary file stays.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_sidecar_in_another_version_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("0-1-0.speech");
        std::fs::write(&path, [9, 1, 0, 0]).unwrap();
        assert!(read_sidecar(&path).is_err());
        std::fs::write(&path, [SIDECAR_VERSION, 2, 0, 0]).unwrap();
        assert!(read_sidecar(&path).is_err());
    }
}
