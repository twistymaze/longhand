//! The assemble stage (KTD4, KD16): a segment's overlapping native-rate
//! chunks become one continuous stream, written twice — the full-band Opus
//! archive that is saved with the session, and the 16 kHz mono f32 tap that
//! recognition and diarization read. Audio streams through in blocks; no
//! segment is ever held in memory (R7).
//!
//! A stereo segment (the microphone on the left, app audio on the right)
//! keeps both raw channels in its archive, as two uncoupled Opus streams
//! (KTD7). Its processing file is the plain mix of the two channels, the
//! same mix that the live transcript reads (KTD9 of the app audio plan). Its
//! echo spans row, with no spans, records that the segment is done (KTD10;
//! see `echo.rs`).

use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use ogg::writing::{PacketWriteEndInfo, PacketWriter};
use serde::{Deserialize, Serialize};
use session_core::{Segment, Stage};

use crate::echo::{self, EchoSpans};
use crate::resample::StreamResampler;
use crate::{Control, RECOVERY_DIR, SharedStore, StageError, checkpoint, opus_ms, proc_file};

pub const PROC_RATE: u32 = 16_000;
/// Opus always runs at 48 kHz internally; other input rates are resampled.
const OPUS_RATE: u32 = 48_000;
const OPUS_FRAME: usize = 960; // 20 ms
/// The bitrate of each channel's stream.
const OPUS_BITRATE: i32 = 64_000;
/// Packets per Ogg page: one second of audio.
const PACKETS_PER_PAGE: u64 = 50;
/// Samples of each channel in one block.
const BLOCK: usize = 4_096;

/// The assemble checkpoint for one segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssembledSegment {
    pub segment: u32,
    pub native_rate: u32,
    /// Samples of each channel in the trimmed native-rate stream.
    pub native_samples: u64,
    /// Samples in the 16 kHz processing file.
    pub proc_samples: u64,
}

/// What assembling one segment made: its checkpoint and, for a stereo
/// segment, its echo spans.
#[derive(Debug, Clone, PartialEq)]
pub struct Assembled {
    pub segment: AssembledSegment,
    pub echo: Option<EchoSpans>,
}

/// Refuse to start when the working location cannot hold what assembling
/// the remaining segments writes (R7b): better named now, with the session
/// whole, than as a write failure part-way through. `available` is the free
/// space; `None` when unknown, which passes.
pub fn check_space(
    store: &SharedStore,
    note_id: &str,
    segments: &[Segment],
    available: Option<u64>,
) -> Result<(), StageError> {
    let mut needed = 0;
    for segment in segments {
        let key = checkpoint::segment_key(segment.index);
        if checkpoint::get::<AssembledSegment>(store, note_id, Stage::Assemble, &key)?.is_none() {
            let seconds = segment.duration_ms.div_ceil(1_000);
            needed += recorder::storage::processing_bytes_per_second(segment.channels) * seconds;
        }
    }
    recorder::storage::check_headroom(available, needed, recorder::storage::DISK_RESERVE_BYTES)
        .map_err(|free| {
            StageError::Failed(format!(
                "processing needs about {} free on the disk holding Longhand's working audio, and {} is free; free some space, then retry",
                gigabytes(needed + recorder::storage::DISK_RESERVE_BYTES),
                gigabytes(free)
            ))
        })
}

fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// Assemble every segment of the session that is not yet assembled. `home`
/// is the folder a resumed session completed into (R6d): a segment whose
/// archive is there is already assembled, and its processing file is decoded
/// back from that archive.
pub fn run(
    store: &SharedStore,
    note_id: &str,
    session_dir: &Path,
    segments: &[Segment],
    home: Option<&Path>,
    control: &dyn Control,
) -> Result<Vec<AssembledSegment>, StageError> {
    let exact = crate::lock(store).recognition_snapshot(note_id)?.is_some();
    let recovery = session_dir.join(RECOVERY_DIR);
    // A crash while capturing leaves a `.part` open; close it from its length.
    recorder::recover_partial_chunks(&recovery)?;
    check_space(
        store,
        note_id,
        segments,
        recorder::storage::available_space(session_dir),
    )?;
    // A resumed saved note reads its old segments from their copy in the
    // working location (KTD8 of the audio removal plan). A copy that the
    // Resume did not complete is made now.
    let old = match home {
        Some(home) => {
            crate::old_audio::copy(session_dir, &|| Some(home.to_path_buf()))?;
            crate::old_audio::old_segments(session_dir)?.unwrap_or_default()
        }
        None => Vec::new(),
    };
    let old_state = |segment: &Segment| {
        old.iter()
            .find(|o| o.audio_file == segment.audio_file)
            .map(|o| o.state)
    };
    let mut out = Vec::new();
    for segment in segments {
        let key = checkpoint::segment_key(segment.index);
        // A synced note record gives the audio name. A name that can point
        // outside the folder counts as a missing file.
        let local = crate::audio_path(session_dir, &segment.audio_file);
        // A stereo segment is done only with its echo spans row too.
        if let Some(done) =
            checkpoint::get::<AssembledSegment>(store, note_id, Stage::Assemble, &key)?
            && local.as_ref().is_some_and(|local| local.exists())
            && session_dir.join(proc_file(&segment.audio_file)).exists()
            && (segment.channels != 2 || echo::saved(store, note_id, segment.index)?.is_some())
            && (!exact
                || provenance_valid(store, note_id, session_dir, std::slice::from_ref(segment))?)
        {
            out.push(done);
            continue;
        }
        let copied = old_state(segment) == Some(crate::old_audio::CopyState::Copied);
        let local_archive = if copied
            || exact
                && checkpoint::get::<AssembledSegment>(store, note_id, Stage::Assemble, &key)?
                    .is_some()
        {
            local.filter(|p| p.is_file())
        } else {
            None
        };
        if let Some(archive) = local_archive.or_else(|| {
            home.and_then(|home| crate::audio_path(home, &segment.audio_file))
                .filter(|archive| archive.exists())
        }) {
            control.proceed()?;
            let before = session_core::recognition::file_digest(&archive)?;
            let assembled = decode_segment(&archive, session_dir, segment, control)?;
            if session_core::recognition::file_digest(&archive)? != before {
                return Err(StageError::Failed(
                    "recorded audio changed during decode".into(),
                ));
            }
            record(
                store,
                note_id,
                session_dir,
                segment,
                Some(&archive),
                &assembled,
            )?;
            log::info!(
                "decoded segment {} of {note_id} from {}",
                segment.index,
                archive.display()
            );
            out.push(assembled.segment);
            continue;
        }
        if let Some(home) = home
            && old_state(segment) == Some(crate::old_audio::CopyState::Removed)
        {
            return Err(StageError::Failed(format!(
                "the audio of segment {} was removed from {} before Longhand copied it for the Resume",
                segment.index + 1,
                home.display()
            )));
        }
        if let Some(home) = home
            && !recorder::list_chunks(&recovery)?
                .iter()
                .any(|c| c.segment == segment.index)
        {
            return Err(StageError::Failed(format!(
                "segment {}'s audio is in {}, which is missing or incomplete; put the folder back and retry",
                segment.index + 1,
                home.display()
            )));
        }
        control.proceed()?;
        let assembled = assemble_segment(&recovery, session_dir, segment, control)?;
        record(store, note_id, session_dir, segment, None, &assembled)?;
        log::info!(
            "assembled segment {} of {note_id}: {} samples at {} Hz",
            segment.index,
            assembled.segment.native_samples,
            assembled.segment.native_rate
        );
        out.push(assembled.segment);
    }
    checkpoint::mark_complete(store, note_id, Stage::Assemble, segments.len())?;
    Ok(out)
}

/// Save the echo spans row, then record the segment as done (step, then
/// record): a crash between the two assembles the segment again and replaces
/// the row.
fn record(
    store: &SharedStore,
    note_id: &str,
    dir: &Path,
    segment: &Segment,
    archive: Option<&Path>,
    assembled: &Assembled,
) -> Result<(), StageError> {
    let index = assembled.segment.segment;
    if let Some(echo) = &assembled.echo {
        checkpoint::put(
            store,
            note_id,
            Stage::Assemble,
            &checkpoint::echo_key(index),
            echo,
        )?;
    }
    checkpoint::put(
        store,
        note_id,
        Stage::Assemble,
        &provenance_key(index),
        &audio_provenance_at(store, note_id, dir, segment, archive)?,
    )?;
    checkpoint::put(
        store,
        note_id,
        Stage::Assemble,
        &checkpoint::segment_key(index),
        &assembled.segment,
    )
}

/// Where each block of a segment goes after the archive.
#[allow(clippy::large_enum_variant, reason = "one value for each segment")]
enum Sink {
    Mono {
        to_opus: StreamResampler,
        to_proc: StreamResampler,
        proc: ProcWriter,
    },
    Stereo {
        to_opus: [StreamResampler; 2],
        to_proc: StreamResampler,
        proc: ProcWriter,
    },
}

/// Run each channel through its resampler and return the outputs
/// interleaved. Both resamplers have one configuration and take the same
/// lengths, so they give the same lengths.
fn resample_pair(
    resamplers: &mut [StreamResampler; 2],
    channels: [&[f32]; 2],
    finish: bool,
) -> [Vec<f32>; 2] {
    let mut out = [Vec::new(), Vec::new()];
    for ((resampler, input), out) in resamplers.iter_mut().zip(channels).zip(&mut out) {
        let mut emit = |s: &[f32]| out.extend_from_slice(s);
        if finish {
            resampler.finish(&mut emit);
        } else {
            resampler.push(input, &mut emit);
        }
    }
    debug_assert_eq!(out[0].len(), out[1].len());
    out
}

fn interleave(left: &[f32], right: &[f32]) -> Vec<f32> {
    left.iter().zip(right).flat_map(|(&l, &r)| [l, r]).collect()
}

/// Split interleaved stereo `pcm` into its left and right channels.
fn deinterleave(pcm: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let left = pcm.iter().step_by(2).copied().collect();
    let right = pcm.iter().skip(1).step_by(2).copied().collect();
    (left, right)
}

impl Sink {
    fn new(rate: u32, channels: u16, processing: &Path) -> Result<Self, StageError> {
        Ok(match channels {
            1 => Sink::Mono {
                to_opus: StreamResampler::new(rate, OPUS_RATE),
                to_proc: StreamResampler::new(rate, PROC_RATE),
                proc: ProcWriter::create(&temp_path(processing))?,
            },
            _ => Sink::Stereo {
                to_opus: [
                    StreamResampler::new(rate, OPUS_RATE),
                    StreamResampler::new(rate, OPUS_RATE),
                ],
                to_proc: StreamResampler::new(rate, PROC_RATE),
                proc: ProcWriter::create(&temp_path(processing))?,
            },
        })
    }

    /// Take a block of interleaved native-rate samples.
    fn push(&mut self, opus: &mut OpusWriter, block: &[f32]) -> Result<(), StageError> {
        match self {
            Sink::Mono {
                to_opus,
                to_proc,
                proc,
            } => {
                let mut out = Vec::new();
                to_opus.push(block, &mut |s| out.extend_from_slice(s));
                opus.push(&out)?;
                out.clear();
                to_proc.push(block, &mut |s| out.extend_from_slice(s));
                proc.push(&out)
            }
            Sink::Stereo {
                to_opus,
                to_proc,
                proc,
            } => {
                let (left, right) = deinterleave(block);
                let [l, r] = resample_pair(to_opus, [&left, &right], false);
                opus.push(&interleave(&l, &r))?;
                let mut out = Vec::new();
                to_proc.push(&audio::live_mix::mix(&left, &right), &mut |s| {
                    out.extend_from_slice(s)
                });
                proc.push(&out)
            }
        }
    }

    /// Flush the resamplers and give the processing file writer.
    fn finish(self, opus: &mut OpusWriter) -> Result<ProcWriter, StageError> {
        match self {
            Sink::Mono {
                mut to_opus,
                mut to_proc,
                mut proc,
            } => {
                let mut out = Vec::new();
                to_opus.finish(&mut |s| out.extend_from_slice(s));
                opus.push(&out)?;
                out.clear();
                to_proc.finish(&mut |s| out.extend_from_slice(s));
                proc.push(&out)?;
                Ok(proc)
            }
            Sink::Stereo {
                mut to_opus,
                mut to_proc,
                mut proc,
            } => {
                let [l, r] = resample_pair(&mut to_opus, [&[], &[]], true);
                opus.push(&interleave(&l, &r))?;
                let mut out = Vec::new();
                to_proc.finish(&mut |s| out.extend_from_slice(s));
                proc.push(&out)?;
                Ok(proc)
            }
        }
    }
}

/// Commit the processing file of a segment. Returns its samples and, for a
/// stereo segment, its echo spans row (KTD10).
fn finish_processing(
    proc: ProcWriter,
    processing: &Path,
    segment: u32,
    channels: u16,
) -> Result<(u64, Option<EchoSpans>), StageError> {
    let samples = proc.finish()?;
    commit(&temp_path(processing), processing)?;
    Ok((
        samples,
        (channels == 2).then(|| EchoSpans::plain_mix(segment)),
    ))
}

pub fn assemble_segment(
    recovery: &Path,
    session_dir: &Path,
    segment: &Segment,
    control: &dyn Control,
) -> Result<Assembled, StageError> {
    let chunks: Vec<_> = recorder::list_chunks(recovery)?
        .into_iter()
        .filter(|c| c.segment == segment.index)
        .collect();
    let Some(first) = chunks.first() else {
        return Err(StageError::Failed(format!(
            "segment {} has no audio chunks",
            segment.index
        )));
    };
    let spec = hound::WavReader::open(&first.path)?.spec();
    let (rate, channels) = (spec.sample_rate, spec.channels);
    if !matches!(channels, 1 | 2) || channels != segment.channels {
        return Err(StageError::Failed(format!(
            "segment {} has {} channels in {} and {} in the store",
            segment.index,
            channels,
            first.path.display(),
            segment.channels
        )));
    }
    let width = usize::from(channels);

    let archive = crate::audio_path(session_dir, &segment.audio_file)
        .ok_or_else(|| StageError::Failed(format!("bad audio name {}", segment.audio_file)))?;
    let processing = session_dir.join(proc_file(&segment.audio_file));
    let _temps = TempFiles(vec![temp_path(&archive), temp_path(&processing)]);
    let mut opus = OpusWriter::create(&temp_path(&archive), rate, channels)?;
    let mut sink = Sink::new(rate, channels, &processing)?;

    let mut native_samples = 0u64;
    let mut expected_ms = first.start_ms;
    let mut block = Vec::with_capacity(BLOCK * width);
    for chunk in &chunks {
        control.proceed()?;
        let mut reader = hound::WavReader::open(&chunk.path)?;
        let spec = reader.spec();
        if spec.sample_rate != rate || spec.channels != channels || spec.bits_per_sample != 16 {
            return Err(StageError::Failed(format!(
                "chunk {} is not 16-bit with {channels} channels at {rate} Hz",
                chunk.path.display()
            )));
        }
        // The audio in the file starts at `audio_start_ms`. Skip the part
        // before `from`, which the previous chunk already has.
        let mut from = expected_ms;
        // A missing chunk leaves a hole. The lookback of this chunk fills the
        // end of the hole, and silence fills the part before the lookback.
        if chunk.start_ms > expected_ms {
            from = expected_ms.max(chunk.audio_start_ms);
            log::warn!(
                "segment {}: {} ms missing before {}; filled with silence",
                segment.index,
                from - expected_ms,
                chunk.path.display()
            );
            // Push the previous chunk's pending samples before the silence.
            block.truncate(block.len() / width * width);
            if !block.is_empty() {
                native_samples += (block.len() / width) as u64;
                sink.push(&mut opus, &block)?;
                block.clear();
            }
            let gap = ms_to_samples(from - expected_ms, rate);
            for part in vec![0.0f32; gap as usize * width].chunks(BLOCK * width) {
                native_samples += (part.len() / width) as u64;
                sink.push(&mut opus, part)?;
            }
        }
        let skip = ms_to_samples(from.saturating_sub(chunk.audio_start_ms), rate) * width as u64;
        for (i, sample) in reader.samples::<i16>().enumerate() {
            if (i as u64) < skip {
                continue;
            }
            block.push(sample? as f32 / 32_768.0);
            if block.len() == BLOCK * width {
                native_samples += BLOCK as u64;
                sink.push(&mut opus, &block)?;
                block.clear();
            }
        }
        expected_ms = expected_ms.max(chunk.end_ms);
    }
    // A chunk cut by a crash can end inside a frame; drop the part frame.
    block.truncate(block.len() / width * width);
    if !block.is_empty() {
        native_samples += (block.len() / width) as u64;
        sink.push(&mut opus, &block)?;
    }
    let proc = sink.finish(&mut opus)?;
    opus.finish()?;
    let (proc_samples, echo) = finish_processing(proc, &processing, segment.index, channels)?;
    commit(&temp_path(&archive), &archive)?;

    Ok(Assembled {
        segment: AssembledSegment {
            segment: segment.index,
            native_rate: rate,
            native_samples,
            proc_samples,
        },
        echo,
    })
}

/// Decode an archive this stage wrote back into a 16 kHz processing file,
/// in the format of the archive: a mono archive as it is, a stereo archive
/// through the plain mix, as when it was assembled (R17).
fn decode_segment(
    archive: &Path,
    session_dir: &Path,
    segment: &Segment,
    control: &dyn Control,
) -> Result<Assembled, StageError> {
    let processing = session_dir.join(proc_file(&segment.audio_file));
    let _temps = TempFiles(vec![temp_path(&processing)]);
    let mut reader = ArchiveReader::open(archive)?;
    let channels = reader.channels();
    let mut to_proc = StreamResampler::new(OPUS_RATE, PROC_RATE);
    let mut proc = ProcWriter::create(&temp_path(&processing))?;
    let mut out = Vec::new();
    let mut pcm = Vec::new();
    let mut packets = 0u64;
    loop {
        pcm.clear();
        if !reader.read(&mut pcm)? {
            break;
        }
        packets += 1;
        if packets.is_multiple_of(PACKETS_PER_PAGE * 60) {
            control.proceed()?;
        }
        out.clear();
        if channels == 2 {
            let (left, right) = deinterleave(&pcm);
            to_proc.push(&audio::live_mix::mix(&left, &right), &mut |s| {
                out.extend_from_slice(s)
            });
        } else {
            to_proc.push(&pcm, &mut |s| out.extend_from_slice(s));
        }
        proc.push(&out)?;
    }
    out.clear();
    to_proc.finish(&mut |s| out.extend_from_slice(s));
    proc.push(&out)?;
    let (proc_samples, echo) = finish_processing(proc, &processing, segment.index, channels)?;
    Ok(Assembled {
        segment: AssembledSegment {
            segment: segment.index,
            native_rate: OPUS_RATE,
            native_samples: proc_samples * (OPUS_RATE / PROC_RATE) as u64,
            proc_samples,
        },
        echo,
    })
}

enum Decoder {
    Mono(opus::Decoder),
    Stereo(opus_ms::Decoder),
}

/// Reads an archive this stage wrote: mapping family 0 with one channel, or
/// mapping family 1 with two uncoupled streams (KTD7). It gives 48 kHz
/// interleaved samples, trimmed as a player would (RFC 7845: pre-skip at the
/// start, the last granule at the end).
pub struct ArchiveReader {
    packets: ogg::PacketReader<BufReader<File>>,
    decoder: Decoder,
    channels: u16,
    pre_skip: u64,
    /// 48 kHz samples of each channel decoded so far, pre-skip included.
    decoded: u64,
    buf: Vec<f32>,
    path: PathBuf,
}

impl ArchiveReader {
    pub fn open(archive: &Path) -> Result<Self, StageError> {
        let bad = |why: &str| StageError::Failed(format!("{}: {why}", archive.display()));
        let mut packets = ogg::PacketReader::new(BufReader::new(File::open(archive)?));
        let head = packets
            .read_packet()
            .map_err(|e| bad(&e.to_string()))?
            .ok_or_else(|| bad("empty"))?
            .data;
        if head.len() < 19 || &head[..8] != b"OpusHead" {
            return Err(bad("not an Opus stream"));
        }
        let (channels, family) = (head[9], head[18]);
        let decoder = match (channels, family) {
            (1, 0) => Decoder::Mono(
                opus::Decoder::new(OPUS_RATE, opus::Channels::Mono)
                    .map_err(|e| bad(&format!("opus decoder: {e}")))?,
            ),
            (2, 1)
                if head.len() >= 23 && head[19] == 2 && head[20] == 0 && head[21..23] == [0, 1] =>
            {
                Decoder::Stereo(
                    opus_ms::Decoder::new(OPUS_RATE, 2)
                        .map_err(|e| bad(&format!("opus decoder: {e}")))?,
                )
            }
            _ => {
                return Err(bad(&format!(
                    "{channels} channels in mapping family {family}, which Longhand does not write"
                )));
            }
        };
        let pre_skip = u16::from_le_bytes([head[10], head[11]]) as u64;
        packets
            .read_packet()
            .map_err(|e| bad(&e.to_string()))?
            .ok_or_else(|| bad("no tags"))?;
        Ok(Self {
            packets,
            decoder,
            channels: u16::from(channels),
            pre_skip,
            decoded: 0,
            buf: vec![0f32; 5_760 * usize::from(channels)],
            path: archive.to_path_buf(),
        })
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Decode the next packet and add its samples to `out`. Returns false
    /// at the end of the stream.
    pub fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, StageError> {
        let bad = |why: &str| StageError::Failed(format!("{}: {why}", self.path.display()));
        let Some(packet) = self
            .packets
            .read_packet()
            .map_err(|e| bad(&e.to_string()))?
        else {
            return Ok(false);
        };
        let n = match &mut self.decoder {
            Decoder::Mono(decoder) => decoder
                .decode_float(&packet.data, &mut self.buf, false)
                .map_err(|e| bad(&format!("opus decoder: {e}")))?,
            Decoder::Stereo(decoder) => decoder
                .decode(&packet.data, &mut self.buf)
                .map_err(|e| bad(&format!("opus decoder: {e}")))?,
        } as u64;
        let end = if packet.last_in_stream() {
            (self.decoded + n).min(packet.absgp_page())
        } else {
            self.decoded + n
        };
        let from = self.pre_skip.max(self.decoded).min(end);
        let width = self.channels as usize;
        out.extend_from_slice(
            &self.buf
                [(from - self.decoded) as usize * width..(end - self.decoded) as usize * width],
        );
        self.decoded += n;
        Ok(true)
    }
}

fn ms_to_samples(ms: u64, rate: u32) -> u64 {
    (ms * rate as u64 + 500) / 1000
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Removes the temporary files of one segment when it drops: after a
/// cancel or an error, and after the success, when the commits have renamed
/// the files that are kept. Declare it before the writers of these files,
/// so the writers drop first and close their handles.
struct TempFiles(Vec<PathBuf>);

impl Drop for TempFiles {
    fn drop(&mut self) {
        for path in &self.0 {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => log::warn!("removing {} failed: {e}", path.display()),
            }
        }
    }
}

/// Create `path` fresh, replacing whatever an interrupted run left there.
pub(crate) fn create_fresh(path: &Path) -> std::io::Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// Make a finished file durable under its real name.
fn commit(temp: &Path, path: &Path) -> std::io::Result<()> {
    crate::sync_file(temp)?;
    std::fs::rename(temp, path)?;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// 16 kHz mono IEEE-float WAV.
struct ProcWriter {
    writer: hound::WavWriter<BufWriter<File>>,
    samples: u64,
}

impl ProcWriter {
    fn create(path: &Path) -> Result<Self, StageError> {
        let file = create_fresh(path)?;
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: PROC_RATE,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        Ok(Self {
            writer: hound::WavWriter::new(BufWriter::new(file), spec)?,
            samples: 0,
        })
    }

    fn push(&mut self, samples: &[f32]) -> Result<(), StageError> {
        for &s in samples {
            self.writer.write_sample(s)?;
        }
        self.samples += samples.len() as u64;
        Ok(())
    }

    fn finish(self) -> Result<u64, StageError> {
        self.writer.finalize()?;
        Ok(self.samples)
    }
}

enum Encoder {
    Mono(opus::Encoder),
    Stereo(opus_ms::Encoder),
}

/// Opus in Ogg (RFC 7845), streamed a 20 ms frame at a time: mono with
/// mapping family 0, or stereo with mapping family 1 and one uncoupled
/// stream for each channel (KTD7).
struct OpusWriter {
    encoder: Encoder,
    channels: usize,
    packets: PacketWriter<'static, BufWriter<File>>,
    serial: u32,
    pre_skip: u64,
    /// Interleaved samples of the frame that is not full yet.
    frame: Vec<f32>,
    /// Real (unpadded) 48 kHz samples of each channel encoded.
    samples: u64,
    /// The newest packet, held back so the last one can close the stream.
    held: Option<Vec<u8>>,
    packets_written: u64,
    buf: Vec<u8>,
}

impl OpusWriter {
    fn create(path: &Path, input_rate: u32, channels: u16) -> Result<Self, StageError> {
        let failed = |e: String| StageError::Failed(format!("opus encoder: {e}"));
        let (encoder, pre_skip) = match channels {
            1 => {
                let opus_error = |e: opus::Error| failed(e.to_string());
                let mut encoder =
                    opus::Encoder::new(OPUS_RATE, opus::Channels::Mono, opus::Application::Audio)
                        .map_err(opus_error)?;
                encoder
                    .set_bitrate(opus::Bitrate::Bits(OPUS_BITRATE))
                    .map_err(opus_error)?;
                let pre_skip = encoder.get_lookahead().map_err(opus_error)?.max(0) as u64;
                (Encoder::Mono(encoder), pre_skip)
            }
            _ => {
                let mut encoder =
                    opus_ms::Encoder::new(OPUS_RATE, 2, OPUS_BITRATE).map_err(failed)?;
                let pre_skip = u64::from(encoder.lookahead().map_err(failed)?);
                (Encoder::Stereo(encoder), pre_skip)
            }
        };
        let mut packets = PacketWriter::new(BufWriter::new(create_fresh(path)?));
        // A fixed serial: one logical stream per file, and assembly stays
        // byte-for-byte reproducible.
        let serial = 0x4c48_0001;

        let mut head = Vec::with_capacity(23);
        head.extend_from_slice(b"OpusHead");
        head.push(1); // version
        head.push(channels as u8);
        head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
        head.extend_from_slice(&input_rate.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes()); // output gain
        if channels == 1 {
            head.push(0); // mapping family: mono/stereo
        } else {
            // Mapping family 1 (RFC 7845 §5.1.1): two streams, none
            // coupled; the left channel is stream 0 and the right stream 1.
            head.extend_from_slice(&[1, 2, 0, 0, 1]);
        }
        packets.write_packet(head, serial, PacketWriteEndInfo::EndPage, 0)?;

        let vendor = b"Longhand";
        let mut tags = Vec::new();
        tags.extend_from_slice(b"OpusTags");
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes()); // no comments
        packets.write_packet(tags, serial, PacketWriteEndInfo::EndPage, 0)?;

        let channels = usize::from(channels);
        Ok(Self {
            encoder,
            channels,
            packets,
            serial,
            pre_skip,
            frame: Vec::with_capacity(OPUS_FRAME * channels),
            samples: 0,
            held: None,
            packets_written: 0,
            buf: vec![0; 4_000 * channels],
        })
    }

    /// Take interleaved 48 kHz samples.
    fn push(&mut self, mut samples: &[f32]) -> Result<(), StageError> {
        let full = OPUS_FRAME * self.channels;
        while !samples.is_empty() {
            let take = (full - self.frame.len()).min(samples.len());
            self.frame.extend_from_slice(&samples[..take]);
            samples = &samples[take..];
            if self.frame.len() == full {
                self.samples += OPUS_FRAME as u64;
                self.encode_frame()?;
            }
        }
        Ok(())
    }

    fn encode_frame(&mut self) -> Result<(), StageError> {
        let len = match &mut self.encoder {
            Encoder::Mono(encoder) => encoder
                .encode_float(&self.frame, &mut self.buf)
                .map_err(|e| e.to_string()),
            Encoder::Stereo(encoder) => encoder.encode(&self.frame, &mut self.buf),
        }
        .map_err(|e| StageError::Failed(format!("opus encoder: {e}")))?;
        self.frame.clear();
        if let Some(previous) = self.held.replace(self.buf[..len].to_vec()) {
            self.write(previous, false)?;
        }
        Ok(())
    }

    fn write(&mut self, packet: Vec<u8>, last: bool) -> Result<(), StageError> {
        self.packets_written += 1;
        let end = if last {
            PacketWriteEndInfo::EndStream
        } else if self.packets_written.is_multiple_of(PACKETS_PER_PAGE) {
            PacketWriteEndInfo::EndPage
        } else {
            PacketWriteEndInfo::NormalPacket
        };
        // A page's granule is the 48 kHz samples decoded through its last
        // packet, pre-skip included; the final one stops at the real end so
        // a decoder trims the padding (RFC 7845 §4).
        let granule = if last {
            self.pre_skip + self.samples
        } else {
            self.packets_written * OPUS_FRAME as u64
        };
        self.packets
            .write_packet(packet, self.serial, end, granule)?;
        Ok(())
    }

    fn finish(mut self) -> Result<(), StageError> {
        // The decoder's output runs `pre_skip` samples behind the input, so
        // frames of silence past the end carry the last real samples out; the
        // final granule trims them again (RFC 7845 §4).
        let full = OPUS_FRAME * self.channels;
        let real = self.samples + (self.frame.len() / self.channels) as u64;
        let needed = (real + self.pre_skip).div_ceil(OPUS_FRAME as u64);
        while self.packets_written + u64::from(self.held.is_some()) < needed {
            self.frame.resize(full, 0.0);
            self.encode_frame()?;
        }
        self.samples = real;
        match self.held.take() {
            Some(last) => self.write(last, true)?,
            None => {
                // No audio at all: close the stream with one frame of silence.
                self.frame.resize(full, 0.0);
                self.encode_frame()?;
                let last = self.held.take().expect("just encoded");
                self.write(last, true)?;
            }
        }
        self.packets.inner_mut().flush()?;
        Ok(())
    }
}

/// Read the 16 kHz processing file of a segment.
pub fn open_proc(
    session_dir: &Path,
    segment: &Segment,
) -> Result<hound::WavReader<BufReader<File>>, StageError> {
    let path = session_dir.join(proc_file(&segment.audio_file));
    let reader = hound::WavReader::open(&path)?;
    let spec = reader.spec();
    if spec.sample_rate != PROC_RATE
        || spec.channels != 1
        || spec.sample_format != hound::SampleFormat::Float
    {
        return Err(StageError::Failed(format!(
            "{} is not 16 kHz mono float",
            path.display()
        )));
    }
    Ok(reader)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AudioProvenance {
    archive: String,
    processing: String,
    echo: String,
}
fn provenance_key(index: u32) -> String {
    format!("audio-provenance-{index:08}")
}
fn archive_path(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segment: &Segment,
) -> Result<PathBuf, StageError> {
    let absent = || StageError::Failed("recorded audio is absent".into());
    // A name that can point outside the folder counts as absent audio.
    let local = crate::audio_path(dir, &segment.audio_file).ok_or_else(absent)?;
    if local.is_file() {
        return Ok(local);
    }
    crate::lock(store)
        .session(id)?
        .and_then(|r| r.home_path)
        .and_then(|home| crate::audio_path(Path::new(&home), &segment.audio_file))
        .ok_or_else(absent)
}

/// In-process change detection only. Exact content checks remain necessary at
/// the start of each attempt and before publication.
pub(crate) fn recognition_input_stamp(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segments: &[Segment],
) -> Result<String, StageError> {
    let mut values = Vec::new();
    for segment in segments {
        let mut files = Vec::new();
        for path in [
            archive_path(store, id, dir, segment)?,
            dir.join(proc_file(&segment.audio_file)),
        ] {
            let metadata = std::fs::metadata(&path)?;
            files.push((
                path,
                metadata.len(),
                metadata.modified()?,
                metadata.created().ok(),
            ));
        }
        let provenance: Option<AudioProvenance> =
            checkpoint::get(store, id, Stage::Assemble, &provenance_key(segment.index))?;
        let echo = saved_echo(store, id, segment)?;
        values.push((segment, files, provenance, echo));
    }
    Ok(session_core::recognition::digest(&serde_json::to_vec(
        &values,
    )?))
}
fn audio_provenance(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segment: &Segment,
) -> Result<AudioProvenance, StageError> {
    audio_provenance_at(store, id, dir, segment, None)
}
fn audio_provenance_at(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segment: &Segment,
    source: Option<&Path>,
) -> Result<AudioProvenance, StageError> {
    use session_core::recognition::file_digest;
    let archive = source
        .map(Path::to_path_buf)
        .map(Ok)
        .unwrap_or_else(|| archive_path(store, id, dir, segment))?;
    let echo: Option<serde_json::Value> = checkpoint::get(
        store,
        id,
        Stage::Assemble,
        &checkpoint::echo_key(segment.index),
    )?;
    Ok(AudioProvenance {
        archive: file_digest(&archive)?,
        processing: file_digest(&dir.join(proc_file(&segment.audio_file)))?,
        echo: echo_digest(echo)?,
    })
}
/// The echo spans checkpoint of `segment` as JSON, with the keys of each
/// object in sorted order (see [`echo_digest`]).
fn saved_echo(
    store: &SharedStore,
    id: &str,
    segment: &Segment,
) -> Result<Option<serde_json::Value>, StageError> {
    let echo: Option<serde_json::Value> = checkpoint::get(
        store,
        id,
        Stage::Assemble,
        &checkpoint::echo_key(segment.index),
    )?;
    Ok(echo.map(session_core::json::sorted))
}
/// The digest of a saved echo spans checkpoint. The keys of each object are
/// put in sorted order first, so the digest does not change between builds:
/// the saved provenance of a note that an earlier build assembled holds the
/// digest of the sorted keys, also when a dependency enables the
/// `preserve_order` feature of `serde_json`.
fn echo_digest(echo: Option<serde_json::Value>) -> Result<String, StageError> {
    let echo = echo.map(session_core::json::sorted);
    Ok(session_core::recognition::digest(&serde_json::to_vec(
        &echo,
    )?))
}
/// A completed Assemble checkpoint alone does not bind an archive to its
/// processing file. Missing provenance requires a decode before exact reuse.
pub fn provenance_valid(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segments: &[Segment],
) -> Result<bool, StageError> {
    for segment in segments {
        let saved: Option<AudioProvenance> =
            checkpoint::get(store, id, Stage::Assemble, &provenance_key(segment.index))?;
        let Some(saved) = saved else {
            return Ok(false);
        };
        let Ok(archive) = archive_path(store, id, dir, segment) else {
            return Ok(false);
        };
        if !archive.is_file() || !dir.join(proc_file(&segment.audio_file)).is_file() {
            return Ok(false);
        }
        let current = audio_provenance(store, id, dir, segment)?;
        if current != saved {
            return Ok(false);
        }
    }
    Ok(true)
}
/// Return the source identities after their processing relationship is checked.
pub fn recognition_audio_identity(
    store: &SharedStore,
    id: &str,
    dir: &Path,
    segments: &[Segment],
) -> Result<String, StageError> {
    if !provenance_valid(store, id, dir, segments)? {
        return Err(StageError::Failed("Recorded audio or processing audio changed, or its provenance is absent. Rebuild the processing audio before recognition.".into()));
    }
    let mut values = Vec::new();
    for segment in segments {
        let saved: AudioProvenance =
            checkpoint::get(store, id, Stage::Assemble, &provenance_key(segment.index))?
                .ok_or_else(|| StageError::Failed("audio provenance is absent".into()))?;
        values.push((
            segment.index,
            segment.offset_ms,
            segment.duration_ms,
            segment.channels,
            saved,
        ));
    }
    Ok(session_core::recognition::digest(&serde_json::to_vec(
        &values,
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_echo_digest_is_the_digest_of_the_sorted_keys_of_the_saved_checkpoint() {
        // `checkpoint::put` writes the fields in declaration order. A build
        // without the `preserve_order` feature of `serde_json` made the digest
        // from the keys in sorted order.
        let stored = serde_json::to_string(&EchoSpans {
            segment: 2,
            spans: vec![asr::Span {
                start_ms: 10,
                end_ms: 20,
            }],
            mic_gain: 1.0,
            system_gain: 0.5,
        })
        .unwrap();
        assert!(stored.starts_with(r#"{"segment":"#), "{stored}");
        let saved: serde_json::Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(
            echo_digest(Some(saved)).unwrap(),
            session_core::recognition::digest(
                br#"{"mic_gain":1.0,"segment":2,"spans":[{"end_ms":20,"start_ms":10}],"system_gain":0.5}"#
            )
        );
        assert_eq!(
            echo_digest(None).unwrap(),
            session_core::recognition::digest(b"null")
        );
    }
}
